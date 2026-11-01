//! `@mentions` (M4.5): Markdown text `@id`, where `id` is a known user, becomes an
//! [`Inline::Mention`]; [`Doc::mentions`] lists who a document mentions.
//!
//! Markup doesn't know the workspace's users, so callers pass `is_user`. A mention starts at an
//! `@` that begins the text or follows a character that isn't a letter, digit or `@` (so
//! `ann@example.org` is an address, not a mention); the id runs to whitespace or punctuation
//! other than `.`, `-`, `_`. Trailing `.`/`-`/`_` are dropped until a known user matches
//! (`@ann.` at the end of a sentence). Code spans and link text are never mentions.

use crate::{Block, Doc, Inline, MAX_DEPTH, Mark};

/// Longest id considered after an `@` (user ids are at most 128 characters).
const MAX_ID: usize = 128;
/// Most distinct users [`Doc::mentions`] returns.
pub const MAX_MENTIONS: usize = 100;

impl Doc {
    /// [`Doc::from_markdown`], then [`Doc::link_mentions`].
    pub fn from_markdown_mentions(src: &str, is_user: &dyn Fn(&str) -> bool) -> Doc {
        let mut d = Doc::from_markdown(src);
        d.link_mentions(is_user);
        d
    }

    /// Turns plain-text `@id` (a known user) into [`Inline::Mention`] `{id, text: "@id"}`, which
    /// writes back to Markdown as the same `@id`.
    pub fn link_mentions(&mut self, is_user: &dyn Fn(&str) -> bool) {
        for b in &mut self.blocks {
            link_block(b, is_user, 0);
        }
    }

    /// Distinct user ids this document mentions, in order of first appearance: structured
    /// mentions (whose id passes `is_user`) and plain-text `@id`. At most [`MAX_MENTIONS`].
    pub fn mentions(&self, is_user: &dyn Fn(&str) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        for b in &self.blocks {
            collect_block(b, is_user, &mut out, 0);
        }
        out
    }
}

fn link_block(b: &mut Block, is_user: &dyn Fn(&str) -> bool, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match b {
        Block::Paragraph(i) | Block::Heading { content: i, .. } => *i = link_inlines(std::mem::take(i), is_user),
        Block::BulletList(items) | Block::OrderedList(items) => {
            items.iter_mut().flatten().for_each(|b| link_block(b, is_user, depth.saturating_add(1)));
        }
        Block::Quote(bs) => bs.iter_mut().for_each(|b| link_block(b, is_user, depth.saturating_add(1))),
        Block::CodeBlock { .. } | Block::Rule | Block::Unknown(_) => {}
    }
}

fn plain_marks(marks: &[Mark]) -> bool {
    !marks.iter().any(|m| matches!(m, Mark::Code | Mark::Link(_)))
}

fn link_inlines(inl: Vec<Inline>, is_user: &dyn Fn(&str) -> bool) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inl.len());
    for i in inl {
        match i {
            Inline::Text { text, marks } if plain_marks(&marks) => {
                let found = scan(&text, is_user);
                if found.is_empty() {
                    out.push(Inline::Text { text, marks });
                    continue;
                }
                let mut at = 0;
                for (start, end) in found {
                    if let Some(before) = text.get(at..start).filter(|s| !s.is_empty()) {
                        out.push(Inline::Text { text: before.to_string(), marks: marks.clone() });
                    }
                    if let Some(id) = text.get(start.saturating_add(1)..end) {
                        out.push(Inline::Mention { id: id.to_string(), text: format!("@{id}") });
                    }
                    at = end;
                }
                if let Some(rest) = text.get(at..).filter(|s| !s.is_empty()) {
                    out.push(Inline::Text { text: rest.to_string(), marks });
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn collect_block(b: &Block, is_user: &dyn Fn(&str) -> bool, out: &mut Vec<String>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match b {
        Block::Paragraph(inl) | Block::Heading { content: inl, .. } => {
            for i in inl {
                let mut add = |id: &str| {
                    if out.len() < MAX_MENTIONS && !out.iter().any(|o| o == id) {
                        out.push(id.to_string());
                    }
                };
                match i {
                    Inline::Mention { id, .. } if is_user(id) => add(id),
                    Inline::Text { text, marks } if plain_marks(marks) => {
                        for (start, end) in scan(text, is_user) {
                            if let Some(id) = text.get(start.saturating_add(1)..end) {
                                add(id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Block::BulletList(items) | Block::OrderedList(items) => {
            items.iter().flatten().for_each(|b| collect_block(b, is_user, out, depth.saturating_add(1)));
        }
        Block::Quote(bs) => bs.iter().for_each(|b| collect_block(b, is_user, out, depth.saturating_add(1))),
        Block::CodeBlock { .. } | Block::Rule | Block::Unknown(_) => {}
    }
}

fn id_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && (c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Byte ranges `(at, end)` of `@id` mentions in `text` (`at` is the `@`).
fn scan(text: &str, is_user: &dyn Fn(&str) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    let mut skip_to = 0usize;
    for (i, c) in text.char_indices() {
        let p = prev;
        prev = Some(c);
        if i < skip_to || c != '@' || p.is_some_and(|p| p.is_alphanumeric() || p == '@') {
            continue;
        }
        let start = i.saturating_add(1);
        let Some(rest) = text.get(start..) else { continue };
        let mut end = start;
        for (n, (j, ch)) in rest.char_indices().enumerate() {
            if n >= MAX_ID || !id_char(ch) {
                break;
            }
            end = start.saturating_add(j).saturating_add(ch.len_utf8());
        }
        // Drop trailing `.`, `-`, `_` until a known user matches.
        while end > start {
            let Some(id) = text.get(start..end) else { break };
            if is_user(id) {
                out.push((i, end));
                skip_to = end;
                break;
            }
            match id.chars().last() {
                Some(last @ ('.' | '-' | '_')) => end = end.saturating_sub(last.len_utf8()),
                _ => break,
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn users(id: &str) -> bool {
        matches!(id, "ann" | "bob" | "jo.k" | "zoë")
    }

    #[test]
    fn markdown_at_known_user_becomes_a_mention() {
        let d = Doc::from_markdown_mentions("Hi @ann and @ghost, ask @bob.", &users);
        let Some(Block::Paragraph(inl)) = d.blocks.first() else { panic!("{d:?}") };
        assert!(inl.contains(&Inline::Mention { id: "ann".into(), text: "@ann".into() }), "{inl:?}");
        assert!(inl.contains(&Inline::Mention { id: "bob".into(), text: "@bob".into() }), "{inl:?}");
        assert_eq!(d.mentions(&users), vec!["ann".to_string(), "bob".to_string()]);
        // Writes back as the same Markdown.
        assert_eq!(d.to_markdown(), "Hi @ann and @ghost, ask @bob.");
        assert_eq!(d.plain_text(), "Hi @ann and @ghost, ask @bob.");
    }

    #[test]
    fn not_mentions() {
        for s in ["mail ann@ann.org", "`@ann` in code", "[@ann](https://x.org)", "@@ann", "```\n@ann\n```", "@", "@ ann", "@anna"] {
            assert!(Doc::from_markdown_mentions(s, &users).mentions(&users).is_empty(), "{s}");
        }
    }

    #[test]
    fn dotted_unicode_nested_and_deduplicated() {
        let d = Doc::from_markdown("- @jo.k.\n  - > (@zoë) and @ann, again @ann");
        assert_eq!(d.mentions(&users), vec!["jo.k".to_string(), "zoë".to_string(), "ann".to_string()]);
        let structured = Doc { blocks: vec![Block::Paragraph(vec![Inline::Mention { id: "bob".into(), text: "@Bob".into() }])] };
        assert_eq!(structured.mentions(&users), vec!["bob".to_string()]);
        assert!(structured.mentions(&|_| false).is_empty());
    }

    #[test]
    fn hostile_text() {
        let all = |_: &str| true;
        let long = format!("@{}", "a".repeat(100_000));
        assert_eq!(Doc::from_markdown(&long).mentions(&all).len(), 1);
        let many = "@a@b @c ".repeat(5_000);
        assert!(Doc::from_markdown(&many).mentions(&all).len() <= MAX_MENTIONS);
        let mut d = Doc::from_markdown("@é\u{0}@\u{301} @-. @...");
        d.link_mentions(&all);
        let _ = d.to_markdown();
    }
}
