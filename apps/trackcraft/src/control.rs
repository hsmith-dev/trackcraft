//! Loopback JSON-lines control server (`docs/control-protocol.md`): one request per line, one
//! reply per line. Requests reach the UI thread as [`ControlRequest`]s; the app answers them in
//! `logic` through `TrackApp::control`.
//!
//! Every request carries the instance's key, a random 256-bit hex string the app writes to
//! `<config dir>/trackcraft/control-key.<port>` (mode 0600 on Unix) and removes on exit.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value, json};

/// Longer request lines are refused (and the connection closed).
pub const MAX_LINE: usize = 16 << 20;

/// Before its first authorised request, a connection waits at most this long for each read.
const FIRST_REQUEST: Duration = Duration::from_secs(30);

/// How long a connection waits for the UI thread to answer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// More simultaneous connections are refused.
const MAX_CONNECTIONS: usize = 16;

/// A request on its way to the UI thread; send the reply (without `id`) on `reply`.
pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Value>,
}

/// The reply for a `TrackApp::control`-style result.
pub fn reply_for(r: Result<Value, String>) -> Value {
    match r {
        Ok(v) => json!({"ok": true, "result": v}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// The key file of a running server; removed by [`KeyFile::remove`] and on drop.
pub struct KeyFile(PathBuf);

impl KeyFile {
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Drop for KeyFile {
    fn drop(&mut self) {
        self.remove();
    }
}

/// A bound listener with its key, ready to [`Server::serve`] once the UI exists.
pub struct Server {
    listener: TcpListener,
    key: String,
    pub key_file: KeyFile,
}

/// Bind `127.0.0.1:port` and write the key file to `key_path`.
pub fn bind(port: u16, key_path: &Path) -> Result<Server, String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    let key = random_key()?;
    write_key_file(key_path, &key)?;
    Ok(Server { listener, key, key_file: KeyFile(key_path.to_path_buf()) })
}

impl Server {
    /// Serve on background threads. `wake` runs whenever a request is queued (the app passes
    /// `ctx.request_repaint`). Returns the queue and the key file (keep it alive while serving).
    pub fn serve(self, wake: impl Fn() + Send + Sync + 'static) -> (Receiver<ControlRequest>, KeyFile) {
        (serve_on(self.listener, self.key, Arc::new(wake)), self.key_file)
    }
}

/// A new key: 32 random bytes from the operating system, as hex.
pub fn random_key() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| format!("no random key: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Write the key file, replacing a stale one; on Unix it is created with mode 0600.
pub fn write_key_file(p: &Path, key: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let _ = std::fs::remove_file(p);
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut f = open.open(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?;
    f.write_all(key.as_bytes()).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

/// Constant-time comparison; an empty key never matches.
pub fn keys_match(given: &str, key: &str) -> bool {
    let (a, b) = (given.as_bytes(), key.as_bytes());
    !b.is_empty() && a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A parsed, authorised request line.
#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

/// What a line comes to: a request to run, or a reply to send before closing the connection.
#[derive(Debug, PartialEq)]
pub enum Line {
    Request(Parsed),
    /// Send this reply, then close.
    Refuse(Value),
}

/// Parse and authorise one request line (already trimmed, not empty).
pub fn parse_line(line: &str, key: &str) -> Line {
    let msg = match serde_json::from_str::<Value>(line) {
        Ok(m) => m,
        Err(e) => return Line::Refuse(json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")})),
    };
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let refuse = |e: &str| Line::Refuse(json!({"id": id.clone(), "ok": false, "error": e}));
    if !msg.is_object() {
        return refuse("a request is a JSON object");
    }
    let given = msg.get("key").and_then(Value::as_str).unwrap_or("");
    if !keys_match(given, key) {
        return refuse("unauthorized");
    }
    let Some(method) = msg.get("method").and_then(Value::as_str).filter(|m| !m.is_empty()) else {
        return refuse("missing \"method\"");
    };
    let params = match msg.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(p @ Value::Object(_)) => p.clone(),
        Some(_) => return refuse("\"params\" must be an object"),
    };
    Line::Request(Parsed { id, method: method.chars().take(200).collect(), params })
}

/// Adds the request id to a reply.
fn with_id(mut v: Value, id: &Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.insert("id".into(), id.clone());
    }
    v
}

/// The next line (trimmed): `None` at the end of the stream or on a read error.
fn read_line(r: &mut impl BufRead) -> Option<Result<String, &'static str>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf).ok()?;
    if n == 0 {
        return None;
    }
    if buf.len() > MAX_LINE {
        return Some(Err("line too long"));
    }
    Some(String::from_utf8(buf).map(|s| s.trim().to_string()).map_err(|_| "not UTF-8"))
}

fn serve_on(listener: TcpListener, key: String, wake: Arc<dyn Fn() + Send + Sync>) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let key: Arc<str> = key.into();
    let live = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            if live.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "too many connections"}));
                continue;
            }
            live.fetch_add(1, Ordering::SeqCst);
            let (tx, key, wake, live) = (tx.clone(), key.clone(), wake.clone(), live.clone());
            std::thread::spawn(move || {
                serve(stream, &tx, &key, wake.as_ref());
                live.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    rx
}

fn serve(stream: TcpStream, tx: &Sender<ControlRequest>, key: &str, wake: &(dyn Fn() + Send + Sync)) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    let _ = read.set_read_timeout(Some(FIRST_REQUEST));
    let mut reader = BufReader::new(read);
    let mut authorised = false;
    while let Some(line) = read_line(&mut reader) {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"id": null, "ok": false, "error": e}));
                break;
            }
        };
        if line.is_empty() {
            continue;
        }
        let req = match parse_line(&line, key) {
            Line::Request(r) => r,
            Line::Refuse(v) => {
                let _ = writeln!(out, "{v}");
                break;
            }
        };
        if !authorised {
            authorised = true;
            let _ = reader.get_ref().set_read_timeout(None);
        }
        let (rtx, rrx) = channel();
        if tx.send(ControlRequest { method: req.method, params: req.params, reply: rtx }).is_err() {
            let _ = writeln!(out, "{}", with_id(json!({"ok": false, "error": "the app is closing"}), &req.id));
            break;
        }
        wake();
        let r = rrx.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| json!({"ok": false, "error": "timeout: the app did not answer"}));
        if writeln!(out, "{}", with_id(r, &req.id)).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn refused(l: Line) -> String {
        match l {
            Line::Refuse(v) => v["error"].as_str().unwrap().to_string(),
            Line::Request(r) => panic!("accepted {r:?}"),
        }
    }

    #[test]
    fn keys_compare_exactly() {
        assert!(keys_match(KEY, KEY));
        assert!(!keys_match("", KEY));
        assert!(!keys_match("", ""));
        assert!(!keys_match(&KEY[1..], KEY));
        assert!(!keys_match(&format!("{KEY}0"), KEY));
        assert!(!keys_match(&KEY.replace('f', "e"), KEY));
    }

    #[test]
    fn random_keys_are_hex_and_differ() {
        let (a, b) = (random_key().unwrap(), random_key().unwrap());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn parses_good_requests() {
        let l = parse_line(&format!(r#"{{"id":7,"key":"{KEY}","method":"ui.palette"}}"#), KEY);
        assert_eq!(l, Line::Request(Parsed { id: json!(7), method: "ui.palette".into(), params: json!({}) }));
        let l = parse_line(&format!(r#"{{"id":"a","key":"{KEY}","method":"engine.execute","params":{{"command":"workspace.info"}}}}"#), KEY);
        let Line::Request(p) = l else { panic!() };
        assert_eq!(p.params["command"], "workspace.info");
        assert_eq!(p.id, json!("a"));
    }

    #[test]
    fn refuses_hostile_lines() {
        assert!(refused(parse_line("not json", KEY)).starts_with("bad JSON"));
        assert!(refused(parse_line("GET / HTTP/1.1", KEY)).starts_with("bad JSON"));
        assert_eq!(refused(parse_line("[1,2]", KEY)), "a request is a JSON object");
        assert_eq!(refused(parse_line("42", KEY)), "a request is a JSON object");
        assert_eq!(refused(parse_line(r#"{"method":"ui.palette"}"#, KEY)), "unauthorized");
        assert_eq!(refused(parse_line(r#"{"key":"nope","method":"ui.palette"}"#, KEY)), "unauthorized");
        assert_eq!(refused(parse_line(r#"{"key":5,"method":"ui.palette"}"#, KEY)), "unauthorized");
        assert_eq!(refused(parse_line(&format!(r#"{{"key":"{KEY}"}}"#), KEY)), "missing \"method\"");
        assert_eq!(refused(parse_line(&format!(r#"{{"key":"{KEY}","method":3}}"#), KEY)), "missing \"method\"");
        assert_eq!(refused(parse_line(&format!(r#"{{"key":"{KEY}","method":"x","params":[1]}}"#), KEY)), "\"params\" must be an object");
        // An empty server key accepts nothing.
        assert_eq!(refused(parse_line(r#"{"key":"","method":"ui.palette"}"#, "")), "unauthorized");
        // The id comes back on refusals.
        let Line::Refuse(v) = parse_line(r#"{"id":9,"method":"x"}"#, KEY) else { panic!() };
        assert_eq!(v["id"], 9);
        // Deep nesting is an error, not a stack overflow.
        let deep = "[".repeat(100_000);
        assert!(refused(parse_line(&deep, KEY)).starts_with("bad JSON"));
    }

    #[test]
    fn long_lines_and_bad_utf8_are_errors() {
        let mut ok = std::io::Cursor::new(b"  hi \n\nthere".to_vec());
        assert_eq!(read_line(&mut ok), Some(Ok("hi".into())));
        assert_eq!(read_line(&mut ok), Some(Ok("".into())));
        assert_eq!(read_line(&mut ok), Some(Ok("there".into())));
        assert_eq!(read_line(&mut ok), None);
        let mut bad = std::io::Cursor::new(vec![0xff, 0xfe, b'\n']);
        assert_eq!(read_line(&mut bad), Some(Err("not UTF-8")));
        let mut long = std::io::Cursor::new(vec![b'a'; MAX_LINE + 10]);
        assert_eq!(read_line(&mut long), Some(Err("line too long")));
    }

    #[test]
    fn key_file_is_private_and_removed() {
        let d = std::env::temp_dir().join(format!("trackcraft-key-{}", std::process::id()));
        let p = d.join("sub").join("control-key.1");
        write_key_file(&p, "old").unwrap();
        write_key_file(&p, KEY).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), KEY);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        drop(KeyFile(p.clone()));
        assert!(!p.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// End to end over a real socket (no window): the UI side is a thread answering requests.
    #[test]
    fn serves_over_loopback() {
        let d = std::env::temp_dir().join(format!("trackcraft-srv-{}", std::process::id()));
        let kp = d.join("control-key.test");
        let server = bind(0, &kp).unwrap();
        let port = server.listener.local_addr().unwrap().port();
        let key = std::fs::read_to_string(&kp).unwrap();
        let (rx, key_file) = server.serve(|| {});
        std::thread::spawn(move || {
            while let Ok(r) = rx.recv() {
                let _ = r.reply.send(reply_for(Ok(json!({"method": r.method, "params": r.params}))));
            }
        });
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut w = s;
        let mut recv = || {
            let mut l = String::new();
            r.read_line(&mut l).unwrap();
            serde_json::from_str::<Value>(&l).unwrap()
        };
        writeln!(w, "{}", json!({"id": 1, "key": key, "method": "ui.theme", "params": {"dark": true}})).unwrap();
        let v = recv();
        assert_eq!(v["id"], 1);
        assert_eq!(v["ok"], true);
        assert_eq!(v["result"]["method"], "ui.theme");
        assert_eq!(v["result"]["params"]["dark"], true);
        writeln!(w, "{}", json!({"id": 2, "key": "wrong", "method": "ui.theme"})).unwrap();
        let v = recv();
        assert_eq!(v["error"], "unauthorized");
        assert_eq!(v["id"], 2);
        drop(key_file);
        assert!(!kp.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
