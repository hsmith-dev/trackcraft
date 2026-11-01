//! A tolerant HTML/XHTML reader (enough for Confluence's HTML export pages and its storage
//! format) and the HTML → rich text conversion.
//!
//! The tokenizer never fails: unknown constructs become text, unmatched end tags are dropped,
//! unclosed elements close at the end. Elements nest at most [`MAX_HTML_DEPTH`] deep (deeper
//! start tags are ignored and their content joins the deepest element) and a page holds at
//! most [`MAX_NODES`] nodes, so hostile nesting can't exhaust the stack or memory.

use serde_json::{Value, json};
use trackcraft_markup::{Block, Doc, Inline, MAX_DEPTH, Mark};

use crate::MAX_HTML_DEPTH;

/// Most nodes kept from one page; the rest of the page is dropped (and reported).
pub const MAX_NODES: usize = 500_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    El(Element),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// Lowercased tag name (`p`, `ac:structured-macro`).
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    /// Byte span of the element in the source (start tag through end tag).
    pub start: usize,
    pub end: usize,
}

const VOID: [&str; 14] = ["br", "hr", "img", "meta", "link", "input", "col", "area", "base", "wbr", "source", "param", "embed", "track"];
const BLOCKS: [&str; 22] = [
    "p",
    "div",
    "ul",
    "ol",
    "li",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "pre",
    "blockquote",
    "table",
    "hr",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "dl",
    "form",
];

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class").is_some_and(|c| c.split_whitespace().any(|x| x == class))
    }

    /// The first element (depth-first, self included) matching `pred`.
    pub fn find(&self, pred: &dyn Fn(&Element) -> bool) -> Option<&Element> {
        if pred(self) {
            return Some(self);
        }
        self.children.iter().find_map(|c| match c {
            Node::El(e) => e.find(pred),
            Node::Text(_) => None,
        })
    }

    /// Every element matching `pred`, depth-first.
    pub fn find_all<'a>(&'a self, pred: &dyn Fn(&Element) -> bool, out: &mut Vec<&'a Element>) {
        if pred(self) {
            out.push(self);
        }
        for c in &self.children {
            if let Node::El(e) = c {
                e.find_all(pred, out);
            }
        }
    }

    pub fn by_id(&self, id: &str) -> Option<&Element> {
        self.find(&|e| e.attr("id") == Some(id))
    }

    pub fn child_elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Node::El(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// All text inside, whitespace collapsed.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.raw_text(&mut s);
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// All text inside, as written.
    pub fn raw_text(&self, out: &mut String) {
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(t),
                Node::El(e) if e.name == "br" => out.push('\n'),
                Node::El(e) => e.raw_text(out),
            }
        }
    }
}

/// Decodes character references (`&amp;`, `&#38;`, `&#x26;` and the common named ones).
pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(rest.get(..p).unwrap_or_default());
        let after = rest.get(p + 1..).unwrap_or_default();
        let semi = after.get(..after.len().min(12)).and_then(|w| w.find(';'));
        let decoded = semi.and_then(|q| {
            let name = after.get(..q)?;
            let c = if let Some(num) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                Some(u32::from_str_radix(num, 16).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            } else if let Some(num) = name.strip_prefix('#') {
                Some(num.parse::<u32>().ok().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            } else {
                named(name)
            }?;
            Some((c, q))
        });
        match decoded {
            Some((c, q)) => {
                out.push(c);
                rest = after.get(q + 1..).unwrap_or_default();
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn named(n: &str) -> Option<char> {
    Some(match n {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "bull" => '•',
        "middot" => '·',
        "times" => '×',
        "rarr" => '→',
        "larr" => '←',
        "deg" => '°',
        "euro" => '€',
        "pound" => '£',
        "sect" => '§',
        "para" => '¶',
        _ => return None,
    })
}

struct Builder {
    stack: Vec<Element>,
    nodes: usize,
    truncated: bool,
}

impl Builder {
    fn top(&mut self) -> Option<&mut Element> {
        self.stack.last_mut()
    }

    fn text(&mut self, t: String) {
        if t.is_empty() {
            return;
        }
        self.nodes = self.nodes.saturating_add(1);
        if let Some(top) = self.top() {
            if let Some(Node::Text(prev)) = top.children.last_mut() {
                prev.push_str(&t);
            } else {
                top.children.push(Node::Text(t));
            }
        }
    }

    fn pop(&mut self, end: usize) {
        if self.stack.len() <= 1 {
            return;
        }
        if let Some(mut el) = self.stack.pop() {
            el.end = end;
            if let Some(top) = self.top() {
                top.children.push(Node::El(el));
            }
        }
    }

    /// Closes the innermost open `names` element above the nearest `bounds` element.
    fn close_open(&mut self, names: &[&str], bounds: &[&str], at: usize) {
        let mut idx = None;
        for (i, e) in self.stack.iter().enumerate().skip(1).rev() {
            if bounds.contains(&e.name.as_str()) {
                break;
            }
            if names.contains(&e.name.as_str()) {
                idx = Some(i);
                break;
            }
        }
        if let Some(i) = idx {
            while self.stack.len() > i {
                self.pop(at);
            }
        }
    }

    fn open(&mut self, el: Element, self_closing: bool) {
        let at = el.start;
        let name = el.name.as_str();
        if BLOCKS.contains(&name) && self.stack.last().is_some_and(|t| t.name == "p") {
            self.pop(at);
        }
        match name {
            "li" => self.close_open(&["li"], &["ul", "ol"], at),
            "tr" => self.close_open(&["tr", "td", "th"], &["table", "tbody", "thead", "tfoot"], at),
            "td" | "th" => self.close_open(&["td", "th"], &["tr", "table"], at),
            _ => {}
        }
        self.nodes = self.nodes.saturating_add(1);
        let void = self_closing || VOID.contains(&el.name.as_str());
        if void || self.stack.len() >= MAX_HTML_DEPTH {
            let mut el = el;
            el.end = at;
            if void && let Some(top) = self.top() {
                top.children.push(Node::El(el));
            }
            return;
        }
        self.stack.push(el);
    }

    fn close(&mut self, name: &str, end: usize) {
        if let Some(i) = self.stack.iter().skip(1).rposition(|e| e.name == name) {
            let i = i + 1;
            while self.stack.len() > i {
                self.pop(end);
            }
        }
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.')
}

/// Parses a start tag at `s[pos] == '<'`; returns the element and the offset after `>`.
fn start_tag(s: &str, pos: usize) -> Option<(Element, bool, usize)> {
    let rest = s.get(pos + 1..)?;
    let name_len = rest.find(|c: char| !is_name_char(c)).unwrap_or(rest.len());
    if name_len == 0 || !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let name = rest.get(..name_len)?.to_ascii_lowercase();
    let mut i = pos + 1 + name_len;
    let mut attrs = Vec::new();
    let bytes = s.as_bytes();
    loop {
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        match bytes.get(i) {
            None => return Some((Element { name, attrs, start: pos, ..Element::default() }, false, s.len())),
            Some(b'>') => return Some((Element { name, attrs, start: pos, ..Element::default() }, false, i + 1)),
            Some(b'/') if bytes.get(i + 1) == Some(&b'>') => return Some((Element { name, attrs, start: pos, ..Element::default() }, true, i + 2)),
            Some(b'/') => i += 1,
            Some(_) => {
                let a = s.get(i..)?;
                let n = a.find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '>' || c == '/').unwrap_or(a.len()).max(1);
                let key = a.get(..n)?.to_ascii_lowercase();
                i += n;
                while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                let mut value = String::new();
                if bytes.get(i) == Some(&b'=') {
                    i += 1;
                    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                        i += 1;
                    }
                    match bytes.get(i) {
                        Some(&q) if q == b'"' || q == b'\'' => {
                            let from = i + 1;
                            let len = s.get(from..)?.find(char::from(q)).unwrap_or(s.len() - from);
                            value = decode(s.get(from..from + len)?);
                            i = (from + len + 1).min(s.len());
                        }
                        _ => {
                            let v = s.get(i..)?;
                            let len = v.find(|c: char| c.is_ascii_whitespace() || c == '>').unwrap_or(v.len());
                            value = decode(v.get(..len)?);
                            i += len;
                        }
                    }
                }
                if attrs.len() < 64 {
                    attrs.push((key, value));
                }
            }
        }
    }
}

/// Byte offset of `needle` (ASCII) in `hay`, ignoring ASCII case, without allocating.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h.get(i..i + n.len()).is_some_and(|w| w.eq_ignore_ascii_case(n)))
}

/// Parses HTML or XHTML into a tree under a `#root` element. The flag says whether
/// [`MAX_NODES`] cut the page short.
pub fn parse(s: &str) -> (Element, bool) {
    let mut b = Builder { stack: vec![Element { name: "#root".into(), end: s.len(), ..Element::default() }], nodes: 0, truncated: false };
    let mut pos = 0usize;
    while pos < s.len() {
        if b.nodes >= MAX_NODES {
            b.truncated = true;
            break;
        }
        let rest = s.get(pos..).unwrap_or_default();
        if !rest.starts_with('<') {
            let len = rest.find('<').unwrap_or(rest.len());
            b.text(decode(rest.get(..len).unwrap_or_default()));
            pos += len.max(1);
            continue;
        }
        if rest.starts_with("<!--") {
            pos += rest.find("-->").map_or(rest.len(), |p| p + 3);
            continue;
        }
        if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            let len = cdata.find("]]>").unwrap_or(cdata.len());
            b.text(cdata.get(..len).unwrap_or_default().to_string());
            pos += 9 + (len + 3).min(cdata.len());
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            pos += rest.find('>').map_or(rest.len(), |p| p + 1);
            continue;
        }
        if let Some(end) = rest.strip_prefix("</") {
            let n = end.find(|c: char| !is_name_char(c)).unwrap_or(end.len());
            let name = end.get(..n).unwrap_or_default().to_ascii_lowercase();
            let close = rest.find('>').map_or(rest.len(), |p| p + 1);
            pos += close;
            if !name.is_empty() {
                b.close(&name, pos);
            }
            continue;
        }
        match start_tag(s, pos) {
            Some((el, self_closing, next)) => {
                let name = el.name.clone();
                b.open(el, self_closing);
                pos = next.max(pos + 1);
                if (name == "script" || name == "style") && !self_closing {
                    let rest = s.get(pos..).unwrap_or_default();
                    pos += find_ci(rest, &format!("</{name}")).unwrap_or(rest.len());
                }
            }
            None => {
                b.text("<".into());
                pos += 1;
            }
        }
    }
    while b.stack.len() > 1 {
        b.pop(s.len());
    }
    let truncated = b.truncated;
    (b.stack.pop().unwrap_or_default(), truncated)
}

/// Converts parsed HTML into rich text, counting what it couldn't keep.
pub struct Converter<'a> {
    src: &'a str,
    pub images: usize,
    pub macros: usize,
}

const INLINE: [&str; 30] = [
    "a", "strong", "b", "em", "i", "cite", "code", "tt", "kbd", "samp", "s", "del", "strike", "u", "ins", "span", "font", "sub", "sup", "br", "img",
    "small", "big", "abbr", "mark", "q", "time", "label", "var", "dfn",
];

fn safe_href(h: &str) -> bool {
    let l = h.trim().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:")
}

fn with(marks: &[Mark], m: Mark) -> Vec<Mark> {
    let mut v = marks.to_vec();
    if !v.contains(&m) {
        v.push(m);
    }
    v
}

fn collapse(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut space = false;
    for c in t.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(if c == '\u{a0}' { ' ' } else { c });
            space = false;
        }
    }
    out
}

/// Tidies a run of inlines into a paragraph's content: merges neighbours with equal marks,
/// collapses spaces across them, trims the ends. `None` when nothing visible is left.
fn tidy(inl: Vec<Inline>) -> Option<Vec<Inline>> {
    let mut out: Vec<Inline> = Vec::new();
    for i in inl {
        match (out.last_mut(), i) {
            (Some(Inline::Text { text: a, marks: ma }), Inline::Text { text: b, marks: mb }) if *ma == mb => {
                if a.ends_with(' ') {
                    a.push_str(b.trim_start_matches(' '));
                } else {
                    a.push_str(&b);
                }
            }
            (Some(Inline::Text { text: a, .. }), Inline::Text { text: b, marks }) => {
                let b = if a.ends_with(' ') { b.trim_start_matches(' ').to_string() } else { b };
                if !b.is_empty() {
                    out.push(Inline::Text { text: b, marks });
                }
            }
            (_, Inline::Text { text, marks }) => {
                let lead = matches!(out.last(), None | Some(Inline::HardBreak));
                let text = if lead { text.trim_start_matches(' ').to_string() } else { text };
                if !text.is_empty() {
                    out.push(Inline::Text { text, marks });
                }
            }
            (_, other) => {
                if matches!(other, Inline::HardBreak)
                    && let Some(Inline::Text { text, .. }) = out.last_mut()
                {
                    let t = text.trim_end_matches(' ').len();
                    text.truncate(t);
                }
                out.push(other);
            }
        }
    }
    while matches!(out.last(), Some(Inline::HardBreak)) {
        out.pop();
    }
    if let Some(Inline::Text { text, .. }) = out.last_mut() {
        let t = text.trim_end_matches(' ').len();
        text.truncate(t);
    }
    out.retain(|i| !matches!(i, Inline::Text { text, .. } if text.is_empty()));
    let visible = out.iter().any(|i| match i {
        Inline::Text { text, .. } => !text.trim().is_empty(),
        Inline::HardBreak => false,
        _ => true,
    });
    visible.then_some(out)
}

impl<'a> Converter<'a> {
    pub fn new(src: &'a str) -> Self {
        Converter { src, images: 0, macros: 0 }
    }

    pub fn doc(&mut self, el: &Element) -> Doc {
        Doc { blocks: self.blocks(el, 0) }
    }

    fn raw(&self, el: &Element) -> String {
        self.src.get(el.start..el.end.max(el.start)).unwrap_or_default().chars().take(200_000).collect()
    }

    fn macro_param(el: &Element, name: &str) -> Option<String> {
        el.child_elements().find(|c| c.name == "ac:parameter" && c.attr("ac:name") == Some(name)).map(Element::text)
    }

    fn extension(&mut self, el: &Element, inline: bool) -> Value {
        self.macros += 1;
        let key = el.attr("ac:name").unwrap_or(el.name.as_str()).to_string();
        let mut params = serde_json::Map::new();
        for p in el.child_elements().filter(|c| c.name == "ac:parameter").take(64) {
            if let Some(n) = p.attr("ac:name") {
                params.insert(n.to_string(), json!({"value": p.text()}));
            }
        }
        json!({
            "type": if inline { "inlineExtension" } else { "extension" },
            "attrs": {"extensionType": "com.atlassian.confluence.macro.core", "extensionKey": key, "parameters": {"macroParams": params}},
            "storage": self.raw(el),
        })
    }

    fn has_block_child(el: &Element) -> bool {
        el.child_elements().any(|c| BLOCKS.contains(&c.name.as_str()) || c.name == "ac:structured-macro" || c.name == "table")
    }

    fn blocks(&mut self, el: &Element, depth: usize) -> Vec<Block> {
        let mut out = Vec::new();
        let mut inl: Vec<Inline> = Vec::new();
        let flush = |inl: &mut Vec<Inline>, out: &mut Vec<Block>| {
            if let Some(p) = tidy(std::mem::take(inl)) {
                out.push(Block::Paragraph(p));
            }
        };
        for c in &el.children {
            let e = match c {
                Node::Text(t) => {
                    inl.push(Inline::Text { text: collapse(t), marks: vec![] });
                    continue;
                }
                Node::El(e) => e,
            };
            let name = e.name.as_str();
            let is_inline = INLINE.contains(&name)
                || (name.starts_with("ac:")
                    && matches!(name, "ac:link" | "ac:emoticon" | "ac:image" | "ac:inline-comment-marker" | "ac:placeholder"))
                || name.starts_with("ri:")
                || (name == "ac:structured-macro" && el.name == "p");
            if is_inline {
                self.inline_el(e, &[], &mut inl);
                continue;
            }
            flush(&mut inl, &mut out);
            match name {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = name.get(1..).and_then(|d| d.parse::<u8>().ok()).unwrap_or(1);
                    let mut h = Vec::new();
                    self.inline(e, &[], &mut h);
                    if let Some(h) = tidy(h) {
                        out.push(Block::Heading { level, content: h });
                    }
                }
                "p" if !Self::has_block_child(e) => {
                    let mut p = Vec::new();
                    self.inline(e, &[], &mut p);
                    if let Some(p) = tidy(p) {
                        out.push(Block::Paragraph(p));
                    }
                }
                "ul" | "ol" if depth < MAX_DEPTH - 4 => {
                    let mut items = Vec::new();
                    for li in e.child_elements() {
                        items.push(self.blocks(li, depth + 1));
                    }
                    items.retain(|i| !i.is_empty());
                    if !items.is_empty() {
                        out.push(if name == "ol" { Block::OrderedList(items) } else { Block::BulletList(items) });
                    }
                }
                "blockquote" if depth < MAX_DEPTH - 4 => {
                    let inner = self.blocks(e, depth + 1);
                    if !inner.is_empty() {
                        out.push(Block::Quote(inner));
                    }
                }
                "pre" => {
                    let mut text = String::new();
                    e.raw_text(&mut text);
                    let language = e
                        .attr("data-syntaxhighlighter-params")
                        .and_then(|p| p.split(';').find_map(|kv| kv.trim().strip_prefix("brush:")))
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty() && l.len() <= 40);
                    out.push(Block::CodeBlock { language, text: text.trim_matches('\n').to_string() });
                }
                "hr" => out.push(Block::Rule),
                "table" => out.push(self.table(e)),
                "ac:structured-macro" => {
                    let m = e.attr("ac:name").unwrap_or_default();
                    if m == "code" || m == "noformat" {
                        let body = e.find(&|x| x.name == "ac:plain-text-body").map(|b| {
                            let mut t = String::new();
                            b.raw_text(&mut t);
                            t
                        });
                        let language = Self::macro_param(e, "language").filter(|l| !l.is_empty() && l.len() <= 40);
                        out.push(Block::CodeBlock { language, text: body.unwrap_or_default().trim_matches('\n').to_string() });
                    } else {
                        out.push(Block::Unknown(self.extension(e, false)));
                    }
                }
                "ac:layout" | "ac:layout-section" | "ac:layout-cell" | "ac:rich-text-body" => out.extend(self.blocks(e, depth)),
                n if n.starts_with("ac:") => out.push(Block::Unknown(self.extension(e, false))),
                "script" | "style" | "head" | "title" | "meta" | "link" | "noscript" | "template" => {}
                _ => out.extend(self.blocks(e, depth)),
            }
        }
        flush(&mut inl, &mut out);
        out
    }

    fn inline(&mut self, el: &Element, marks: &[Mark], out: &mut Vec<Inline>) {
        for c in &el.children {
            match c {
                Node::Text(t) => out.push(Inline::Text { text: collapse(t), marks: marks.to_vec() }),
                Node::El(e) => self.inline_el(e, marks, out),
            }
        }
    }

    fn inline_el(&mut self, e: &Element, marks: &[Mark], out: &mut Vec<Inline>) {
        let m = |mk: Mark| with(marks, mk);
        match e.name.as_str() {
            "strong" | "b" => self.inline(e, &m(Mark::Strong), out),
            "em" | "i" | "cite" | "dfn" | "var" => self.inline(e, &m(Mark::Em), out),
            "code" | "tt" | "kbd" | "samp" => self.inline(e, &m(Mark::Code), out),
            "s" | "del" | "strike" => self.inline(e, &m(Mark::Strike), out),
            "u" | "ins" => self.inline(e, &m(Mark::Underline), out),
            "a" => match e.attr("href").filter(|h| safe_href(h)) {
                Some(h) => self.inline(e, &m(Mark::Link(h.trim().chars().take(2000).collect())), out),
                None => self.inline(e, marks, out),
            },
            "br" => out.push(Inline::HardBreak),
            "img" => {
                if !e.has_class("emoticon") {
                    self.images += 1;
                    let name = e.attr("alt").filter(|a| !a.trim().is_empty()).or_else(|| e.attr("src").and_then(|s| s.rsplit('/').next()));
                    if let Some(n) = name {
                        out.push(Inline::Text { text: format!("[image: {}]", crate::clip(n.trim(), 200)), marks: marks.to_vec() });
                    }
                }
            }
            "ac:image" => {
                self.images += 1;
                let name = e.find(&|x| x.name == "ri:attachment" || x.name == "ri:url").and_then(|r| r.attr("ri:filename").or(r.attr("ri:value")));
                out.push(Inline::Text { text: format!("[image: {}]", crate::clip(name.unwrap_or("image"), 200)), marks: marks.to_vec() });
            }
            "ac:emoticon" | "ac:placeholder" | "script" | "style" => {}
            "ac:link" => {
                let body = e.child_elements().find(|c| c.name == "ac:link-body" || c.name == "ac:plain-text-link-body");
                match body {
                    Some(b) => self.inline(b, marks, out),
                    None => {
                        let target = e.find(&|x| x.name.starts_with("ri:"));
                        let text = target
                            .and_then(|t| t.attr("ri:content-title").or(t.attr("ri:filename")).or(t.attr("ri:space-key")).or(t.attr("ri:account-id")))
                            .unwrap_or_default();
                        if !text.is_empty() {
                            out.push(Inline::Text { text: text.to_string(), marks: marks.to_vec() });
                        }
                    }
                }
            }
            "ac:structured-macro" => out.push(Inline::Unknown(self.extension(e, true))),
            _ => self.inline(e, marks, out),
        }
    }

    /// An HTML table as an ADF `table` node (kept as an unknown block until tables are modelled).
    fn table(&mut self, t: &Element) -> Block {
        let mut rows: Vec<&Element> = Vec::new();
        t.find_all(&|e| e.name == "tr", &mut rows);
        let rows: Vec<Value> = rows
            .into_iter()
            .take(10_000)
            .map(|r| {
                let cells: Vec<Value> = r
                    .child_elements()
                    .filter(|c| c.name == "td" || c.name == "th")
                    .take(1_000)
                    .map(|c| {
                        let mut inl = Vec::new();
                        self.inline(c, &[], &mut inl);
                        let para = crate::wiki::paragraph_adf(tidy(inl).unwrap_or_default());
                        json!({"type": if c.name == "th" { "tableHeader" } else { "tableCell" }, "attrs": {}, "content": [para]})
                    })
                    .collect();
                json!({"type": "tableRow", "content": cells})
            })
            .collect();
        Block::Unknown(json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": rows}))
    }
}

/// Converts an HTML fragment to rich text (images and macros counted in the returned tuple).
pub fn to_doc(src: &str) -> (Doc, usize, usize) {
    let (root, _) = parse(src);
    let mut c = Converter::new(src);
    let d = c.doc(&root);
    (d, c.images, c.macros)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(s: &str) -> String {
        to_doc(s).0.to_markdown()
    }

    #[test]
    fn basic_html() {
        assert_eq!(md("<h2>Plan</h2><p>Some <strong>bold</strong> and <em>it</em>.</p>"), "## Plan\n\nSome **bold** and *it*.");
        assert_eq!(md("<ul><li>a</li><li>b<ol><li>c</li></ol></li></ul>"), "- a\n- b\n  1. c");
        assert_eq!(md("<pre>let x = 1;\n  y</pre>"), "```\nlet x = 1;\n  y\n```");
        assert_eq!(md("<blockquote><p>said</p></blockquote><hr/>"), "> said\n\n---");
        assert_eq!(md("<p><a href=\"https://e.com\">e</a> <a href=\"javascript:x\">j</a></p>"), "[e](https://e.com) j");
        assert_eq!(md("<p>a<br>b &amp; &lt;c&gt; &#x41;&#66;&bogus;</p>"), "a  \nb & <c> AB&bogus;");
        assert_eq!(md("<div>loose <b>text</b><p>para</p></div>"), "loose **text**\n\npara");
        assert_eq!(md("<p>one<p>two"), "one\n\ntwo");
        assert_eq!(md("<ul><li>x<li>y</ul>"), "- x\n- y");
        assert_eq!(md("<script>alert(1)</script><style>p{}</style><p>ok</p>"), "ok");
        assert_eq!(md("<p>   spaced    out  </p>"), "spaced out");
    }

    #[test]
    fn storage_format() {
        let src = r#"<p>Intro <ac:structured-macro ac:name="status"><ac:parameter ac:name="title">OK</ac:parameter></ac:structured-macro></p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">rust</ac:parameter><ac:plain-text-body><![CDATA[fn main() { <x> }]]></ac:plain-text-body></ac:structured-macro>
<ac:structured-macro ac:name="info"><ac:rich-text-body><p>Note this</p></ac:rich-text-body></ac:structured-macro>
<p><ac:link><ri:page ri:content-title="Other page"/></ac:link></p>"#;
        let (d, _, macros) = to_doc(src);
        assert_eq!(macros, 2);
        assert!(matches!(&d.blocks[1], Block::CodeBlock { language: Some(l), text } if l == "rust" && text == "fn main() { <x> }"));
        let Block::Unknown(info) = &d.blocks[2] else { panic!("{d:?}") };
        assert_eq!(info["attrs"]["extensionKey"], "info");
        assert!(info["storage"].as_str().unwrap().starts_with("<ac:structured-macro ac:name=\"info\">"));
        assert!(info["storage"].as_str().unwrap().ends_with("</ac:structured-macro>"));
        assert!(matches!(&d.blocks[0], Block::Paragraph(i) if matches!(i.last(), Some(Inline::Unknown(_)))));
        assert_eq!(d.blocks[3].plain_text(), "Other page");
    }

    #[test]
    fn hostile_html() {
        let deep = "<div>".repeat(100_000) + "x" + &"</div>".repeat(100_000);
        let (root, _) = parse(&deep);
        assert_eq!(to_doc(&deep).0.plain_text(), "x");
        drop(root);
        let lists = "<ul><li>".repeat(5_000) + "x";
        assert!(Doc::from_adf(&to_doc(&lists).0.to_adf()).is_ok());
        let quotes = "<blockquote>".repeat(5_000) + "x";
        assert!(Doc::from_adf(&to_doc(&quotes).0.to_adf()).is_ok());
        for junk in [
            "<",
            "</",
            "<a",
            "<a href='",
            "<!--",
            "<![CDATA[",
            "&#xFFFFFFFF;",
            "&#99999999999;",
            "<p\u{0}>",
            "<é>",
            "</p></p></div>",
            "<a b c=d e='f' g=\"h\"/>",
        ] {
            let _ = to_doc(junk);
        }
        let many = "<b>".repeat(MAX_NODES + 10);
        assert!(parse(&many).1);
    }
}
