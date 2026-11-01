//! `trackcraft-cli`: headless TrackCraft — run commands, search with JQL, serve MCP.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};
use trackcraft_engine::{Session, catalog, commands};

const USAGE: &str = "\
usage: trackcraft-cli <command> [options]

commands:
  commands                         list every engine command (id, location, shortcut, params)
  run [--in F | --sample] [--as USER] --cmd 'id={json}' … [--out F]
                                   run commands in order and print each result as JSON
  jql [--in F | --sample] [--as USER] '<query>'
                                   search issues and print KEY  STATUS  SUMMARY lines
  new <file.track> [--sample]      create a workspace file (empty, or the sample)
  mcp [--in F | --sample] [--as USER]
                                   serve the MCP protocol on stdio (saves to --in after each change)
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
