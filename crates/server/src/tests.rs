//! In-process router tests: every route, auth failures, hostile input, pagination, persistence and
//! a Jira-style client flow end to end.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use trackcraft_api::{Hosted, Hub, MemorySink};
use trackcraft_engine::Session;

use super::*;

const TOKEN: &str = "tct_riley_secret";
const OUTSIDER: &str = "tct_outsider_secret";

fn rec(id: &str, user: &str, email: Option<&str>, token: &str) -> TokenRecord {
    TokenRecord {
        id: id.into(),
        user: user.into(),
        email: email.map(str::to_string),
        label: String::new(),
        sha256: config::hash(token),
        created: String::new(),
    }
}

struct T {
    server: Server,
    router: Router,
    events: Arc<MemorySink>,
}

fn setup_with(opts: Options, names: &[&str]) -> T {
    let hub = Arc::new(Hub::new());
    for n in names {
        hub.add(Hosted::new(n, Session::sample(), None));
    }
    let ident = TokenIdentity::new(vec![rec("t1", "riley", Some("riley@example.com"), TOKEN), rec("t2", "stranger", None, OUTSIDER)]);
    let events = Arc::new(MemorySink::default());
    let server = Server::new(hub, Arc::new(ident), events.clone(), opts);
    let router = server.router();
    T { server, router, events }
}

fn quiet() -> Options {
    Options { log_requests: false, ..Options::default() }
}

fn setup() -> T {
    setup_with(quiet(), &["lab"])
}

fn b64(input: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

enum Auth<'a> {
    None,
    Bearer(&'a str),
    Basic(&'a str, &'a str),
    Raw(&'a str),
}

impl T {
    async fn req(&self, method: &str, path: &str, auth: Auth<'_>, body: Option<Body>) -> (StatusCode, Value, HeaderMap) {
        let mut b = Request::builder().method(method).uri(path).header("host", "track.test");
        b = match auth {
            Auth::None => b,
            Auth::Bearer(t) => b.header("authorization", format!("Bearer {t}")),
            Auth::Basic(u, t) => b.header("authorization", format!("Basic {}", b64(format!("{u}:{t}").as_bytes()))),
            Auth::Raw(h) => b.header("authorization", h),
        };
        let req = b.header("content-type", "application/json").body(body.unwrap_or_else(Body::empty)).unwrap();
        let resp = self.router.clone().oneshot(req).await.unwrap();
        let (status, headers) = (resp.status(), resp.headers().clone());
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&bytes)}))
        };
        (status, v, headers)
    }
    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let (s, v, _) = self.req("GET", path, Auth::Bearer(TOKEN), None).await;
        (s, v)
    }
    async fn send(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let (s, v, _) = self.req(method, path, Auth::Bearer(TOKEN), Some(Body::from(body.to_string()))).await;
        (s, v)
    }
    fn session<R>(&self, f: impl FnOnce(&Session) -> R) -> R {
        let h = self.server.hub.get("lab").unwrap();
        let s = h.lock();
        f(&s)
    }
}

fn jira_msg(v: &Value) -> String {
    v["errorMessages"][0].as_str().unwrap_or_default().to_string()
}

// ── auth ──────────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn auth_failures_and_successes() {
    let t = setup();
    let (s, v, h) = t.req("GET", "/rest/api/3/myself", Auth::None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(jira_msg(&v).contains("authentication"));
    assert!(h.get("www-authenticate").is_some());
    assert_eq!(v["errors"], json!({}));
    for bad in [
        Auth::Bearer("tct_wrong"),
        Auth::Bearer("nope"),
        Auth::Raw("Basic !!!"),
        Auth::Raw("Digest x"),
        Auth::Basic("sam", TOKEN),
        Auth::Raw("Bearer"),
    ] {
        let (s, _, _) = t.req("GET", "/rest/api/3/myself", bad, None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    // Basic auth as Jira clients send it: email:token, or user id:token.
    let (s, v, _) = t.req("GET", "/rest/api/3/myself", Auth::Basic("riley@example.com", TOKEN), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["accountId"], "riley");
    assert_eq!(v["displayName"], "Riley Park");
    let (s, _, _) = t.req("GET", "/rest/api/3/myself", Auth::Basic("riley", TOKEN), None).await;
    assert_eq!(s, StatusCode::OK);
    // A valid token for someone who isn't in the workspace.
    let (s, v, _) = t.req("GET", "/rest/api/3/project", Auth::Bearer(OUTSIDER), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(jira_msg(&v).contains("not a member"));
    // Confluence and native shapes for 401.
    let (s, v, _) = t.req("GET", "/wiki/api/v2/spaces", Auth::None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(v["errors"][0]["status"], 401);
    let (_, v, _) = t.req("POST", "/api/command", Auth::None, None).await;
    assert_eq!(v["status"], 401);
    // Health needs no token.
    let (s, v, _) = t.req("GET", "/api/health", Auth::None, None).await;
    assert_eq!((s, v["status"].as_str()), (StatusCode::OK, Some("ok")));
}

#[tokio::test]
async fn token_file_round_trip_and_live_reload() {
    let dir = temp_dir("tokens");
    let ident = TokenIdentity::from_dir(&dir).unwrap();
    let (r, token) = mint_token(&dir, "ann", Some("ann@example.com"), "ci").unwrap();
    assert!(token.starts_with(config::TOKEN_PREFIX) && token.len() > 60);
    let text = std::fs::read_to_string(dir.join(config::CONFIG_FILE)).unwrap();
    assert!(!text.contains(&token), "only the hash is stored");
    assert!(text.contains(&r.sha256));
    // Minted after the provider was created: picked up without a restart.
    assert_eq!(ident.authenticate(&trackcraft_api::Credential::Bearer(&token)).map(|i| i.user), Some("ann".into()));
    assert!(revoke_token(&dir, &r.id).unwrap());
    assert!(!revoke_token(&dir, &r.id).unwrap());
    // mtime resolution: make sure the change is visible.
    std::thread::sleep(Duration::from_millis(20));
    let cfg = Config::load(&dir).unwrap();
    cfg.save(&dir).unwrap();
    assert!(ident.authenticate(&trackcraft_api::Credential::Bearer(&token)).is_none());
    assert!(mint_token(&dir, "has space", None, "").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Jira ──────────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn projects_and_issue_reads() {
    let t = setup();
    let (s, v) = t.get("/rest/api/3/project").await;
    assert_eq!(s, StatusCode::OK);
    let lume = v.as_array().unwrap().iter().find(|p| p["key"] == "LUME").unwrap().clone();
    assert_eq!(lume["projectTypeKey"], "software");
    assert!(lume["self"].as_str().unwrap().starts_with("http://track.test/rest/api/3/project/"));
    let id = lume["id"].as_str().unwrap().to_string();
    let (s, by_id) = t.get(&format!("/rest/api/3/project/{id}")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(by_id["key"], "LUME");
    assert_eq!(by_id["lead"]["accountId"], "riley");
    assert!(by_id["issueTypes"].as_array().unwrap().iter().any(|t| t["name"] == "Sub-task" && t["subtask"] == true));
    let (s, v) = t.get("/rest/api/3/project/NOPE").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(jira_msg(&v).contains("NOPE"));

    let (s, i) = t.get("/rest/api/3/issue/LUME-4").await;
    assert_eq!(s, StatusCode::OK);
    let f = &i["fields"];
    assert_eq!(f["status"]["name"], "In Review");
    assert_eq!(f["status"]["statusCategory"]["key"], "indeterminate");
    assert_eq!(f["priority"]["name"], "Highest");
    assert_eq!(f["assignee"]["accountId"], "jo");
    assert_eq!(f["comment"]["total"], 2);
    assert_eq!(f["comment"]["comments"][0]["body"]["type"], "doc");
    assert!(f["created"].as_str().unwrap().ends_with("+0000"));
    assert_eq!(f["sprint"]["state"], "active");
    // By numeric id, and with a field selection.
    let iid = i["id"].as_str().unwrap().to_string();
    let (s, j) = t.get(&format!("/rest/api/3/issue/{iid}?fields=summary,status")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(j["key"], "LUME-4");
    assert_eq!(j["fields"].as_object().unwrap().len(), 2);
    let (_, j) = t.get("/rest/api/3/issue/LUME-4?fields=*all,-comment").await;
    assert!(j["fields"].get("comment").is_none() && j["fields"].get("summary").is_some());
    // Links both ways: LUME-6 blocks LUME-2.
    let (_, six) = t.get("/rest/api/3/issue/LUME-6?fields=issuelinks").await;
    assert_eq!(six["fields"]["issuelinks"][0]["outwardIssue"]["key"], "LUME-2");
    assert_eq!(six["fields"]["issuelinks"][0]["type"]["name"], "Blocks");
    let (_, two) = t.get("/rest/api/3/issue/LUME-2?fields=issuelinks,subtasks").await;
    assert!(two["fields"]["issuelinks"].as_array().unwrap().iter().any(|l| l["inwardIssue"]["key"] == "LUME-6"));
    assert_eq!(two["fields"]["subtasks"][0]["key"], "LUME-7");
    let (s, v) = t.get("/rest/api/3/issue/LUME-999").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(jira_msg(&v).contains("LUME-999"));
}

#[tokio::test]
async fn search_jql_pagination_and_errors() {
    let t = setup();
    let (s, v) = t.get("/rest/api/3/search/jql?jql=project%20%3D%20LUME%20ORDER%20BY%20key&maxResults=3&fields=summary").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["issues"].as_array().unwrap().len(), 3);
    assert_eq!(v["issues"][0]["key"], "LUME-1");
    assert!(v["issues"][0]["fields"]["summary"].is_string());
    assert_eq!(v["isLast"], false);
    // Walk every page with nextPageToken.
    let mut keys: Vec<String> = v["issues"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap().to_string()).collect();
    let mut token = v["nextPageToken"].as_str().unwrap().to_string();
    loop {
        let (s, p) = t.get(&format!("/rest/api/3/search/jql?jql=project%20%3D%20LUME%20ORDER%20BY%20key&maxResults=3&nextPageToken={token}")).await;
        assert_eq!(s, StatusCode::OK);
        keys.extend(p["issues"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap().to_string()));
        if p["isLast"] == true {
            assert!(p.get("nextPageToken").is_none());
            break;
        }
        token = p["nextPageToken"].as_str().unwrap().to_string();
    }
    let total = t.session(|s| s.ws.issues.keys().filter(|k| k.project.as_str() == "LUME").count());
    assert_eq!(keys.len(), total);
    // Default fields: ids only (no "fields" object).
    let (_, v) = t.get("/rest/api/3/search/jql?jql=key%3DLUME-1").await;
    assert!(v["issues"][0].get("fields").is_none());
    assert_eq!(v["issues"][0]["key"], "LUME-1");
    // maxResults is capped; startAt still works.
    let (_, v) = t.get("/rest/api/3/search/jql?jql=&maxResults=100000&startAt=1").await;
    assert!(v["issues"].as_array().unwrap().len() <= trackcraft_api::jira::MAX_SEARCH);
    // POST form.
    let (s, v) = t
        .send("POST", "/rest/api/3/search/jql", json!({"jql": "assignee = currentUser()", "fields": ["summary", "assignee"], "maxResults": 50}))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["issues"].as_array().unwrap().iter().all(|i| i["fields"]["assignee"]["accountId"] == "riley"));
    // Bad JQL and bad params are 400 with a message.
    let (s, v) = t.get("/rest/api/3/search/jql?jql=project%20%3D%20(").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(jira_msg(&v).starts_with("JQL"), "{v}");
    let (s, _) = t.get("/rest/api/3/search/jql?maxResults=lots").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = t.get("/rest/api/3/search/jql?nextPageToken=garbage").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(jira_msg(&v).contains("nextPageToken"));
    let (s, _) = t.send("POST", "/rest/api/3/search/jql", json!({"jql": 5})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = t.send("POST", "/rest/api/3/search/jql", json!([1, 2])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn issue_create_edit_delete() {
    let t = setup();
    let adf = json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "From REST"}]}]});
    let (s, v) = t
        .send(
            "POST",
            "/rest/api/3/issue",
            json!({"fields": {"project": {"key": "LUME"}, "summary": "Made over REST", "issuetype": {"name": "Bug"}, "description": adf, "priority": {"name": "Low"}, "assignee": {"accountId": "sam"}, "labels": ["api"], "customfield_10000": 1}}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let key = v["key"].as_str().unwrap().to_string();
    assert!(v["self"].as_str().unwrap().contains("/rest/api/3/issue/"));
    let (_, i) = t.get(&format!("/rest/api/3/issue/{key}")).await;
    assert_eq!(i["fields"]["issuetype"]["name"], "Bug");
    assert_eq!(i["fields"]["reporter"]["accountId"], "riley", "runs as the token's user");
    assert_eq!(i["fields"]["assignee"]["accountId"], "sam");
    assert_eq!(i["fields"]["description"]["content"][0]["content"][0]["text"], "From REST");
    // The shared session's acting user is restored after the request (the sample's first user).
    assert_eq!(t.session(|s| s.user.clone()), "jo");
    // Validation errors.
    for bad in [
        json!({}),
        json!({"fields": {"summary": "x"}}),
        json!({"fields": {"project": {"key": "NOPE"}, "summary": "x"}}),
        json!({"fields": {"project": {"key": "LUME"}}}),
        json!({"fields": {"project": {"key": "LUME"}, "summary": 7}}),
        json!({"fields": {"project": {"key": "LUME"}, "summary": "x", "bogus": 1}}),
        json!({"fields": {"project": {"key": "LUME"}, "summary": "x", "priority": {"name": "Urgentest"}}}),
    ] {
        let (s, v) = t.send("POST", "/rest/api/3/issue", bad.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad} → {v}");
        assert!(!jira_msg(&v).is_empty());
    }

    // Edit: fields + update verbs; returnIssue.
    let (s, _) = t
        .send(
            "PUT",
            &format!("/rest/api/3/issue/{key}"),
            json!({"fields": {"summary": "Renamed"}, "update": {"labels": [{"add": "triage"}, {"remove": "api"}]}}),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, i) =
        t.send("PUT", &format!("/rest/api/3/issue/{key}?returnIssue=true"), json!({"fields": {"assignee": null, "priority": {"id": "1"}}})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(i["fields"]["summary"], "Renamed");
    assert_eq!(i["fields"]["labels"], json!(["triage"]));
    assert_eq!(i["fields"]["assignee"], Value::Null);
    assert_eq!(i["fields"]["priority"]["name"], "Highest");
    // All or nothing: a bad assignee leaves the summary alone.
    let (s, _) = t
        .send("PUT", &format!("/rest/api/3/issue/{key}"), json!({"fields": {"summary": "Should not stick", "assignee": {"accountId": "ghost"}}}))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(t.session(|s| s.ws.issue(&key).unwrap().summary.clone()), "Renamed");
    let (s, _) = t.send("PUT", &format!("/rest/api/3/issue/{key}"), json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = t.send("PUT", &format!("/rest/api/3/issue/{key}"), json!({"update": {"labels": "add"}})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Delete: refused for an issue with sub-tasks unless asked.
    let (s, v, _) = t.req("DELETE", "/rest/api/3/issue/LUME-2", Auth::Bearer(TOKEN), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(jira_msg(&v).contains("deleteSubtasks"));
    let (s, _, _) = t.req("DELETE", "/rest/api/3/issue/LUME-2?deleteSubtasks=true", Auth::Bearer(TOKEN), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(t.session(|s| s.ws.issue("LUME-7").is_err()));
    let (s, _, _) = t.req("DELETE", &format!("/rest/api/3/issue/{key}"), Auth::Bearer(TOKEN), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = t.get(&format!("/rest/api/3/issue/{key}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Events carry the acting user.
    let ev = t.events.events();
    assert!(ev.iter().any(|e| e.command == "issue.create" && e.user == "riley" && e.workspace == "lab"));
    assert!(ev.iter().all(|e| trackcraft_engine::command(&e.command).is_some_and(|c| c.mutates)));
}

#[tokio::test]
async fn transitions_comments_assignee_links_changelog() {
    let t = setup();
    let (s, v) = t.get("/rest/api/3/issue/LUME-3/transitions").await;
    assert_eq!(s, StatusCode::OK);
    let list = v["transitions"].as_array().unwrap();
    assert!(!list.iter().any(|x| x["to"]["name"] == "To Do"), "no transition to the current status");
    let done = list.iter().find(|x| x["to"]["name"] == "Done").unwrap();
    assert_eq!(done["to"]["statusCategory"]["key"], "done");
    let (s, _) = t
        .send(
            "POST",
            "/rest/api/3/issue/LUME-3/transitions",
            json!({"transition": {"id": done["id"]}, "update": {"comment": [{"add": {"body": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Closing"}]}]}}}]}}),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, i) = t.get("/rest/api/3/issue/LUME-3?fields=status,resolution,comment").await;
    assert_eq!(i["fields"]["status"]["name"], "Done");
    assert_eq!(i["fields"]["resolution"]["name"], "Done");
    assert_eq!(i["fields"]["comment"]["total"], 1);
    let (s, v) = t.send("POST", "/rest/api/3/issue/LUME-3/transitions", json!({"transition": {"id": "999"}})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(jira_msg(&v).contains("999"));
    let (s, _) = t.send("POST", "/rest/api/3/issue/LUME-3/transitions", json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = t.get("/rest/api/3/issue/LUME-3/transitions?transitionId=1").await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["transitions"].as_array().unwrap().len() <= 1);

    // Comments: list with paging, add, get, edit, delete.
    let (s, v) = t.get("/rest/api/3/issue/LUME-4/comment?maxResults=1").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((v["total"].as_u64(), v["comments"].as_array().unwrap().len()), (Some(2), 1));
    let (_, newest) = t.get("/rest/api/3/issue/LUME-4/comment?orderBy=-created&maxResults=1").await;
    assert_eq!(newest["comments"][0]["author"]["accountId"], "jo");
    let (s, c) = t.send("POST", "/rest/api/3/issue/LUME-4/comment", json!({"body": "plain *markdown* works too"})).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    assert_eq!(c["author"]["accountId"], "riley");
    let cid = c["id"].as_str().unwrap().to_string();
    let (s, _) = t.get(&format!("/rest/api/3/issue/LUME-4/comment/{cid}")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, c) = t.send("PUT", &format!("/rest/api/3/issue/LUME-4/comment/{cid}"), json!({"body": "edited"})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(c["body"]["content"][0]["content"][0]["text"], "edited");
    let (s, _, _) = t.req("DELETE", &format!("/rest/api/3/issue/LUME-4/comment/{cid}"), Auth::Bearer(TOKEN), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = t.get(&format!("/rest/api/3/issue/LUME-4/comment/{cid}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t.get("/rest/api/3/issue/LUME-4/comment/abc").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = t.send("POST", "/rest/api/3/issue/LUME-4/comment", json!({"text": "no body"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Someone else's comment can't be edited.
    let other = t.session(|s| s.ws.issue("LUME-4").unwrap().comments[1].id.0);
    let (s, _) = t.send("PUT", &format!("/rest/api/3/issue/LUME-4/comment/{other}"), json!({"body": "hijack"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Assignee.
    let (s, _) = t.send("PUT", "/rest/api/3/issue/LUME-6/assignee", json!({"accountId": "sam"})).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(t.session(|s| s.ws.issue("LUME-6").unwrap().assignee.clone()), Some("sam".into()));
    let (s, _) = t.send("PUT", "/rest/api/3/issue/LUME-6/assignee", json!({"accountId": null})).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(t.session(|s| s.ws.issue("LUME-6").unwrap().assignee.clone()), None);
    let (s, _) = t.send("PUT", "/rest/api/3/issue/LUME-6/assignee", json!({"accountId": "ghost"})).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t.send("PUT", "/rest/api/3/issue/LUME-6/assignee", json!({"accountId": 5})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Links: inwardIssue <type outward> outwardIssue.
    let (s, _) = t
        .send(
            "POST",
            "/rest/api/3/issueLink",
            json!({"type": {"name": "Duplicate"}, "inwardIssue": {"key": "LUME-5"}, "outwardIssue": {"key": "LUME-3"}}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, five) = t.get("/rest/api/3/issue/LUME-5?fields=issuelinks").await;
    assert_eq!(five["fields"]["issuelinks"][0]["outwardIssue"]["key"], "LUME-3");
    assert_eq!(five["fields"]["issuelinks"][0]["type"]["outward"], "duplicates");
    for bad in [
        json!({"type": {"name": "Haunts"}, "inwardIssue": {"key": "LUME-5"}, "outwardIssue": {"key": "LUME-3"}}),
        json!({"type": {"name": "Blocks"}, "inwardIssue": {"key": "LUME-5"}}),
        json!({"type": {"name": "Blocks"}, "inwardIssue": {"key": "LUME-5"}, "outwardIssue": {"key": "LUME-5"}}),
    ] {
        let (s, _) = t.send("POST", "/rest/api/3/issueLink", bad).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    // Changelog: oldest first, grouped per change.
    let (s, v) = t.get("/rest/api/3/issue/LUME-3/changelog").await;
    assert_eq!(s, StatusCode::OK);
    let values = v["values"].as_array().unwrap();
    let done = values.iter().find(|g| g["items"].as_array().unwrap().iter().any(|i| i["field"] == "status" && i["toString"] == "Done")).unwrap();
    assert_eq!(done["author"]["accountId"], "riley");
    assert!(values.last().unwrap()["items"].as_array().unwrap().iter().any(|i| i["field"] == "comment"), "oldest first: the comment came last");
    let (_, p) = t.get("/rest/api/3/issue/LUME-3/changelog?maxResults=1").await;
    assert_eq!(p["values"].as_array().unwrap().len(), 1);
    assert_eq!(p["isLast"], values.len() <= 1);
}

// ── Agile ─────────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn agile_boards_sprints_backlog() {
    let t = setup();
    let (s, v) = t.get("/rest/agile/1.0/board").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["total"], 2);
    let board = v["values"].as_array().unwrap().iter().find(|b| b["location"]["projectKey"] == "LUME").unwrap().clone();
    assert_eq!(board["type"], "scrum");
    let id = board["id"].as_u64().unwrap();
    let (_, k) = t.get("/rest/agile/1.0/board?type=kanban").await;
    assert_eq!(k["values"][0]["location"]["projectKey"], "OPS");
    let (_, one) = t.get("/rest/agile/1.0/board?projectKeyOrId=LUME&maxResults=1").await;
    assert_eq!(one["values"].as_array().unwrap().len(), 1);
    let (s, b) = t.get(&format!("/rest/agile/1.0/board/{id}")).await;
    assert_eq!((s, b["id"].as_u64()), (StatusCode::OK, Some(id)));

    let (s, sp) = t.get(&format!("/rest/agile/1.0/board/{id}/sprint")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(sp["values"].as_array().unwrap().len(), 2);
    let (_, active) = t.get(&format!("/rest/agile/1.0/board/{id}/sprint?state=active")).await;
    let active = &active["values"][0];
    assert_eq!(active["state"], "active");
    assert!(active["startDate"].as_str().unwrap().ends_with('Z'));
    let sid = active["id"].as_u64().unwrap();
    let (s, si) = t.get(&format!("/rest/agile/1.0/sprint/{sid}/issue?fields=summary")).await;
    assert_eq!(s, StatusCode::OK);
    let keys: Vec<&str> = si["issues"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap()).collect();
    assert!(keys.contains(&"LUME-2") && keys.contains(&"LUME-4"));
    let (_, filtered) = t.get(&format!("/rest/agile/1.0/sprint/{sid}/issue?jql=assignee%3Djo")).await;
    assert_eq!(filtered["total"], 1);
    let (s, _) = t.get(&format!("/rest/agile/1.0/sprint/{sid}/issue?jql=(((")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, one) = t.get(&format!("/rest/agile/1.0/sprint/{sid}")).await;
    assert_eq!((s, one["id"].as_u64()), (StatusCode::OK, Some(sid)));

    let (s, bl) = t.get(&format!("/rest/agile/1.0/board/{id}/backlog")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(bl["issues"].as_array().unwrap().iter().any(|i| i["key"] == "LUME-6"));
    assert!(!bl["issues"].as_array().unwrap().iter().any(|i| i["key"] == "LUME-2"), "sprint issues aren't in the backlog");
    let (s, bi) = t.get(&format!("/rest/agile/1.0/board/{id}/issue?maxResults=2")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(bi["issues"].as_array().unwrap().len(), 2);
    assert!(bi["total"].as_u64().unwrap() > 2);

    for (path, code) in [
        ("/rest/agile/1.0/board/999".to_string(), StatusCode::NOT_FOUND),
        ("/rest/agile/1.0/board/abc/sprint".to_string(), StatusCode::NOT_FOUND),
        ("/rest/agile/1.0/sprint/999999/issue".to_string(), StatusCode::NOT_FOUND),
        ("/rest/agile/1.0/sprint/x".to_string(), StatusCode::BAD_REQUEST),
    ] {
        let (s, v) = t.get(&path).await;
        assert_eq!(s, code, "{path}: {v}");
        assert!(v["errorMessages"].is_array());
    }
    let ops = trackcraft_api::ids::key_id("OPS");
    let (s, _) = t.get(&format!("/rest/agile/1.0/board/{ops}/sprint")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "kanban boards have no sprints");
}

// ── Confluence ────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn wiki_spaces_and_pages() {
    let t = setup();
    let (s, v, h) = t.req("GET", "/wiki/api/v2/spaces?limit=1", Auth::Bearer(TOKEN), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["results"].as_array().unwrap().len(), 1);
    let next = v["_links"]["next"].as_str().unwrap().to_string();
    assert!(next.starts_with("/wiki/api/v2/spaces?") && next.contains("cursor="));
    assert!(h.get("link").unwrap().to_str().unwrap().contains("rel=\"next\""));
    let (_, v2) = t.get(&next).await;
    assert_eq!(v2["results"].as_array().unwrap().len(), 1);
    assert!(v2["_links"].get("next").is_none());
    assert_ne!(v["results"][0]["key"], v2["results"][0]["key"]);
    let (_, eng) = t.get("/wiki/api/v2/spaces?keys=ENG").await;
    let eng = eng["results"][0].clone();
    let space_id = eng["id"].as_str().unwrap().to_string();
    let (s, _) = t.get(&format!("/wiki/api/v2/spaces/{space_id}")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = t.get("/wiki/api/v2/spaces?cursor=bogus").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["errors"][0]["code"], "BAD_REQUEST");

    // Pages: filter by space, ADF body.
    let (s, v) = t.get(&format!("/wiki/api/v2/pages?space-id={space_id}&body-format=atlas_doc_format&limit=250")).await;
    assert_eq!(s, StatusCode::OK);
    let pages = v["results"].as_array().unwrap();
    assert!(pages.iter().all(|p| p["spaceId"] == space_id.as_str()));
    let arch = pages.iter().find(|p| p["title"] == "Architecture overview").unwrap().clone();
    let adf: Value = serde_json::from_str(arch["body"]["atlas_doc_format"]["value"].as_str().unwrap()).unwrap();
    assert_eq!(adf["type"], "doc");
    let (_, no_body) = t.get("/wiki/api/v2/pages?title=Architecture%20overview").await;
    assert!(no_body["results"][0].get("body").is_none());
    let (s, _) = t.get("/wiki/api/v2/pages?body-format=view").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, sp) = t.get(&format!("/wiki/api/v2/spaces/{space_id}/pages")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!sp["results"].as_array().unwrap().is_empty());

    // Create (storage XHTML → paragraphs), read back as storage (Markdown) and ADF.
    let arch_id = arch["id"].as_str().unwrap().to_string();
    let (s, p) = t
        .send("POST", "/wiki/api/v2/pages", json!({"spaceId": space_id, "status": "current", "title": "REST page", "parentId": arch_id, "body": {"representation": "storage", "value": "<p>Hello &amp; welcome</p><p>Second</p>"}}))
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["version"]["number"], 1);
    assert_eq!(p["parentId"], arch_id.as_str());
    assert_eq!(p["authorId"], "riley");
    assert_eq!(p["body"]["storage"]["value"], "Hello & welcome\n\nSecond");
    let pid = p["id"].as_str().unwrap().to_string();
    let (_, kids) = t.get(&format!("/wiki/api/v2/pages/{arch_id}/children")).await;
    assert!(kids["results"].as_array().unwrap().iter().any(|c| c["id"] == pid.as_str()));
    for bad in [
        json!({"title": "no space"}),
        json!({"spaceId": space_id}),
        json!({"spaceId": "424242", "title": "x"}),
        json!({"spaceId": space_id, "title": "x", "status": "draft"}),
        json!({"spaceId": space_id, "title": "x", "body": {"representation": "wiki", "value": "h1. x"}}),
        json!({"spaceId": space_id, "title": "x", "body": {"representation": "atlas_doc_format", "value": "{not json"}}),
    ] {
        let (s, v) = t.send("POST", "/wiki/api/v2/pages", bad.clone()).await;
        assert!(s == StatusCode::BAD_REQUEST || s == StatusCode::NOT_FOUND, "{bad} → {s} {v}");
        assert!(v["errors"][0]["title"].is_string());
    }

    // Update needs the next version number; a stale one is 409.
    let adf = json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "v2 body"}]}]});
    let body = json!({"id": pid, "status": "current", "title": "REST page v2", "body": {"representation": "atlas_doc_format", "value": adf.to_string()}, "version": {"number": 2, "message": "second"}});
    let (s, v) = t.send("PUT", &format!("/wiki/api/v2/pages/{pid}"), body.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["version"]["number"], 2);
    assert_eq!(v["title"], "REST page v2");
    let (s, v) = t.send("PUT", &format!("/wiki/api/v2/pages/{pid}"), body).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["errors"][0]["status"], 409);
    let (s, _) = t.send("PUT", &format!("/wiki/api/v2/pages/{pid}"), json!({"title": "no version"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Storage round trip keeps Markdown.
    let (_, cur) = t.get(&format!("/wiki/api/v2/pages/{pid}?body-format=storage")).await;
    let md = cur["body"]["storage"]["value"].as_str().unwrap().to_string();
    let (s, _) = t
        .send(
            "PUT",
            &format!("/wiki/api/v2/pages/{pid}"),
            json!({"title": "REST page v2", "body": {"representation": "storage", "value": format!("{md}\n\n**bold**")}, "version": {"number": 3}}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, cur) = t.get(&format!("/wiki/api/v2/pages/{pid}?body-format=storage")).await;
    assert!(cur["body"]["storage"]["value"].as_str().unwrap().contains("**bold**"));

    // Versions (newest first by default) and a specific old version.
    let (s, vs) = t.get(&format!("/wiki/api/v2/pages/{pid}/versions")).await;
    assert_eq!(s, StatusCode::OK);
    let nums: Vec<u64> = vs["results"].as_array().unwrap().iter().map(|v| v["number"].as_u64().unwrap()).collect();
    assert_eq!(nums, vec![3, 2, 1]);
    assert_eq!(vs["results"][1]["message"], "second");
    let (_, old) = t.get(&format!("/wiki/api/v2/pages/{pid}?version=1&body-format=storage")).await;
    assert_eq!(old["title"], "REST page");
    assert_eq!(old["version"]["number"], 1);
    let (s, _) = t.get(&format!("/wiki/api/v2/pages/{pid}?version=77")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Footer comments.
    let (s, c) = t
        .send("POST", "/wiki/api/v2/footer-comments", json!({"pageId": pid, "body": {"representation": "storage", "value": "<p>Nice page</p>"}}))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let cid = c["id"].as_str().unwrap().to_string();
    let (s, r) = t
        .send("POST", "/wiki/api/v2/footer-comments", json!({"parentCommentId": cid, "body": {"representation": "storage", "value": "reply"}}))
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r["parentCommentId"], cid.as_str());
    let (s, list) = t.get(&format!("/wiki/api/v2/pages/{pid}/footer-comments?body-format=storage")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["results"].as_array().unwrap().len(), 1, "replies are not top-level");
    assert_eq!(list["results"][0]["body"]["storage"]["value"], "Nice page");
    let (s, _) = t.send("POST", "/wiki/api/v2/footer-comments", json!({"body": {"representation": "storage", "value": "x"}})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Delete.
    let (s, _, _) = t.req("DELETE", &format!("/wiki/api/v2/pages/{pid}"), Auth::Bearer(TOKEN), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, v) = t.get(&format!("/wiki/api/v2/pages/{pid}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["errors"][0]["code"], "NOT_FOUND");
    let (s, _) = t.get("/wiki/api/v2/pages/abc").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// ── native ────────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn native_commands() {
    let t = setup();
    let (s, v) = t.get("/api/commands").await;
    assert_eq!(s, StatusCode::OK);
    let list = v.as_array().unwrap();
    assert!(list.iter().any(|c| c["id"] == "issue.create"));
    assert!(list.iter().find(|c| c["id"] == "file.open").unwrap()["disabled"].is_string());
    let (s, v) = t.send("POST", "/api/command", json!({"command": "issue.create", "params": {"project": "OPS", "summary": "Native"}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let key = v["result"]["key"].as_str().unwrap().to_string();
    assert_eq!(t.session(|s| s.ws.issue(&key).unwrap().reporter.clone()), "riley");
    let (s, v) = t.send("POST", "/api/command", json!({"command": "search.jql", "params": {"jql": "project = OPS"}})).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["result"]["total"].as_u64().unwrap() >= 1);
    for blocked in ["file.open", "file.save", "edit.undo", "user.switch", "workspace.new"] {
        let (s, v) = t.send("POST", "/api/command", json!({"command": blocked, "params": {"path": "/etc/passwd", "id": "sam"}})).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{blocked}: {v}");
        assert!(v["error"].as_str().unwrap().contains("not available"));
    }
    let (s, _) = t.send("POST", "/api/command", json!({"params": {}})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = t.send("POST", "/api/command", json!({"command": "no.such"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("unknown command"));
    let (s, v) = t.send("POST", "/api/command", json!({"command": "issue.get", "params": {"key": "LUME-404"}})).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (s, v) = t.get("/api/workspace").await;
    assert_eq!((s, v["name"].as_str()), (StatusCode::OK, Some("lab")));
}

// ── hostile input and limits ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn hostile_requests_get_json_errors() {
    let t = setup_with(Options { max_body_bytes: 1024, ..quiet() }, &["lab"]);
    let huge = format!("{{\"fields\": {{\"summary\": \"{}\"}}}}", "x".repeat(10_000));
    let (s, v, _) = t.req("POST", "/rest/api/3/issue", Auth::Bearer(TOKEN), Some(Body::from(huge))).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(jira_msg(&v).contains("too large"));
    for (path, body) in [
        ("/rest/api/3/issue", "{not json"),
        (
            "/rest/api/3/issue",
            "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[",
        ),
        ("/wiki/api/v2/pages", "\u{0}"),
        ("/api/command", "nul"),
    ] {
        let (s, v, _) = t.req("POST", path, Auth::Bearer(TOKEN), Some(Body::from(body))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{path} {body}");
        assert!(v.get("errorMessages").is_some() || v.get("errors").is_some() || v.get("error").is_some(), "{v}");
    }
    // Wrong JSON shapes never panic: every route with junk bodies.
    let junk = [
        json!(null),
        json!(1),
        json!("s"),
        json!([]),
        json!({"fields": []}),
        json!({"fields": {"project": 5, "summary": []}}),
        json!({"body": {"representation": 1}}),
        json!({"transition": []}),
        json!({"version": "x"}),
    ];
    let routes = [
        ("POST", "/rest/api/3/issue"),
        ("PUT", "/rest/api/3/issue/LUME-1"),
        ("POST", "/rest/api/3/issue/LUME-1/transitions"),
        ("POST", "/rest/api/3/issue/LUME-1/comment"),
        ("PUT", "/rest/api/3/issue/LUME-1/assignee"),
        ("POST", "/rest/api/3/issueLink"),
        ("POST", "/rest/api/3/search/jql"),
        ("POST", "/wiki/api/v2/pages"),
        ("PUT", "/wiki/api/v2/pages/1"),
        ("POST", "/wiki/api/v2/footer-comments"),
        ("POST", "/api/command"),
    ];
    for (m, p) in routes {
        for j in &junk {
            let (s, v, _) = t.req(m, p, Auth::Bearer(TOKEN), Some(Body::from(j.to_string()))).await;
            assert!(s.is_client_error() || s.is_success(), "{m} {p} {j} → {s} {v}");
            assert!(s != StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    // Unknown endpoints and methods answer in JSON.
    let (s, v) = t.get("/rest/api/3/nope").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(jira_msg(&v).contains("no such endpoint"));
    let (s, v) = t.get("/wiki/api/v2/nope").await;
    assert_eq!((s, v["errors"][0]["status"].as_u64()), (StatusCode::NOT_FOUND, Some(404)));
    let (s, v, _) = t.req("PATCH", "/rest/api/3/issue/LUME-1", Auth::Bearer(TOKEN), None).await;
    assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
    assert!(v["errorMessages"].is_array());
    let (s, _) = t.get("/rest/api/3/issue/%FF%FE").await;
    assert!(s.is_client_error());
    let long_key = "A".repeat(5000);
    let (s, v) = t.get(&format!("/rest/api/3/issue/{long_key}-1")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(jira_msg(&v).len() < 1100);
    // A huge Basic header doesn't allocate unbounded.
    let (s, _, _) = t.req("GET", "/rest/api/3/myself", Auth::Raw(&format!("Basic {}", "QUFB".repeat(5000))), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // deliberate: the held lock stands in for a long command
async fn request_timeout_is_a_json_503() {
    let t = setup_with(Options { timeout: Duration::from_millis(50), ..quiet() }, &["lab"]);
    let h = t.server.hub.get("lab").unwrap();
    let guard = h.lock(); // a long command holding the workspace
    let (s, v) = t.get("/rest/api/3/myself").await;
    drop(guard);
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(jira_msg(&v).contains("timed out"));
    tokio::time::sleep(Duration::from_millis(20)).await;
    let (s, _) = t.get("/rest/api/3/myself").await;
    assert_eq!(s, StatusCode::OK);
}

// ── workspaces and persistence ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn several_workspaces_by_prefix() {
    let t = setup_with(quiet(), &["alpha", "beta"]);
    let (s, v) = t.get("/rest/api/3/myself").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(jira_msg(&v).contains("/w/{name}"));
    let (s, v) = t.send("POST", "/w/alpha/rest/api/3/issue", json!({"fields": {"project": {"key": "OPS"}, "summary": "Only in alpha"}})).await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(v["self"].as_str().unwrap().starts_with("http://track.test/w/alpha/rest/api/3/issue/"));
    let key = v["key"].as_str().unwrap().to_string();
    let (s, _) = t.get(&format!("/w/alpha/rest/api/3/issue/{key}")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = t.get(&format!("/w/beta/rest/api/3/issue/{key}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, v) = t.get("/w/gamma/wiki/api/v2/spaces").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(v["errors"][0]["title"].as_str().unwrap().contains("gamma"));
    t.server.hub.set_default(Some("beta"));
    let (s, _) = t.get("/rest/api/3/myself").await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = t.get("/w/beta/wiki/api/v2/pages?limit=1").await;
    assert_eq!(s, StatusCode::OK);
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("trackcraft-server-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn persistence_round_trip_and_debounced_saver() {
    let dir = temp_dir("persist");
    workspaces::create(&dir, "team", true, None).unwrap();
    assert!(workspaces::create(&dir, "team", true, None).is_err(), "never overwrites");
    assert!(workspaces::create(&dir, "Bad Name", false, None).is_err());
    workspaces::create(&dir, "empty", false, Some("ann")).unwrap();
    let (_, token) = mint_token(&dir, "riley", None, "test").unwrap();
    let mut cfg = Config::load(&dir).unwrap();
    cfg.default_workspace = Some("team".into());
    cfg.save(&dir).unwrap();

    let server = Server::open(&dir, Options { log_requests: false, save_debounce: Duration::from_millis(10), ..Options::default() }).unwrap();
    assert_eq!(server.hub.names(), vec!["empty".to_string(), "team".to_string()]);
    let t = T { router: server.router(), server: server.clone(), events: Arc::new(MemorySink::default()) };
    let call = |m: &'static str, p: String, b: Value| {
        let t = &t;
        let token = token.clone();
        async move { t.req(m, &p, Auth::Bearer(&token), Some(Body::from(b.to_string()))).await }
    };
    let (s, v, _) = call("POST", "/rest/api/3/issue".into(), json!({"fields": {"project": {"key": "LUME"}, "summary": "Survives a restart"}})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let key = v["key"].as_str().unwrap().to_string();
    // riley isn't a member of "empty" (only ann is).
    let (s, _, _) = call("GET", "/w/empty/rest/api/3/project".into(), Value::Null).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(server.flush().unwrap(), 1);
    assert_eq!(server.flush().unwrap(), 0, "nothing changed since");

    let reopened = Server::open(&dir, Options { log_requests: false, ..Options::default() }).unwrap();
    let h = reopened.hub.get("team").unwrap();
    assert_eq!(h.lock().ws.issue(&key).unwrap().summary, "Survives a restart");

    // The debounced saver writes without an explicit flush.
    let saver = server.spawn_saver();
    let (s, _, _) = call("PUT", format!("/rest/api/3/issue/{key}"), json!({"fields": {"summary": "Saved by the saver"}})).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let path = workspaces::workspace_path(&dir, "team");
    let mut saved = false;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        if std::fs::read_to_string(&path).unwrap_or_default().contains("Saved by the saver") {
            saved = true;
            break;
        }
    }
    saver.abort();
    assert!(saved, "the saver wrote the change");
    assert!(!dir.join(".team.track.saving").exists(), "no temp file left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bad_config_refuses_to_start() {
    let dir = temp_dir("badcfg");
    std::fs::write(dir.join(config::CONFIG_FILE), "{oops").unwrap();
    assert!(Server::open(&dir, quiet()).is_err());
    std::fs::write(dir.join(config::CONFIG_FILE), r#"{"defaultWorkspace": "missing"}"#).unwrap();
    assert!(Server::open(&dir, quiet()).is_err());
    std::fs::write(dir.join(config::CONFIG_FILE), "{}").unwrap();
    std::fs::write(dir.join("broken.track"), "not a workspace").unwrap();
    assert!(Server::open(&dir, quiet()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ── a Jira-style client, end to end ───────────────────────────────────────────────────────────

#[tokio::test]
async fn jira_client_flow_end_to_end() {
    let t = setup();
    let auth = || Auth::Basic("riley@example.com", TOKEN);
    let send = |m: &'static str, p: String, b: Value| {
        let t = &t;
        async move { t.req(m, &p, auth(), Some(Body::from(b.to_string()))).await }
    };
    // 1. Who am I, which projects?
    let (s, me, _) = t.req("GET", "/rest/api/3/myself", auth(), None).await;
    assert_eq!((s, me["accountId"].as_str()), (StatusCode::OK, Some("riley")));
    // 2. Create an issue.
    let (s, created, _) = send("POST", "/rest/api/3/issue".into(), json!({"fields": {"project": {"key": "LUME"}, "issuetype": {"name": "Task"}, "summary": "Client flow", "assignee": {"accountId": me["accountId"]}}})).await;
    assert_eq!(s, StatusCode::CREATED);
    let key = created["key"].as_str().unwrap().to_string();
    // 3. Find the "In Progress" transition and take it.
    let (_, tr, _) = t.req("GET", &format!("/rest/api/3/issue/{key}/transitions"), auth(), None).await;
    let id = tr["transitions"].as_array().unwrap().iter().find(|x| x["name"] == "In Progress").unwrap()["id"].clone();
    let (s, _, _) = send("POST", format!("/rest/api/3/issue/{key}/transitions"), json!({"transition": {"id": id}})).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // 4. Comment.
    let body = json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Started"}]}]});
    let (s, _, _) = send("POST", format!("/rest/api/3/issue/{key}/comment"), json!({"body": body})).await;
    assert_eq!(s, StatusCode::CREATED);
    // 5. Search finds it with the new status and the comment.
    let (s, found, _) = send(
        "POST",
        "/rest/api/3/search/jql".into(),
        json!({"jql": "project = LUME AND status = \"In Progress\" AND assignee = currentUser() ORDER BY created DESC", "fields": ["summary", "status", "comment"], "maxResults": 10}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{found}");
    let hit = found["issues"].as_array().unwrap().iter().find(|i| i["key"] == key.as_str()).unwrap();
    assert_eq!(hit["fields"]["status"]["name"], "In Progress");
    assert_eq!(hit["fields"]["comment"]["comments"][0]["body"]["content"][0]["content"][0]["text"], "Started");
    // 6. The changelog tells the story.
    let (_, log, _) = t.req("GET", &format!("/rest/api/3/issue/{key}/changelog"), auth(), None).await;
    assert!(
        log["values"].as_array().unwrap().iter().any(|g| g["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["field"] == "status" && i["toString"] == "In Progress"))
    );
}
