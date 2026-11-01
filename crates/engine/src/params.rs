//! Reading command params. Params are hostile: wrong types are errors, never panics.

use serde_json::Value;
use trackcraft_core::Id;
use trackcraft_model::{Doc, Timestamp};

pub fn str<'a>(p: &'a Value, k: &str) -> Result<&'a str, String> {
    match p.get(k) {
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(format!("\"{k}\" must be a string")),
        None => Err(format!("missing \"{k}\"")),
    }
}

pub fn opt_str<'a>(p: &'a Value, k: &str) -> Result<Option<&'a str>, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("\"{k}\" must be a string")),
    }
}

pub fn opt_f64(p: &Value, k: &str) -> Result<Option<f64>, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_f64().map(Some).ok_or_else(|| format!("\"{k}\" must be a number")),
        Some(_) => Err(format!("\"{k}\" must be a number")),
    }
}

pub fn opt_u32(p: &Value, k: &str) -> Result<Option<u32>, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).map(Some).ok_or_else(|| format!("\"{k}\" must be a whole number")),
    }
}

pub fn usize_or(p: &Value, k: &str, default: usize, max: usize) -> Result<usize, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).map(|n| n.min(max)).ok_or_else(|| format!("\"{k}\" must be a whole number")),
    }
}

pub fn id(p: &Value, k: &str) -> Result<Id, String> {
    opt_id(p, k)?.ok_or_else(|| format!("missing \"{k}\""))
}

/// Ids are accepted as numbers or numeric strings (the REST APIs send strings).
pub fn opt_id(p: &Value, k: &str) -> Result<Option<Id>, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(|n| Some(Id(n))).ok_or_else(|| format!("\"{k}\" must be a page id")),
        Some(Value::String(s)) => s.trim().parse().map(|n| Some(Id(n))).map_err(|_| format!("\"{k}\" must be a page id")),
        Some(_) => Err(format!("\"{k}\" must be a page id")),
    }
}

pub fn strings(p: &Value, k: &str) -> Result<Option<Vec<String>>, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("\"{k}\" must be a list of strings")))
            .collect::<Result<_, _>>()
            .map(Some),
        // A single comma- or space-separated string, for the CLI.
        Some(Value::String(s)) => Ok(Some(s.split([',', ' ']).filter(|x| !x.is_empty()).map(str::to_string).collect())),
        Some(_) => Err(format!("\"{k}\" must be a list of strings")),
    }
}

/// Rich text: an ADF document object, or a Markdown string (what the editors and CLI send).
pub fn opt_doc(p: &Value, k: &str) -> Result<Option<Doc>, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(Doc::from_markdown(s))),
        Some(v @ Value::Object(_)) => Doc::from_adf(v).map(Some).map_err(|e| format!("\"{k}\": {e}")),
        Some(_) => Err(format!("\"{k}\" must be text or an ADF document")),
    }
}

/// A list of issue keys, capped at [`trackcraft_model::MAX_MOVE`] before anything is copied.
pub fn keys(p: &Value, k: &str) -> Result<Vec<String>, String> {
    let max = trackcraft_model::MAX_MOVE;
    let too_many = || format!("\"{k}\" takes at most {max} issue keys");
    match p.get(k) {
        Some(Value::Array(a)) if a.len() > max => return Err(too_many()),
        Some(Value::String(s)) if s.split([',', ' ']).filter(|x| !x.is_empty()).nth(max).is_some() => return Err(too_many()),
        _ => {}
    }
    match strings(p, k)? {
        Some(v) if !v.is_empty() => Ok(v),
        _ => Err(format!("missing \"{k}\" (a list of issue keys)")),
    }
}

/// A date: `YYYY-MM-DD` (midnight UTC) or an RFC 3339 date-time.
pub fn opt_date(p: &Value, k: &str) -> Result<Option<Timestamp>, String> {
    let Some(s) = opt_str(p, k)? else { return Ok(None) };
    let s = s.trim();
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0).map(|d| Some(d.and_utc())).ok_or_else(|| format!("\"{k}\" is not a valid date"));
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| Some(d.with_timezone(&chrono::Utc)))
        .map_err(|_| format!("\"{k}\" must be a date (2026-11-03) or an RFC 3339 date-time"))
}

pub fn date(p: &Value, k: &str) -> Result<Timestamp, String> {
    opt_date(p, k)?.ok_or_else(|| format!("missing \"{k}\" (a date such as 2026-11-03)"))
}
