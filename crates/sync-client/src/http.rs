//! A minimal blocking HTTP/1.1 client over `std::net::TcpStream`: `http://` only, one request per
//! connection (`Connection: close`), `Content-Length` and chunked bodies, every size capped.
//! Enough for the sync protocol's three calls; HTTPS goes through a TLS-terminating proxy for
//! now (see `docs/sync.md`, "Limits").

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Longest status line + headers accepted.
const MAX_HEAD: usize = 64 * 1024;
/// Longest chunk-size line.
const MAX_CHUNK_LINE: u64 = 1024;

/// Where a workspace lives: `http://host:port/prefix` plus `/w/{workspace}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    /// Path prefix without a trailing slash (`""`, `/track/w/team`).
    pub base: String,
}

impl Endpoint {
    /// Parses `http://host[:port][/prefix]` and appends `/w/{workspace}` when one is given.
    pub fn parse(url: &str, workspace: Option<&str>) -> Result<Self, String> {
        let url = url.trim();
        if url.len() > 2048 {
            return Err("the server URL is too long".into());
        }
        let rest = if let Some(r) = url.strip_prefix("http://") {
            r
        } else if url.starts_with("https://") {
            return Err("https:// is not supported yet: run a local TLS-terminating proxy and use http://".into());
        } else {
            return Err("the server URL must start with http://".into());
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (rest.get(..i).unwrap_or_default(), rest.get(i..).unwrap_or_default()),
            None => (rest, ""),
        };
        // `[v6]:port`, `[v6]`, `host:port`, `host`.
        let (host, port) = match authority.rfind(':') {
            Some(i) if authority.get(i..).is_some_and(|p| !p.contains(']')) => {
                let p = authority.get(i + 1..).unwrap_or_default();
                let port = p.parse::<u16>().map_err(|_| format!("bad port in server URL: '{}'", p.chars().take(20).collect::<String>()))?;
                (authority.get(..i).unwrap_or_default(), port)
            }
            _ => (authority, 80),
        };
        if host.starts_with('[') != host.ends_with(']') || (!host.starts_with('[') && host.contains(':')) {
            return Err("the server URL has a malformed host".into());
        }
        let host_ok =
            !host.is_empty() && host.len() <= 255 && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
        if !host_ok || port == 0 {
            return Err("the server URL needs a host name or address".into());
        }
        if !path.chars().all(|c| c.is_ascii_graphic()) || path.contains(['?', '#']) {
            return Err("the server URL path may not contain spaces, '?' or '#'".into());
        }
        let mut base = path.trim_end_matches('/').to_string();
        if let Some(w) = workspace {
            if w.is_empty() || w.len() > 64 || !w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
                return Err("workspace names are 1-64 of a-z, 0-9, - and _".into());
            }
            base.push_str("/w/");
            base.push_str(w);
        }
        Ok(Endpoint { host: host.to_string(), port, base })
    }

    pub fn display(&self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.base)
    }
}

/// A bearer token as sent in the header: printable ASCII only (no header injection).
pub fn valid_token(t: &str) -> bool {
    !t.is_empty() && t.len() <= 1024 && t.bytes().all(|b| b.is_ascii_graphic())
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    /// The server's error message (`{"error": …}` / Jira / Confluence shapes), or the status.
    pub fn error_message(&self) -> String {
        let v: serde_json::Value = serde_json::from_slice(&self.body).unwrap_or_default();
        let msg = v
            .get("error")
            .and_then(|e| e.as_str())
            .or_else(|| v.get("errorMessages").and_then(|m| m.get(0)).and_then(|m| m.as_str()))
            .map(str::to_string)
            .unwrap_or_else(|| format!("HTTP {}", self.status));
        msg.chars().take(300).collect()
    }
}

fn connect(ep: &Endpoint, timeout: Duration) -> Result<TcpStream, String> {
    let host = ep.host.trim_start_matches('[').trim_end_matches(']');
    let addrs = (host, ep.port).to_socket_addrs().map_err(|e| format!("{}: {e}", ep.host))?;
    let mut last = format!("{}: no address", ep.host);
    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(s) => {
                s.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
                s.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = format!("{}:{}: {e}", ep.host, ep.port),
        }
    }
    Err(last)
}

fn send_head(s: &mut TcpStream, ep: &Endpoint, method: &str, path: &str, token: &str, body: Option<&[u8]>, accept: &str) -> Result<(), String> {
    let mut head = format!(
        "{method} {}{path} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {token}\r\nAccept: {accept}\r\nConnection: close\r\nUser-Agent: trackcraft-sync/{}\r\n",
        ep.base,
        ep.host,
        ep.port,
        env!("CARGO_PKG_VERSION")
    );
    if let Some(b) = body {
        head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
    if let Some(b) = body {
        s.write_all(b).map_err(|e| e.to_string())?;
    }
    s.flush().map_err(|e| e.to_string())
}

/// How the body is framed.
enum Framing {
    Length(u64),
    Chunked,
    UntilClose,
}

fn read_head<R: BufRead>(r: &mut R) -> Result<(u16, Framing), String> {
    let mut total = 0usize;
    let mut status = None;
    let mut framing = Framing::UntilClose;
    loop {
        let mut line = Vec::new();
        let n = r.by_ref().take((MAX_HEAD.saturating_sub(total)) as u64 + 1).read_until(b'\n', &mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("the server closed the connection".into());
        }
        total = total.saturating_add(n);
        if total > MAX_HEAD {
            return Err("the response headers are too large".into());
        }
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end_matches(['\r', '\n']);
        if status.is_none() {
            let code = line.split(' ').nth(1).and_then(|c| c.parse::<u16>().ok()).filter(|_| line.starts_with("HTTP/1."));
            status = Some(code.ok_or_else(|| format!("not an HTTP response: '{}'", line.chars().take(60).collect::<String>()))?);
            continue;
        }
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "content-length" {
                framing = Framing::Length(v.parse().map_err(|_| "bad Content-Length")?);
            } else if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
                framing = Framing::Chunked;
            }
        }
    }
    Ok((status.unwrap_or(0), framing))
}

/// Decodes a chunked body as a plain reader.
pub struct Chunked<R> {
    inner: R,
    left: u64,
    done: bool,
}

impl<R: BufRead> Chunked<R> {
    pub fn new(inner: R) -> Self {
        Chunked { inner, left: 0, done: false }
    }

    fn line(&mut self) -> std::io::Result<String> {
        let mut v = Vec::new();
        self.inner.by_ref().take(MAX_CHUNK_LINE).read_until(b'\n', &mut v)?;
        if !v.ends_with(b"\n") {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk framing"));
        }
        Ok(String::from_utf8_lossy(&v).trim_end_matches(['\r', '\n']).to_string())
    }
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let line = self.line()?;
            let hex = line.split(';').next().unwrap_or_default().trim();
            let size = u64::from_str_radix(hex, 16).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size"))?;
            if size == 0 {
                // Trailers, up to the blank line.
                while !self.line()?.is_empty() {}
                self.done = true;
                return Ok(0);
            }
            self.left = size;
        }
        let want = usize::try_from(self.left).unwrap_or(usize::MAX).min(buf.len());
        let n = self.inner.read(buf.get_mut(..want).unwrap_or_default())?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the body ended inside a chunk"));
        }
        self.left = self.left.saturating_sub(n as u64);
        if self.left == 0 && !self.line()?.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "missing CRLF after a chunk"));
        }
        Ok(n)
    }
}

fn body_reader(r: BufReader<TcpStream>, framing: Framing) -> Box<dyn Read + Send> {
    match framing {
        Framing::Length(n) => Box::new(r.take(n)),
        Framing::Chunked => Box::new(Chunked::new(r)),
        Framing::UntilClose => Box::new(r),
    }
}

/// One request; the body is read whole, up to `max_body` bytes.
pub fn request(
    ep: &Endpoint,
    method: &str,
    path: &str,
    token: &str,
    body: Option<&[u8]>,
    timeout: Duration,
    max_body: usize,
) -> Result<Response, String> {
    let mut s = connect(ep, timeout)?;
    send_head(&mut s, ep, method, path, token, body, "application/json")?;
    let mut r = BufReader::new(s);
    let (status, framing) = read_head(&mut r)?;
    let mut out = Vec::new();
    body_reader(r, framing).take(max_body as u64 + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() > max_body {
        return Err(format!("the response is larger than {max_body} bytes"));
    }
    Ok(Response { status, body: out })
}

/// An open streaming response: a line reader over the decoded body, and a handle that
/// [`Stream::closer`] can shut down from another thread to interrupt a blocked read.
pub struct Stream {
    pub status: u16,
    pub reader: BufReader<Box<dyn Read + Send>>,
    pub handle: TcpStream,
}

impl Stream {
    /// Reads one line (without its ending), at most `max` bytes. `Ok(None)` = end of stream.
    pub fn line(&mut self, max: usize) -> Result<Option<String>, String> {
        let mut v = Vec::new();
        let n = self.reader.by_ref().take(max as u64 + 1).read_until(b'\n', &mut v).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        if v.len() > max {
            return Err("an event line is too long".into());
        }
        let s = String::from_utf8(v).map_err(|_| "an event line is not UTF-8".to_string())?;
        Ok(Some(s.trim_end_matches(['\n', '\r']).to_string()))
    }
}

/// Opens a long-lived GET (Server-Sent Events). `read_timeout` bounds the silence between
/// bytes (the server sends keep-alives).
pub fn open_stream(ep: &Endpoint, path: &str, token: &str, connect_timeout: Duration, read_timeout: Duration) -> Result<Stream, String> {
    let mut s = connect(ep, connect_timeout)?;
    send_head(&mut s, ep, "GET", path, token, None, "text/event-stream")?;
    s.set_read_timeout(Some(read_timeout)).map_err(|e| e.to_string())?;
    let handle = s.try_clone().map_err(|e| e.to_string())?;
    let mut r = BufReader::new(s);
    let (status, framing) = read_head(&mut r)?;
    Ok(Stream { status, reader: BufReader::new(body_reader(r, framing)), handle })
}

/// Interrupts a blocked read on `s` (from another thread).
pub fn close(s: &TcpStream) {
    let _ = s.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints() {
        let e = Endpoint::parse("http://host:8080", Some("team")).unwrap();
        assert_eq!((e.host.as_str(), e.port, e.base.as_str()), ("host", 8080, "/w/team"));
        let e = Endpoint::parse("http://10.0.0.1/track/", None).unwrap();
        assert_eq!((e.port, e.base.as_str()), (80, "/track"));
        assert_eq!(Endpoint::parse("http://[::1]:9", None).unwrap().host, "[::1]");
        for bad in ["host:80", "https://x", "http://", "http://h:0", "http://h:99999", "http://h/a b", "http://h/?q", "ftp://h"] {
            assert!(Endpoint::parse(bad, None).is_err(), "{bad}");
        }
        assert!(Endpoint::parse("http://h", Some("Team")).is_err());
        assert!(Endpoint::parse("http://h", Some("../x")).is_err());
        assert!(valid_token("tct_abc") && !valid_token("a\r\nX: y") && !valid_token(""));
    }

    #[test]
    fn chunked_bodies() {
        let raw = b"5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nTrailer: x\r\n\r\n";
        let mut out = String::new();
        Chunked::new(&raw[..]).read_to_string(&mut out).unwrap();
        assert_eq!(out, "hello world");
        for bad in [&b"zz\r\nhello"[..], b"5\r\nhel", b"5\r\nhelloXX0\r\n\r\n", b"ffffffffffffffffff\r\n"] {
            let mut s = String::new();
            assert!(Chunked::new(bad).read_to_string(&mut s).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn heads() {
        let mut r = &b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc"[..];
        let (s, f) = read_head(&mut r).unwrap();
        assert!(s == 200 && matches!(f, Framing::Length(3)));
        let mut r = &b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n"[..];
        assert!(matches!(read_head(&mut r).unwrap().1, Framing::Chunked));
        for bad in [&b""[..], b"SSH-2.0\r\n\r\n", b"HTTP/1.1 abc\r\n\r\n", b"HTTP/1.1 200 OK\r\nContent-Length: x\r\n\r\n"] {
            let mut r = bad;
            assert!(read_head(&mut r).is_err());
        }
        let huge = format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD));
        assert!(read_head(&mut huge.as_bytes()).is_err());
    }
}
