//! M2.4: custom fields, components and versions — commands, issue values, JQL, permissions,
//! sync scope and replay, hostile params, and the catalog floor.

use serde_json::{Value, json};

use crate::authz::{Delivery, RULED, scope};
use crate::sync::Journal;
use crate::{Session, catalog, command};

fn fixed_now() -> trackcraft_model::Timestamp {
    chrono::DateTime::from_timestamp(1_793_448_000, 0).unwrap_or_default() // 2026-10-31T12:00:00Z
}

fn session() -> Session {
    Session::new(crate::sample::workspace(fixed_now()), "riley").with_clock(fixed_now)
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.run(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn keys(s: &mut Session, jql: &str) -> Vec<String> {
    let r = run(s, "search.jql", json!({"jql": jql}));
    r["issues"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap().to_string()).collect()
}

/// LUME with a select, a number, a date, a user and a multi-select field, two components and
/// two versions.
fn setup(s: &mut Session) -> (u64, u64) {
    let sev = run(s, "field.create", json!({"project": "LUME", "name": "Severity", "type": "select", "options": ["Minor", "Major", "Critical"]}));
    run(s, "field.create", json!({"project": "LUME", "name": "Cost", "type": "number"}));
    run(s, "field.create", json!({"project": "LUME", "name": "Due on", "type": "date"}));
    run(s, "field.create", json!({"project": "LUME", "name": "Customer", "type": "text"}));
    run(s, "field.create", json!({"project": "LUME", "name": "Reviewer", "type": "user"}));
    run(s, "field.create", json!({"project": "LUME", "name": "Platforms", "type": "multiSelect", "options": ["iOS", "Android"]}));
    run(s, "component.create", json!({"project": "LUME", "name": "Sync", "lead": "sam"}));
    run(s, "component.create", json!({"project": "LUME", "name": "Editor"}));
    let v1 = run(s, "version.create", json!({"project": "LUME", "name": "1.2", "releaseDate": "2026-11-20"}));
    run(s, "version.create", json!({"project": "LUME", "name": "1.3"}));
    (sev["id"].as_u64().unwrap(), v1["id"].as_str().unwrap().parse().unwrap())
}

#[test]
fn catalog_floor_with_fields() {
    for id in [
        "field.list",
        "field.create",
        "field.edit",
        "field.delete",
        "component.list",
        "component.create",
        "component.edit",
        "component.delete",
        "version.list",
        "version.create",
        "version.edit",
        "version.delete",
        "version.release",
    ] {
        assert!(command(id).is_some(), "{id} missing");
    }
    for row in ["Components", "Versions / releases", "Custom fields"] {
        assert!(catalog::FEATURES.iter().any(|f| f.name == row && !f.command.is_empty()), "{row} not mapped");
    }
    let (done, total) = catalog::coverage();
    assert!(done >= 78, "catalog coverage fell to {done}/{total}");
}

#[test]
fn create_edit_and_get_issue_values() {
    let mut s = session();
    let (sev, v1) = setup(&mut s);
    let k = run(
        &mut s,
        "issue.create",
        json!({"project": "LUME", "summary": "Typed fields", "custom": {"Severity": "major", "Cost": 12.5, "Due on": "2026-12-01", "Reviewer": "jo", "Platforms": ["iOS"]},
               "components": ["Sync", "editor"], "fixVersions": ["1.2"], "affectsVersions": [v1]}),
    )["key"]
        .as_str()
        .unwrap()
        .to_string();
    let i = run(&mut s, "issue.get", json!({"key": k}));
    assert_eq!(i[format!("customfield_{sev}")], json!({"value": "Major"}));
    assert_eq!(i["components"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Sync", "Editor"]);
    assert_eq!(i["fixVersions"][0]["name"], "1.2");
    assert_eq!(i["versions"][0]["name"], "1.2");
    assert_eq!(i["names"][format!("customfield_{sev}")], "Severity");
    // Creating the issue isn't a value change: only "created" in its history.
    assert_eq!(run(&mut s, "issue.history", json!({"key": k})).as_array().unwrap().len(), 1);
    // Edit by name, by customfield id, and REST shapes; one history group per command.
    run(
        &mut s,
        "issue.edit",
        json!({"key": k, "custom": {format!("customfield_{sev}"): {"value": "Critical"}, "Customer": "Acme", "Cost": null}, "components": [], "fixVersions": [{"name": "1.3"}]}),
    );
    let i = run(&mut s, "issue.get", json!({"key": k}));
    assert_eq!(i[format!("customfield_{sev}")]["value"], "Critical");
    assert!(i["components"].as_array().unwrap().is_empty());
    assert_eq!(i["fixVersions"][0]["name"], "1.3");
    let h = run(&mut s, "issue.history", json!({"key": k}));
    let fields: Vec<&str> = h[0]["items"].as_array().unwrap().iter().map(|x| x["field"].as_str().unwrap()).collect();
    assert_eq!(fields, ["Cost", "Customer", "Severity", "Component", "Fix Version"], "custom in key order, then the rest");
    // Validation per kind; a failed edit changes nothing.
    for bad in [
        json!({"Severity": "Blocker"}),
        json!({"Cost": "lots"}),
        json!({"Cost": 1e300}),
        json!({"Due on": "2026-02-30"}),
        json!({"Reviewer": "ghost"}),
        json!({"Platforms": ["Windows"]}),
        json!({"Nope": 1}),
        json!({"Customer": 5}),
    ] {
        let before = s.ws.clone();
        assert!(s.run("issue.edit", &json!({"key": k, "custom": bad})).is_err(), "{bad} accepted");
        assert!(s.ws == before);
    }
    assert!(s.run("issue.edit", &json!({"key": k, "components": ["Ghost"]})).is_err());
    assert!(s.run("issue.edit", &json!({"key": "OPS-1", "components": ["Sync"]})).is_err(), "another project's component");
}

#[test]
fn required_fields_on_create() {
    let mut s = session();
    run(&mut s, "field.create", json!({"project": "LUME", "name": "Customer", "type": "text", "required": true}));
    let e = s.run("issue.create", &json!({"project": "LUME", "summary": "x"})).unwrap_err();
    assert!(e.contains("Customer"), "{e}");
    let n = s.ws.issues.len();
    assert_eq!(s.ws.issues.len(), n);
    run(&mut s, "issue.create", json!({"project": "LUME", "summary": "x", "custom": {"customer": "Acme"}}));
    run(&mut s, "issue.create", json!({"project": "OPS", "summary": "other projects aren't affected"}));
    let k = format!("LUME-{}", s.ws.issues.keys().filter(|k| k.project.as_str() == "LUME").map(|k| k.number).max().unwrap());
    assert!(s.run("issue.edit", &json!({"key": k, "custom": {"Customer": null}})).is_err(), "required can't be cleared");
}

#[test]
fn jql_over_fields_components_and_versions() {
    let mut s = session();
    let (sev, v1) = setup(&mut s);
    run(
        &mut s,
        "issue.edit",
        json!({"key": "LUME-2", "custom": {"Severity": "Critical", "Cost": 40, "Reviewer": "riley", "Customer": "Acme Corp"}, "components": ["Sync"], "fixVersions": ["1.2"]}),
    );
    run(
        &mut s,
        "issue.edit",
        json!({"key": "LUME-3", "custom": {"Severity": "Minor", "Cost": 3, "Due on": "2026-11-05"}, "fixVersions": ["1.3"], "affectsVersions": ["1.2"]}),
    );
    run(&mut s, "issue.edit", json!({"key": "LUME-4", "custom": {"Severity": "Major"}, "components": ["Editor"]}));
    assert_eq!(keys(&mut s, "component = Sync"), ["LUME-2"]);
    assert_eq!(keys(&mut s, "fixVersion = \"1.2\""), ["LUME-2"]);
    assert_eq!(keys(&mut s, "affectedVersion = \"1.2\""), ["LUME-3"]);
    assert_eq!(keys(&mut s, "\"Customer\" ~ Acme"), ["LUME-2"]);
    assert_eq!(keys(&mut s, "Customer = \"Acme Corp\""), ["LUME-2"]);
    assert_eq!(keys(&mut s, &format!("cf[{sev}] = Minor")), ["LUME-3"]);
    assert_eq!(keys(&mut s, "Severity IN (Major, Critical) ORDER BY key"), ["LUME-2", "LUME-4"]);
    assert_eq!(keys(&mut s, "Cost >= 10"), ["LUME-2"]);
    assert_eq!(keys(&mut s, "\"Due on\" < 2026-12-01"), ["LUME-3"]);
    assert_eq!(keys(&mut s, "Reviewer = currentUser()"), ["LUME-2"]);
    assert_eq!(keys(&mut s, "project = LUME AND Severity IS NOT EMPTY ORDER BY Severity DESC"), ["LUME-2", "LUME-4", "LUME-3"]);
    assert_eq!(keys(&mut s, "project = LUME AND Cost IS NOT EMPTY ORDER BY Cost"), ["LUME-3", "LUME-2"]);
    run(&mut s, "version.release", json!({"id": v1}));
    assert_eq!(keys(&mut s, "fixVersion IN releasedVersions()"), ["LUME-2"]);
    assert_eq!(keys(&mut s, "fixVersion IN unreleasedVersions()"), ["LUME-3"]);
    assert_eq!(keys(&mut s, "fixVersion IN unreleasedVersions(LUME) OR fixVersion IN releasedVersions(LUME) ORDER BY key"), ["LUME-2", "LUME-3"]);
}

#[test]
fn definitions_lists_edits_and_cascading_deletes() {
    let mut s = session();
    let (sev, v1) = setup(&mut s);
    run(&mut s, "issue.edit", json!({"key": "LUME-2", "custom": {"Severity": "Major"}, "components": ["Sync"], "fixVersions": ["1.2"]}));
    run(&mut s, "issue.transition", json!({"key": "LUME-2", "to": "Done"}));
    run(&mut s, "issue.edit", json!({"key": "LUME-3", "fixVersions": ["1.2"]}));
    let fields = run(&mut s, "field.list", json!({"project": "LUME"}));
    assert_eq!(fields.as_array().unwrap().len(), 6);
    assert_eq!(fields[0]["clauseNames"][0], format!("cf[{sev}]"));
    let comps = run(&mut s, "component.list", json!({"project": "LUME"}));
    assert_eq!((comps[0]["name"].as_str(), comps[0]["lead"].as_str(), comps[0]["issueCount"].as_u64()), (Some("Sync"), Some("sam"), Some(1)));
    let vers = run(&mut s, "version.list", json!({"project": "LUME"}));
    assert_eq!(vers[0]["issues"], json!({"total": 2, "done": 1}));
    assert_eq!(vers[0]["releaseDate"], "2026-11-20");
    // Edits.
    run(&mut s, "field.edit", json!({"id": sev, "name": "Impact", "options": ["Minor", "Critical"]}));
    assert!(s.ws.issue("LUME-2").unwrap().custom.is_empty(), "the removed option's value is gone");
    run(&mut s, "component.edit", json!({"id": comps[0]["id"], "name": "Sync engine", "lead": null}));
    run(&mut s, "version.edit", json!({"id": v1, "name": "1.2.0", "releaseDate": null, "archived": true}));
    let vers = run(&mut s, "version.list", json!({"project": "LUME"}));
    assert_eq!((vers[0]["name"].as_str(), vers[0]["releaseDate"].is_null(), vers[0]["archived"].as_bool()), (Some("1.2.0"), true, Some(true)));
    // Release with unresolved issues moved on.
    let next = vers[1]["id"].as_str().unwrap().to_string();
    let r = run(&mut s, "version.release", json!({"id": v1, "date": "2026-11-21", "moveUnresolvedTo": next}));
    assert_eq!(
        (r["moved"].as_u64(), r["version"]["released"].as_bool(), r["version"]["releaseDate"].as_str()),
        (Some(1), Some(true), Some("2026-11-21"))
    );
    assert!(s.run("version.release", &json!({"id": v1})).is_err(), "already released");
    // Deletes cascade, recorded on the issues.
    let d = run(&mut s, "component.delete", json!({"id": comps[0]["id"]}));
    assert_eq!(d["issues"], 1);
    assert!(s.ws.issue("LUME-2").unwrap().components.is_empty());
    run(&mut s, "version.delete", json!({"id": v1}));
    assert!(s.ws.issue("LUME-2").unwrap().fix_versions.is_empty());
    let h = run(&mut s, "issue.history", json!({"key": "LUME-2"}));
    assert_eq!(h[0]["items"][0]["field"], "Fix Version");
    run(&mut s, "field.delete", json!({"id": sev}));
    assert_eq!(run(&mut s, "field.list", json!({"project": "LUME"})).as_array().unwrap().len(), 5);
    // Undo restores the field with one step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "field.list", json!({"project": "LUME"})).as_array().unwrap().len(), 6);
    assert!(s.run("field.delete", &json!({"id": 999_999})).is_err());
}

#[test]
fn permissions_for_definitions_and_values() {
    let mut s = session();
    setup(&mut s);
    s.set_enforce(true);
    // sam is a LUME member: values yes, definitions no.
    s.user = "sam".into();
    let e = s.run("field.create", &json!({"project": "LUME", "name": "X", "type": "text"})).unwrap_err();
    assert!(e.starts_with(crate::authz::DENIED), "{e}");
    let id = s.ws.project_fields(&trackcraft_core::ProjectKey::parse("LUME").unwrap()).unwrap().versions[0].id.0;
    assert!(s.run("version.release", &json!({"id": id})).unwrap_err().starts_with(crate::authz::DENIED));
    assert!(s.run("component.create", &json!({"project": "LUME", "name": "Y"})).is_err());
    run(&mut s, "issue.edit", json!({"key": "LUME-2", "custom": {"Severity": "Minor"}, "components": ["Sync"]}));
    // jo administers OPS: definitions there.
    s.user = "jo".into();
    run(&mut s, "field.create", json!({"project": "OPS", "name": "Room", "type": "text"}));
    run(&mut s, "version.create", json!({"project": "OPS", "name": "Q4"}));
    // A private project's definitions read as missing to non-members.
    s.user = "riley".into();
    run(&mut s, "access.setProjectAccess", json!({"project": "OPS", "access": "private"}));
    let ops_field = s.ws.project_fields(&trackcraft_core::ProjectKey::parse("OPS").unwrap()).unwrap().fields[0].id.0;
    s.user = "sam".into();
    let e = s.run("field.delete", &json!({"id": ops_field})).unwrap_err();
    assert!(e.contains("no field"), "{e}");
    assert!(s.run("field.list", &json!({"project": "OPS"})).is_err(), "hidden project");
    // The view leaves out hidden projects' definitions.
    assert!(s.ws.view_for("sam").unwrap().fields.keys().all(|k| k.as_str() != "OPS"));
    for id in RULED.iter().filter(|c| c.starts_with("field.") || c.starts_with("component.") || c.starts_with("version.")) {
        assert!(command(id).is_some_and(|c| c.mutates), "{id}");
    }
}

#[test]
fn scope_names_the_owning_project() {
    let mut s = session();
    let (sev, v1) = setup(&mut s);
    let lume = trackcraft_core::ProjectKey::parse("LUME").unwrap();
    for (cmd, p) in [
        ("field.create", json!({"project": "lume", "name": "N", "type": "text"})),
        ("field.edit", json!({"id": sev, "name": "N"})),
        ("field.delete", json!({"id": sev})),
        ("version.release", json!({"id": v1})),
        ("component.create", json!({"project": "LUME", "name": "C"})),
    ] {
        let sc = scope(&s.ws, cmd, &p);
        assert_eq!((sc.projects.clone(), sc.unresolved), (vec![lume.clone()], false), "{cmd}");
        assert_eq!(s.delivery("sam", &sc), Delivery::Send);
    }
    assert!(scope(&s.ws, "field.delete", &json!({"id": 999_999})).unresolved);
    assert!(scope(&s.ws, "version.edit", &json!({})).unresolved);
}

#[test]
fn field_commands_replay_deterministically() {
    let mut a = session();
    a.enable_journal(Journal::new(1000, 1 << 20));
    let mut b = session();
    b.set_remote(true);
    let (sev, v1) = setup(&mut a);
    run(
        &mut a,
        "issue.edit",
        json!({"key": "LUME-2", "custom": {"Severity": "Major", "Due on": "2026-12-24"}, "components": ["Sync"], "fixVersions": ["1.2"]}),
    );
    run(&mut a, "issue.create", json!({"project": "LUME", "summary": "n", "custom": {"Cost": 2}, "affectsVersions": ["1.3"]}));
    run(&mut a, "version.release", json!({"id": v1}));
    run(&mut a, "field.edit", json!({"id": sev, "options": ["Major"]}));
    let comp = a.ws.project_fields(&trackcraft_core::ProjectKey::parse("LUME").unwrap()).unwrap().components[0].id.0;
    run(&mut a, "component.delete", json!({"id": comp}));
    for e in a.journal_since(0, 1000).unwrap() {
        b.apply_remote(&e).unwrap();
    }
    assert!(a.ws == b.ws, "replica diverged");
}

#[test]
fn hostile_field_params() {
    let junk = [
        json!({"project": "LUME", "name": "x", "type": "select", "options": vec!["o"; 10_000]}),
        json!({"project": "LUME", "name": "x", "type": "select", "options": "a,b"}),
        json!({"project": "LUME", "name": "\u{0}", "type": "nope", "required": "yes"}),
        json!({"project": "LUME", "name": "status", "type": "text"}),
        json!({"id": u64::MAX, "name": [], "options": {}, "lead": 3, "startDate": "9999-99-99", "releaseDate": 7}),
        json!({"id": "x", "date": "2026-02-30", "moveUnresolvedTo": "y"}),
        json!({"key": "LUME-1", "custom": {"x".repeat(10_000): 1}}),
        json!({"key": "LUME-1", "custom": [1, 2], "components": {}, "fixVersions": 7.5}),
        json!({"key": "LUME-1", "components": vec!["Sync"; 5_000], "fixVersions": [{"id": []}], "affectsVersions": [null]}),
        json!({"project": "LUME", "summary": "s", "custom": {"Severity": {"value": 3}}, "components": [{"name": "Sync"}], "versions": ["1.2"]}),
        json!({"project": "LUME", "summary": "s", "custom": {"Platforms": vec!["iOS"; 5_000]}}),
        json!({"project": "LUME", "name": "1.9", "startDate": "2026-12-01", "releaseDate": "2026-11-01"}),
    ];
    let ids = [
        "field.create",
        "field.edit",
        "field.delete",
        "component.create",
        "component.edit",
        "component.delete",
        "version.create",
        "version.edit",
        "version.delete",
        "version.release",
        "issue.create",
        "issue.edit",
    ];
    for id in ids {
        for p in &junk {
            let mut s = session();
            setup(&mut s);
            let before = s.ws.clone();
            if let Err(e) = s.run(id, p) {
                assert!(!e.starts_with("internal error"), "{id} panicked on {p}: {e}");
                assert!(s.ws == before, "{id} changed the workspace on error ({e})");
            }
        }
    }
}
