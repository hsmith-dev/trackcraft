//! Markdown in: the subset `Doc::to_markdown` writes, so the editor round-trips.
//!
//! Blocks: `#`–`######` headings, `-`/`*`/`+` and `1.` lists (nesting by indentation), fenced
//! code, `>` quotes, `---` rules, paragraphs (a trailing two spaces or `\` is a hard break).
//! Inline: `**strong**`, `*em*`/`_em_`, `` `code` ``, `~~strike~~`, `[text](url)`.
//! Hostile input: nesting is capped at [`crate::MAX_DEPTH`]; deeper lists flatten.

use crate::{Block, Doc, Inline, MAX_DEPTH, Mark};

impl Doc {
    pub fn from_markdown(src: &str) -> Doc {
        let lines: Vec<&str> = src.lines().collect();
        Doc { blocks: blocks(&lines, 0) }
    }
}

fn indent_of(l: &str) -> usize {
    l.chars().take_while(|c| *c == ' ').count()
}

/// `- x`, `* x`, `+ x` → (false, "x"); `12. x` → (true, "x").
fn list_marker(l: &str) -> Option<(bool, &str)> {
    let t = l.trim_start();
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(m) {
            return Some((false, rest));
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits < 10 {
        let rest = t.get(digits..)?;
        return rest.strip_prefix(". ").map(|r| (true, r));
    }
    None
}

fn is_rule(t: &str) -> bool {
    let t = t.trim();
    t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}

fn blocks(lines: &[&str], depth: usize) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&line) = lines.get(i) {
        let t = line.trim();
        if t.is_empty() {
            i += 1;
            continue;
        }
        if let Some(fence) = t.strip_prefix("```") {
            let language = Some(fence.trim().to_string()).filter(|s| !s.is_empty());
            let mut text = Vec::new();
            i += 1;
            while let Some(&l) = lines.get(i) {
                i += 1;
                if l.trim_start().starts_with("```") {
                    break;
                }
                text.push(l);
            }
            out.push(Block::CodeBlock { language, text: text.join("\n") });
            continue;
        }
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && t.get(hashes..).is_some_and(|r| r.starts_with(' ') || r.is_empty()) {
            let level = u8::try_from(hashes).unwrap_or(1);
            out.push(Block::Heading { level, content: inline(t.get(hashes..).unwrap_or_default().trim()) });
            i += 1;
            continue;
        }
        if is_rule(t) {
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if t.starts_with('>') {
            let mut inner = Vec::new();
            while let Some(&l) = lines.get(i) {
                let lt = l.trim_start();
                let Some(rest) = lt.strip_prefix('>') else { break };
                inner.push(rest.strip_prefix(' ').unwrap_or(rest));
                i += 1;
            }
            if depth < MAX_DEPTH {
                out.push(Block::Quote(blocks(&inner, depth + 1)));
            } else {
                out.push(Block::Paragraph(inline(&inner.join(" "))));
            }
            continue;
        }
        if let Some((ordered, _)) = list_marker(line) {
            let base = indent_of(line);
            let mut items: Vec<Vec<String>> = Vec::new();
            while let Some(&l) = lines.get(i) {
                if l.trim().is_empty() {
                    // A blank line ends the list unless the next line continues it.
                    let next = lines.get(i + 1).copied().unwrap_or_default();
                    if indent_of(next) > base || list_marker(next).is_some_and(|(o, _)| o == ordered && indent_of(next) == base) {
                        i += 1;
                        continue;
                    }
                    break;
                }
                match list_marker(l) {
                    Some((o, rest)) if indent_of(l) == base && o == ordered => items.push(vec![rest.to_string()]),
                    _ if indent_of(l) > base => {
                        if let Some(last) = items.last_mut() {
                            last.push(l.get(base.min(l.len())..).unwrap_or(l).trim_start_matches(' ').to_string());
                            // Keep relative indentation of nested lists.
                            if let Some(v) = last.last_mut() {
                                *v = format!("{}{}", " ".repeat(indent_of(l).saturating_sub(base).saturating_sub(2)), v);
                            }
                        }
                    }
                    _ => break,
                }
                i += 1;
            }
            let items: Vec<Vec<Block>> = items
                .iter()
                .map(|lines| {
                    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
                    if depth < MAX_DEPTH { blocks(&refs, depth + 1) } else { vec![Block::Paragraph(inline(&refs.join(" ")))] }
                })
                .collect();
            out.push(if ordered { Block::OrderedList(items) } else { Block::BulletList(items) });
            continue;
        }
        // Paragraph: until a blank line or the start of another block. Lines ending in two
        // spaces or `\` end with a hard break; other line ends are soft (a space).
        let mut para: Vec<Inline> = Vec::new();
        let mut first = true;
        while let Some(&l) = lines.get(i) {
            let lt = l.trim();
            let starts_block = lt.starts_with('#') || lt.starts_with("```") || lt.starts_with('>') || list_marker(l).is_some();
            if lt.is_empty() || (!first && starts_block) {
                break;
            }
            let hard = l.ends_with("  ") || l.ends_with('\\');
            push_spans(&mut para, &inline(l.trim_end().trim_end_matches('\\').trim_start()));
            i += 1;
            first = false;
            let more = lines.get(i).is_some_and(|n| {
                let nt = n.trim();
                !nt.is_empty() && !(nt.starts_with('#') || nt.starts_with("```") || nt.starts_with('>') || list_marker(n).is_some())
            });
            if more {
                if hard {
                    para.push(Inline::HardBreak);
                } else {
                    push_text(&mut para, " ", &[]);
                }
            }
        }
        out.push(Block::Paragraph(para));
    }
    out
}

fn push_spans(out: &mut Vec<Inline>, spans: &[Inline]) {
    for sp in spans {
        match sp {
            Inline::Text { text, marks } => push_text(out, text, marks),
            other => out.push(other.clone()),
        }
    }
}

fn push_text(out: &mut Vec<Inline>, s: &str, marks: &[Mark]) {
    if s.is_empty() {
        return;
    }
    if let Some(Inline::Text { text, marks: m }) = out.last_mut()
        && m.as_slice() == marks
    {
        text.push_str(s);
        return;
    }
    out.push(Inline::Text { text: s.to_string(), marks: marks.to_vec() });
}

/// Inline spans. Unclosed markers are literal text.
pub(crate) fn inline(s: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    spans(s, &[], &mut out, 0);
    out
}

fn spans(s: &str, marks: &[Mark], out: &mut Vec<Inline>, depth: usize) {
    let mut rest = s;
    let mut plain = String::new();
    while let Some(c) = rest.chars().next() {
        let try_delim = |rest: &str, d: &str, mark: Mark| -> Option<(String, Mark, usize)> {
            let body = rest.strip_prefix(d)?;
            let end = body.find(d)?;
            if end == 0 {
                return None;
            }
            Some((body.get(..end)?.to_string(), mark, d.len() * 2 + end))
        };
        let found = if depth >= 8 {
            None
        } else if rest.starts_with('`') {
            try_delim(rest, "`", Mark::Code)
        } else if rest.starts_with("**") {
            try_delim(rest, "**", Mark::Strong)
        } else if rest.starts_with("~~") {
            try_delim(rest, "~~", Mark::Strike)
        } else if rest.starts_with('*') {
            try_delim(rest, "*", Mark::Em)
        } else if rest.starts_with('_') && plain.chars().last().is_none_or(|p| !p.is_alphanumeric()) {
            try_delim(rest, "_", Mark::Em)
        } else if rest.starts_with('[') {
            link(rest)
        } else {
            None
        };
        match found {
            Some((inner, mark, used)) => {
                push_text(out, &plain, marks);
                plain.clear();
                let mut m = marks.to_vec();
                m.push(mark.clone());
                if mark == Mark::Code {
                    push_text(out, &inner, &m);
                } else {
                    spans(&inner, &m, out, depth + 1);
                }
                rest = rest.get(used..).unwrap_or_default();
            }
            None => {
                plain.push(c);
                rest = rest.get(c.len_utf8()..).unwrap_or_default();
            }
        }
    }
    push_text(out, &plain, marks);
}

fn link(rest: &str) -> Option<(String, Mark, usize)> {
    let close = rest.find("](")?;
    let text = rest.get(1..close)?;
    let after = rest.get(close + 2..)?;
    let end = after.find(')')?;
    let href = after.get(..end)?;
    if text.is_empty() || href.is_empty() || href.contains(' ') {
        return None;
    }
    Some((text.to_string(), Mark::Link(href.to_string()), close + 2 + end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(md: &str) {
        let d = Doc::from_markdown(md);
        assert_eq!(d.to_markdown(), md, "{d:?}");
    }

    #[test]
    fn round_trips_what_we_write() {
        round_trip(
            "## Plan\n\n**bold** and *em* and `code` and [site](https://example.org)\n\n- a\n- b\n\n1. one\n2. two\n\n> quoted\n\n---\n\n```rust\nfn x() {}\n```",
        );
        round_trip("- outer\n  - inner\n- next");
    }

    #[test]
    fn hostile_markdown() {
        for s in ["**", "*a", "[x](", "```", "> > > >", "#######", "1.", "- ", "`", "_a_b_", "[](x)", "\u{0}"] {
            let _ = Doc::from_markdown(s).to_markdown();
        }
        let deep = (0..500).map(|i| format!("{}- x", " ".repeat(i * 2))).collect::<Vec<_>>().join("\n");
        let _ = Doc::from_markdown(&deep);
        let quotes = ">".repeat(10_000);
        let _ = Doc::from_markdown(&quotes);
        let stars = "*a ".repeat(10_000);
        let _ = Doc::from_markdown(&stars);
    }
}
