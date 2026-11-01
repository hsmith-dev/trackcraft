//! TrackCraft rich text (L1): the block/inline model shared by issue descriptions, comments and
//! pages.
//!
//! The JSON shape follows the publicly documented Atlassian Document Format (ADF: `{"type":"doc",
//! "version":1,"content":[…]}`), so content moves to and from Jira/Confluence REST APIs without a
//! lossy conversion. Unknown node types are kept (`Block::Unknown` / `Inline::Unknown`) so a
//! round trip never drops what we don't render yet.
//!
//! Parsing is hostile-input safe: nesting is bounded by [`MAX_DEPTH`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Deepest block nesting accepted from input (lists inside quotes inside panels…).
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Doc {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    BulletList(Vec<Vec<Block>>),
    OrderedList(Vec<Vec<Block>>),
    CodeBlock {
        language: Option<String>,
        text: String,
    },
    Quote(Vec<Block>),
    Rule,
    /// A node we don't model yet, kept verbatim for round trips.
    Unknown(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Inline {
    Text { text: String, marks: Vec<Mark> },
    HardBreak,
    Mention { id: String, text: String },
    Unknown(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mark {
    Strong,
    Em,
    Code,
    Strike,
    Underline,
    Link(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkupError {
    NotADoc,
    TooDeep,
}

impl std::fmt::Display for MarkupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarkupError::NotADoc => f.write_str("not an ADF document (expected {\"type\":\"doc\"})"),
            MarkupError::TooDeep => write!(f, "document nesting is deeper than {MAX_DEPTH}"),
        }
    }
}
impl std::error::Error for MarkupError {}

impl Doc {
    /// A document of plain-text paragraphs, one per non-empty line group.
    pub fn plain(text: &str) -> Doc {
        let blocks = text
            .split("\n\n")
            .filter(|p| !p.trim().is_empty())
            .map(|p| {
                let mut inl = Vec::new();
                for (i, line) in p.lines().enumerate() {
                    if i > 0 {
                        inl.push(Inline::HardBreak);
                    }
                    inl.push(Inline::Text { text: line.to_string(), marks: vec![] });
                }
                Block::Paragraph(inl)
            })
            .collect();
        Doc { blocks }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// All text, blocks separated by newlines (search indexing, JQL `text ~`).
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for b in &self.blocks {
            block_text(b, &mut out);
        }
        out.trim_end().to_string()
    }

    pub fn from_adf(v: &Value) -> Result<Doc, MarkupError> {
        if v.get("type").and_then(Value::as_str) != Some("doc") {
            return Err(MarkupError::NotADoc);
        }
        Ok(Doc { blocks: blocks_from(v.get("content"), 0)? })
    }

    pub fn to_adf(&self) -> Value {
        json!({"type": "doc", "version": 1, "content": self.blocks.iter().map(block_to).collect::<Vec<_>>()})
    }

    /// CommonMark-flavoured Markdown (export, CLI display).
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        for b in &self.blocks {
            block_md(b, "", &mut out);
            out.push('\n');
        }
        out.trim_end().to_string()
    }
}

fn block_text(b: &Block, out: &mut String) {
    match b {
        Block::Paragraph(i) | Block::Heading { content: i, .. } => {
            inline_text(i, out);
            out.push('\n');
        }
        Block::BulletList(items) | Block::OrderedList(items) => {
            for item in items {
                for b in item {
                    block_text(b, out);
                }
            }
        }
        Block::CodeBlock { text, .. } => {
            out.push_str(text);
            out.push('\n');
        }
        Block::Quote(bs) => bs.iter().for_each(|b| block_text(b, out)),
        Block::Rule | Block::Unknown(_) => {}
    }
}

fn inline_text(inl: &[Inline], out: &mut String) {
    for i in inl {
        match i {
            Inline::Text { text, .. } => out.push_str(text),
            Inline::HardBreak => out.push('\n'),
            Inline::Mention { text, .. } => out.push_str(text),
            Inline::Unknown(_) => {}
        }
    }
}

fn blocks_from(v: Option<&Value>, depth: usize) -> Result<Vec<Block>, MarkupError> {
    if depth > MAX_DEPTH {
        return Err(MarkupError::TooDeep);
    }
    let Some(arr) = v.and_then(Value::as_array) else { return Ok(vec![]) };
    arr.iter().map(|n| block_from(n, depth)).collect()
}

fn block_from(n: &Value, depth: usize) -> Result<Block, MarkupError> {
    let ty = n.get("type").and_then(Value::as_str).unwrap_or_default();
    let attrs = n.get("attrs");
    Ok(match ty {
        "paragraph" => Block::Paragraph(inlines_from(n.get("content"))),
        "heading" => {
            let level = attrs.and_then(|a| a.get("level")).and_then(Value::as_u64).unwrap_or(1).clamp(1, 6);
            Block::Heading { level: u8::try_from(level).unwrap_or(1), content: inlines_from(n.get("content")) }
        }
        "bulletList" | "orderedList" => {
            let mut items = Vec::new();
            for li in n.get("content").and_then(Value::as_array).into_iter().flatten() {
                items.push(blocks_from(li.get("content"), depth + 1)?);
            }
            if ty == "bulletList" { Block::BulletList(items) } else { Block::OrderedList(items) }
        }
        "codeBlock" => {
            let mut text = String::new();
            inline_text(&inlines_from(n.get("content")), &mut text);
            let language = attrs.and_then(|a| a.get("language")).and_then(Value::as_str).map(str::to_string);
            Block::CodeBlock { language, text }
        }
        "blockquote" => Block::Quote(blocks_from(n.get("content"), depth + 1)?),
        "rule" => Block::Rule,
        _ => Block::Unknown(n.clone()),
    })
}

fn inlines_from(v: Option<&Value>) -> Vec<Inline> {
    let Some(arr) = v.and_then(Value::as_array) else { return vec![] };
    arr.iter()
        .map(|n| match n.get("type").and_then(Value::as_str).unwrap_or_default() {
            "text" => Inline::Text {
                text: n.get("text").and_then(Value::as_str).unwrap_or_default().to_string(),
                marks: n.get("marks").and_then(Value::as_array).map(|m| m.iter().filter_map(mark_from).collect()).unwrap_or_default(),
            },
            "hardBreak" => Inline::HardBreak,
            "mention" => {
                let a = n.get("attrs");
                let s = |k: &str| a.and_then(|a| a.get(k)).and_then(Value::as_str).unwrap_or_default().to_string();
                Inline::Mention { id: s("id"), text: s("text") }
            }
            _ => Inline::Unknown(n.clone()),
        })
        .collect()
}

fn mark_from(m: &Value) -> Option<Mark> {
    Some(match m.get("type").and_then(Value::as_str)? {
        "strong" => Mark::Strong,
        "em" => Mark::Em,
        "code" => Mark::Code,
        "strike" => Mark::Strike,
        "underline" => Mark::Underline,
        "link" => Mark::Link(m.get("attrs")?.get("href")?.as_str()?.to_string()),
        _ => return None,
    })
}

fn block_to(b: &Block) -> Value {
    let list = |ty: &str, items: &[Vec<Block>]| json!({"type": ty, "content": items.iter().map(|i| json!({"type": "listItem", "content": i.iter().map(block_to).collect::<Vec<_>>()})).collect::<Vec<_>>()});
    match b {
        Block::Paragraph(i) => json!({"type": "paragraph", "content": inlines_to(i)}),
        Block::Heading { level, content } => json!({"type": "heading", "attrs": {"level": level}, "content": inlines_to(content)}),
        Block::BulletList(items) => list("bulletList", items),
        Block::OrderedList(items) => list("orderedList", items),
        Block::CodeBlock { language, text } => {
            let mut o = Map::new();
            o.insert("type".into(), "codeBlock".into());
            if let Some(l) = language {
                o.insert("attrs".into(), json!({"language": l}));
            }
            o.insert("content".into(), json!([{"type": "text", "text": text}]));
            Value::Object(o)
        }
        Block::Quote(bs) => json!({"type": "blockquote", "content": bs.iter().map(block_to).collect::<Vec<_>>()}),
        Block::Rule => json!({"type": "rule"}),
        Block::Unknown(v) => v.clone(),
    }
}

fn inlines_to(inl: &[Inline]) -> Vec<Value> {
    inl.iter()
        .map(|i| match i {
            Inline::Text { text, marks } if marks.is_empty() => json!({"type": "text", "text": text}),
            Inline::Text { text, marks } => json!({"type": "text", "text": text, "marks": marks.iter().map(mark_to).collect::<Vec<_>>()}),
            Inline::HardBreak => json!({"type": "hardBreak"}),
            Inline::Mention { id, text } => json!({"type": "mention", "attrs": {"id": id, "text": text}}),
            Inline::Unknown(v) => v.clone(),
        })
        .collect()
}

fn mark_to(m: &Mark) -> Value {
    match m {
        Mark::Strong => json!({"type": "strong"}),
        Mark::Em => json!({"type": "em"}),
        Mark::Code => json!({"type": "code"}),
        Mark::Strike => json!({"type": "strike"}),
        Mark::Underline => json!({"type": "underline"}),
        Mark::Link(h) => json!({"type": "link", "attrs": {"href": h}}),
    }
}

fn block_md(b: &Block, indent: &str, out: &mut String) {
    match b {
        Block::Paragraph(i) => {
            out.push_str(indent);
            inline_md(i, out);
            out.push('\n');
        }
        Block::Heading { level, content } => {
            out.push_str(&"#".repeat(usize::from(*level)));
            out.push(' ');
            inline_md(content, out);
            out.push('\n');
        }
        Block::BulletList(items) | Block::OrderedList(items) => {
            let ordered = matches!(b, Block::OrderedList(_));
            for (n, item) in items.iter().enumerate() {
                let bullet = if ordered { format!("{}. ", n.saturating_add(1)) } else { "- ".to_string() };
                let mut sub = String::new();
                for b in item {
                    block_md(b, "", &mut sub);
                }
                let pad = format!("{indent}{}", " ".repeat(bullet.len()));
                for (li, line) in sub.lines().enumerate() {
                    out.push_str(if li == 0 { indent } else { &pad });
                    if li == 0 {
                        out.push_str(&bullet);
                    }
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        Block::CodeBlock { language, text } => {
            out.push_str(&format!("```{}\n{text}\n```\n", language.as_deref().unwrap_or_default()));
        }
        Block::Quote(bs) => {
            let mut sub = String::new();
            bs.iter().for_each(|b| block_md(b, "", &mut sub));
            for line in sub.lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
        }
        Block::Rule => out.push_str("---\n"),
        Block::Unknown(_) => {}
    }
}

fn inline_md(inl: &[Inline], out: &mut String) {
    for i in inl {
        match i {
            Inline::Text { text, marks } => {
                let mut s = text.clone();
                for m in marks {
                    s = match m {
                        Mark::Strong => format!("**{s}**"),
                        Mark::Em => format!("*{s}*"),
                        Mark::Code => format!("`{s}`"),
                        Mark::Strike => format!("~~{s}~~"),
                        Mark::Underline => s,
                        Mark::Link(h) => format!("[{s}]({h})"),
                    };
                }
                out.push_str(&s);
            }
            Inline::HardBreak => out.push_str("  \n"),
            Inline::Mention { text, .. } => out.push_str(text),
            Inline::Unknown(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adf_round_trip_keeps_unknown_nodes() {
        let v = json!({"type":"doc","version":1,"content":[
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Plan"}]},
            {"type":"paragraph","content":[{"type":"text","text":"bold","marks":[{"type":"strong"}]}]},
            {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}]},
            {"type":"panel","attrs":{"panelType":"info"},"content":[]}
        ]});
        let d = Doc::from_adf(&v).unwrap();
        assert_eq!(d.to_adf(), v);
        assert_eq!(d.to_markdown(), "## Plan\n\n**bold**\n\n- a");
        assert_eq!(d.plain_text(), "Plan\nbold\na");
    }

    #[test]
    fn hostile_input() {
        assert_eq!(Doc::from_adf(&json!(42)), Err(MarkupError::NotADoc));
        let mut deep = json!({"type":"paragraph"});
        for _ in 0..200 {
            deep = json!({"type":"blockquote","content":[deep]});
        }
        assert_eq!(Doc::from_adf(&json!({"type":"doc","content":[deep]})), Err(MarkupError::TooDeep));
        let weird = json!({"type":"doc","content":[{"type":"heading","attrs":{"level":99999}}, 7, null]});
        assert!(Doc::from_adf(&weird).is_ok());
    }
}
