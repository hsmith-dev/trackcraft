//! `trackcraft-server`: the TrackCraft team server (a thin binary over `trackcraft-server-lib`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use trackcraft_server::{Config, Options, Server, mint_token, revoke_token, workspaces};

const USAGE: &str = "\
usage: trackcraft-server <command> [options]

commands:
  serve [--data DIR] [--listen ADDR] [--base-url URL] [--timeout SECS] [--max-body BYTES] [--quiet]
        serve every DIR/NAME.track (default DIR ./data, ADDR 127.0.0.1:8080)
  token create --user ID [--email EMAIL] [--label TEXT] [--data DIR]
        mint an API token for a workspace user (printed once; only its hash is stored)
  token list [--data DIR]
  token revoke TOKEN_ID [--data DIR]
  workspace create NAME [--sample] [--user ID] [--data DIR]
        create DIR/NAME.track (the sample workspace, or empty with ID as its first user)
  workspace list [--data DIR]

Clients authenticate with `Authorization: Bearer <token>` or Basic `<user id or email>:<token>`.
See docs/server.md.
";

struct Args {
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

/// Flags that take no value.
const SWITCHES: &[&str] = &["--sample", "--quiet", "--help"];

fn parse(raw: &[String]) -> Result<Args, String> {
    let mut a = Args { positional: vec![], flags: vec![] };
    let mut it = raw.iter();
    while let Some(x) = it.next() {
        if x.starts_with("--") {
            if SWITCHES.contains(&x.as_str()) {
                a.flags.push((x.clone(), None));
            } else {
                let v = it.next().ok_or_else(|| format!("{x} needs a value"))?;
                a.flags.push((x.clone(), Some(v.clone())));
            }
        } else {
            a.positional.push(x.clone());
        }
    }
    Ok(a)
}

impl Args {
    fn get(&self, k: &str) -> Option<&str> {
        self.flags.iter().find(|(n, _)| n == k).and_then(|(_, v)| v.as_deref())
    }
    fn has(&self, k: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == k)
    }
    fn data(&self) -> PathBuf {
        PathBuf::from(self.get("--data").unwrap_or("./data"))
    }
    fn pos(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(String::as_str)
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match parse(&raw).and_then(|a| run(&a)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("trackcraft-server: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(a: &Args) -> Result<(), String> {
    if a.has("--help") {
        print!("{USAGE}");
        return Ok(());
    }
    match (a.pos(0), a.pos(1)) {
        (Some("serve"), _) => serve(a),
        (Some("token"), Some("create")) => {
            let user = a.get("--user").ok_or("token create needs --user ID")?;
            let (rec, token) = mint_token(&a.data(), user, a.get("--email"), a.get("--label").unwrap_or_default()).map_err(|e| e.to_string())?;
            eprintln!("token {} for user {} (shown once; store it somewhere safe):", rec.id, rec.user);
            println!("{token}");
            Ok(())
        }
        (Some("token"), Some("list")) => {
            let cfg = Config::load(&a.data()).map_err(|e| e.to_string())?;
            for t in cfg.tokens {
                println!("{}\t{}\t{}\t{}\t{}", t.id, t.user, t.email.unwrap_or_default(), t.created, t.label);
            }
            Ok(())
        }
        (Some("token"), Some("revoke")) => {
            let id = a.pos(2).ok_or("token revoke needs a TOKEN_ID (see token list)")?;
            if revoke_token(&a.data(), id).map_err(|e| e.to_string())? {
                println!("revoked {id}");
                Ok(())
            } else {
                Err(format!("no token with id {id}"))
            }
        }
        (Some("workspace"), Some("create")) => {
            let name = a.pos(2).ok_or("workspace create needs a NAME")?;
            let path = workspaces::create(&a.data(), name, a.has("--sample"), a.get("--user")).map_err(|e| e.to_string())?;
            println!("created {}", path.display());
            Ok(())
        }
        (Some("workspace"), Some("list")) => {
            for (name, path) in workspaces::list(&a.data()).map_err(|e| e.to_string())? {
                println!("{name}\t{}", path.display());
            }
            Ok(())
        }
        _ => {
            print!("{USAGE}");
            Err("unknown command".into())
        }
    }
}

fn serve(a: &Args) -> Result<(), String> {
    let mut opts = Options::default();
    if let Some(t) = a.get("--timeout") {
        opts.timeout = Duration::from_secs(t.parse().map_err(|_| "--timeout is a number of seconds")?);
    }
    if let Some(b) = a.get("--max-body") {
        opts.max_body_bytes = b.parse().map_err(|_| "--max-body is a number of bytes")?;
    }
    opts.base_url = a.get("--base-url").map(str::to_string);
    opts.log_requests = !a.has("--quiet");
    let listen = a.get("--listen").unwrap_or("127.0.0.1:8080").to_string();
    let data = a.data();
    let server = Server::open(&data, opts).map_err(|e| e.to_string())?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| format!("could not start the runtime: {e}"))?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&listen).await.map_err(|e| format!("could not listen on {listen}: {e}"))?;
        let addr = listener.local_addr().map(|a| a.to_string()).unwrap_or(listen.clone());
        let names = server.hub.names();
        eprintln!(
            "trackcraft-server {} on http://{addr} — {} workspace(s) from {}: {}",
            env!("CARGO_PKG_VERSION"),
            names.len(),
            data.display(),
            names.join(", ")
        );
        if listener.local_addr().is_ok_and(|a| !a.ip().is_loopback()) {
            eprintln!("note: listening beyond this machine over plain HTTP; put a TLS reverse proxy in front so tokens aren't sent in the clear");
        }
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("trackcraft-server: shutting down (saving changes)");
        };
        server.serve(listener, shutdown).await.map_err(|e| e.to_string())
    })
}
