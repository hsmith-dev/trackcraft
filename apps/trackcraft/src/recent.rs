//! Recently opened and saved workspaces: `<config dir>/trackcraft/recent.json`, a JSON array of
//! paths, most recent first. A missing, oversized or corrupt file reads as an empty list.

use std::path::{Path, PathBuf};

/// How many paths are kept.
pub const MAX: usize = 10;

/// Larger files are not read (ten paths are far smaller).
const MAX_FILE: u64 = 1 << 20;

/// The list in `file`; empty when it is missing or unreadable.
pub fn load(file: &Path) -> Vec<PathBuf> {
    let Ok(meta) = std::fs::metadata(file) else { return vec![] };
    if meta.len() > MAX_FILE {
        return vec![];
    }
    let Ok(text) = std::fs::read_to_string(file) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else { return vec![] };
    let mut out: Vec<PathBuf> = vec![];
    for p in v.iter().filter_map(serde_json::Value::as_str).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(p);
        if !out.contains(&p) && out.len() < MAX {
            out.push(p);
        }
    }
    out
}

/// `list` with `path` moved (or added) to the front, at most [`MAX`] long.
pub fn push(mut list: Vec<PathBuf>, path: &Path) -> Vec<PathBuf> {
    let path = absolute(path);
    list.retain(|p| p != &path);
    list.insert(0, path);
    list.truncate(MAX);
    list
}

/// Writes the list atomically (temp file + rename), creating the folder as needed.
pub fn save(file: &Path, list: &[PathBuf]) -> Result<(), String> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let v: Vec<String> = list.iter().map(|p| p.display().to_string()).collect();
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", file.display())
    })
}

/// Records `path` in `file` (load, push, save).
pub fn remember(file: &Path, path: &Path) -> Result<(), String> {
    save(file, &push(load(file), path))
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("trackcraft-recent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn push_dedups_orders_and_caps() {
        let mut l = vec![];
        for i in 0..15 {
            l = push(l, Path::new(&format!("/w/{i}.track")));
        }
        assert_eq!(l.len(), MAX);
        assert_eq!(l[0], PathBuf::from("/w/14.track"));
        l = push(l, Path::new("/w/10.track"));
        assert_eq!(l[0], PathBuf::from("/w/10.track"));
        assert_eq!(l.len(), MAX);
        assert_eq!(l.iter().filter(|p| p.ends_with("10.track")).count(), 1);
    }

    #[test]
    fn round_trip_creates_folder() {
        let d = dir("rt");
        let f = d.join("nested").join("recent.json");
        remember(&f, Path::new("/a.track")).unwrap();
        remember(&f, Path::new("/b.track")).unwrap();
        remember(&f, Path::new("/a.track")).unwrap();
        assert_eq!(load(&f), vec![PathBuf::from("/a.track"), PathBuf::from("/b.track")]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_files_read_as_empty() {
        let d = dir("bad");
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("recent.json");
        for junk in ["", "{", "{\"a\":1}", "[1, null, {}]", "\u{0}\u{1}", "[\"\"]"] {
            std::fs::write(&f, junk).unwrap();
            assert!(load(&f).is_empty(), "{junk:?}");
        }
        std::fs::write(&f, "[\"/x.track\", 3, \"/x.track\", \"/y.track\"]").unwrap();
        assert_eq!(load(&f), vec![PathBuf::from("/x.track"), PathBuf::from("/y.track")]);
        // Still writable after a corrupt read.
        std::fs::write(&f, "garbage").unwrap();
        remember(&f, Path::new("/z.track")).unwrap();
        assert_eq!(load(&f), vec![PathBuf::from("/z.track")]);
        assert!(load(&d.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
