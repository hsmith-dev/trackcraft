//! M4.3 permissions over every surface: Jira/Confluence routes (403 vs 404), the native command
//! endpoint, and per-user sync (filtered snapshot, withheld entries, resync on access change).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use trackcraft_api::{Hosted, Hub, NullSink};
use trackcraft_engine::Session;
use trackcraft_engine::sync::proto::{self, Entry, SseEvent, SseParser};

use super::*;

const RILEY: &str = "tct_riley_admin"; // workspace admin
const SAM: &str = "tct_sam_member"; // member
const CY: &str = "tct_cy_viewer"; // workspace viewer

struct T {
    server: Server,
    router: Router,
    restricted: u64,
}

fn setup() -> T {
    let mut s = Session::sample();
    s.user = "riley".into();
    let mut run = |id: &str, p: Value| s.run(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run("user.add", json!({"id": "cy", "name": "Cy"}));
    run("access.setWorkspaceRole", json!({"user": "cy", "role": "viewer"}));
    run("project.create", json!({"key": "SEC", "name": "Security"}));
    run("issue.create", json!({"project": "SEC", "summary": "Rotate keys"}));
    run("access.setProjectAccess", json!({"project": "SEC", "access": "private"}));
    let restricted = run("page.create", json!({"space": "ENG", "title": "Salaries", "body": "pay bands"}))["id"].as_u64().unwrap();
    run("page.restrict", json!({"id": restricted, "view": ["riley", "sam"]}));
    let hub = Arc::new(Hub::new());
    hub.add(Hosted::new("lab", s, None));
    let rec = |id: &str, user: &str, token: &str| TokenRecord {
        id: id.into(),
        user: user.into(),
        email: None,
        label: String::new(),
        sha256: config::hash(token),
        created: String::new(),
    };
    let ident = TokenIdentity::new(vec![rec("t1", "riley", RILEY), rec("t2", "sam", SAM), rec("t3", "cy", CY)]);
    let server = Server::new(hub, Arc::new(ident), Arc::new(NullSink), Options { log_requests: false, ..Options::default() });
    let router = server.router();
    T { server, router, restricted }
}

fn req(token: &str, method: &str, path: &str, body: Option<Value>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "track.test")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
        .unwrap()
}

async fn call(r: &Router, token: &str, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = r.clone().oneshot(req(token, method, path, body)).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn command(r: &Router, token: &str, cmd: &str, params: Value) -> (StatusCode, Value) {
    call(r, token, "POST", "/api/command", Some(json!({"command": cmd, "params": params}))).await
}

struct Events {
    body: axum::body::BodyDataStream,
    buf: String,
    parser: SseParser,
    ready: std::collections::VecDeque<SseEvent>,
}

impl Events {
    async fn open(r: &Router, token: &str, query: &str) -> Events {
        let resp = r.clone().oneshot(req(token, "GET", &format!("/api/sync/events{query}"), None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let mut ev = Events { body: resp.into_body().into_data_stream(), buf: String::new(), parser: SseParser::new(), ready: Default::default() };
        let hello = ev.next().await.unwrap();
        assert_eq!(hello.event, proto::EVENT_HELLO);
        ev
    }

    async fn next(&mut self) -> Option<SseEvent> {
        loop {
            if let Some(e) = self.ready.pop_front() {
                return Some(e);
            }
            let chunk = tokio::time::timeout(Duration::from_secs(5), self.body.next()).await.ok()??.ok()?;
            self.buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(i) = self.buf.find('\n') {
                let line: String = self.buf.drain(..=i).collect();
                if let Some(e) = self.parser.feed_line(line.trim_end_matches('\n')) {
                    self.ready.push_back(e);
                }
            }
        }
    }

    async fn entry(&mut self) -> Entry {
        let e = self.next().await.expect("an event");
        assert_eq!(e.event, proto::EVENT_ENTRY, "{e:?}");
        serde_json::from_str(&e.data).unwrap()
    }
}

#[tokio::test]
async fn viewers_cannot_change_things() {
    let t = setup();
    let fields = json!({"fields": {"project": {"key": "LUME"}, "summary": "From a viewer", "issuetype": {"name": "Task"}}});
    let (s, v) = call(&t.router, CY, "POST", "/rest/api/3/issue", Some(fields.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert!(v["errorMessages"][0].as_str().unwrap().starts_with("permission denied"), "{v}");
    let (s, _) = command(&t.router, CY, "issue.create", json!({"project": "LUME", "summary": "x"})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&t.router, CY, "PUT", "/rest/api/3/issue/LUME-1", Some(json!({"fields": {"summary": "nope"}}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // Viewers may read and comment.
    let (s, _) = call(&t.router, CY, "GET", "/rest/api/3/issue/LUME-1", None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&t.router, CY, "POST", "/rest/api/3/issue/LUME-1/comment", Some(json!({"body": "looks good"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    // Members may create; only admins change roles.
    let (s, _) = call(&t.router, SAM, "POST", "/rest/api/3/issue", Some(fields)).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = command(&t.router, SAM, "access.setWorkspaceRole", json!({"user": "sam", "role": "admin"})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = command(&t.router, RILEY, "access.setWorkspaceRole", json!({"user": "riley", "role": "viewer"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the last admin stays: {v}");
}

#[tokio::test]
async fn hidden_projects_are_404() {
    let t = setup();
    for path in ["/rest/api/3/project/SEC", "/rest/api/3/issue/SEC-1", "/rest/api/3/issue/SEC-1/comment", "/w/lab/rest/api/3/issue/SEC-1/changelog"] {
        let (s, v) = call(&t.router, SAM, "GET", path, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}: {v}");
        let (s, _) = call(&t.router, RILEY, "GET", path, None).await;
        assert_eq!(s, StatusCode::OK, "{path} as admin");
    }
    let (s, _) = call(&t.router, SAM, "POST", "/rest/api/3/issue/SEC-1/comment", Some(json!({"body": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = command(&t.router, SAM, "issue.comment", json!({"key": "SEC-1", "body": "x"})).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = call(&t.router, SAM, "GET", "/rest/api/3/project", None).await;
    assert!(!v.to_string().contains("\"SEC\""), "{v}");
    let (_, v) = call(&t.router, SAM, "GET", "/rest/api/3/search/jql?jql=ORDER%20BY%20key&maxResults=100", None).await;
    assert!(!v.to_string().contains("SEC-1"), "{v}");
    let (_, v) = call(&t.router, SAM, "GET", "/api/workspace", None).await;
    let (_, all) = call(&t.router, RILEY, "GET", "/api/workspace", None).await;
    assert_eq!(v["projects"].as_u64().unwrap() + 1, all["projects"].as_u64().unwrap());
}

#[tokio::test]
async fn restricted_pages_are_invisible() {
    let t = setup();
    let page = format!("/wiki/api/v2/pages/{}", t.restricted);
    let (s, _) = call(&t.router, CY, "GET", &page, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&t.router, SAM, "GET", &page, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&t.router, CY, "GET", &format!("{page}/versions"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = call(&t.router, CY, "GET", "/wiki/api/v2/pages?limit=250", None).await;
    assert!(!v.to_string().contains("Salaries"), "{v}");
    let (_, v) = command(&t.router, CY, "space.tree", json!({"key": "ENG"})).await;
    assert!(!v.to_string().contains("Salaries"), "{v}");
    // Not in the viewer's snapshot either.
    let (_, snap) = call(&t.router, CY, "GET", "/api/sync/snapshot", None).await;
    assert!(snap["workspace"]["workspace"]["pages"].get(t.restricted.to_string()).is_none());
    assert!(snap["workspace"]["workspace"]["projects"].get("SEC").is_none() && snap["workspace"]["workspace"]["issues"].get("SEC-1").is_none());
    assert!(!snap.to_string().contains("pay bands"));
    let (_, full) = call(&t.router, RILEY, "GET", "/api/sync/snapshot", None).await;
    assert!(full["workspace"]["workspace"]["pages"].get(t.restricted.to_string()).is_some());
}

#[tokio::test]
async fn events_are_filtered_and_access_changes_resync() {
    let t = setup();
    let (_, snap) = call(&t.router, CY, "GET", "/api/sync/snapshot", None).await;
    let ws = trackcraft_store::from_str(&snap["workspace"].to_string()).unwrap();
    let mut client = Session::new(ws, "cy");
    let epoch = snap["epoch"].as_str().unwrap().to_string();
    let mut cy = Events::open(&t.router, CY, &format!("?since=0&epoch={epoch}")).await;
    let mut riley = Events::open(&t.router, RILEY, &format!("?since=0&epoch={epoch}")).await;

    // Hidden edits arrive as placeholders that only move the revision and id counter.
    let (s, _) = command(&t.router, RILEY, "page.edit", json!({"id": t.restricted, "body": "new pay bands"})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = command(&t.router, SAM, "issue.comment", json!({"key": "LUME-1", "body": "visible"})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = command(&t.router, RILEY, "issue.comment", json!({"key": "SEC-1", "body": "secret"})).await;
    assert_eq!(s, StatusCode::OK);
    for want in ["sync.skip", "issue.comment", "sync.skip"] {
        let e = cy.entry().await;
        assert_eq!(e.command, want);
        if want == "sync.skip" {
            assert!(e.actor.is_empty() && !e.params.to_string().contains("pay"), "{e:?}");
        }
        client.apply_remote(&e).unwrap();
    }
    let h = t.server.hub.get("lab").unwrap();
    assert!(h.lock().ws.view_for("cy").is_some_and(|v| v == client.ws), "the viewer's replica equals their view");
    for want in ["page.edit", "issue.comment", "issue.comment"] {
        assert_eq!(riley.entry().await.command, want, "admins get everything");
    }

    // An access change ends non-admin streams with a resync; the new snapshot shows more.
    let (s, _) = command(&t.router, RILEY, "access.setProjectAccess", json!({"project": "SEC", "access": "limited"})).await;
    assert_eq!(s, StatusCode::OK);
    let e = cy.next().await.unwrap();
    assert_eq!(e.event, proto::EVENT_RESYNC, "{e:?}");
    assert!(cy.next().await.is_none(), "the stream ends after resync");
    assert_eq!(riley.entry().await.command, "access.setProjectAccess");
    let (_, snap) = call(&t.router, CY, "GET", "/api/sync/snapshot", None).await;
    assert!(snap["workspace"]["workspace"]["issues"].get("SEC-1").is_some());
}

#[test]
fn user_role_cli_edits_the_file() {
    let dir = std::env::temp_dir().join(format!("trackcraft-server-test-{}-roles", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    workspaces::create(&dir, "team", true, None).unwrap();
    let path = workspaces::set_role(&dir, None, "sam", "viewer").unwrap();
    let ws = trackcraft_store::load(&path).unwrap();
    assert_eq!(ws.workspace_role("sam"), Some(trackcraft_engine::model::WorkspaceRole::Viewer));
    assert!(workspaces::set_role(&dir, None, "riley", "member").is_err(), "the last admin stays");
    assert!(workspaces::set_role(&dir, None, "ghost", "admin").is_err());
    assert!(workspaces::set_role(&dir, Some("team"), "sam", "root").is_err());
    assert!(workspaces::set_role(&dir, Some("../x"), "sam", "admin").is_err());
    workspaces::create(&dir, "other", false, Some("ann")).unwrap();
    assert!(workspaces::set_role(&dir, None, "sam", "admin").is_err(), "several workspaces need --workspace");
    workspaces::set_role(&dir, Some("team"), "sam", "admin").unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
