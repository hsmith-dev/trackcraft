//! `trackcraft-cli`: headless TrackCraft — run commands, search with JQL, serve MCP.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use trackcraft_engine::{Session, catalog, commands};

/// The desktop app's config paths (one definition, so both agree on the key file).
#[allow(dead_code)]
#[path = "../../trackcraft/src/paths.rs"]
mod paths;

const USAGE: &str = "\
usage: trackcraft-cli <command> [options]

commands:
  commands                         list every engine command (id, location, shortcut, params)
  run [--in F | --sample] [--as USER] --cmd 'id={json}' … [--out F]
                                   run commands in order and print each result as JSON
  jql [--in F | --sample] [--as USER] '<query>'
                                   search issues and print KEY  STATUS  SUMMARY lines
  new <file.track> [--sample]      create a workspace file (empty, or the sample)
  import --into F (--jira-csv EXPORT.csv | --jira-json PAGE.json | --confluence DIR|ZIP|PAGE [--space KEY]) [--as USER]
                                   import a Jira or Confluence export into F (created if missing),
                                   save it atomically and print the report (see docs/import.md)
  mcp [--in F | --sample] [--as USER]
                                   serve the MCP protocol on stdio (saves to --in after each change)
  send <port> <method> ['{json}']  send one control request to a running desktop app
                                   (`trackcraft --control <port>`) and print the reply;
                                   the key comes from TRACKCRAFT_CONTROL_KEY or the key file
  parity                           print Jira/Confluence feature coverage
  version
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match real_main(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("trackcraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Opts {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    sample: bool,
    user: Option<String>,
    cmds: Vec<String>,
    rest: Vec<String>,
}

fn opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { input: None, output: None, sample: false, user: None, cmds: vec![], rest: vec![] };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--in" => o.input = Some(PathBuf::from(val("--in")?)),
            "--out" => o.output = Some(PathBuf::from(val("--out")?)),
            "--as" => o.user = Some(val("--as")?),
            "--cmd" => o.cmds.push(val("--cmd")?),
            "--sample" => o.sample = true,
            _ => o.rest.push(a.clone()),
        }
    }
    Ok(o)
}

fn session(o: &Opts) -> Result<Session, String> {
    let mut s = match &o.input {
        Some(p) => {
            let ws = trackcraft_store::load(p).map_err(|e| e.to_string())?;
            let user = ws.users.keys().next().cloned().unwrap_or_default();
            let mut s = Session::new(ws, &user);
            s.path = Some(p.clone());
            s
        }
        None if o.sample => Session::sample(),
        None => return Err("pass --in <file.track> or --sample".into()),
    };
    if let Some(u) = &o.user {
        s.run("user.switch", &json!({"id": u}))?;
    }
    Ok(s)
}

/// `id={json}` or `id` alone.
fn split_cmd(c: &str) -> Result<(&str, Value), String> {
    match c.split_once('=') {
        Some((id, p)) => Ok((id.trim(), serde_json::from_str(p).map_err(|e| format!("--cmd {id}: params are not JSON: {e}"))?)),
        None => Ok((c.trim(), json!({}))),
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn real_main(args: &[String]) -> Result<(), String> {
    let Some(sub) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };
    let o = opts(args.get(1..).unwrap_or_default())?;
    match sub.as_str() {
        "commands" => {
            for c in commands() {
                println!("{:<22} {:<22} {:<12} {}", c.id, c.location, c.shortcut, c.params);
            }
        }
        "run" => {
            let mut s = session(&o)?;
            if o.cmds.is_empty() {
                return Err("nothing to run: pass --cmd 'id={json}'".into());
            }
            for c in &o.cmds {
                let (id, p) = split_cmd(c)?;
                let r = s.run(id, &p).map_err(|e| format!("{id}: {e}"))?;
                println!("{}", pretty(&r));
            }
            if let Some(out) = &o.output {
                trackcraft_store::save(&s.ws, out).map_err(|e| e.to_string())?;
            }
        }
        "jql" => {
            let mut s = session(&o)?;
            let q = o.rest.join(" ");
            let r = s.run("search.jql", &json!({"jql": q, "maxResults": 1000}))?;
            for i in r["issues"].as_array().into_iter().flatten() {
                println!(
                    "{:<12} {:<12} {}",
                    i["key"].as_str().unwrap_or_default(),
                    i["status"].as_str().unwrap_or_default(),
                    i["summary"].as_str().unwrap_or_default()
                );
            }
            eprintln!("{} issue(s)", r["total"]);
        }
        "new" => {
            let path = o.rest.first().ok_or("new: which file?")?;
            let ws = if o.sample { Session::sample().ws } else { trackcraft_engine::model::Workspace::default() };
            trackcraft_store::save(&ws, std::path::Path::new(path)).map_err(|e| e.to_string())?;
            println!("{path}");
        }
        "import" => import(&o)?,
        "mcp" => {
            let s = session(&o)?;
            let mut server = trackcraft_mcp::Server::new(s);
            if let Some(path) = o.input.clone() {
                server.after_change = Some(Box::new(move |s: &Session| {
                    if let Err(e) = trackcraft_store::save(&s.ws, &path) {
                        eprintln!("trackcraft-cli: autosave failed: {e}");
                    }
                }));
            }
            server.serve_stdio().map_err(|e| e.to_string())?;
        }
        "send" => {
            let [port, method, rest @ ..] = o.rest.as_slice() else {
                return Err("usage: send <port> <method> ['{json params}']".into());
            };
            let params = match rest {
                [] => json!({}),
                [p] => serde_json::from_str(p).map_err(|e| format!("send: params are not JSON: {e}"))?,
                _ => return Err("send: pass the params as one JSON argument".into()),
            };
            let reply = send(port, method, params)?;
            println!("{}", pretty(&reply));
            if reply.get("ok") != Some(&Value::Bool(true)) {
                return Err(reply.get("error").and_then(Value::as_str).unwrap_or("request failed").to_string());
            }
        }
        "parity" => {
            let (done, total) = catalog::coverage();
            let pct = if total == 0 { 0.0 } else { done as f64 * 100.0 / total as f64 };
            println!("{done} / {total} catalog features ({pct:.1}%)");
        }
        "version" | "--version" | "-V" => println!("trackcraft-cli {}", env!("CARGO_PKG_VERSION")),
        "help" | "--help" | "-h" => print!("{USAGE}"),
        other => return Err(format!("unknown command '{other}'\n\n{USAGE}")),
    }
    Ok(())
}

/// `import --into F (--jira-csv P | --jira-json P | --confluence P [--space KEY]) [--as USER]`.
fn import(o: &Opts) -> Result<(), String> {
    let mut into: Option<PathBuf> = None;
    let mut source: Option<(trackcraft_import::Kind, PathBuf)> = None;
    let mut space: Option<String> = None;
    let mut it = o.rest.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        let kind = match a.as_str() {
            "--into" => {
                into = Some(PathBuf::from(val("--into")?));
                continue;
            }
            "--space" => {
                space = Some(val("--space")?);
                continue;
            }
            "--jira-csv" => trackcraft_import::Kind::JiraCsv,
            "--jira-json" => trackcraft_import::Kind::JiraJson,
            "--confluence" => trackcraft_import::Kind::Confluence,
            other => return Err(format!("import: unknown option '{other}'")),
        };
        if source.is_some() {
            return Err("import: pass one of --jira-csv, --jira-json or --confluence".into());
        }
        source = Some((kind, PathBuf::from(val(a)?)));
    }
    let into = into.or_else(|| o.input.clone()).ok_or("import: pass --into <file.track>")?;
    let (kind, path) = source.ok_or("import: pass --jira-csv, --jira-json or --confluence <path>")?;
    if space.is_some() && kind != trackcraft_import::Kind::Confluence {
        return Err("import: --space only applies to --confluence".into());
    }
    let mut ws = if into.exists() {
        trackcraft_store::load(&into).map_err(|e| format!("{}: {e}", into.display()))?
    } else {
        let name = into.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported workspace".into());
        let mut ws = trackcraft_engine::model::Workspace::default();
        ws.rename(&name).map_err(|e| e.to_string())?;
        ws
    };
    let user = o.user.clone().or_else(|| ws.users.keys().next().cloned()).unwrap_or_else(|| "importer".into());
    let actor = trackcraft_import::Actor { user: &user, now: trackcraft_engine::system_now() };
    let report = trackcraft_import::import_path(&mut ws, kind, &path, space.as_deref(), actor).map_err(|e| format!("import: {e}"))?;
    trackcraft_store::save(&ws, &into).map_err(|e| format!("{}: {e}", into.display()))?;
    println!("{}", pretty(&report.to_json()));
    eprintln!(
        "imported {} issue(s), {} page(s), {} user(s) into {} ({} warning(s))",
        report.issues,
        report.pages,
        report.users.len(),
        into.display(),
        report.warnings.len().saturating_add(report.more_warnings)
    );
    Ok(())
}

/// Longest reply line read back from the app.
const MAX_REPLY: u64 = 64 << 20;

/// One request to the control channel on `127.0.0.1:port`; returns the reply.
fn send(port: &str, method: &str, params: Value) -> Result<Value, String> {
    let port: u16 = port.parse().ok().filter(|p| *p > 0).ok_or_else(|| format!("send: invalid port '{port}'"))?;
    let key = match std::env::var("TRACKCRAFT_CONTROL_KEY").ok().filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => {
            let kp = paths::key_path(port).ok_or("send: no config folder (set HOME or TRACKCRAFT_CONFIG_DIR)")?;
            let k = std::fs::read_to_string(&kp)
                .map_err(|e| format!("send: cannot read the key file {} ({e}); is `trackcraft --control {port}` running?", kp.display()))?;
            k.trim().to_string()
        }
    };
    let stream = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("send: cannot connect to 127.0.0.1:{port}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(90))).map_err(|e| e.to_string())?;
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    let req = json!({"id": 1, "key": key, "method": method, "params": params});
    writeln!(w, "{req}").map_err(|e| format!("send: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream).take(MAX_REPLY).read_line(&mut line).map_err(|e| format!("send: no reply: {e}"))?;
    serde_json::from_str(line.trim()).map_err(|e| format!("send: bad reply: {e}"))
}
