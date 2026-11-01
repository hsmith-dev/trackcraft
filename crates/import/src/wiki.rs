//! Jira wiki markup (the text-formatting notation of Jira Server/Data Center and of CSV exports)
//! into our rich text.
//!
//! Blocks: `h1.`–`h6.` headings; `*`/`#`/`-` bullet and numbered lists (nested by repeating the
//! marker, `*#` mixes); `{code[:lang]}`…`{code}` and `{noformat}`…`{noformat}` code blocks;
//! `{quote}`…`{quote}` and `bq.` quotes; `{panel}`/`{info}`/`{note}`/`{tip}`/`{warning}` as
//! quotes; `----` rules; `||head||` / `|cell|` tables (as ADF `table` nodes, kept as unknown
//! blocks until tables are modelled). Paragraphs are separated by blank lines; a single line
//! break is a hard break.
//!
//! Inline: `*strong*`, `_emphasis_`, `??citation??` (emphasis), `-strike-`, `+underline+`,
//! `{{monospace}}`, `[text|url]` and `[url]` links, `[~user]` / `[~accountid:…]` mentions,
//! `\\` line breaks, `\x` escapes. `^sup^`/`~sub~` and `{color}` keep their text only;
//! `!image.png!` becomes the text `[image: image.png]`.
//!
//! Hostile input: list nesting stops at [`MAX_LIST_DEPTH`], emphasis nesting at
//! [`MAX_INLINE_DEPTH`], block nesting (quotes in panels in quotes…) at [`MAX_BLOCK_DEPTH`];
//! everything runs in O(n log n) per nesting level, so unclosed markers and marker bombs stay
//! cheap.

use serde_json::{Value, json};
use trackcraft_markup::{Block, Doc, Inline, Mark};

/// Deepest list nesting (`*****…`); deeper items join the deepest level.
pub const MAX_LIST_DEPTH: usize = 10;
/// Deepest emphasis nesting; deeper markers stay literal.
pub const MAX_INLINE_DEPTH: usize = 6;
/// Deepest nesting of quotes and panels; deeper ones are read as plain paragraphs.
pub const MAX_BLOCK_DEPTH: usize = 16;

/// Converts Jira wiki markup into a document.
pub fn to_doc(src: &str) -> Doc {
    let src = src.replace("\r\n", "\n");
    let lines: Vec<&str> = src.split('\n').collect();
    Doc { blocks: blocks(&lines, 0) }
}

const PANELS: [&str; 6] = ["quote", "panel", "info", "note", "tip", "warning"];

/// `{name}` or `{name:params}` at the start of `t` → (name, params, rest after the brace).
fn macro_open(t: &str) -> Option<(&'static str, &str, &str)> {
    let inner = t.strip_prefix('{')?;
    let close = inner.find('}')?;
    let (head, rest) = (inner.get(..close)?, inner.get(close + 1..)?);
    let (name, params) = head.split_once(':').unwrap_or((head, ""));
    let name = name.trim().to_ascii_lowercase();
    let known = ["code", "noformat"].into_iter().chain(PANELS).find(|k| *k == name)?;
    Some((known, params, rest))
}

fn code_language(params: &str) -> Option<String> {
    let mut lang = None;
    for p in params.split('|') {
        match p.split_once('=') {
            Some((k, v)) if k.trim().eq_ignore_ascii_case("language") || k.trim().eq_ignore_ascii_case("lang") => lang = Some(v.trim()),
            Some(_) => {}
            None if !p.trim().is_empty() => lang = Some(p.trim()),
            None => {}
        }
    }
    lang.filter(|l| !l.is_empty() && l.len() <= 40 && l.chars().all(|c| c.is_ascii_alphanumeric() || "+#-_.".contains(c))).map(str::to_string)
}

fn heading(t: &str) -> Option<(u8, &str)> {
    let &[h, d, b'.'] = t.as_bytes().get(..3)? else { return None };
    if !(h == b'h' || h == b'H') || !(b'1'..=b'6').contains(&d) {
        return None;
    }
    let rest = t.get(3..)?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| (d - b'0', rest.trim()))
}

/// `** text` → ("**", "text"); a lone `-` bullet too. `----` and `*bold*` are not list items.
fn list_item(t: &str) -> Option<(&str, &str)> {
    let n = t.chars().take_while(|c| *c == '*' || *c == '#' || *c == '-').count();
    let (markers, rest) = (t.get(..n)?, t.get(n..)?);
    if n == 0 || !rest.starts_with(' ') {
        return None;
    }
    if markers.contains('-') && markers != "-" {
        return None;
    }
    Some((markers, rest.trim()))
}

fn is_rule(t: &str) -> bool {
    t.len() >= 4 && t.chars().all(|c| c == '-')
}

fn starts_block(t: &str) -> bool {
    heading(t).is_some() || list_item(t).is_some() || is_rule(t) || t.starts_with('|') || t.starts_with("bq. ") || macro_open(t).is_some()
}

/// Finds the closing `{name}` (case-insensitive) in `lines` from (`li`, byte `col`).
fn find_close(lines: &[&str], li: usize, col: usize, name: &str) -> Option<(usize, usize)> {
    let tag = format!("{{{name}}}");
    let mut i = li;
    let mut c = col;
    while let Some(line) = lines.get(i) {
        let hay = line.get(c..).unwrap_or_default().to_ascii_lowercase();
        if let Some(p) = hay.find(&tag) {
            return Some((i, c + p));
        }
        i += 1;
        c = 0;
    }
    None
}

/// The text between an opening macro (ending at `lines[li][col]`) and its close, plus the
/// position to continue from.
fn macro_body(lines: &[&str], li: usize, col: usize, name: &str) -> (String, usize, Option<String>) {
    let tag_len = name.len() + 2;
    match find_close(lines, li, col, name) {
        Some((ci, cc)) => {
            let mut parts: Vec<&str> = Vec::new();
            if ci == li {
                parts.push(lines.get(li).and_then(|l| l.get(col..cc)).unwrap_or_default());
            } else {
                parts.push(lines.get(li).and_then(|l| l.get(col..)).unwrap_or_default());
                parts.extend(lines.get(li + 1..ci).unwrap_or_default().iter().copied());
                parts.push(lines.get(ci).and_then(|l| l.get(..cc)).unwrap_or_default());
            }
            let after = lines.get(ci).and_then(|l| l.get(cc + tag_len..)).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
            (trim_blank_edges(&parts.join("\n")), ci + 1, after)
        }
        None => {
            let mut parts = vec![lines.get(li).and_then(|l| l.get(col..)).unwrap_or_default()];
            parts.extend(lines.get(li + 1..).unwrap_or_default().iter().copied());
            (trim_blank_edges(&parts.join("\n")), lines.len(), None)
        }
    }
}

fn trim_blank_edges(s: &str) -> String {
    s.trim_start_matches('\n').trim_end().to_string()
}

fn blocks(lines: &[&str], depth: usize) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut i = 0usize;
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            let mut inl = Vec::new();
            for (n, l) in para.iter().enumerate() {
                if n > 0 {
                    inl.push(Inline::HardBreak);
                }
                inline(l.trim(), &[], 0, &mut inl);
            }
            if !inl.is_empty() {
                out.push(Block::Paragraph(inl));
            }
            para.clear();
        }
    };
    while let Some(line) = lines.get(i) {
        let t = line.trim();
        if t.is_empty() {
            flush(&mut para, &mut out);
            i += 1;
            continue;
        }
        if let Some((name, params, _)) = macro_open(t) {
            flush(&mut para, &mut out);
            let lead = line.len() - line.trim_start().len();
            let col = lead + line.trim_start().find('}').map_or(0, |p| p + 1);
            let (body, next, after) = macro_body(lines, i, col, name);
            match name {
                "code" | "noformat" => {
                    let language = if name == "code" { code_language(params) } else { None };
                    out.push(Block::CodeBlock { language, text: body });
                }
                _ => {
                    let inner: Vec<&str> = body.split('\n').collect();
                    if depth < MAX_BLOCK_DEPTH {
                        out.push(Block::Quote(blocks(&inner, depth + 1)));
                    } else {
                        let mut inl = Vec::new();
                        inline(&inner.join(" "), &[], MAX_INLINE_DEPTH, &mut inl);
                        out.push(Block::Paragraph(inl));
                    }
                }
            }
            if let Some(a) = after {
                let mut inl = Vec::new();
                inline(&a, &[], 0, &mut inl);
                out.push(Block::Paragraph(inl));
            }
            i = next.max(i + 1);
            continue;
        }
        if let Some((level, text)) = heading(t) {
            flush(&mut para, &mut out);
            let mut inl = Vec::new();
            inline(text, &[], 0, &mut inl);
            out.push(Block::Heading { level, content: inl });
        } else if let Some(text) = t.strip_prefix("bq. ") {
            flush(&mut para, &mut out);
            let mut inl = Vec::new();
            inline(text, &[], 0, &mut inl);
            out.push(Block::Quote(vec![Block::Paragraph(inl)]));
        } else if is_rule(t) {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
        } else if list_item(t).is_some() {
            flush(&mut para, &mut out);
            let mut items: Vec<(Vec<char>, Vec<Inline>)> = Vec::new();
            while let Some((m, text)) = lines.get(i).and_then(|l| list_item(l.trim())) {
                let mut inl = Vec::new();
                inline(text, &[], 0, &mut inl);
                let mut markers: Vec<char> = m.chars().map(|c| if c == '-' { '*' } else { c }).collect();
                markers.truncate(MAX_LIST_DEPTH);
                items.push((markers, inl));
                i += 1;
            }
            out.extend(list(&items, 0));
            continue;
        } else if t.starts_with('|') {
            flush(&mut para, &mut out);
            let mut rows = Vec::new();
            while let Some(l) = lines.get(i).map(|l| l.trim()).filter(|l| l.starts_with('|')) {
                rows.push(l);
                i += 1;
            }
            out.push(table(&rows));
            continue;
        } else {
            if !para.is_empty() && starts_block(t) {
                flush(&mut para, &mut out);
            }
            para.push(line);
        }
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

/// Builds lists from items whose marker strings all have length > `depth`.
fn list(items: &[(Vec<char>, Vec<Inline>)], depth: usize) -> Vec<Block> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < items.len() {
        let kind = items.get(idx).and_then(|(m, _)| m.get(depth)).copied().unwrap_or('*');
        let mut entries: Vec<Vec<Block>> = Vec::new();
        while let Some((m, inl)) = items.get(idx) {
            let here = m.get(depth).copied().unwrap_or('*');
            if m.len() <= depth + 1 {
                if here != kind && !entries.is_empty() {
                    break;
                }
                entries.push(vec![Block::Paragraph(inl.clone())]);
                idx += 1;
            } else {
                // Deeper items: gather the run and nest it under the last entry.
                let start = idx;
                while items.get(idx).is_some_and(|(m, _)| m.len() > depth + 1) {
                    idx += 1;
                }
                let nested = list(items.get(start..idx).unwrap_or_default(), depth + 1);
                match entries.last_mut() {
                    Some(e) => e.extend(nested),
                    None => entries.push(nested),
                }
            }
        }
        out.push(if kind == '#' { Block::OrderedList(entries) } else { Block::BulletList(entries) });
    }
    out
}

/// Splits a table row at `|` / `||`, ignoring bars inside `[…]` links and `{{…}}`.
fn table_cells(row: &str) -> Vec<(bool, String)> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut header = false;
    let mut bracket = 0usize;
    let mut started = false;
    let mut it = row.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '[' => {
                bracket = bracket.saturating_add(1);
                cur.push(c);
            }
            ']' => {
                bracket = bracket.saturating_sub(1);
                cur.push(c);
            }
            '|' if bracket == 0 => {
                if started {
                    cells.push((header, std::mem::take(&mut cur)));
                }
                started = true;
                header = it.peek() == Some(&'|');
                if header {
                    it.next();
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        cells.push((header, cur));
    }
    cells
}

/// A wiki table as an ADF `table` node.
fn table(rows: &[&str]) -> Block {
    let rows: Vec<Value> = rows
        .iter()
        .map(|r| {
            let cells: Vec<Value> = table_cells(r)
                .into_iter()
                .map(|(header, text)| {
                    let mut inl = Vec::new();
                    inline(text.trim(), &[], 0, &mut inl);
                    let para = paragraph_adf(inl);
                    json!({"type": if header { "tableHeader" } else { "tableCell" }, "attrs": {}, "content": [para]})
                })
                .collect();
            json!({"type": "tableRow", "content": cells})
        })
        .collect();
    Block::Unknown(json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": rows}))
}

/// One paragraph's ADF JSON (through the markup crate, so marks are written its way).
pub(crate) fn paragraph_adf(inl: Vec<Inline>) -> Value {
    let d = Doc { blocks: vec![Block::Paragraph(inl)] };
    d.to_adf().get("content").and_then(|c| c.get(0)).cloned().unwrap_or_else(|| json!({"type": "paragraph", "content": []}))
}

fn mark_of(c: char) -> Option<Option<Mark>> {
    Some(match c {
        '*' => Some(Mark::Strong),
        '_' => Some(Mark::Em),
        '-' => Some(Mark::Strike),
        '+' => Some(Mark::Underline),
        '^' | '~' => None,
        _ => return None,
    })
}

struct Text<'m> {
    buf: String,
    marks: &'m [Mark],
}

impl Text<'_> {
    fn flush(&mut self, out: &mut Vec<Inline>) {
        if !self.buf.is_empty() {
            out.push(Inline::Text { text: std::mem::take(&mut self.buf), marks: self.marks.to_vec() });
        }
    }
}

/// The next occurrence of a pattern at or after a position. Positions only ever move forward
/// in a scan, so each pattern's search covers the text once in total (no quadratic rescans on
/// inputs like `[[[[…` or `{{{{…`).
struct Next<'p> {
    pat: &'p str,
    /// `Some(found)` once searched: the absolute position found (`None` = no more).
    cached: Option<Option<usize>>,
}

impl<'p> Next<'p> {
    fn new(pat: &'p str) -> Self {
        Next { pat, cached: None }
    }

    fn at_or_after(&mut self, s: &str, from: usize) -> Option<usize> {
        match self.cached {
            Some(None) => return None,
            Some(Some(q)) if q >= from => return Some(q),
            _ => {}
        }
        let found = s.get(from..).and_then(|r| r.find(self.pat)).map(|r| r + from);
        self.cached = Some(found);
        found
    }
}

fn with(marks: &[Mark], m: Mark) -> Vec<Mark> {
    let mut v = marks.to_vec();
    if !v.contains(&m) {
        v.push(m);
    }
    v
}

/// Converts one line (or cell) of inline markup.
fn inline(s: &str, marks: &[Mark], depth: usize, out: &mut Vec<Inline>) {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    // Valid closing positions per emphasis character, for O(log n) lookups.
    let closers = |m: char| -> Vec<usize> {
        (0..chars.len())
            .filter(|&j| {
                chars.get(j).map(|(_, c)| *c) == Some(m)
                    && j > 0
                    && chars.get(j - 1).is_some_and(|(_, p)| !p.is_whitespace())
                    && chars.get(j + 1).is_none_or(|(_, n)| !n.is_alphanumeric())
            })
            .collect()
    };
    let emphasis = depth < MAX_INLINE_DEPTH;
    let table: Vec<(char, Vec<usize>)> = if emphasis { ['*', '_', '-', '+', '^', '~'].iter().map(|&m| (m, closers(m))).collect() } else { vec![] };
    let at = |j: usize| chars.get(j).map(|(_, c)| *c);
    let byte = |j: usize| chars.get(j).map_or(s.len(), |(b, _)| *b);
    let mut t = Text { buf: String::new(), marks };
    let (mut close_code, mut close_brace) = (Next::new("}}"), Next::new("}"));
    let (mut close_bracket, mut open_bracket) = (Next::new("]"), Next::new("["));
    let (mut bang, mut cite) = (Next::new("!"), Next::new("??"));
    let mut i = 0usize;
    while let Some(c) = at(i) {
        let next = at(i + 1);
        // Line break `\\` and escapes.
        if c == '\\' {
            match next {
                Some('\\') => {
                    t.flush(out);
                    out.push(Inline::HardBreak);
                    i += 2;
                }
                Some(n) => {
                    t.buf.push(n);
                    i += 2;
                }
                None => {
                    t.buf.push(c);
                    i += 1;
                }
            }
            continue;
        }
        // {{monospace}}
        if c == '{' && next == Some('{') {
            let from = byte(i + 2);
            if let Some(rel) = close_code.at_or_after(s, from).map(|q| q - from) {
                t.flush(out);
                let code = s.get(from..from + rel).unwrap_or_default();
                if !code.is_empty() {
                    out.push(Inline::Text { text: code.to_string(), marks: with(marks, Mark::Code) });
                }
                let end_byte = from + rel + 2;
                while byte(i) < end_byte {
                    i += 1;
                }
                continue;
            }
        }
        // {color:…} and {color} tags: dropped, their text kept.
        if c == '{' {
            let rest = s.get(byte(i)..).unwrap_or_default();
            let lower = rest.get(..rest.len().min(7)).unwrap_or_default().to_ascii_lowercase();
            let close = close_brace.at_or_after(s, byte(i));
            if (lower.starts_with("{color") || lower.starts_with("{anchor"))
                && let Some(q) = close
            {
                let end_byte = q + 1;
                while byte(i) < end_byte {
                    i += 1;
                }
                continue;
            }
        }
        // [link], [text|url], [~mention]
        if c == '[' {
            let from = byte(i + 1);
            if let Some(rel) = close_bracket.at_or_after(s, from).map(|q| q - from) {
                let inner = s.get(from..from + rel).unwrap_or_default();
                let nested = open_bracket.at_or_after(s, from).is_some_and(|o| o < from + rel);
                if !inner.is_empty() && !nested {
                    t.flush(out);
                    link(inner, marks, out);
                    let end_byte = from + rel + 1;
                    while byte(i) < end_byte {
                        i += 1;
                    }
                    continue;
                }
            }
        }
        // !image.png!
        if c == '!' && next.is_some_and(|n| !n.is_whitespace()) {
            let from = byte(i + 1);
            if let Some(rel) = bang.at_or_after(s, from).map(|q| q - from).filter(|r| *r <= 256) {
                let inner = s.get(from..from + rel).unwrap_or_default();
                if !inner.contains(char::is_whitespace) && inner.contains('.') {
                    let name = inner.split('|').next().unwrap_or_default();
                    t.buf.push_str(&format!("[image: {name}]"));
                    let end_byte = from + rel + 1;
                    while byte(i) < end_byte {
                        i += 1;
                    }
                    continue;
                }
            }
        }
        // ??citation??
        if emphasis && c == '?' && next == Some('?') && at(i + 2).is_some_and(|n| !n.is_whitespace()) {
            let from = byte(i + 2);
            if let Some(rel) = cite.at_or_after(s, from).map(|q| q - from).filter(|r| *r > 0) {
                t.flush(out);
                inline(s.get(from..from + rel).unwrap_or_default(), &with(marks, Mark::Em), depth + 1, out);
                let end_byte = from + rel + 2;
                while byte(i) < end_byte {
                    i += 1;
                }
                continue;
            }
        }
        // *strong* _em_ -strike- +underline+ ^sup^ ~sub~
        if let Some(mark) = mark_of(c).filter(|_| emphasis) {
            let opens = (i == 0 || at(i - 1).is_some_and(|p| !p.is_alphanumeric())) && next.is_some_and(|n| !n.is_whitespace() && n != c);
            if opens && let Some(list) = table.iter().find(|(m, _)| *m == c).map(|(_, l)| l) {
                let k = list.partition_point(|&j| j <= i + 1);
                if let Some(&j) = list.get(k) {
                    t.flush(out);
                    let inner = s.get(byte(i + 1)..byte(j)).unwrap_or_default();
                    match mark {
                        Some(m) => inline(inner, &with(marks, m), depth + 1, out),
                        None => inline(inner, marks, depth + 1, out),
                    }
                    i = j + 1;
                    continue;
                }
            }
        }
        t.buf.push(c);
        i += 1;
    }
    t.flush(out);
}

fn link(inner: &str, marks: &[Mark], out: &mut Vec<Inline>) {
    if let Some(user) = inner.strip_prefix('~') {
        let id = user.strip_prefix("accountid:").unwrap_or(user).trim();
        out.push(Inline::Mention { id: id.to_string(), text: format!("@{id}") });
        return;
    }
    let (text, url) = match inner.split_once('|') {
        Some((t, rest)) => (t.trim(), rest.split('|').next().unwrap_or_default().trim()),
        None => (inner.trim(), inner.trim()),
    };
    let safe =
        url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") || url.starts_with('#') || url.starts_with('/');
    if !safe || url.is_empty() {
        out.push(Inline::Text { text: text.to_string(), marks: marks.to_vec() });
        return;
    }
    let text = if text.is_empty() { url } else { text };
    out.push(Inline::Text { text: text.to_string(), marks: with(marks, Mark::Link(url.to_string())) });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(s: &str) -> String {
        to_doc(s).to_markdown()
    }

    #[test]
    fn blocks_convert() {
        assert_eq!(md("h1. Title\n\nSome *bold* and _it_ text."), "# Title\n\nSome **bold** and *it* text.");
        assert_eq!(md("* a\n* b\n** b1\n# n"), "- a\n- b\n  - b1\n\n1. n");
        assert_eq!(md("# a\n#* b\n# c"), "1. a\n   - b\n2. c");
        assert_eq!(md("{code:java}\nint x = 1;\n{code}"), "```java\nint x = 1;\n```");
        assert_eq!(md("{noformat}*raw*{noformat}"), "```\n*raw*\n```");
        assert_eq!(md("{quote}\nsaid\n{quote}"), "> said");
        assert_eq!(md("bq. quoted"), "> quoted");
        assert_eq!(md("a\n----\nb"), "a\n\n---\n\nb");
        assert_eq!(md("line one\nline two"), "line one  \nline two");
        assert_eq!(md("- dash item"), "- dash item");
    }

    #[test]
    fn inline_converts() {
        assert_eq!(md("[Docs|https://example.com/x]"), "[Docs](https://example.com/x)");
        assert_eq!(md("[https://example.com]"), "[https://example.com](https://example.com)");
        assert_eq!(md("[bad|javascript:alert(1)]"), "bad");
        assert_eq!(md("use {{cargo test}} now"), "use `cargo test` now");
        assert_eq!(md("-gone- and well-known"), "~~gone~~ and well-known");
        let d = to_doc("*bold _both_*");
        let Block::Paragraph(i) = &d.blocks[0] else { panic!("{d:?}") };
        assert_eq!(i[1], Inline::Text { text: "both".into(), marks: vec![Mark::Strong, Mark::Em] });
        assert_eq!(md("a \\* b"), "a * b");
        assert_eq!(md("mc ^2^ {color:red}red{color}"), "mc 2 red");
        assert_eq!(md("see !shot.png|thumbnail!"), "see [image: shot.png]");
        let d = to_doc("hi [~accountid:5b10a2844c20165700ede21g]");
        assert!(matches!(d.blocks[0], Block::Paragraph(ref i) if matches!(i[1], Inline::Mention { ref id, .. } if id == "5b10a2844c20165700ede21g")));
    }

    #[test]
    fn tables_become_adf() {
        let d = to_doc("||A||B||\n|1|[x|https://e.com]|");
        let Block::Unknown(v) = &d.blocks[0] else { panic!("{d:?}") };
        assert_eq!(v["type"], "table");
        assert_eq!(v["content"][0]["content"][0]["type"], "tableHeader");
        assert_eq!(v["content"][1]["content"].as_array().unwrap().len(), 2);
        assert!(Doc::from_adf(&d.to_adf()).is_ok());
    }

    #[test]
    fn bombs_stay_cheap() {
        let stars = "*".repeat(100_000);
        let _ = to_doc(&stars);
        let nested_list = format!("{} deep", "*".repeat(5_000));
        let d = to_doc(&nested_list);
        assert!(Doc::from_adf(&d.to_adf()).is_ok());
        let openers = "*a _b -c +d ".repeat(20_000);
        let _ = to_doc(&openers);
        let nested = "*_-+".repeat(2_000) + "x" + &"+-_*".repeat(2_000);
        assert!(Doc::from_adf(&to_doc(&nested).to_adf()).is_ok());
        let quotes = "{quote}\n".repeat(5_000);
        assert!(Doc::from_adf(&to_doc(&quotes).to_adf()).is_ok());
        let panels: String = (0..2_000).map(|_| "{panel}\n").collect::<String>() + "x" + &"\n{panel}".repeat(2_000);
        assert!(Doc::from_adf(&to_doc(&panels).to_adf()).is_ok());
        let _ = to_doc(&"[".repeat(50_000));
        let _ = to_doc(&"{{".repeat(50_000));
        let _ = to_doc(&"{code}".repeat(10_000));
        let _ = to_doc(&"|".repeat(50_000));
        let _ = to_doc("h1.\nh7. x\n\u{0}\u{feff}é*ü*");
    }
}
