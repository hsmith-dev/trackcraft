//! Importer tests. Every fixture is synthetic and written here, in code (AGENTS.md: never
//! commit files exported from Atlassian products).

use std::io::Write;

use serde_json::json;
use trackcraft_markup::{Block, Doc};
use trackcraft_model::{LinkKind, SprintState, StatusCategory, Workspace};

use crate::{Actor, ConfluenceSource, ImportError, confluence_html, confluence_path, jira_csv, jira_json};

fn now() -> chrono::DateTime<chrono::Utc> {
    "2026-10-31T12:00:00Z".parse().unwrap()
}

fn actor() -> Actor<'static> {
    Actor { user: "importer", now: now() }
}

fn at(s: &str) -> chrono::DateTime<chrono::Utc> {
    s.parse().unwrap()
}

/// A CSV row builder for the documented export columns (repeated ones included).
fn csv_fixture() -> String {
    let header = [
        "Summary",
        "Issue key",
        "Issue id",
        "Issue Type",
        "Status",
        "Project key",
        "Project name",
        "Project type",
        "Priority",
        "Assignee",
        "Assignee Id",
        "Reporter",
        "Reporter Id",
        "Created",
        "Updated",
        "Resolved",
        "Labels",
        "Labels",
        "Description",
        "Sprint",
        "Sprint",
        "Custom field (Story point estimate)",
        "Comment",
        "Comment",
        "Parent",
        "Inward issue link (Blocks)",
        "Outward issue link (Blocks)",
        "Outward issue link (Relates)",
        "Outward issue link (Mystery)",
    ];
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let rows: Vec<Vec<String>> = vec![
        vec![
            "Make the orbit map",
            "ORB-3",
            "10003",
            "Epic",
            "In Progress",
            "ORB",
            "Orbit",
            "software",
            "High",
            "Ann Lee",
            "acc-ann-0001",
            "Bo Ng",
            "acc-bo-0002",
            "02/Jan/24 9:00 AM",
            "05/Jan/24 4:30 PM",
            "",
            "map",
            "",
            "h2. Goal\n\nA *bold* plan.",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
        ],
        vec![
            "Draw the axes",
            "ORB-7",
            "10007",
            "Story",
            "Waiting for Design",
            "ORB",
            "Orbit",
            "software",
            "Blocker",
            "Bo Ng",
            "acc-bo-0002",
            "Ann Lee",
            "acc-ann-0001",
            "03/Jan/24 10:00 AM",
            "06/Jan/24 11:00 AM",
            "",
            "ui",
            "ui",
            "* one\n* two",
            "ORB Sprint 1",
            "ORB Sprint 2",
            "5",
            "03/Jan/24 11:00 AM;acc-ann-0001;Looks {{good}}; ship it",
            "garbage comment",
            "10003",
            "",
            "ORB-9",
            "ORB-3",
            "ORB-3",
        ],
        vec![
            "Label the ticks",
            "ORB-8",
            "10008",
            "Sub-task",
            "Shipped",
            "ORB",
            "Orbit",
            "software",
            "Minor",
            "",
            "",
            "Ann Lee",
            "acc-ann-0001",
            "04/Jan/24 10:00 AM",
            "08/Jan/24 10:00 AM",
            "07/Jan/24 10:00 AM",
            "",
            "",
            "",
            "ORB Sprint 2",
            "",
            "",
            "",
            "",
            "ORB-7",
            "",
            "",
            "",
            "",
        ],
        vec![
            "Fix the legend",
            "ORB-9",
            "10009",
            "Bug",
            "Closed",
            "ORB",
            "Orbit",
            "software",
            "Trivial",
            "Cy Doe",
            "",
            "Cy Doe",
            "",
            "2024-01-01 08:00",
            "2024-01-02 08:00",
            "2024-01-02 08:00",
            "",
            "",
            "",
            "ORB Sprint 1",
            "",
            "not-a-number",
            "",
            "",
            "",
            "",
            "",
            "MISSING-1",
            "",
        ],
        vec!["Done elsewhere", "ORB-12", "10012", "Task", "To Do", "ORB", "Orbit", "software", "Medium", "", "", "", "", "garbage date"]
            .into_iter()
            .chain(std::iter::repeat_n("", 15))
            .collect(),
        vec!["Not a key", "orb_x", "", "Task"].into_iter().chain(std::iter::repeat_n("", 25)).collect(),
    ]
    .into_iter()
    .map(|r| r.into_iter().map(str::to_string).collect())
    .collect();
    let mut out = header.iter().map(|h| q(h)).collect::<Vec<_>>().join(",");
    out.push_str("\r\n");
    for r in rows {
        out.push_str(&r.iter().map(|c| q(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

fn check_csv_epic_and_story(ws: &Workspace) {
    let epic = ws.issue("ORB-3").unwrap();
    assert_eq!((epic.issue_type.as_str(), epic.status.as_str(), epic.priority.as_str()), ("Epic", "In Progress", "High"));
    assert_eq!(epic.created, at("2024-01-02T09:00:00Z"));
    assert_eq!(epic.updated, at("2024-01-05T16:30:00Z"));
    assert_eq!(epic.description.to_markdown(), "## Goal\n\nA **bold** plan.");
    assert_eq!(epic.assignee.as_deref(), Some("ann.lee"));
    assert_eq!(epic.reporter, "bo.ng");

    let story = ws.issue("ORB-7").unwrap();
    assert_eq!(story.parent.as_ref().map(ToString::to_string).as_deref(), Some("ORB-3"), "parent by issue id");
    assert_eq!(story.priority, "Highest");
    assert_eq!(story.labels, vec!["ui"]);
    assert_eq!(story.story_points, Some(5.0));
    assert_eq!(story.status, "Waiting for Design");
    let wf = &ws.project("ORB").unwrap().workflow;
    assert_eq!(wf.status("Waiting for Design").unwrap().category, StatusCategory::ToDo);
    assert_eq!(wf.status("Shipped").unwrap().category, StatusCategory::Done);
    assert_eq!(wf.status("Closed").unwrap().category, StatusCategory::Done);
    assert_eq!(story.comments.len(), 2);
    assert_eq!(story.comments[0].author, "ann.lee");
    assert_eq!(story.comments[0].created, at("2024-01-03T11:00:00Z"));
    assert_eq!(story.comments[0].body.to_markdown(), "Looks `good`; ship it");
    assert_eq!(story.comments[1].author, "importer");
    assert_eq!(story.description.to_markdown(), "- one\n- two");
    // Outward Blocks ORB-9, outward Relates ORB-3, unknown "Mystery" → relates (deduplicated).
    assert!(story.links.iter().any(|l| l.kind == LinkKind::Blocks && l.target.to_string() == "ORB-9"));
    assert_eq!(story.links.iter().filter(|l| l.kind == LinkKind::Relates).count(), 1);
}

fn check_csv_rest(ws: &Workspace) {
    let sub = ws.issue("ORB-8").unwrap();
    assert_eq!(sub.parent.as_ref().map(ToString::to_string).as_deref(), Some("ORB-7"));
    assert!(ws.is_subtask(sub));
    assert_eq!(sub.resolved, Some(at("2024-01-07T10:00:00Z")));

    let bug = ws.issue("ORB-9").unwrap();
    assert_eq!(bug.priority, "Lowest");
    assert_eq!(bug.assignee.as_deref(), Some("cy.doe"));
    assert_eq!(bug.story_points, None);

    // Sprint 1 was carried over by ORB-7 → closed; Sprint 2 is the open one → active.
    let sprints = ws.project_sprints(&ws.project("ORB").unwrap().key);
    let s1 = sprints.iter().find(|s| s.name == "ORB Sprint 1").unwrap();
    let s2 = sprints.iter().find(|s| s.name == "ORB Sprint 2").unwrap();
    assert_eq!((s1.state, s2.state), (SprintState::Closed, SprintState::Active));
    assert_eq!(ws.issue("ORB-7").unwrap().sprint, Some(s2.id));
    assert_eq!(ws.issue("ORB-7").unwrap().past_sprints, vec![s1.id]);
    assert_eq!(ws.issue("ORB-8").unwrap().sprint, Some(s2.id), "sub-tasks follow their parent");
    assert_eq!(ws.issue("ORB-9").unwrap().past_sprints, vec![s1.id]);

    let ws_users: Vec<&str> = ws.users.keys().map(String::as_str).collect();
    assert_eq!(ws_users, vec!["ann.lee", "bo.ng", "cy.doe", "importer"]);
}

#[test]
fn csv_maps_everything() {
    let mut ws = Workspace::new("t");
    let r = jira_csv(&mut ws, &csv_fixture(), actor()).unwrap();
    assert_eq!(r.projects, vec!["ORB"]);
    assert_eq!(r.issues, 5, "{:#?}", r.warnings);
    assert_eq!(ws.project("ORB").unwrap().name, "Orbit");
    assert_eq!(ws.project("ORB").unwrap().next_number, 13);
    check_csv_epic_and_story(&ws);
    check_csv_rest(&ws);
    let all = r.warnings.join("\n");
    for expect in ["orb_x", "MISSING-1", "Mystery", "not-a-number", "garbage date", "isn't 'date;author;body'"] {
        assert!(all.contains(expect), "missing warning about {expect}:\n{all}");
    }

    // Re-importing skips existing keys and changes nothing else.
    let before = ws.clone();
    let again = jira_csv(&mut ws, &csv_fixture(), actor()).unwrap();
    assert_eq!(again.issues, 0);
    assert_eq!(ws.issues, before.issues);
}

#[test]
fn csv_hostile_inputs() {
    let mut ws = Workspace::new("t");
    // Not a Jira CSV, too wide, empty: errors, and nothing changes.
    for bad in ["a,b\n1,2\n", "", &vec!["Summary"; 10_000].join(",")] {
        let before = ws.clone();
        assert!(jira_csv(&mut ws, bad, actor()).is_err());
        assert_eq!(ws, before);
    }
    // Unbalanced quotes, ragged rows, a wiki-markup bomb, control characters.
    let bomb = format!("{}x{}", "*_-+".repeat(5_000), "{quote}".repeat(3_000));
    let csv = format!(
        "Summary,Issue key,Description,Labels\nok,ABC-2,\"{}\",a b\n\u{0}\u{7},ABC-3\n,ABC-4,,{}\n\"unclosed,ABC-1,x\nlast,ABC-5\n",
        bomb.replace('"', "\"\""),
        "l".repeat(300)
    );
    let r = jira_csv(&mut ws, &csv, actor()).unwrap();
    assert!(r.issues >= 2, "{:#?}", r.warnings);
    assert_eq!(ws.issue("ABC-2").unwrap().labels, vec!["a", "b"]);
    assert!(ws.issue("ABC-5").is_ok(), "the row after an unclosed quote survives");
    assert!(r.warnings.iter().any(|w| w.contains("quote is never closed")));
    assert_eq!(ws.issue("ABC-4").unwrap().summary, "(no summary)");
    assert!(ws.issue("ABC-3").unwrap().summary.chars().all(|c| !c.is_control()));
    assert!(Doc::from_adf(&ws.issue("ABC-2").unwrap().description.to_adf()).is_ok());

    // Deterministic junk never panics.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let alphabet: Vec<char> = "Summary,Issue key\"\n\r;|*_{}[]#-ABC-1 é\u{0}".chars().collect();
    for _ in 0..200 {
        let mut s = String::from("Summary,Issue key,Comment,Sprint,Parent\n");
        for _ in 0..400 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
        }
        let mut w = Workspace::new("j");
        let _ = jira_csv(&mut w, &s, actor());
    }
}

fn adf(text: &str) -> serde_json::Value {
    json!({"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]})
}

fn json_page() -> serde_json::Value {
    let person = |id: &str, name: &str| json!({"accountId": id, "displayName": name, "emailAddress": format!("{}@example.com", name.to_lowercase().replace(' ', "."))});
    json!({
        "startAt": 0, "maxResults": 50, "isLast": true,
        "names": {"customfield_10016": "Story point estimate", "customfield_10020": "Sprint"},
        "issues": [
            {"id": "20001", "key": "LUNA-1", "fields": {
                "summary": "Moon base", "issuetype": {"name": "Epic", "subtask": false},
                "project": {"key": "LUNA", "name": "Luna", "projectTypeKey": "software"},
                "status": {"name": "Doing", "statusCategory": {"key": "indeterminate"}},
                "priority": {"name": "Medium"}, "assignee": person("acc-1", "Ann Lee"), "reporter": person("acc-2", "Bo Ng"),
                "created": "2024-02-01T10:00:00.000+0000", "updated": "2024-02-03T10:00:00.000+0100",
                "description": adf("base plans"), "labels": ["space"],
                "comment": {"comments": [{"author": person("acc-2", "Bo Ng"), "created": "2024-02-02T10:00:00.000+0000", "body": adf("nice")}]},
                "customfield_10020": [{"id": 1, "name": "Luna 1", "state": "closed", "boardId": 1, "startDate": "2024-01-01T00:00:00.000Z", "endDate": "2024-01-14T00:00:00.000Z", "completeDate": "2024-01-14T00:00:00.000Z"},
                                      {"id": 2, "name": "Luna 2", "state": "active", "boardId": 1, "startDate": "2024-01-15T00:00:00.000Z", "endDate": "2024-01-29T00:00:00.000Z"}]
            }},
            {"id": "20002", "key": "LUNA-2", "fields": {
                "summary": "Dig", "issuetype": {"name": "Story"}, "status": {"name": "Finished", "statusCategory": {"key": "done"}},
                "resolutiondate": "2024-02-05T00:00:00.000+0000", "created": "2024-02-01T11:00:00.000+0000",
                "parent": {"id": "20001", "key": "LUNA-1"}, "customfield_10016": 3.5, "description": "wiki *markup* still works",
                "issuelinks": [{"type": {"name": "Blocks", "inward": "is blocked by", "outward": "blocks"}, "inwardIssue": {"key": "LUNA-1"}},
                               {"type": {"name": "Cloners"}, "outwardIssue": {"key": "LUNA-404"}}],
                "customfield_10020": [{"id": 2, "name": "Luna 2", "state": "active", "boardId": 1}]
            }},
            {"key": "LUNA-3", "fields": 7},
            {"fields": {}},
            {"key": "LUNA-5", "fields": {"summary": ["not", "text"], "status": "Open", "assignee": "nobody", "comment": {"comments": [{"body": 3}]}, "description": {"type":"panel"}}}
        ]
    })
}

#[test]
fn json_maps_everything() {
    let mut ws = Workspace::new("t");
    let r = jira_json(&mut ws, &json_page().to_string(), actor()).unwrap();
    assert_eq!(r.issues, 3, "{:#?}", r.warnings);
    let epic = ws.issue("LUNA-1").unwrap();
    assert_eq!(epic.status, "Doing");
    assert_eq!(ws.project("LUNA").unwrap().workflow.status("Doing").unwrap().category, StatusCategory::InProgress);
    assert_eq!(epic.description.plain_text(), "base plans");
    assert_eq!(epic.updated, at("2024-02-03T09:00:00Z"));
    assert_eq!(epic.assignee.as_deref(), Some("ann.lee"));
    assert_eq!(ws.user("ann.lee").unwrap().email.as_deref(), Some("ann.lee@example.com"));
    assert_eq!(epic.comments[0].author, "bo.ng");
    let story = ws.issue("LUNA-2").unwrap();
    assert_eq!(story.parent.as_ref().map(ToString::to_string).as_deref(), Some("LUNA-1"));
    assert_eq!(story.story_points, Some(3.5));
    assert_eq!(story.resolved, Some(at("2024-02-05T00:00:00Z")));
    assert_eq!(story.description.to_markdown(), "wiki **markup** still works");
    // Inward "Blocks" from LUNA-1: LUNA-1 blocks LUNA-2.
    assert!(ws.issue("LUNA-1").unwrap().links.iter().any(|l| l.kind == LinkKind::Blocks && l.target.to_string() == "LUNA-2"));
    let key = ws.project("LUNA").unwrap().key.clone();
    let sprints = ws.project_sprints(&key);
    assert_eq!(sprints.len(), 2);
    let active = ws.active_sprint(&key).unwrap();
    assert_eq!(active.name, "Luna 2");
    assert_eq!(active.start, Some(at("2024-01-15T00:00:00Z")));
    assert_eq!(epic.sprint, Some(active.id));
    assert_eq!(epic.past_sprints.len(), 1);
    assert!(ws.issue("LUNA-5").is_ok());
    assert!(r.warnings.iter().any(|w| w.contains("LUNA-404")));

    // An array of pages, and a bare array of issues.
    let mut ws2 = Workspace::new("t");
    let pages = json!([json_page(), {"issues": [{"key": "MARS-1", "fields": {"summary": "Red"}}]}]);
    assert_eq!(jira_json(&mut ws2, &pages.to_string(), actor()).unwrap().issues, 4);
    let mut ws3 = Workspace::new("t");
    let bare = json!([{"key": "MARS-1", "fields": {"summary": "Red"}}]);
    assert_eq!(jira_json(&mut ws3, &bare.to_string(), actor()).unwrap().issues, 1);
}

#[test]
fn json_hostile_inputs() {
    let mut ws = Workspace::new("t");
    let deep = "[".repeat(100_000) + &"]".repeat(100_000);
    for bad in ["", "nope", "42", "{\"issues\": 3}", &deep, "\"str\""] {
        let before = ws.clone();
        assert!(jira_json(&mut ws, bad, actor()).is_err(), "{}", bad.chars().take(20).collect::<String>());
        assert_eq!(ws, before);
    }
    let deep_adf = {
        let mut d = json!({"type": "paragraph"});
        for _ in 0..100 {
            d = json!({"type": "blockquote", "content": [d]});
        }
        json!({"issues": [{"key": "A-1", "fields": {"summary": "s", "description": {"type": "doc", "content": [d]}}}]})
    };
    // Deeper than serde_json's nesting limit: refused as a whole, nothing changes.
    let before = ws.clone();
    assert!(matches!(jira_json(&mut ws, &deep_adf.to_string(), actor()), Err(ImportError::Malformed(_))));
    assert_eq!(ws, before);
    // Junk shapes in every field.
    for junk in [json!(null), json!(1), json!("x"), json!([1, 2]), json!({"name": 5, "key": [], "state": "active"})] {
        let f: serde_json::Map<String, serde_json::Value> = [
            "summary",
            "issuetype",
            "status",
            "priority",
            "assignee",
            "reporter",
            "created",
            "labels",
            "parent",
            "comment",
            "issuelinks",
            "customfield_1",
            "project",
        ]
        .into_iter()
        .map(|k| (k.to_string(), junk.clone()))
        .collect();
        let page = json!({"names": junk.clone(), "issues": [{"key": "J-1", "id": junk.clone(), "fields": f}]});
        let mut w = Workspace::new("j");
        let _ = jira_json(&mut w, &page.to_string(), actor());
    }
}

fn page_html(space: &str, title: &str, crumbs: &[(&str, &str)], body: &str, meta: &str) -> Vec<u8> {
    let crumbs: String = crumbs.iter().map(|(href, t)| format!("<li><span><a href=\"{href}\">{t}</a></span></li>")).collect();
    format!(
        "<!DOCTYPE html><html><head><title>{space} : {title}</title><meta charset=\"utf-8\"></head><body><div id=\"page\"><div id=\"main\">\
         <div id=\"breadcrumb-section\"><ol id=\"breadcrumbs\">{crumbs}</ol></div>\
         <h1 id=\"title-heading\" class=\"pagetitle\"><span id=\"title-text\"> {space} : {title} </span></h1>\
         <div id=\"content\" class=\"view\"><div class=\"page-metadata\">{meta}</div>\
         <div id=\"main-content\" class=\"wiki-content group\">{body}</div></div></div></div></body></html>"
    )
    .into_bytes()
}

fn export_files() -> Vec<(String, Vec<u8>)> {
    let sp = "Field Notes";
    let index = format!(
        "<html><head><title>{sp}</title></head><body><div id=\"main-content\"><table><tr><th>Key</th><td>FN</td></tr></table>\
         <h2 id=\"pageTree\">Available Pages:</h2><ul><li><a href=\"Home_100.html\">Home</a><ul>\
         <li><a href=\"Guide_101.html\">Guide</a><ul><li><a href=\"Setup_102.html\">Setup</a></li></ul></li>\
         <li><a href=\"FAQ_103.html\">FAQ</a></li></ul></li></ul></div></body></html>"
    );
    let home = [("index.html", sp)];
    vec![
        ("FN/index.html".into(), index.into_bytes()),
        (
            "FN/Home_100.html".into(),
            page_html(sp, "Home", &home, "<p>Welcome <strong>all</strong>.</p>", "Created by Ann Lee, last modified on Mar 21, 2024"),
        ),
        (
            "FN/Guide_101.html".into(),
            page_html(
                sp,
                "Guide",
                &[("index.html", sp), ("Home_100.html", "Home")],
                "<h2>Steps</h2><ol><li>one</li><li>two</li></ol><table><tr><th>A</th></tr><tr><td>1</td></tr></table>",
                "Created by Bo Ng on Apr 02, 2024",
            ),
        ),
        (
            "FN/Setup_102.html".into(),
            page_html(
                sp,
                "Setup",
                &[("index.html", sp), ("Home_100.html", "Home"), ("Guide_101.html", "Guide")],
                "<pre>cargo run</pre><p><img src=\"attachments/1/x.png\" alt=\"x.png\"></p>",
                "",
            ),
        ),
        ("FN/FAQ_103.html".into(), page_html(sp, "FAQ", &[("index.html", sp), ("Home_100.html", "Home")], "<blockquote><p>Q?</p></blockquote>", "")),
        // Not in the tree: placed by its breadcrumbs.
        (
            "FN/Orphan_104.html".into(),
            page_html(sp, "Orphan", &[("index.html", sp), ("Home_100.html", "Home"), ("FAQ_103.html", "FAQ")], "<p>lost</p>", ""),
        ),
        ("FN/attachments/1/x.png".into(), vec![0x89, b'P', b'N', b'G']),
        ("FN/styles/site.css".into(), b"body{}".to_vec()),
    ]
}

fn check_export(ws: &Workspace) {
    let space = ws.space("FN").unwrap();
    assert_eq!(space.name, "Field Notes");
    let home = ws.page(space.homepage).unwrap();
    assert_eq!(home.title, "Home");
    assert_eq!(home.body.to_markdown(), "Welcome **all**.");
    assert_eq!(home.author, "ann.lee");
    assert_eq!(home.updated, at("2024-03-21T00:00:00Z"));
    let by_title = |t: &str| ws.pages.values().find(|p| p.space == space.key && p.title == t).unwrap();
    let guide = by_title("Guide");
    assert_eq!(guide.parent, Some(home.id));
    assert_eq!(guide.author, "bo.ng");
    assert!(matches!(guide.body.blocks[1], Block::OrderedList(_)));
    assert!(matches!(&guide.body.blocks[2], Block::Unknown(v) if v["type"] == "table"));
    let setup = by_title("Setup");
    assert_eq!(setup.parent, Some(guide.id));
    assert_eq!(setup.body.to_markdown(), "```\ncargo run\n```\n\n[image: x.png]");
    let faq = by_title("FAQ");
    assert_eq!(faq.parent, Some(home.id));
    assert!(faq.position > guide.position);
    assert_eq!(by_title("Orphan").parent, Some(faq.id));
}

#[test]
fn confluence_export_from_files() {
    let mut ws = Workspace::new("t");
    let r = confluence_html(&mut ws, ConfluenceSource::Files(export_files()), None, actor()).unwrap();
    assert_eq!(r.spaces, vec!["FN"]);
    assert_eq!(r.pages, 5, "{:#?}", r.warnings);
    check_export(&ws);
    assert!(r.warnings.iter().any(|w| w.contains("non-page file")));
    // Again into the same space: titles stay unique.
    let r2 = confluence_html(&mut ws, ConfluenceSource::Files(export_files()), Some("fn"), actor()).unwrap();
    assert_eq!(r2.pages, 5);
    assert!(ws.pages.values().any(|p| p.title == "Guide (2)"));
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            z.start_file(*name, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn confluence_zip_and_dir() {
    let files = export_files();
    let entries: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    let bytes = zip_of(&entries);
    let mut ws = Workspace::new("t");
    let r = confluence_html(&mut ws, ConfluenceSource::Zip(bytes.clone()), None, actor()).unwrap();
    assert_eq!(r.pages, 5);
    check_export(&ws);

    // From disk: the .zip, and the unpacked directory.
    let dir = std::env::temp_dir().join(format!("trackcraft-import-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let zpath = dir.join("export.zip");
    std::fs::write(&zpath, &bytes).unwrap();
    let mut ws = Workspace::new("t");
    confluence_path(&mut ws, &zpath, None, actor()).unwrap();
    check_export(&ws);
    for (name, data) in &files {
        let p = dir.join("unpacked").join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }
    let mut ws = Workspace::new("t");
    confluence_path(&mut ws, &dir.join("unpacked"), None, actor()).unwrap();
    check_export(&ws);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn zip_path_traversal_is_refused() {
    for evil in ["../../x.html", "FN/../../etc/passwd", "/abs/index.html", "C:\\win\\index.html", "a\\..\\..\\b.html"] {
        let bytes = zip_of(&[("FN/index.html", b"<html></html>"), (evil, b"<p>x</p>")]);
        let mut ws = Workspace::new("t");
        let before = ws.clone();
        let e = confluence_html(&mut ws, ConfluenceSource::Zip(bytes), None, actor());
        assert!(matches!(e, Err(ImportError::Unsafe(_))), "{evil}: {e:?}");
        assert_eq!(ws, before);
    }
    let mut ws = Workspace::new("t");
    assert!(matches!(confluence_html(&mut ws, ConfluenceSource::Zip(b"PK\x03\x04junk".to_vec()), None, actor()), Err(ImportError::Malformed(_))));
    assert!(confluence_html(&mut ws, ConfluenceSource::Files(vec![]), None, actor()).is_err());
}

#[test]
fn confluence_storage_page() {
    let xhtml = r#"<h1>Runbook</h1><p>Call <ac:link><ri:user ri:account-id="abc"/></ac:link>.</p>
<ac:structured-macro ac:name="toc" ac:schema-version="1"><ac:parameter ac:name="maxLevel">2</ac:parameter></ac:structured-macro>
<ac:structured-macro ac:name="code"><ac:plain-text-body><![CDATA[restart --now]]></ac:plain-text-body></ac:structured-macro>
<table><tbody><tr><th>k</th><td>v</td></tr></tbody></table>"#;
    let mut ws = Workspace::new("t");
    let src = || ConfluenceSource::Storage { title: "Runbook".into(), xhtml: xhtml.into() };
    assert!(confluence_html(&mut ws, src(), None, actor()).is_err(), "needs a space key");
    assert!(confluence_html(&mut ws, src(), Some("bad key"), actor()).is_err());
    let r = confluence_html(&mut ws, src(), Some("OPS"), actor()).unwrap();
    assert_eq!((r.pages, r.spaces.clone()), (1, vec!["OPS".to_string()]));
    let p = ws.pages.values().find(|p| p.title == "Runbook").unwrap();
    assert!(
        matches!(&p.body.blocks[2], Block::Unknown(v) if v["attrs"]["extensionKey"] == "toc" && v["storage"].as_str().unwrap().contains("maxLevel"))
    );
    assert!(matches!(&p.body.blocks[3], Block::CodeBlock { text, .. } if text == "restart --now"));
    assert!(Doc::from_adf(&p.body.to_adf()).is_ok());
}

#[test]
fn confluence_hostile_pages() {
    let sp = "Deep";
    let deep_body = "<div><ul><li>".repeat(20_000) + "bottom" + &"</li></ul></div>".repeat(20_000);
    let crumbs_loop = page_html(sp, "Loop A", &[("index.html", sp), ("B.html", "Loop B")], "<p>a</p>", "");
    let crumbs_loop_b = page_html(sp, "Loop B", &[("index.html", sp), ("A.html", "Loop A")], "<p>b</p>", "");
    let files = vec![
        ("index.html".to_string(), format!("<title>{sp}</title>").into_bytes()),
        ("Deep_1.html".to_string(), page_html(sp, "Deep page", &[("index.html", sp)], &deep_body, "")),
        ("A.html".to_string(), crumbs_loop),
        ("B.html".to_string(), crumbs_loop_b),
        ("Same.html".to_string(), page_html(sp, "Deep page", &[], "<p>dup title</p>", "")),
        ("Junk.html".to_string(), b"<<<>>></a></b><!--".to_vec()),
        ("Bin.html".to_string(), vec![0xff, 0xfe, 0x00, 0x3c]),
    ];
    let mut ws = Workspace::new("t");
    let r = confluence_html(&mut ws, ConfluenceSource::Files(files), Some("DEEP"), actor()).unwrap();
    assert!(r.pages >= 6, "{:#?}", r.warnings);
    for p in ws.pages.values() {
        assert!(Doc::from_adf(&p.body.to_adf()).is_ok(), "{}", p.title);
    }
    assert!(ws.pages.values().any(|p| p.title == "Deep page (2)"));
    assert!(ws.pages.values().any(|p| p.body.plain_text().contains("bottom")));
}

#[test]
fn import_path_dispatch() {
    let dir = std::env::temp_dir().join(format!("trackcraft-import-path-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("e.csv");
    std::fs::write(&csv, format!("\u{feff}{}", csv_fixture())).unwrap();
    let mut ws = Workspace::new("t");
    let r = crate::import_path(&mut ws, crate::Kind::JiraCsv, &csv, None, actor()).unwrap();
    assert_eq!(r.issues, 5);
    assert!(crate::import_path(&mut ws, crate::Kind::JiraJson, &dir.join("missing.json"), None, actor()).is_err());
    assert_eq!(crate::Kind::parse("jira-csv"), Some(crate::Kind::JiraCsv));
    assert_eq!(crate::Kind::parse("nope"), None);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(r.to_json()["issues"], 5);
}
