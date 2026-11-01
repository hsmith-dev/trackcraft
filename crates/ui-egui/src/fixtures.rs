//! Hostile-data fixtures for UI tests and `ui_shot` (`{"fixture": "stress"|"empty"}`): not part of
//! the app. Built through engine commands where a command exists and the size is small; the bulk
//! (10 000 issues, 500 comments) uses the same validated model primitives the commands call,
//! because every mutating command snapshots the whole workspace for undo (quadratic at that size).
//! States no command can produce (a removed user, a status outside the workflow, a deleted
//! parent) are made by editing the workspace struct directly, as a corrupt or migrated file would.
//!
//! Scripts: egui's bundled faces (Ubuntu Light, the egui icon font) cover Latin (with
//! diacritics), Greek and Cyrillic, but **not** emoji, CJK, Arabic or Devanagari. The app's font
//! stack (`crate::fonts`) adds those from embedded craft-fonts faces (builds with
//! `CRAFT_FONTS_DIR`) and the installed system fonts; a machine with neither draws tofu
//! (`MissingGlyphPolicy::Tofu`, no panic). egui_kittest panics on a missing glyph by default and
//! its plain harness has only egui's faces, so [`stress`] keeps to the bundled scripts and
//! [`StressSpec::uncovered_scripts`] adds [`UNCOVERED_TITLES`] for harnesses that install the
//! font stack and either check coverage first (`crate::fonts::missing`) or are built with
//! `allow_missing_glyphs()`.

use serde_json::{Value, json};
use trackcraft_engine::Session;
use trackcraft_markup::Doc;
use trackcraft_model::{NewIssue, Timestamp, Workspace};

/// What [`stress_with`] builds.
#[derive(Debug, Clone, Copy)]
pub struct StressSpec {
    /// Issues in the BIG project.
    pub issues: usize,
    /// Add [`UNCOVERED_TITLES`] (emoji, CJK, Arabic, Devanagari) as INTL issues and an
    /// [`INTL_PAGE`] page: not in egui's bundled faces.
    pub uncovered_scripts: bool,
}

impl Default for StressSpec {
    fn default() -> Self {
        StressSpec { issues: 10_000, uncovered_scripts: false }
    }
}

/// The acting user of both fixtures.
pub const ME: &str = "riley";
/// The issue with unknown users, 500 comments, 100 labels and a huge description.
pub const HOSTILE_ISSUE: &str = "LONG-1";
/// The issue whose status is not in its workflow.
pub const ORPHAN_ISSUE: &str = "LONG-2";
/// The sub-task whose parent was deleted.
pub const ORPHAN_CHILD: &str = "LONG-4";
/// The issue whose summary holds a 200-character word, assigned to a user whose name is one.
pub const LONG_WORD_ISSUE: &str = "LONG-5";
/// The status [`ORPHAN_ISSUE`] is in.
pub const ORPHAN_STATUS: &str = "Legacy triage";
/// The user id removed from the users map (assignee / reporter / comment author).
pub const GONE_USER: &str = "ghost-user-0042";

/// Issue titles in scripts egui's bundled faces lack (INTL-4 to INTL-7 with
/// [`StressSpec::uncovered_scripts`]).
pub const UNCOVERED_TITLES: [&str; 4] =
    ["Emoji 🚀🔥✅ everywhere 🎉 in the title 🐛", "同步大量笔记时出现错误", "خطأ في المزامنة عند وجود ملاحظات كثيرة", "बहुत सारे नोट्स होने पर सिंक त्रुटि"];
/// The title of the HUGE-space page whose body is every INTL title.
pub const INTL_PAGE: &str = "多语言 · متعدد اللغات · बहुभाषी";

/// A workspace with nothing in it but the acting user.
pub fn empty() -> Workspace {
    let mut w = Workspace::new("Empty workspace");
    let _ = w.add_user(ME, "Riley Park");
    w
}

/// The stress workspace with [`StressSpec::default`].
pub fn stress(clock: fn() -> Timestamp) -> Workspace {
    stress_with(clock, StressSpec::default())
}

/// 255 characters of `seed` repeated (the summary limit), never ending in a space.
fn long_title(seed: &str) -> String {
    let s: String = seed.chars().cycle().take(255).collect();
    let mut s = s.trim_end().to_string();
    while s.chars().count() < 255 {
        s.push('x');
    }
    s
}

pub fn stress_with(clock: fn() -> Timestamp, spec: StressSpec) -> Workspace {
    let mut w = Workspace::new("Stress test: every hostile shape at once");
    let _ = w.add_user(ME, "Riley Park");
    let mut s = Session::new(w, ME).with_clock(clock);
    let now = clock();
    let run = |s: &mut Session, id: &str, p: Value| -> Value { s.run(id, &p).unwrap_or(Value::Null) };

    // Users, including one whose name is one unbreakable word and one that will be removed.
    let word = "Supercalifragilistic".repeat(10);
    for (id, name) in [
        ("sam", "Sam Okafor".to_string()),
        ("zoe", format!("Zoë {word}")),
        ("ivan", "Иван Петров".to_string()),
        ("eleni", "Ελένη Παπαδοπούλου".to_string()),
        (GONE_USER, "Soon Gone".to_string()),
    ] {
        run(&mut s, "user.add", json!({"id": id, "name": name}));
    }

    // Five projects: one huge, one hostile, one non-Latin, one empty, one with an empty sprint.
    for (key, name) in [
        ("BIG", "Ten thousand issues".to_string()),
        ("LONG", long_title("Project with a very long name that keeps going ")),
        ("INTL", "Международный проект · Διεθνές έργο".to_string()),
        ("NONE", "Nothing here yet".to_string()),
        ("SPR", "Sprint with no issues".to_string()),
    ] {
        run(&mut s, "project.create", json!({"key": key, "name": name, "kind": "software", "lead": ME}));
    }
    run(&mut s, "sprint.create", json!({"project": "SPR", "name": "Empty sprint", "goal": "Nothing planned yet"}));

    // LONG: every hostile issue shape.
    let labels: Vec<String> = (0..100).map(|n| format!("label-{n:03}-{}", "x".repeat(n % 17))).collect();
    let hostile = run(
        &mut s,
        "issue.create",
        json!({"project": "LONG", "summary": long_title("A summary at the 255 character limit that wraps and wraps "),
               "type": "Story", "assignee": GONE_USER, "labels": labels, "priority": "Highest"}),
    );
    run(&mut s, "issue.create", json!({"project": "LONG", "summary": format!("Orphan status {word}"), "type": "Bug"}));
    run(&mut s, "issue.create", json!({"project": "LONG", "summary": "Parent that gets deleted", "type": "Task"}));
    run(&mut s, "issue.create", json!({"project": "LONG", "summary": "Sub-task whose parent is gone", "type": "Sub-task", "parent": "LONG-3"}));
    run(&mut s, "issue.create", json!({"project": "LONG", "summary": format!("Unbreakable: {word}"), "type": "Task", "assignee": "zoe"}));
    let mut intl = vec![
        "Ошибка синхронизации при большом количестве заметок",
        "Σφάλμα στην εμφάνιση των σημειώσεων",
        "Ünïcödé dïäcrïtïcs: façade, naïve, Ångström, ŁÓDŹ",
    ];
    if spec.uncovered_scripts {
        intl.extend(UNCOVERED_TITLES);
    }
    for t in &intl {
        run(&mut s, "issue.create", json!({"project": "INTL", "summary": t, "assignee": "ivan"}));
    }
    run(&mut s, "issue.transition", json!({"key": "INTL-1", "to": "In Progress"}));

    // Wiki: a 60-deep tree with long titles, a 5 000-line page, a space with no pages.
    run(&mut s, "space.create", json!({"key": "DEEP", "name": "Deep tree"}));
    run(&mut s, "space.create", json!({"key": "HUGE", "name": "Huge page"}));
    run(&mut s, "space.create", json!({"key": "BARE", "name": "Space with no pages"}));
    let mut parent: Option<u64> = None;
    for d in 1..=60 {
        let title = if d % 10 == 0 { long_title(&format!("Level {d} with a title at the limit ")) } else { format!("Level {d}") };
        let mut p = json!({"space": "DEEP", "title": title, "body": format!("Page at depth {d}.")});
        if let Some(id) = parent {
            p["parent"] = json!(id);
        }
        parent = run(&mut s, "page.create", p)["id"].as_u64().or(parent);
    }
    run(&mut s, "page.create", json!({"space": "DEEP", "title": word, "body": format!("{word}\n\n{word}{word}")}));
    let mut body = String::new();
    for n in 1..=5_000u32 {
        if n % 500 == 1 {
            body.push_str(&format!("## Section {}\n\n", n / 500 + 1));
        }
        body.push_str(&format!("Line {n}: the quick brown fox jumps over the lazy dog.\n\n"));
    }
    run(&mut s, "page.create", json!({"space": "HUGE", "title": "Five thousand lines", "body": body}));
    if spec.uncovered_scripts {
        run(&mut s, "page.create", json!({"space": "HUGE", "title": INTL_PAGE, "body": intl.join("\n\n")}));
    }

    let mut w = std::mem::take(&mut s.ws);
    let key = hostile["key"].as_str().unwrap_or(HOSTILE_ISSUE).to_string();
    let huge: String = (1..=3_000).map(|n| format!("Description line {n} {}\n\n", if n % 100 == 0 { word.as_str() } else { "" })).collect();
    let _ = w.set_description(&key, Doc::from_markdown(&huge), ME, now);
    for n in 1..=500u32 {
        let author = match n % 3 {
            0 => GONE_USER,
            1 => ME,
            _ => "sam",
        };
        let text = if n % 50 == 0 { format!("Comment {n}: {word}") } else { format!("Comment {n}") };
        let _ =
            w.add_comment(&key, author, Doc::plain(&text), Timestamp::from_timestamp(now.timestamp() - 60 * i64::from(500 - n), 0).unwrap_or(now));
    }

    // BIG: 10 000 issues across the workflow.
    let statuses = ["To Do", "In Progress", "In Review", "Done"];
    for n in 0..spec.issues {
        let new = NewIssue {
            project: "BIG".into(),
            summary: if n % 97 == 0 { long_title(&format!("Bulk issue {n} with a long title ")) } else { format!("Bulk issue {n}") },
            assignee: match n % 4 {
                0 => Some(ME.into()),
                1 => Some("sam".into()),
                2 => Some(GONE_USER.into()),
                _ => None,
            },
            ..NewIssue::default()
        };
        if let Ok(k) = w.create_issue(new, ME, now)
            && let Some(st) = statuses.get(n % 4)
            && let Some(i) = w.issues.get_mut(&k)
        {
            // Direct: transition() records history per issue; the board only reads the status.
            i.status = (*st).to_string();
        }
    }

    // Struct edits no command can make.
    w.users.remove(GONE_USER);
    if let Some(i) = trackcraft_core::IssueKey::parse(ORPHAN_ISSUE).ok().and_then(|k| w.issues.get_mut(&k)) {
        i.status = ORPHAN_STATUS.into();
    }
    if let Some(i) = trackcraft_core::IssueKey::parse(&key).ok().and_then(|k| w.issues.get_mut(&k)) {
        i.reporter = GONE_USER.into();
    }
    if let Ok(k) = trackcraft_core::IssueKey::parse("LONG-3") {
        w.issues.remove(&k);
    }
    w
}
