//! Numeric ids for things TrackCraft identifies by key.
//!
//! The public APIs give projects, boards, spaces and issues numeric ids (sent as strings).
//! TrackCraft keys them by `LUME` / `LUME-7`, so ids are derived from keys, stably:
//!
//! - a project or space id encodes its key in base 38 when the key has at most 10 characters
//!   (always below 2^53, so JavaScript clients keep it exact); longer keys get a 52-bit hash with
//!   bit 52 set. Lookups scan for the matching id, so neither form needs decoding.
//! - an issue id is `project id × 1 000 000 + number` when that stays below 2^53; otherwise the
//!   issue's id is its key. Every `{issueIdOrKey}` path accepts either.
//! - boards: one per project, with the project's id.

use trackcraft_engine::model::Workspace;

const JS_SAFE: u64 = 1 << 53;
const ISSUE_FACTOR: u64 = 1_000_000;

fn digit(c: char) -> Option<u64> {
    match c {
        '0'..='9' => Some(u64::from(c) - u64::from('0') + 1),
        'A'..='Z' => Some(u64::from(c) - u64::from('A') + 11),
        '_' => Some(37),
        _ => None,
    }
}

/// The numeric id of a project (or space) key.
pub fn key_id(key: &str) -> u64 {
    if key.chars().count() <= 10 {
        let mut n: u64 = 0;
        let mut ok = true;
        for c in key.chars() {
            match digit(c).and_then(|d| n.checked_mul(38).and_then(|m| m.checked_add(d))) {
                Some(v) => n = v,
                None => ok = false,
            }
        }
        if ok && n < (1 << 52) {
            return n;
        }
    }
    // FNV-1a, folded into 52 bits, tagged with bit 52.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h & ((1 << 52) - 1)) | (1 << 52)
}

/// An issue's id: numeric when it fits, else the key itself.
pub fn issue_id(project: &str, number: u64) -> String {
    let n = key_id(project).checked_mul(ISSUE_FACTOR).and_then(|m| m.checked_add(number)).filter(|n| *n < JS_SAFE && number < ISSUE_FACTOR);
    match n {
        Some(n) => n.to_string(),
        None => format!("{project}-{number}"),
    }
}

/// Resolves `{issueIdOrKey}` to a key string (`LUME-7`); unknown numeric ids pass through so the
/// engine reports them as missing.
pub fn issue_key(ws: &Workspace, id_or_key: &str) -> String {
    let s = id_or_key.trim();
    let Ok(n) = s.parse::<u64>() else { return s.to_ascii_uppercase() };
    let (pid, number) = (n / ISSUE_FACTOR, n % ISSUE_FACTOR);
    match ws.projects.keys().find(|k| key_id(k.as_str()) == pid) {
        Some(k) => format!("{}-{number}", k.as_str()),
        None => s.to_string(),
    }
}

/// Resolves `{projectIdOrKey}` (or a board id) to a project key.
pub fn project_key(ws: &Workspace, id_or_key: &str) -> Option<String> {
    let s = id_or_key.trim();
    match s.parse::<u64>() {
        Ok(n) => ws.projects.keys().find(|k| key_id(k.as_str()) == n).map(|k| k.as_str().to_string()),
        Err(_) => ws.projects.keys().find(|k| k.as_str().eq_ignore_ascii_case(s)).map(|k| k.as_str().to_string()),
    }
}

/// Resolves a space id (or key) to a space key.
pub fn space_key(ws: &Workspace, id_or_key: &str) -> Option<String> {
    let s = id_or_key.trim();
    match s.parse::<u64>() {
        Ok(n) => ws.spaces.keys().find(|k| key_id(k.as_str()) == n).map(|k| k.as_str().to_string()),
        Err(_) => ws.spaces.keys().find(|k| k.as_str().eq_ignore_ascii_case(s)).map(|k| k.as_str().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_safe() {
        assert_eq!(key_id("A"), 11);
        assert_ne!(key_id("LUME"), key_id("OPS"));
        assert!(key_id("ABCDEFGHIJ") < 1 << 52);
        let long = key_id("ABCDEFGHIJKLMNOPQRSTUVWXYZ_12345");
        assert!((1 << 52..JS_SAFE).contains(&long));
        let ws = trackcraft_engine::sample::workspace(chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap_or_default());
        let id = issue_id("LUME", 7);
        assert_eq!(issue_key(&ws, &id), "LUME-7");
        assert_eq!(issue_key(&ws, "lume-7"), "LUME-7");
        assert_eq!(issue_key(&ws, "99999999999"), "99999999999");
        assert_eq!(issue_id("ABCDEFGHIJ", 1), "ABCDEFGHIJ-1");
        assert_eq!(project_key(&ws, &key_id("LUME").to_string()).as_deref(), Some("LUME"));
        assert_eq!(project_key(&ws, "ops").as_deref(), Some("OPS"));
        assert_eq!(space_key(&ws, &key_id("ENG").to_string()).as_deref(), Some("ENG"));
    }
}
