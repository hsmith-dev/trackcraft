//! Wiki depth (M3.1): page and inline comments, version diff, templates, copy and ordered move.

use serde_json::{Value, json};

use crate::{Session, catalog, commands};

fn fixed_now() -> trackcraft_model::Timestamp {
    chrono::DateTime::from_timestamp(1_793_448_000, 0).unwrap_or_default() // 2026-10-31T12:00:00Z
}

fn session() -> Session {
    Session::new(crate::sample::workspace(fixed_now()), "riley").with_clock(fixed_now)
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.run(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn fails(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.ws.clone();
    let e = s.run(id, &p).err().unwrap_or_else(|| panic!("{id} {p} should fail"));
    assert!(s.ws == before, "{id} changed the workspace on error");
    e
}

fn page_id(s: &Session, title: &str) -> u64 {
    s.ws.pages.values().find(|p| p.title == title).map(|p| p.id.0).unwrap_or_else(|| panic!("no page {title}"))
}

fn titles_under(s: &Session, parent: u64) -> Vec<String> {
    s.ws.children(trackcraft_core::Id(parent)).iter().map(|p| p.title.clone()).collect()
}

#[test]
fn catalog_floor_with_wiki_depth() {
    let (done, total) = catalog::coverage();
    assert!(done >= 60, "catalog coverage fell to {done}/{total}");
    for id in ["page.diff", "page.copy", "template.list", "page.comment", "page.comments"] {
        assert!(catalog::FEATURES.iter().any(|f| f.command == id), "{id} not in the catalog");
    }
}

#[test]
fn footer_comments_with_replies_edit_delete() {
    let mut s = session();
    let id = page_id(&s, "Sync protocol");
    let top = run(&mut s, "page.comment", json!({"id": id, "body": "Does this handle clock skew?"}))["comment"].as_u64().unwrap_or(0);
    let reply =
        run(&mut s, "page.comment", json!({"id": id, "body": "Yes, via **logical** clocks.", "parent": top}))["comment"].as_u64().unwrap_or(0);
    // One level of threading only.
    assert!(fails(&mut s, "page.comment", json!({"id": id, "body": "nested", "parent": reply})).contains("one level"));
    let list = run(&mut s, "page.comments", json!({"id": id}));
    assert_eq!(list["total"], 2);
    assert_eq!(list["comments"][0]["replies"][0]["body"], "Yes, via **logical** clocks.");
    assert_eq!(list["comments"][0]["inline"], false);

    // Author only.
    s.user = "sam".into();
    assert!(fails(&mut s, "page.editComment", json!({"id": id, "comment": top, "body": "x"})).contains("author"));
    assert!(fails(&mut s, "page.deleteComment", json!({"id": id, "comment": top})).contains("author"));
    s.user = "riley".into();
    run(&mut s, "page.editComment", json!({"id": id, "comment": top, "body": "Does it handle skew?"}));
    let list = run(&mut s, "page.comments", json!({"id": id}));
    assert_eq!(list["comments"][0]["body"], "Does it handle skew?");
    assert!(list["comments"][0]["edited"].is_string());
    // Footer comments aren't resolved.
    assert!(fails(&mut s, "page.resolveComment", json!({"id": id, "comment": top, "resolved": true})).contains("inline"));
    // Deleting the top comment takes its replies.
    let d = run(&mut s, "page.deleteComment", json!({"id": id, "comment": top}));
    assert_eq!(d["deleted"].as_array().map(Vec::len), Some(2));
    assert_eq!(run(&mut s, "page.comments", json!({"id": id}))["total"], 0);
    // Undo brings them back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "page.comments", json!({"id": id}))["total"], 2);
}

#[test]
fn inline_comment_anchors_and_goes_outdated() {
    let mut s = session();
    let id = page_id(&s, "Sync protocol");
    // "Each device keeps an operation log; …" — chars 5..11 are "device".
    let a = json!({"block": 0, "start": 5, "end": 11, "quote": "device"});
    let c = run(&mut s, "page.comment", json!({"id": id, "body": "Phones too?", "anchor": a}))["comment"].as_u64().unwrap_or(0);
    let list = run(&mut s, "page.comments", json!({"id": id, "inline": true}));
    assert_eq!(list["comments"][0]["anchor"]["current"], json!({"block": 0, "start": 5, "end": 11}));
    assert_eq!(list["comments"][0]["outdated"], false);
    assert_eq!(list["comments"][0]["anchor"]["version"], 1);
    assert_eq!(run(&mut s, "page.comments", json!({"id": id, "inline": false}))["comments"], json!([]));

    // Wrong quote / out-of-range selections are refused.
    assert!(
        fails(&mut s, "page.comment", json!({"id": id, "body": "x", "anchor": {"block": 0, "start": 5, "end": 11, "quote": "devise"}}))
            .contains("match")
    );
    fails(&mut s, "page.comment", json!({"id": id, "body": "x", "anchor": {"block": 9, "start": 0, "end": 1, "quote": "E"}}));
    fails(&mut s, "page.comment", json!({"id": id, "body": "x", "anchor": {"block": 0, "start": 3, "end": 3, "quote": ""}}));
    fails(&mut s, "page.comment", json!({"id": id, "body": "x", "anchor": {"block": 0, "start": 0, "end": u32::MAX, "quote": "E"}}));
    fails(&mut s, "page.comment", json!({"id": id, "body": "x", "parent": c, "anchor": a}));

    // Text moves: the anchor follows it.
    run(&mut s, "page.edit", json!({"id": id, "body": "Intro.\n\nEvery device keeps an operation log."}));
    let list = run(&mut s, "page.comments", json!({"id": id}));
    assert_eq!(list["comments"][0]["anchor"]["current"], json!({"block": 1, "start": 6, "end": 12}));
    // Text removed: outdated, but the comment stays.
    run(&mut s, "page.edit", json!({"id": id, "body": "Each phone keeps an operation log."}));
    let list = run(&mut s, "page.comments", json!({"id": id}));
    assert_eq!(list["comments"][0]["outdated"], true);
    assert_eq!(list["comments"][0]["anchor"]["current"], Value::Null);
    assert_eq!(list["comments"][0]["anchor"]["quote"], "device");

    // Resolve / reopen (anyone may).
    s.user = "jo".into();
    run(&mut s, "page.resolveComment", json!({"id": id, "comment": c, "resolved": true}));
    let list = run(&mut s, "page.comments", json!({"id": id}));
    assert_eq!((list["comments"][0]["resolved"].clone(), list["comments"][0]["resolvedBy"].clone()), (json!(true), json!("jo")));
    run(&mut s, "page.resolveComment", json!({"id": id, "comment": c, "resolved": false}));
    assert_eq!(run(&mut s, "page.comments", json!({"id": id}))["comments"][0]["resolved"], false);
    fails(&mut s, "page.resolveComment", json!({"id": id, "comment": c, "resolved": "yes"}));
}

#[test]
fn diff_between_versions() {
    let mut s = session();
    let id = page_id(&s, "Release checklist");
    run(
        &mut s,
        "page.edit",
        json!({"id": id, "title": "Release steps", "body": "1. Freeze the branch\n2. Run the **full** test suite\n3. Write the release notes\n\nThen tag the build."}),
    );
    run(
        &mut s,
        "page.edit",
        json!({"id": id, "body": "1. Freeze the branch\n2. Run the **full** test suite\n3. Write the release notes\n\nThen tag the signed build."}),
    );
    let d = run(&mut s, "page.diff", json!({"id": id, "from": 1, "to": 2}));
    assert_eq!(d["title"]["changed"], true);
    assert_eq!(d["blocks"][0]["kind"], "equal");
    assert_eq!(d["blocks"][1], json!({"kind": "insert", "text": "Then tag the build."}));
    assert_eq!(d["stats"]["inserted"], 1);
    // Defaults: current against the one before; word-level hunks inside a changed block.
    let d = run(&mut s, "page.diff", json!({"id": id}));
    assert_eq!((d["from"]["number"].clone(), d["to"]["number"].clone()), (json!(2), json!(3)));
    assert_eq!(d["title"]["changed"], false);
    let change = &d["blocks"][1];
    assert_eq!(change["kind"], "change");
    assert!(change["inline"].as_array().is_some_and(|h| h.iter().any(|h| h["kind"] == "insert" && h["text"] == "signed ")));
    // Reverse direction works too; unknown versions are errors.
    assert_eq!(run(&mut s, "page.diff", json!({"id": id, "from": 3, "to": 1}))["blocks"][1]["kind"], "delete");
    fails(&mut s, "page.diff", json!({"id": id, "from": 0, "to": 2}));
    fails(&mut s, "page.diff", json!({"id": id, "from": 1, "to": 99}));
    let fresh = page_id(&s, "Onboarding");
    assert!(fails(&mut s, "page.diff", json!({"id": fresh})).contains("first version"));
}

#[test]
fn diff_huge_page_bounded() {
    let mut s = session();
    let id = page_id(&s, "Onboarding");
    let a: String = (0..3000).map(|i| format!("Paragraph {i} with some words.\n\n")).collect();
    let b: String = (0..3000).map(|i| format!("Paragraph {} with other words.\n\n", i * 7)).collect();
    run(&mut s, "page.edit", json!({"id": id, "body": a}));
    run(&mut s, "page.edit", json!({"id": id, "body": b}));
    let started = std::time::Instant::now();
    let d = run(&mut s, "page.diff", json!({"id": id, "from": 2, "to": 3}));
    assert!(d["blocks"].as_array().is_some_and(|b| !b.is_empty()));
    assert!(started.elapsed().as_secs() < 30, "diff took {:?}", started.elapsed());
}

#[test]
fn templates_builtin_and_saved() {
    let mut s = session();
    let list = run(&mut s, "template.list", json!({}));
    let builtins: Vec<&str> = list.as_array().into_iter().flatten().filter(|t| t["builtin"] == true).filter_map(|t| t["id"].as_str()).collect();
    assert!(builtins.len() >= 5 && builtins.contains(&"meeting-notes"));

    // New page from a built-in: title suggested with {date} expanded, body filled.
    let p = run(&mut s, "page.create", json!({"space": "TEAM", "template": "meeting-notes"}))["id"].as_u64().unwrap_or(0);
    let page = run(&mut s, "page.get", json!({"id": p}));
    assert_eq!(page["title"], "Meeting 2026-10-31");
    assert!(page["body"].as_str().is_some_and(|b| b.contains("Follow-ups") && !b.contains("{date}")));
    // By name too, with an explicit title and body overriding.
    let q =
        run(&mut s, "page.create", json!({"space": "ENG", "template": "How-to guide", "title": "How to cut a release", "body": "Own body"}))["id"]
            .as_u64()
            .unwrap_or(0);
    assert_eq!(run(&mut s, "page.get", json!({"id": q}))["body"], "Own body");
    // A template without a full title still needs one.
    fails(&mut s, "page.create", json!({"space": "ENG", "template": "nope", "title": "x"}));

    // Saved templates: workspace-wide and per space.
    let t = run(
        &mut s,
        "template.create",
        json!({"space": "ENG", "name": "Incident review", "title": "Incident {date}", "body": "## Timeline\n\n- {date}: noticed"}),
    )["id"]
        .as_u64()
        .unwrap_or(0);
    let g = run(&mut s, "template.create", json!({"name": "One-pager", "body": "## Summary"}))["id"].as_u64().unwrap_or(0);
    assert!(fails(&mut s, "template.create", json!({"space": "eng", "name": "incident REVIEW", "body": "x"})).contains("already exists"));
    assert!(fails(&mut s, "template.create", json!({"name": "Meeting notes", "body": "x"})).contains("built-in"));
    fails(&mut s, "template.create", json!({"name": "x\ny", "body": "x"}));
    fails(&mut s, "template.create", json!({"space": "NOPE", "name": "x", "body": "x"}));
    let in_team = run(&mut s, "template.list", json!({"space": "TEAM"}));
    let ids: Vec<Value> = in_team.as_array().into_iter().flatten().map(|t| t["id"].clone()).collect();
    assert!(ids.contains(&json!(g)) && !ids.contains(&json!(t)));
    let r = run(&mut s, "page.create", json!({"space": "ENG", "template": t}))["id"].as_u64().unwrap_or(0);
    let page = run(&mut s, "page.get", json!({"id": r}));
    assert_eq!(page["title"], "Incident 2026-10-31");
    assert_eq!(page["body"], "## Timeline\n\n- 2026-10-31: noticed");
    // A space template stays in its space; a string id and a name work too.
    assert!(fails(&mut s, "page.create", json!({"space": "TEAM", "template": t, "title": "x"})).contains("belongs"));
    run(&mut s, "page.create", json!({"space": "TEAM", "template": g.to_string(), "title": "Summary A"}));
    run(&mut s, "page.create", json!({"space": "TEAM", "template": "one-pager", "title": "Summary B"}));

    assert!(fails(&mut s, "template.delete", json!({"id": "meeting-notes"})).contains("built-in"));
    run(&mut s, "template.delete", json!({"id": t}));
    fails(&mut s, "template.delete", json!({"id": t}));
    // Saved templates survive a save/load round trip.
    let saved = serde_json::to_string(&s.ws).unwrap_or_default();
    let back: trackcraft_model::Workspace = serde_json::from_str(&saved).unwrap_or_default();
    assert_eq!(back.templates.len(), 1);
}

#[test]
fn copy_page_and_subtree() {
    let mut s = session();
    let arch = page_id(&s, "Architecture overview");
    run(&mut s, "page.labels", json!({"id": arch, "labels": ["design"]}));
    run(&mut s, "page.comment", json!({"id": arch, "body": "note"}));
    let home = s.ws.space("ENG").map(|sp| sp.homepage.0).unwrap_or(0);

    // Alone, beside the original.
    let c1 = run(&mut s, "page.copy", json!({"id": arch}));
    assert_eq!(c1["title"], "Architecture overview (copy)");
    assert_eq!(c1["parentId"], home);
    assert_eq!(c1["copied"], 1);
    let c1id = c1["id"].as_u64().unwrap_or(0);
    let got = run(&mut s, "page.get", json!({"id": c1id}));
    assert_eq!(got["labels"], json!(["design"]));
    assert_eq!(got["version"]["number"], 1);
    assert_eq!(run(&mut s, "page.comments", json!({"id": c1id}))["total"], 0);
    // With children: every clashing title gets a suffix, numbering continues.
    let c2 = run(&mut s, "page.copy", json!({"id": arch, "children": true}));
    assert_eq!(c2["title"], "Architecture overview (copy 2)");
    assert_eq!(c2["copied"], 3);
    let c2id = c2["id"].as_u64().unwrap_or(0);
    assert_eq!(titles_under(&s, c2id), ["Sync protocol (copy)", "Offline cache (copy)"]);
    // Into another space: free titles are kept.
    let c3 = run(&mut s, "page.copy", json!({"id": arch, "space": "TEAM", "children": true}));
    assert_eq!((c3["title"].clone(), c3["space"].clone()), (json!("Architecture overview"), json!("TEAM")));
    // A given title must be free.
    fails(&mut s, "page.copy", json!({"id": arch, "title": "Release checklist"}));
    let c4 = run(&mut s, "page.copy", json!({"id": arch, "title": "Arch v2"}));
    assert_eq!(c4["title"], "Arch v2");
    // Into its own subtree terminates (sources are snapshotted).
    let sync = page_id(&s, "Sync protocol");
    let c5 = run(&mut s, "page.copy", json!({"id": arch, "parent": sync, "children": true}));
    assert_eq!(c5["copied"], 3);
    // A parent in another space than `space` is refused; bad shapes are errors.
    let team_home = s.ws.space("TEAM").map(|sp| sp.homepage.0).unwrap_or(0);
    fails(&mut s, "page.copy", json!({"id": arch, "space": "ENG", "parent": team_home}));
    fails(&mut s, "page.copy", json!({"id": arch, "children": "yes"}));
    fails(&mut s, "page.copy", json!({"id": arch, "space": "NOPE"}));
    // Copying a home page puts the copy under that home.
    let hc = run(&mut s, "page.copy", json!({"id": home}));
    assert_eq!(hc["parentId"], home);
}

#[test]
fn copy_title_suffix_respects_max_title() {
    let mut s = session();
    let long = "L".repeat(trackcraft_model::MAX_TITLE);
    let id = run(&mut s, "page.create", json!({"space": "ENG", "title": long}))["id"].as_u64().unwrap_or(0);
    let c = run(&mut s, "page.copy", json!({"id": id}));
    let t = c["title"].as_str().unwrap_or_default();
    assert!(t.ends_with(" (copy)") && t.chars().count() <= trackcraft_model::MAX_TITLE);
}

#[test]
fn move_before_and_after_siblings() {
    let mut s = session();
    let home = s.ws.space("ENG").map(|sp| sp.homepage.0).unwrap_or(0);
    let arch = page_id(&s, "Architecture overview");
    let rel = page_id(&s, "Release checklist");
    let sync = page_id(&s, "Sync protocol");
    let cache = page_id(&s, "Offline cache");
    assert_eq!(titles_under(&s, home), ["Architecture overview", "Release checklist"]);
    run(&mut s, "page.move", json!({"id": rel, "before": arch}));
    assert_eq!(titles_under(&s, home), ["Release checklist", "Architecture overview"]);
    // Positions are renumbered 0..n.
    let pos: Vec<u32> = s.ws.children(trackcraft_core::Id(home)).iter().map(|p| p.position).collect();
    assert_eq!(pos, [0, 1]);
    // Out of a parent to after a sibling elsewhere.
    let r = run(&mut s, "page.move", json!({"id": cache, "after": rel}));
    assert_eq!((r["parentId"].clone(), r["position"].clone()), (json!(home), json!(1)));
    assert_eq!(titles_under(&s, home), ["Release checklist", "Offline cache", "Architecture overview"]);
    assert_eq!(titles_under(&s, arch), ["Sync protocol"]);
    run(&mut s, "page.move", json!({"id": cache, "parent": arch, "before": sync}));
    assert_eq!(titles_under(&s, arch), ["Offline cache", "Sync protocol"]);
    // Plain parent move still appends.
    run(&mut s, "page.move", json!({"id": rel, "parent": arch}));
    assert_eq!(titles_under(&s, arch), ["Offline cache", "Sync protocol", "Release checklist"]);

    // Refusals.
    fails(&mut s, "page.move", json!({"id": cache, "before": sync, "after": sync}));
    fails(&mut s, "page.move", json!({"id": cache, "before": cache}));
    fails(&mut s, "page.move", json!({"id": cache, "before": home}));
    fails(&mut s, "page.move", json!({"id": cache, "parent": home, "before": sync}));
    fails(&mut s, "page.move", json!({"id": arch, "after": sync})); // into its own subtree
    fails(&mut s, "page.move", json!({"id": cache}));
}

#[test]
fn move_page_across_spaces() {
    let mut s = session();
    let arch = page_id(&s, "Architecture overview");
    let sync = page_id(&s, "Sync protocol");
    let onboarding = page_id(&s, "Onboarding");
    let team_home = s.ws.space("TEAM").map(|sp| sp.homepage.0).unwrap_or(0);
    run(&mut s, "page.comment", json!({"id": sync, "body": "keep this"}));
    run(&mut s, "page.edit", json!({"id": sync, "body": "v2"}));

    // `space` alone: under the space's home, with the whole subtree, comments and history.
    let r = run(&mut s, "page.move", json!({"id": arch, "space": "team"}));
    assert_eq!((r["space"].clone(), r["parentId"].clone(), r["pages"].clone()), (json!("TEAM"), json!(team_home), json!(3)));
    for id in [arch, sync, page_id(&s, "Offline cache")] {
        assert_eq!(s.ws.page(trackcraft_core::Id(id)).map(|p| p.space.as_str().to_string()).unwrap_or_default(), "TEAM");
    }
    assert_eq!(run(&mut s, "page.comments", json!({"id": sync}))["total"], 1);
    assert_eq!(run(&mut s, "page.history", json!({"id": sync})).as_array().map(Vec::len), Some(2));
    assert_eq!(run(&mut s, "page.search", json!({"text": "operation log", "space": "TEAM"})).as_array().map(Vec::len), Some(0));
    assert!(titles_under(&s, team_home).contains(&"Architecture overview".to_string()));
    // One undo step puts everything back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.ws.page(trackcraft_core::Id(sync)).map(|p| p.space.as_str().to_string()).unwrap_or_default(), "ENG");

    // A sibling in another space works without `space` (the sibling says where).
    let r = run(&mut s, "page.move", json!({"id": sync, "after": onboarding}));
    assert_eq!(r["space"], "TEAM");
    run(&mut s, "edit.undo", json!({}));
    // ... and `space` must agree with parent/before/after.
    let e = fails(&mut s, "page.move", json!({"id": sync, "space": "ENG", "after": onboarding}));
    assert!(e.contains("not in space ENG"), "{e}");
    fails(&mut s, "page.move", json!({"id": sync, "space": "NOPE"}));
    // A space's home never moves.
    let eng_home = s.ws.space("ENG").map(|sp| sp.homepage.0).unwrap_or(0);
    fails(&mut s, "page.move", json!({"id": eng_home, "space": "TEAM"}));

    // A title already taken in the target space is refused, naming the clash.
    run(&mut s, "page.create", json!({"space": "TEAM", "title": "offline cache"}));
    let e = fails(&mut s, "page.move", json!({"id": arch, "space": "TEAM"}));
    assert!(e.contains("offline cache") && e.contains("TEAM") && e.contains("Offline cache"), "{e}");
    // Moving back and forth within the same space is unaffected by its own titles.
    run(&mut s, "page.move", json!({"id": sync, "space": "ENG"}));
}

/// The wiki commands fed junk shaped like their own params: never a panic, and a failed
/// command never changes the workspace.
#[test]
fn hostile_wiki_params_never_panic() {
    let big = "é".repeat(50_000);
    let junk = [
        json!({"id": 2, "body": big, "parent": u64::MAX}),
        json!({"id": 2, "body": "x", "anchor": "nope"}),
        json!({"id": 2, "body": "x", "anchor": {"block": -1, "start": "a", "end": null, "quote": 3}}),
        json!({"id": 2, "body": "x", "anchor": {"block": u64::MAX, "start": u64::MAX, "end": 0, "quote": "x"}}),
        json!({"id": 2, "body": "x", "anchor": {"blockPath": [0, 1, 2], "start": 0, "end": 1, "quote": "x"}}),
        json!({"id": 2, "body": "x", "anchor": {"blockPath": ["a"], "start": 0, "end": 1, "quote": "x"}}),
        json!({"id": 2, "body": "x", "anchor": {"block": 0, "start": 0, "end": 3000, "quote": "q".repeat(3000)}}),
        json!({"id": 2, "body": "   ", "comment": "x", "resolved": 1}),
        json!({"id": 2, "comment": u64::MAX, "body": {"type": "doc", "content": "x"}, "resolved": null}),
        json!({"id": 2, "from": u64::MAX, "to": -3}),
        json!({"id": 2, "from": 1, "to": u32::MAX}),
        json!({"id": 2, "from": "1", "to": [1]}),
        json!({"id": 2, "parent": 2, "before": 2, "after": 3, "children": "x"}),
        json!({"id": 2, "before": u64::MAX}),
        json!({"id": 2, "after": "nope", "parent": null}),
        json!({"id": 2, "space": "TEAM", "parent": u64::MAX}),
        json!({"id": 2, "space": ["TEAM"], "before": 1}),
        json!({"id": 2, "space": "\u{0}", "after": 2}),
        json!({"id": 2, "title": "\u{0}", "space": "x".repeat(10_000), "children": true}),
        json!({"id": 2, "title": "t".repeat(10_000), "children": true}),
        json!({"space": "ENG", "title": "t", "template": {}}),
        json!({"space": "ENG", "title": "t", "template": u64::MAX}),
        json!({"space": "ENG", "template": "meeting-notes", "title": null}),
        json!({"space": "ENG", "template": "x".repeat(100_000)}),
        json!({"space": "ENG", "name": "n", "body": "b", "title": "a\nb", "description": big}),
        json!({"space": 5, "name": [], "body": 3}),
        json!({"name": "n".repeat(1_000), "body": "b"}),
        json!({"id": "meeting-notes"}),
        json!({"id": -1, "inline": "yes"}),
    ];
    let wiki: Vec<&str> = commands().iter().map(|c| c.id).filter(|id| id.starts_with("page.") || id.starts_with("template.")).collect();
    assert!(wiki.len() >= 19);
    let real = page_id(&session(), "Sync protocol");
    for id in wiki {
        // Each shape as written, then aimed at a real page.
        let aimed = junk.iter().map(|p| {
            let mut p = p.clone();
            if let Some(o) = p.as_object_mut()
                && o.contains_key("id")
            {
                o.insert("id".into(), json!(real));
            }
            p
        });
        for p in &junk.iter().cloned().chain(aimed).collect::<Vec<_>>() {
            let mut s = session();
            let before = s.ws.clone();
            if let Err(e) = s.run(id, p) {
                assert!(!e.starts_with("internal error"), "{id} panicked on {p}: {e}");
                assert!(s.ws == before, "{id} changed the workspace on error ({e})");
            }
        }
    }
}
