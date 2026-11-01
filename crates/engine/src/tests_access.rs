//! M4.3: permissions — the role × command matrix, filtered queries, admin commands, last-admin
//! protection, hostile params, and per-user sync delivery that keeps a filtered replica exact.

use serde_json::{Value, json};
use trackcraft_model::Workspace;

use crate::authz::{DENIED, Delivery, RULED};
use crate::sync::{Journal, skip_entry};
use crate::{LOCAL_ONLY, Session, commands};

fn fixed_now() -> trackcraft_model::Timestamp {
    chrono::DateTime::from_timestamp(1_793_448_000, 0).unwrap_or_default()
}

/// The sample workspace plus: `cy` (workspace viewer), `dee` (member, viewer in LUME), a
/// private project SEC (only `dee` is a member of it), a page restricted to riley and sam.
struct Fx {
    s: Session,
    restricted: u64,
    child: u64,
    open_page: u64,
}

fn as_user<'a>(s: &'a mut Session, user: &str) -> &'a mut Session {
    s.user = user.to_string();
    s
}

fn ok(s: &mut Session, user: &str, id: &str, p: Value) -> Value {
    as_user(s, user).run(id, &p).unwrap_or_else(|e| panic!("{user} {id}: {e}"))
}

fn fx() -> Fx {
    let ws = crate::sample::workspace(fixed_now());
    let mut s = Session::new(ws, "riley").with_clock(fixed_now);
    ok(&mut s, "riley", "user.add", json!({"id": "cy", "name": "Cy"}));
    ok(&mut s, "riley", "user.add", json!({"id": "dee", "name": "Dee"}));
    ok(&mut s, "riley", "access.setWorkspaceRole", json!({"user": "cy", "role": "viewer"}));
    ok(&mut s, "riley", "access.setProjectRole", json!({"project": "LUME", "user": "dee", "role": "viewer"}));
    ok(&mut s, "riley", "project.create", json!({"key": "SEC", "name": "Security"}));
    ok(&mut s, "riley", "issue.create", json!({"project": "SEC", "summary": "Rotate keys"}));
    ok(&mut s, "riley", "access.setProjectAccess", json!({"project": "SEC", "access": "private"}));
    ok(&mut s, "riley", "access.setProjectRole", json!({"project": "SEC", "user": "dee", "role": "member"}));
    ok(&mut s, "riley", "issue.create", json!({"project": "LUME", "summary": "Dee's report", "assignee": "dee"}));
    let restricted = ok(&mut s, "riley", "page.create", json!({"space": "ENG", "title": "Salaries", "body": "secret words"}))["id"].as_u64().unwrap();
    let child = ok(&mut s, "riley", "page.create", json!({"space": "ENG", "title": "Bands", "parent": restricted}))["id"].as_u64().unwrap();
    let open_page =
        ok(&mut s, "riley", "page.create", json!({"space": "ENG", "title": "Open notes", "body": "secret words too"}))["id"].as_u64().unwrap();
    ok(&mut s, "riley", "page.restrict", json!({"id": restricted, "view": ["riley", "sam"]}));
    s.set_enforce(true);
    Fx { s, restricted, child, open_page }
}

/// What a command does for a user: allowed, forbidden (403) or hidden (404-style message).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Out {
    Ok,
    Forbidden,
    Hidden,
}

fn check(s: &Session, user: &str, id: &str, p: &Value) -> Out {
    match s.authorize(user, id, p) {
        Ok(()) => Out::Ok,
        Err(crate::authz::Denied::Forbidden(_)) => Out::Forbidden,
        Err(crate::authz::Denied::Hidden(_)) => Out::Hidden,
    }
}

#[test]
fn role_by_command_matrix() {
    let f = fx();
    let new_key = "LUME-".to_string() + &f.s.ws.project("LUME").unwrap().next_number.saturating_sub(1).to_string();
    use Out::*;
    // riley = workspace admin, sam = member, cy = workspace viewer, dee = LUME viewer + SEC member.
    let cases: Vec<(&str, Value, [Out; 4])> = vec![
        ("issue.create", json!({"project": "LUME", "summary": "x"}), [Ok, Ok, Forbidden, Forbidden]),
        ("issue.create", json!({"project": "SEC", "summary": "x"}), [Ok, Hidden, Hidden, Ok]),
        ("issue.edit", json!({"key": "LUME-1", "summary": "x"}), [Ok, Ok, Forbidden, Forbidden]),
        // dee is the assignee of the newest LUME issue: may edit and transition it as a viewer.
        ("issue.edit", json!({"key": new_key, "summary": "x"}), [Ok, Ok, Forbidden, Ok]),
        ("issue.transition", json!({"key": new_key, "to": "Done"}), [Ok, Ok, Forbidden, Ok]),
        ("issue.assign", json!({"key": new_key, "assignee": null}), [Ok, Ok, Forbidden, Forbidden]),
        ("issue.comment", json!({"key": "LUME-1", "body": "hi"}), [Ok, Ok, Ok, Ok]),
        ("issue.comment", json!({"key": "SEC-1", "body": "hi"}), [Ok, Hidden, Hidden, Ok]),
        ("issue.delete", json!({"key": "LUME-1"}), [Ok, Ok, Forbidden, Forbidden]),
        ("issue.delete", json!({"key": new_key}), [Ok, Ok, Forbidden, Forbidden]),
        ("issue.link", json!({"key": "LUME-1", "type": "blocks", "target": "SEC-1"}), [Ok, Hidden, Forbidden, Forbidden]),
        ("issue.rank", json!({"keys": ["LUME-1"], "before": "LUME-2"}), [Ok, Ok, Forbidden, Forbidden]),
        ("sprint.create", json!({"project": "LUME"}), [Ok, Ok, Forbidden, Forbidden]),
        ("project.create", json!({"key": "NEW", "name": "New"}), [Ok, Ok, Forbidden, Ok]),
        ("page.edit", json!({"id": f.open_page, "body": "x"}), [Ok, Ok, Forbidden, Ok]),
        ("page.edit", json!({"id": f.restricted, "body": "x"}), [Ok, Ok, Hidden, Hidden]),
        ("page.edit", json!({"id": f.child, "body": "x"}), [Ok, Ok, Hidden, Hidden]),
        ("page.comment", json!({"id": f.open_page, "body": "x"}), [Ok, Ok, Ok, Ok]),
        ("page.create", json!({"space": "ENG", "title": "x", "parent": f.restricted}), [Ok, Ok, Forbidden, Hidden]),
        ("page.delete", json!({"id": f.open_page}), [Ok, Ok, Forbidden, Ok]),
        ("page.restrict", json!({"id": f.open_page, "edit": ["sam"]}), [Ok, Ok, Forbidden, Forbidden]),
        ("access.setWorkspaceRole", json!({"user": "sam", "role": "viewer"}), [Ok, Forbidden, Forbidden, Forbidden]),
        ("access.setProjectRole", json!({"project": "SEC", "user": "sam", "role": "viewer"}), [Ok, Hidden, Hidden, Forbidden]),
        ("access.setSpaceRole", json!({"space": "ENG", "user": "sam", "role": "viewer"}), [Ok, Forbidden, Forbidden, Forbidden]),
        ("workspace.rename", json!({"name": "x"}), [Ok, Forbidden, Forbidden, Forbidden]),
        ("user.edit", json!({"id": "cy", "name": "Cy Two"}), [Ok, Forbidden, Ok, Forbidden]),
        ("template.create", json!({"name": "T", "body": "b"}), [Ok, Forbidden, Forbidden, Forbidden]),
        // Queries are never refused (they are filtered instead).
        ("search.jql", json!({"jql": "project = SEC"}), [Ok, Ok, Ok, Ok]),
    ];
    for (id, p, want) in cases {
        for (user, w) in ["riley", "sam", "cy", "dee"].iter().zip(want) {
            assert_eq!(check(&f.s, user, id, &p), w, "{user} {id} {p}");
        }
    }
}

#[test]
fn run_enforces_and_denials_change_nothing() {
    let mut f = fx();
    let before = f.s.ws.clone();
    let e = as_user(&mut f.s, "cy").run("issue.create", &json!({"project": "LUME", "summary": "x"})).unwrap_err();
    assert!(e.starts_with(DENIED), "{e}");
    let e = as_user(&mut f.s, "sam").run("issue.comment", &json!({"key": "SEC-1", "body": "x"})).unwrap_err();
    assert_eq!(e, "no issue SEC-1", "hidden things read as missing");
    let e = as_user(&mut f.s, "cy").run("page.get", &json!({"id": f.restricted})).unwrap_err();
    assert_eq!(e, format!("no page with id {}", f.restricted));
    assert_eq!(f.s.ws, before);
    // Off (desktop): no checks.
    f.s.set_enforce(false);
    ok(&mut f.s, "cy", "issue.create", json!({"project": "LUME", "summary": "x"}));
}

#[test]
fn queries_are_filtered() {
    let mut f = fx();
    let keys = |v: &Value| v["issues"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let all = ok(&mut f.s, "riley", "search.jql", json!({"jql": "ORDER BY key", "maxResults": 1000}));
    assert!(keys(&all).contains(&"SEC-1".to_string()));
    let sam = ok(&mut f.s, "sam", "search.jql", json!({"jql": "ORDER BY key", "maxResults": 1000}));
    assert!(!keys(&sam).iter().any(|k| k.starts_with("SEC-")));
    assert_eq!(sam["total"].as_u64().unwrap() + 1, all["total"].as_u64().unwrap());
    let projects = ok(&mut f.s, "sam", "project.list", json!({}));
    assert!(!projects.as_array().unwrap().iter().any(|p| p["key"] == "SEC"));
    assert_eq!(as_user(&mut f.s, "sam").run("board.view", &json!({"project": "SEC"})).unwrap_err(), "no project with key SEC");
    assert!(as_user(&mut f.s, "dee").run("board.view", &json!({"project": "SEC"})).is_ok());
    // The page tree is pruned under a view restriction (and below it).
    let tree = |s: &mut Session, u: &str| ok(s, u, "space.tree", json!({"key": "ENG"})).to_string();
    let (r, c) = (format!("\"id\":{}", f.restricted), format!("\"id\":{}", f.child));
    let sam_tree = tree(&mut f.s, "sam");
    assert!(sam_tree.contains(&r) && sam_tree.contains(&c));
    let cy_tree = tree(&mut f.s, "cy");
    assert!(!cy_tree.contains(&r) && !cy_tree.contains(&c) && cy_tree.contains(&format!("\"id\":{}", f.open_page)));
    let hits = ok(&mut f.s, "cy", "page.search", json!({"text": "secret words"}));
    assert_eq!(hits.as_array().unwrap().len(), 1, "{hits}");
    // access.get shows only what the user can see.
    let a = ok(&mut f.s, "cy", "access.get", json!({}));
    assert_eq!(a["role"], "viewer");
    assert!(!a.to_string().contains("\"SEC\"") && a["pages"].as_array().unwrap().is_empty());
    let check = ok(&mut f.s, "cy", "access.check", json!({"command": "issue.create", "params": {"project": "LUME", "summary": "x"}}));
    assert_eq!(check["allowed"], false);
    // The full workspace is back after every filtered query.
    assert!(f.s.ws.projects.keys().any(|k| k.as_str() == "SEC"));
}

#[test]
fn last_admin_is_protected_and_roles_change() {
    let mut f = fx();
    let e = as_user(&mut f.s, "riley").run("access.setWorkspaceRole", &json!({"user": "riley", "role": "member"})).unwrap_err();
    assert!(e.contains("last workspace admin"), "{e}");
    ok(&mut f.s, "riley", "access.setWorkspaceRole", json!({"user": "sam", "role": "admin"}));
    ok(&mut f.s, "sam", "access.setWorkspaceRole", json!({"user": "riley", "role": "member"}));
    assert_eq!(f.s.ws.admin_count(), 1);
    // A member who creates a project administers it.
    ok(&mut f.s, "riley", "project.create", json!({"key": "RIL", "name": "Riley's"}));
    ok(&mut f.s, "riley", "access.setProjectAccess", json!({"project": "RIL", "access": "private"}));
    assert_eq!(
        f.s.authorize("jo", "issue.create", &json!({"project": "RIL", "summary": "x"})),
        Err(crate::authz::Denied::Hidden("no project with key RIL".into()))
    );
    // Page editors may restrict, but must keep themselves on the list.
    let e = as_user(&mut f.s, "jo").run("page.restrict", &json!({"id": f.open_page, "view": ["sam"]})).unwrap_err();
    assert!(e.starts_with(DENIED), "{e}");
    ok(&mut f.s, "jo", "page.restrict", json!({"id": f.open_page, "view": ["jo"], "edit": null}));
    assert!(!f.s.ws.page_visible("riley", trackcraft_core::Id(f.open_page)), "riley is no longer a workspace admin");
}

#[test]
fn every_mutating_command_has_a_rule() {
    for c in commands().iter().filter(|c| c.mutates && !LOCAL_ONLY.contains(&c.id)) {
        assert!(RULED.contains(&c.id), "{} has no permission rule in authz::authorize (unruled commands need a workspace admin)", c.id);
    }
    for id in RULED {
        assert!(crate::command(id).is_some(), "RULED lists unknown command {id}");
    }
}

#[test]
fn hostile_access_params_never_panic() {
    let junk = [
        Value::Null,
        json!([]),
        json!({"user": 5, "role": [], "project": {}, "space": 7, "access": null, "id": "x"}),
        json!({"user": "riley", "role": "root"}),
        json!({"user": "ghost", "role": "admin", "project": "NOPE", "space": "NOPE", "access": "secret"}),
        json!({"user": "sam", "role": null, "project": "LUME", "space": "ENG", "access": "private"}),
        json!({"id": u64::MAX, "view": "riley", "edit": 3}),
        json!({"id": 1, "view": vec!["sam"; 5_000], "edit": [1, 2]}),
        json!({"id": 1, "view": ["ghost"], "edit": []}),
        json!({"id": 1}),
        json!({"command": "nope.nope", "params": []}),
        json!({"command": "issue.edit", "params": {"key": ["x"]}, "user": 9}),
        json!({"command": "page.copy", "params": {"id": 1, "children": true, "space": "X".repeat(10_000)}}),
        json!({"key": "X".repeat(100_000), "keys": vec!["SEC-1"; 2_000], "target": {}, "parent": "SEC-1", "sprint": "x"}),
    ];
    for user in ["riley", "sam", "cy", "nobody"] {
        let mut f = fx();
        for c in commands() {
            if LOCAL_ONLY.contains(&c.id) {
                continue;
            }
            for p in &junk {
                let before = f.s.ws.clone();
                let r = as_user(&mut f.s, user).run(c.id, p);
                if r.is_err() {
                    assert_eq!(f.s.ws, before, "{user} {} {p} failed but changed the workspace", c.id);
                }
            }
        }
    }
}

/// A server with a journal and enforcement; a client replica for `user` that follows it entry
/// by entry with the per-user delivery rules. After every entry the replica must equal the
/// server's view for that user.
#[test]
fn filtered_replica_follows_the_journal() {
    let steps: Vec<(&str, &str, Value)> = vec![
        ("riley", "issue.comment", json!({"key": "SEC-1", "body": "hidden"})),
        ("sam", "issue.comment", json!({"key": "LUME-1", "body": "visible"})),
        ("dee", "issue.create", json!({"project": "SEC", "summary": "hidden too"})),
        ("riley", "page.edit", json!({"id": "RESTRICTED", "body": "new secret"})),
        ("riley", "issue.link", json!({"key": "LUME-1", "type": "blocks", "target": "SEC-1"})),
        ("sam", "issue.edit", json!({"key": "LUME-2", "summary": "renamed"})),
        ("riley", "page.create", json!({"space": "ENG", "title": "Structural"})),
        ("riley", "page.create", json!({"space": "TEAM", "title": "Elsewhere"})),
        ("riley", "user.add", json!({"id": "eve"})),
        ("riley", "access.setProjectAccess", json!({"project": "SEC", "access": "limited"})),
        ("sam", "issue.comment", json!({"key": "SEC-1", "body": "now visible"})),
        ("riley", "page.restrict", json!({"id": "RESTRICTED", "view": null})),
        ("riley", "page.edit", json!({"id": "RESTRICTED", "body": "public now"})),
    ];
    for user in ["sam", "cy", "dee", "riley"] {
        let mut f = fx();
        f.s.enable_journal(Journal::new(10_000, 64 << 20));
        let view = |ws: &Workspace| ws.view_for(user).unwrap_or_else(|| ws.clone());
        let mut client = Session::new(view(&f.s.ws), user);
        let mut seen = (0, 0, 0);
        for (actor, id, p) in &steps {
            let mut p = p.clone();
            if p["id"] == "RESTRICTED" {
                p["id"] = json!(f.restricted);
            }
            let before = f.s.sync_rev();
            as_user(&mut f.s, actor).run(id, &p).unwrap_or_else(|e| panic!("{actor} {id}: {e}"));
            for (e, meta) in f.s.journal_since_meta(before, 100).unwrap() {
                match f.s.delivery(user, &meta.scope) {
                    Delivery::Send => {
                        client.apply_remote(&e).unwrap_or_else(|err| panic!("{user}: {id}: {err}"));
                        seen.0 += 1;
                    }
                    Delivery::Skip => {
                        client.apply_remote(&skip_entry(&e, &meta)).unwrap();
                        seen.1 += 1;
                    }
                    Delivery::Resync => {
                        client.load_synced(view(&f.s.ws), e.rev);
                        seen.2 += 1;
                    }
                }
            }
            assert_eq!(client.ws, view(&f.s.ws), "{user} diverged after {actor} {id}");
            assert_eq!(client.sync_rev(), f.s.sync_rev());
        }
        if user == "riley" {
            assert_eq!(seen.1 + seen.2, 0, "admins get every entry as it is");
        } else {
            assert!(seen.1 > 0 && seen.2 > 0, "{user}: {seen:?}");
        }
    }
}

#[test]
fn skip_placeholders_are_checked() {
    let mut s = Session::new(Workspace::new("t"), "a");
    let e = trackcraft_sync_proto::Entry {
        rev: 1,
        command: crate::sync::SKIP_COMMAND.into(),
        params: json!({"idsAfter": 5}),
        actor: String::new(),
        at: fixed_now(),
        ids_before: 0,
        origin: None,
    };
    s.apply_remote(&e).unwrap();
    assert_eq!((s.sync_rev(), crate::sync::ids_next(&s.ws)), (1, 5));
    let back = trackcraft_sync_proto::Entry { rev: 2, params: json!({"idsAfter": 1}), ids_before: 5, ..e.clone() };
    assert!(s.apply_remote(&back).is_err(), "never backwards");
    let junk = trackcraft_sync_proto::Entry { rev: 2, params: json!({"idsAfter": "x"}), ids_before: 5, ..e };
    assert!(s.apply_remote(&junk).is_err());
}

/// M4.3 × M4.5: notifications and watches about hidden things must not reach a user's
/// filtered copy (titles and excerpts would leak), and visible ones must.
#[test]
fn filtered_view_hides_notifications_about_hidden_things() {
    let mut f = fx();
    // dee (a SEC member) is assigned a SEC issue by riley, and is mentioned on the restricted page.
    ok(&mut f.s, "riley", "issue.create", json!({"project": "SEC", "summary": "Audit the vault", "assignee": "dee"}));
    ok(&mut f.s, "riley", "page.comment", json!({"id": f.restricted, "body": "@sam please review the numbers"}));
    // sam sees LUME but not SEC; dee sees SEC but not the restricted page.
    let sam = f.s.ws.view_for("sam").expect("SEC is hidden from sam");
    let sam_text = serde_json::to_string(&sam.inbox).unwrap_or_default();
    assert!(!sam_text.contains("Audit the vault"), "sam's copy holds a notification about a hidden SEC issue");
    assert!(sam.inbox.watches.values().all(|w| w.issues.iter().all(|k| k.project.as_str() != "SEC")), "sam's copy lists watches on SEC issues");
    let dee = f.s.ws.view_for("dee").expect("the restricted page is hidden from dee");
    let dee_text = serde_json::to_string(&dee.inbox).unwrap_or_default();
    assert!(dee_text.contains("Audit the vault"), "dee should keep her own SEC notification");
    assert!(!dee_text.contains("please review the numbers"), "dee's copy holds a notification about a hidden page");
    assert!(dee.inbox.watches.values().all(|w| !w.pages.contains(&trackcraft_core::Id(f.restricted))), "dee's copy lists watches on a hidden page");
    // Counters are kept, so later visible notifications are numbered the same on every copy.
    assert_eq!(sam.inbox.next_id, f.s.ws.inbox.next_id);
}

#[test]
fn marking_read_is_allowed_for_everyone_and_sent_as_is() {
    let mut f = fx();
    for u in ["riley", "sam", "cy", "dee"] {
        assert!(as_user(&mut f.s, u).run("inbox.markRead", &json!({"all": true})).is_ok(), "{u} can mark their inbox read");
    }
    let scope = crate::authz::scope(&f.s.ws, "inbox.markRead", &json!({"all": true}));
    assert!(!scope.unresolved && !scope.access, "marking read must not force everyone to resync");
}
