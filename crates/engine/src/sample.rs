//! The sample workspace (`--sample`): a small fictional team, original text throughout.

use chrono::Duration;
use trackcraft_markup::Doc;
use trackcraft_model::{LinkKind, NewIssue, NewPage, ProjectKind, Timestamp, Workspace};

/// Builds the sample workspace with timestamps relative to `now`. Every step uses the same
/// validated primitives as the commands; a step that fails is skipped (the sample never panics).
pub fn workspace(now: Timestamp) -> Workspace {
    let mut w = Workspace::new("Lighthouse Labs");
    for (id, name) in [("riley", "Riley Park"), ("sam", "Sam Okafor"), ("jo", "Jo Lindqvist")] {
        let _ = w.add_user(id, name);
    }
    let _ = w.create_project("LUME", "Lume Mobile App", ProjectKind::Software, Some("riley"));
    let _ = w.create_project("OPS", "Office Operations", ProjectKind::Business, Some("jo"));
    // Riley administers the workspace; Sam and Jo are members, Jo administers OPS.
    for u in ["sam", "jo"] {
        let _ = w.set_workspace_role(u, trackcraft_model::WorkspaceRole::Member);
    }
    let _ = w.set_project_role("OPS", "jo", Some(trackcraft_model::ProjectRole::Administrator));

    let ago = |d: i64| now - Duration::days(d);
    let issue = |project: &str, ty: &str, summary: &str, prio: &str, who: Option<&str>, labels: &[&str], points: Option<f64>| NewIssue {
        project: project.into(),
        issue_type: Some(ty.into()),
        summary: summary.into(),
        description: Doc::default(),
        priority: Some(prio.into()),
        assignee: who.map(str::to_string),
        labels: labels.iter().map(|s| s.to_string()).collect(),
        parent: None,
        story_points: points,
    };
    let items = [
        (issue("LUME", "Epic", "Offline mode", "High", Some("riley"), &["offline"], None), 20, None),
        (issue("LUME", "Story", "Cache the last 50 notes on the device", "High", Some("sam"), &["offline"], Some(5.0)), 18, Some("In Progress")),
        (issue("LUME", "Story", "Show a banner while offline", "Medium", None, &["offline", "ui"], Some(2.0)), 15, None),
        (
            issue("LUME", "Bug", "Dark mode: note titles are unreadable on the share sheet", "Highest", Some("jo"), &["ui"], Some(1.0)),
            6,
            Some("In Review"),
        ),
        (issue("LUME", "Task", "Upgrade the sync library", "Low", Some("sam"), &[], Some(3.0)), 30, Some("Done")),
        (issue("LUME", "Bug", "Crash when a note title is exactly 255 characters", "High", None, &["crash"], None), 2, None),
        (issue("OPS", "Task", "Order standing desks for the new room", "Medium", Some("jo"), &[], None), 9, None),
        (issue("OPS", "Task", "Renew the building access cards", "Low", Some("jo"), &[], None), 40, Some("Done")),
    ];
    for (new, days, status) in items {
        let reporter = new.assignee.clone().unwrap_or_else(|| "riley".into());
        if let Ok(k) = w.create_issue(new, &reporter, ago(days))
            && let Some(st) = status
        {
            let key = k.to_string();
            // Software issues past In Progress went through it first, so the history (and JQL
            // `status WAS "In Progress"`) has something to show.
            if k.project.as_str() == "LUME" && st != "In Progress" {
                let _ = w.transition(&key, "In Progress", &reporter, ago(days * 3 / 4));
            }
            let _ = w.transition(&key, st, &reporter, ago(days / 2));
        }
    }
    let _ = w.set_description(
        "LUME-2",
        Doc::plain("Keep the most recently opened notes readable without a connection.\n\nOut of scope: editing offline (see LUME-1)."),
        "riley",
        ago(18),
    );
    let _ = w.create_issue(
        NewIssue {
            project: "LUME".into(),
            issue_type: Some("Sub-task".into()),
            summary: "Measure cache size on low-storage devices".into(),
            parent: Some("LUME-2".into()),
            assignee: Some("sam".into()),
            ..NewIssue::default()
        },
        "sam",
        ago(10),
    );
    let _ = w.link("LUME-6", LinkKind::Blocks, "LUME-2", "riley", ago(2));
    let _ = w.add_comment("LUME-4", "riley", Doc::plain("Reproduced on the latest build. Contrast ratio is about 2:1."), ago(5));
    let _ = w.add_comment("LUME-4", "jo", Doc::plain("Fix is up for review; using the theme's text token now."), ago(1));

    let _ = w.create_space("ENG", "Engineering Handbook", "riley", ago(60));
    let _ = w.create_space("TEAM", "Team Home", "jo", ago(60));
    let page = |w: &mut Workspace, space: &str, parent, title: &str, body: &str, who: &str, d: i64| {
        w.create_page(NewPage { space: space.into(), parent, title: title.into(), body: Doc::from_markdown(body) }, who, ago(d)).ok()
    };
    let arch = page(
        &mut w,
        "ENG",
        None,
        "Architecture overview",
        "The app is a thin client over a local note store that syncs in the background.",
        "riley",
        50,
    );
    page(&mut w, "ENG", arch, "Sync protocol", "Each device keeps an operation log; the server merges logs and sends back the result.", "sam", 40);
    page(&mut w, "ENG", arch, "Offline cache", "Design notes for LUME-2: least-recently-used eviction, capped by item count.", "sam", 17);
    page(&mut w, "ENG", None, "Release checklist", "1. Freeze the branch\n2. Run the **full** test suite\n3. Write the release notes", "riley", 45);
    page(
        &mut w,
        "TEAM",
        None,
        "Meeting notes 2026-10-27",
        "Attendees: Riley, Sam, Jo\n\nDecided to ship offline mode behind a setting first.",
        "jo",
        4,
    );
    page(&mut w, "TEAM", None, "Onboarding", "Welcome! Start with the Engineering Handbook, then pick an issue labelled good-first-issue.", "jo", 55);
    plan(&mut w, now);
    w
}

/// LUME's backlog order and sprints: an active two-week sprint (started five days ago) holding
/// LUME-2, LUME-3 and LUME-4 (LUME-7 follows its parent), an empty planned sprint, and LUME-6
/// waiting in the backlog.
/// Runs last so the sprint ids don't shift the page ids above.
fn plan(w: &mut Workspace, now: Timestamp) {
    let ago = |d: i64| now - Duration::days(d);
    let Ok(lume) = w.project("LUME").map(|p| p.key.clone()) else { return };
    let order: Vec<_> = [4, 2, 7, 3, 6, 1, 5].into_iter().map(|n| trackcraft_core::IssueKey::new(lume.clone(), n)).collect();
    w.rebalance_ranks(&lume, &order);
    let keys = |ks: &[&str]| ks.iter().map(|k| k.to_string()).collect::<Vec<_>>();
    if let Ok(s1) = w.create_sprint("LUME", None, Some("Recent notes open without a connection"), None, None) {
        let _ = w.set_sprint(&keys(&["LUME-4", "LUME-2", "LUME-3"]), Some(s1), "riley", ago(5));
        let _ = w.start_sprint(s1, None, ago(5), ago(5) + Duration::days(14), None);
    }
    let _ = w.create_sprint("LUME", None, Some("Harden long titles"), None, None);
}
