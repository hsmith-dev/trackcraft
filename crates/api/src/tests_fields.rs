//! M2.4: REST shapes for components, versions, the field list and custom fields on issues.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use trackcraft_engine::Session;

use crate::{ApiState, Credential, Hosted, Hub, Identity, IdentityProvider, LocalSyncHub, NullSink, router};

struct Users;
impl IdentityProvider for Users {
    fn authenticate(&self, c: &Credential<'_>) -> Option<Identity> {
        match c {
            Credential::Bearer(t) => Some(Identity { user: (*t).to_string() }),
            Credential::Basic { .. } => None,
        }
    }
}

fn app() -> (axum::Router, Arc<Hub>) {
    let hub = Arc::new(Hub::new());
    let mut s = Session::sample();
    s.user = "riley".into();
    for (c, p) in [
        ("field.create", json!({"project": "LUME", "name": "Severity", "type": "select", "options": ["Minor", "Major"]})),
        ("field.create", json!({"project": "LUME", "name": "Customer", "type": "text"})),
        ("field.create", json!({"project": "LUME", "name": "Reviewer", "type": "user"})),
        ("component.create", json!({"project": "LUME", "name": "Sync", "lead": "sam"})),
        ("version.create", json!({"project": "LUME", "name": "1.2", "releaseDate": "2026-11-20"})),
        ("version.create", json!({"project": "LUME", "name": "1.3"})),
        ("field.create", json!({"project": "OPS", "name": "Room", "type": "text"})),
        ("access.setProjectAccess", json!({"project": "OPS", "access": "private"})),
    ] {
        s.run(c, &p).unwrap();
    }
    hub.add(Hosted::new("lab", s, None));
    hub.set_default(Some("lab"));
    let state =
        ApiState { hub: hub.clone(), identity: Arc::new(Users), events: Arc::new(NullSink), sync: Arc::new(LocalSyncHub::new()), base_url: None };
    (router(state), hub)
}

async fn call(r: &axum::Router, user: &str, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "track.test")
        .header("authorization", format!("Bearer {user}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let resp = r.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

fn field_id(list: &Value, name: &str) -> String {
    list.as_array().unwrap().iter().find(|f| f["name"] == name).unwrap()["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn components_versions_and_fields() {
    let (r, _) = app();
    let (st, comps) = call(&r, "riley", "GET", "/rest/api/3/project/LUME/components", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(comps[0]["name"], "Sync");
    assert_eq!(comps[0]["lead"]["accountId"], "sam");
    assert!(comps[0]["self"].as_str().unwrap().contains("/rest/api/3/component/"));
    let (_, vers) = call(&r, "riley", "GET", "/rest/api/3/project/LUME/versions", None).await;
    assert_eq!(
        (vers[0]["name"].as_str(), vers[0]["releaseDate"].as_str(), vers[0]["released"].as_bool()),
        (Some("1.2"), Some("2026-11-20"), Some(false))
    );
    assert!(vers[1].get("releaseDate").is_none());
    assert_eq!(call(&r, "riley", "GET", "/rest/api/3/project/NOPE/versions", None).await.0, StatusCode::NOT_FOUND);
    let (_, fields) = call(&r, "riley", "GET", "/rest/api/3/field", None).await;
    let sev = field_id(&fields, "Severity");
    assert!(sev.starts_with("customfield_"));
    let f = fields.as_array().unwrap().iter().find(|f| f["id"] == sev.as_str()).unwrap();
    assert_eq!((f["custom"].as_bool(), f["schema"]["type"].as_str()), (Some(true), Some("option")));
    assert_eq!(f["clauseNames"][0], format!("cf[{}]", sev.trim_start_matches("customfield_")));
    assert!(fields.as_array().unwrap().iter().any(|f| f["id"] == "summary" && f["custom"] == false));
    // sam can't see OPS: its fields aren't listed, its components 404.
    let (_, fields) = call(&r, "sam", "GET", "/rest/api/3/field", None).await;
    assert!(fields.as_array().unwrap().iter().all(|f| f["name"] != "Room"));
    assert_eq!(call(&r, "sam", "GET", "/rest/api/3/project/OPS/components", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn issue_create_get_and_edit_with_fields() {
    let (r, hub) = app();
    let (_, fields) = call(&r, "riley", "GET", "/rest/api/3/field", None).await;
    let (sev, cust, rev) = (field_id(&fields, "Severity"), field_id(&fields, "Customer"), field_id(&fields, "Reviewer"));
    let body = json!({"fields": {"project": {"key": "LUME"}, "summary": "REST fields", "issuetype": {"name": "Task"},
        "components": [{"name": "Sync"}], "fixVersions": [{"name": "1.2"}], "versions": [{"name": "1.3"}],
        sev.clone(): {"value": "Major"}, cust.clone(): "Acme", rev.clone(): {"accountId": "jo"}, "customfield_99999": 3}});
    let (st, created) = call(&r, "riley", "POST", "/rest/api/3/issue", Some(body)).await;
    assert_eq!(st, StatusCode::CREATED, "{created}");
    let key = created["key"].as_str().unwrap().to_string();
    let (_, i) = call(&r, "riley", "GET", &format!("/rest/api/3/issue/{key}"), None).await;
    let f = &i["fields"];
    assert_eq!(f["components"][0]["name"], "Sync");
    assert_eq!(f["fixVersions"][0]["name"], "1.2");
    assert_eq!(f["versions"][0]["name"], "1.3");
    assert_eq!(f[&sev], json!({"value": "Major"}));
    assert_eq!(f[&cust], "Acme");
    assert_eq!(f[&rev]["accountId"], "jo");
    // fields= picks custom fields too.
    let (_, i) = call(&r, "riley", "GET", &format!("/rest/api/3/issue/{key}?fields=summary,{sev}"), None).await;
    assert_eq!(i["fields"].as_object().unwrap().len(), 2);
    // PUT: change and clear.
    let put = json!({"fields": {sev.clone(): {"value": "Minor"}, cust.clone(): null, "components": [], "fixVersions": [{"name": "1.3"}]}});
    let (st, _) = call(&r, "riley", "PUT", &format!("/rest/api/3/issue/{key}"), Some(put)).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, i) = call(&r, "riley", "GET", &format!("/rest/api/3/issue/{key}"), None).await;
    let f = &i["fields"];
    assert_eq!((f[&sev]["value"].as_str(), f[&cust].is_null(), f["components"].as_array().map(Vec::len)), (Some("Minor"), true, Some(0)));
    assert_eq!(f["fixVersions"][0]["name"], "1.3");
    // A bad option is a 400 and changes nothing.
    let before = hub.get("lab").unwrap().lock().ws.clone();
    let (st, e) = call(&r, "riley", "PUT", &format!("/rest/api/3/issue/{key}"), Some(json!({"fields": {sev.clone(): {"value": "Nope"}}}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    assert!(hub.get("lab").unwrap().lock().ws == before);
    // JQL over REST.
    let (_, s) = call(&r, "riley", "GET", "/rest/api/3/search/jql?jql=Severity%20%3D%20Minor%20AND%20fixVersion%20%3D%201.3", None).await;
    assert_eq!(s["issues"].as_array().unwrap().len(), 1);
}
