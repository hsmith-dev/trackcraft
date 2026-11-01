//! A small RFC 4180 CSV reader with hard caps (columns, rows, field size) and recovery from
//! unbalanced quotes: a record whose quote never closes is re-read up to the end of its line
//! with the quotes taken literally, and a warning says so.

use crate::{ImportError, ImportReport, MAX_COLUMNS, MAX_FIELD_BYTES, MAX_ISSUES, Result};

pub struct Table {
    pub header: Vec<String>,
    /// Data rows (each at most `header.len()` fields), with the 1-based line each starts on.
    pub rows: Vec<(usize, Vec<String>)>,
}

struct Record {
    fields: Vec<String>,
    /// Byte offset just past the record.
    next: usize,
    closed: bool,
    clipped: bool,
    too_wide: bool,
    newlines: usize,
}

/// Reads one record from `pos`. With `literal`, quotes are ordinary characters and the record
/// ends at the first newline.
fn record(text: &str, pos: usize, literal: bool, max_cols: usize) -> Record {
    let mut r = Record { fields: Vec::new(), next: text.len(), closed: true, clipped: false, too_wide: false, newlines: 0 };
    let mut field = String::new();
    let mut in_quotes = false;
    let mut at_field_start = true;
    let rest = text.get(pos..).unwrap_or_default();
    let mut it = rest.char_indices().peekable();
    let push = |r: &mut Record, field: &mut String| {
        if r.fields.len() < max_cols {
            r.fields.push(std::mem::take(field));
        } else {
            r.too_wide = true;
            field.clear();
        }
    };
    let append = |r: &mut Record, field: &mut String, c: char| {
        if field.len().saturating_add(c.len_utf8()) <= MAX_FIELD_BYTES {
            field.push(c);
        } else {
            r.clipped = true;
        }
    };
    while let Some((i, c)) = it.next() {
        if in_quotes {
            if c == '"' {
                if it.peek().map(|(_, n)| *n) == Some('"') {
                    it.next();
                    append(&mut r, &mut field, '"');
                } else {
                    in_quotes = false;
                }
            } else {
                if c == '\n' {
                    r.newlines += 1;
                }
                append(&mut r, &mut field, c);
            }
            continue;
        }
        match c {
            '"' if at_field_start && !literal => {
                in_quotes = true;
                at_field_start = false;
            }
            ',' => {
                push(&mut r, &mut field);
                at_field_start = true;
            }
            '\r' if it.peek().map(|(_, n)| *n) == Some('\n') => {}
            '\n' => {
                push(&mut r, &mut field);
                r.next = pos.saturating_add(i).saturating_add(1);
                r.newlines += 1;
                return r;
            }
            _ => {
                append(&mut r, &mut field, c);
                at_field_start = false;
            }
        }
    }
    if in_quotes {
        r.closed = false;
    }
    push(&mut r, &mut field);
    r.next = text.len();
    r
}

pub fn parse(text: &str, report: &mut ImportReport) -> Result<Table> {
    let mut pos = 0usize;
    let mut line = 1usize;
    let mut header: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    while pos < text.len() {
        let start_line = line;
        let mut r = record(text, pos, false, header.as_ref().map_or(MAX_COLUMNS.saturating_add(1), Vec::len));
        if !r.closed {
            report.warn(format!("line {start_line}: a quote is never closed; the line was read with its quotes taken literally"));
            r = record(text, pos, true, header.as_ref().map_or(MAX_COLUMNS.saturating_add(1), Vec::len));
        }
        if r.next <= pos {
            break;
        }
        pos = r.next;
        line = line.saturating_add(r.newlines);
        if r.fields.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        if r.clipped {
            report.warn(format!("line {start_line}: a field longer than {MAX_FIELD_BYTES} bytes was cut short"));
        }
        match &header {
            None => {
                if r.too_wide || r.fields.len() > MAX_COLUMNS {
                    return Err(ImportError::TooLarge(format!("the CSV header has more than {MAX_COLUMNS} columns")));
                }
                header = Some(r.fields.iter().map(|h| h.trim().to_string()).collect());
            }
            Some(_) => {
                if r.too_wide {
                    report.warn(format!("line {start_line}: more fields than header columns; the extra ones were ignored"));
                }
                if rows.len() >= MAX_ISSUES {
                    report.warn(format!("more than {MAX_ISSUES} rows: the rest were not imported"));
                    break;
                }
                rows.push((start_line, r.fields));
            }
        }
    }
    let header = header.ok_or_else(|| ImportError::Malformed("the CSV file is empty".into()))?;
    Ok(Table { header, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_newlines() {
        let mut r = ImportReport::default();
        let t = parse("a,b,c\r\n1,\"x, \"\"y\"\"\nz\",3\n\n4,5\n", &mut r).unwrap();
        assert_eq!(t.header, vec!["a", "b", "c"]);
        assert_eq!(t.rows[0].1, vec!["1", "x, \"y\"\nz", "3"]);
        assert_eq!(t.rows[1], (5, vec!["4".to_string(), "5".to_string()]));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn unbalanced_quote_recovers() {
        let mut r = ImportReport::default();
        let t = parse("a,b\n1,\"oops\n2,fine\n", &mut r).unwrap();
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0].1, vec!["1", "\"oops"]);
        assert_eq!(t.rows[1].1, vec!["2", "fine"]);
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn caps() {
        let mut r = ImportReport::default();
        let wide = vec!["c"; 10_000].join(",");
        assert!(matches!(parse(&wide, &mut r), Err(ImportError::TooLarge(_))));
        let t = parse("a\n1,2,3\n", &mut r).unwrap();
        assert_eq!(t.rows[0].1, vec!["1"]);
        assert!(parse("", &mut r).is_err());
    }
}
