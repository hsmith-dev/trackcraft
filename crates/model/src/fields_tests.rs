use chrono::{TimeZone, Utc};
use trackcraft_jql::{Context, matches, parse};

use super::*;
use crate::{IssueRecord, NewIssue, ProjectKind};

fn t(h: u32) -> Timestamp {
    Utc.with_ymd_and_hms(2026, 10, 31, h, 0, 0).single().unwrap()
}

fn ws() -> Workspace {
    let mut w = Workspace::new("Test");
    w.add_user("ann", "Ann").unwrap();
    w.add_user("bob", "Bob").unwrap();
    w.create_project("ATLAS", "Atlas", ProjectKind::Software, Some("ann")).unwrap();
    w.create_project("BETA", "Beta", ProjectKind::Software, None).unwrap();
    for s in ["one", "two", "three"] {
        w.create_issue(NewIssue { project: "ATLAS".into(), summary: s.into(), ..NewIssue::default() }, "ann", t(1)).unwrap();
    }
    w
}

fn select(opts: &[&str]) -> FieldKind {
    FieldKind::parse("select", &opts.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
}

fn hits(w: &Workspace, jql: &str) -> Vec<String> {
    let q = parse(jql).unwrap();
    let cx = Context { current_user: Some("ann".into()), now: t(12) };
    let mut v: Vec<IssueRecord<'_>> = w.issues.values().map(|issue| IssueRecord { ws: w, issue }).filter(|r| matches(&q, r, &cx).unwrap()).collect();
    trackcraft_jql::sort(&q, &mut v);
    v.iter().map(|r| r.issue.key.to_string()).collect()
}

#[test]
fn definitions_validate() {
    let mut w = ws();
    assert!(w.create_field("ATLAS", "Status", FieldKind::Text, false, "").is_err(), "system names are reserved");
    assert!(w.create_field("ATLAS", "cf[3]", FieldKind::Text, false, "").is_err());
    assert!(w.create_field("ATLAS", "12", FieldKind::Text, false, "").is_err());
    assert!(w.create_field("NOPE", "X", FieldKind::Text, false, "").is_err());
    assert!(FieldKind::parse("select", &[]).is_err());
    assert!(FieldKind::parse("select", &["a".into(), "A".into()]).is_err());
    assert!(FieldKind::parse("blob", &[]).is_err());
    assert!(FieldKind::parse("select", &vec!["x".to_string(); MAX_OPTIONS + 1]).is_err());
    let id = w.create_field("ATLAS", "Customer", FieldKind::Text, false, "who asked").unwrap();
    assert!(w.create_field("ATLAS", "customer", FieldKind::Number, false, "").is_err(), "duplicate names");
    w.create_field("BETA", "Customer", FieldKind::Text, false, "").unwrap();
    assert!(w.edit_field(id, FieldEdit { options: Some(vec!["a".into()]), ..FieldEdit::default() }, "ann", t(2)).is_err());
    let c = w.create_component("ATLAS", "Docs", Some("bob"), "").unwrap();
    assert!(w.create_component("ATLAS", "docs", None, "").is_err());
    assert!(w.create_component("ATLAS", "Other", Some("ghost"), "").is_err());
    assert!(w.edit_component(c, ComponentEdit { lead: Some(None), ..ComponentEdit::default() }).is_ok());
    let d = |s| parse_day(s).ok();
    let bad = NewVersion { name: "1.0".into(), start: d("2026-11-02"), release_date: d("2026-11-01"), ..NewVersion::default() };
    assert!(w.create_version("ATLAS", bad).is_err());
    assert!(parse_day("2026-02-30").is_err() && parse_day("26-1-1").is_err() && parse_day("0999-01-01").is_err());
    // The saved shape round-trips (integer map keys included).
    let v = w.create_version("ATLAS", NewVersion { name: "1.0".into(), ..NewVersion::default() }).unwrap();
    w.set_custom("ATLAS-1", id, Some(FieldValue::Text("Acme".into())), "ann", t(2)).unwrap();
    w.set_versions("ATLAS-1", VersionList::Fix, &[v], "ann", t(2)).unwrap();
    let json = serde_json::to_string(&w).unwrap();
    let back: Workspace = serde_json::from_str(&json).unwrap();
    assert_eq!(back, w);
}

#[test]
fn values_validate_per_kind() {
    let mut w = ws();
    let num = w.create_field("ATLAS", "Cost", FieldKind::Number, false, "").unwrap();
    let date = w.create_field("ATLAS", "Due", FieldKind::Date, false, "").unwrap();
    let sel = w.create_field("ATLAS", "Severity", select(&["Low", "High"]), false, "").unwrap();
    let multi = w.create_field("ATLAS", "Areas", FieldKind::parse("multiSelect", &["ui".into(), "api".into()]).unwrap(), false, "").unwrap();
    let user = w.create_field("ATLAS", "Owner", FieldKind::User, false, "").unwrap();
    let url = w.create_field("ATLAS", "Spec", FieldKind::Url, false, "").unwrap();
    let k = "ATLAS-1";
    let set = |w: &mut Workspace, id, v| w.set_custom(k, id, Some(v), "ann", t(3));
    assert!(set(&mut w, num, FieldValue::Number(f64::NAN)).is_err());
    assert!(set(&mut w, num, FieldValue::Number(1e300)).is_err());
    assert!(set(&mut w, num, FieldValue::Text("3".into())).is_err(), "kind mismatch");
    set(&mut w, num, FieldValue::Number(2.5)).unwrap();
    assert!(FieldValue::from_text(&FieldKind::Date, "2026-13-01").is_err());
    set(&mut w, date, FieldValue::from_text(&FieldKind::Date, "2026-12-01").unwrap().unwrap()).unwrap();
    assert!(set(&mut w, sel, FieldValue::Select("Medium".into())).is_err());
    set(&mut w, sel, FieldValue::Select("high".into())).unwrap();
    assert_eq!(w.issue(k).unwrap().custom.get(&sel), Some(&FieldValue::Select("High".into())), "canonical option");
    assert!(set(&mut w, multi, FieldValue::MultiSelect(vec!["ui".into(), "db".into()])).is_err());
    set(&mut w, multi, FieldValue::MultiSelect(vec!["ui".into(), "UI".into(), "api".into()])).unwrap();
    assert_eq!(w.issue(k).unwrap().custom.get(&multi), Some(&FieldValue::MultiSelect(vec!["ui".into(), "api".into()])));
    assert!(set(&mut w, user, FieldValue::User("ghost".into())).is_err());
    set(&mut w, user, FieldValue::User("bob".into())).unwrap();
    assert!(set(&mut w, url, FieldValue::Url("javascript:alert(1)".into())).is_err());
    assert!(set(&mut w, url, FieldValue::Url("https://a b".into())).is_err());
    set(&mut w, url, FieldValue::Url("https://example.org/spec".into())).unwrap();
    // Fields of another project can't be set.
    let other = w.create_field("BETA", "Cost", FieldKind::Number, false, "").unwrap();
    assert!(set(&mut w, other, FieldValue::Number(1.0)).is_err());
    // History names the field.
    let h: Vec<_> = w.issue(k).unwrap().history.iter().map(|c| (c.field.clone(), c.to.clone())).collect();
    for want in [("Cost", "2.5"), ("Severity", "High"), ("Areas", "ui, api")] {
        assert!(h.contains(&(want.0.to_string(), Some(want.1.to_string()))), "{want:?} in {h:?}");
    }
    // Required fields can't be cleared.
    w.edit_field(sel, FieldEdit { required: Some(true), ..FieldEdit::default() }, "ann", t(4)).unwrap();
    assert!(w.set_custom(k, sel, None, "ann", t(4)).is_err());
    assert_eq!(w.missing_required("ATLAS-2").unwrap(), vec!["Severity".to_string()]);
}

#[test]
fn deletes_cascade_with_history() {
    let mut w = ws();
    let sel = w.create_field("ATLAS", "Severity", select(&["Low", "High"]), false, "").unwrap();
    let c = w.create_component("ATLAS", "Docs", None, "").unwrap();
    let c2 = w.create_component("ATLAS", "Core", None, "").unwrap();
    let v = w.create_version("ATLAS", NewVersion { name: "1.0".into(), ..NewVersion::default() }).unwrap();
    for k in ["ATLAS-1", "ATLAS-2"] {
        w.set_custom(k, sel, Some(FieldValue::Select("Low".into())), "ann", t(2)).unwrap();
        w.set_components(k, &[c, c2], "ann", t(2)).unwrap();
        w.set_versions(k, VersionList::Fix, &[v], "ann", t(2)).unwrap();
        w.set_versions(k, VersionList::Affects, &[v], "ann", t(2)).unwrap();
    }
    assert!(w.set_components("ATLAS-1", &[Id(9999)], "ann", t(2)).is_err());
    // Removing an option clears values that used it.
    w.edit_field(sel, FieldEdit { options: Some(vec!["High".into()]), ..FieldEdit::default() }, "bob", t(3)).unwrap();
    assert!(w.issue("ATLAS-1").unwrap().custom.is_empty());
    let last = |w: &Workspace, k: &str| w.issue(k).unwrap().history.last().cloned().unwrap();
    let h = last(&w, "ATLAS-2");
    assert_eq!((h.field.as_str(), h.from.as_deref(), h.to.as_deref(), h.author.as_str()), ("Severity", Some("Low"), None, "bob"));
    assert_eq!(w.delete_component(c, "bob", t(4)).unwrap(), 2);
    assert_eq!(w.issue("ATLAS-1").unwrap().components, vec![c2]);
    let h = last(&w, "ATLAS-1");
    assert_eq!((h.field.as_str(), h.from.as_deref(), h.to.as_deref()), ("Component", Some("Docs, Core"), Some("Core")));
    assert_eq!(w.delete_version(v, "bob", t(5)).unwrap(), 2);
    let i = w.issue("ATLAS-2").unwrap();
    assert!(i.fix_versions.is_empty() && i.affects_versions.is_empty());
    let fields: Vec<_> = i.history.iter().rev().take(2).map(|c| c.field.as_str()).collect();
    assert_eq!(fields, ["Version", "Fix Version"]);
    assert_eq!(w.delete_field(sel, "bob", t(6)).unwrap(), 0);
    assert!(w.find_field(sel).is_none());
}

#[test]
fn release_moves_unresolved_and_counts_progress() {
    let mut w = ws();
    let a = w.create_version("ATLAS", NewVersion { name: "1.0".into(), ..NewVersion::default() }).unwrap();
    let b = w.create_version("ATLAS", NewVersion { name: "1.1".into(), ..NewVersion::default() }).unwrap();
    for k in ["ATLAS-1", "ATLAS-2"] {
        w.set_versions(k, VersionList::Fix, &[a], "ann", t(2)).unwrap();
    }
    w.transition("ATLAS-1", "Done", "ann", t(2)).unwrap();
    assert_eq!(w.version_progress(a), (1, 2));
    assert!(w.release_version(a, None, Some(a), "ann", t(3)).is_err());
    assert_eq!(w.release_version(a, None, Some(b), "ann", t(3)).unwrap(), 1);
    let v = w.find_version(a).unwrap().1;
    assert!(v.released && v.release_date == Some(t(3).date_naive()));
    assert_eq!(w.issue("ATLAS-2").unwrap().fix_versions, vec![b]);
    assert!(w.release_version(a, None, None, "ann", t(3)).is_err(), "already released");
    assert_eq!(hits(&w, "fixVersion IN releasedVersions()"), ["ATLAS-1"]);
    assert_eq!(hits(&w, "fixVersion IN unreleasedVersions()"), ["ATLAS-2"]);
    assert_eq!(hits(&w, "fixVersion IN unreleasedVersions(BETA)"), Vec::<String>::new());
}

#[test]
fn jql_clauses_and_order() {
    let mut w = ws();
    let cost = w.create_field("ATLAS", "Cost", FieldKind::Number, false, "").unwrap();
    let sev = w.create_field("ATLAS", "Severity", select(&["Low", "Mid", "High"]), false, "").unwrap();
    let due = w.create_field("ATLAS", "Due date x", FieldKind::Date, false, "").unwrap();
    let owner = w.create_field("ATLAS", "Owner", FieldKind::User, false, "").unwrap();
    let docs = w.create_component("ATLAS", "Docs", None, "").unwrap();
    let v = w.create_version("ATLAS", NewVersion { name: "1.2".into(), ..NewVersion::default() }).unwrap();
    let set = |w: &mut Workspace, k: &str, id, v| w.set_custom(k, id, Some(v), "ann", t(2)).unwrap();
    set(&mut w, "ATLAS-1", cost, FieldValue::Number(5.0));
    set(&mut w, "ATLAS-2", cost, FieldValue::Number(50.0));
    set(&mut w, "ATLAS-1", sev, FieldValue::Select("High".into()));
    set(&mut w, "ATLAS-2", sev, FieldValue::Select("Low".into()));
    set(&mut w, "ATLAS-3", sev, FieldValue::Select("Mid".into()));
    set(&mut w, "ATLAS-2", due, FieldValue::Date(parse_day("2026-11-05").unwrap()));
    set(&mut w, "ATLAS-3", owner, FieldValue::User("ann".into()));
    w.set_components("ATLAS-3", &[docs], "ann", t(2)).unwrap();
    w.set_versions("ATLAS-1", VersionList::Fix, &[v], "ann", t(2)).unwrap();
    w.set_versions("ATLAS-2", VersionList::Affects, &[v], "ann", t(2)).unwrap();
    assert_eq!(hits(&w, "Cost > 10"), ["ATLAS-2"]);
    assert_eq!(hits(&w, &format!("cf[{cost}] <= 5")), ["ATLAS-1"]);
    assert_eq!(hits(&w, "Severity IN (High, Mid) ORDER BY key"), ["ATLAS-1", "ATLAS-3"]);
    assert_eq!(hits(&w, "\"Due date x\" < 2026-12-01"), ["ATLAS-2"]);
    assert_eq!(hits(&w, "Owner = currentUser()"), ["ATLAS-3"]);
    assert_eq!(hits(&w, "Cost IS EMPTY"), ["ATLAS-3"]);
    assert_eq!(hits(&w, "component = Docs"), ["ATLAS-3"]);
    assert_eq!(hits(&w, &format!("component = {docs}")), ["ATLAS-3"]);
    assert_eq!(hits(&w, "fixVersion = \"1.2\""), ["ATLAS-1"]);
    assert_eq!(hits(&w, "affectedVersion = \"1.2\""), ["ATLAS-2"]);
    assert_eq!(hits(&w, "component IS EMPTY ORDER BY key"), ["ATLAS-1", "ATLAS-2"]);
    assert_eq!(hits(&w, "project = ATLAS ORDER BY Severity"), ["ATLAS-2", "ATLAS-3", "ATLAS-1"], "option order");
    assert_eq!(hits(&w, "project = ATLAS ORDER BY Cost DESC"), ["ATLAS-3", "ATLAS-2", "ATLAS-1"], "empty first descending");
    assert_eq!(hits(&w, &format!("project = ATLAS ORDER BY cf[{cost}] ASC")), ["ATLAS-1", "ATLAS-2", "ATLAS-3"]);
}
