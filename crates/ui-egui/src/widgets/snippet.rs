//! Short excerpts of longer text (UI-fixes): cut at a word boundary, never inside a character,
//! with an ellipsis when something was left out.

/// The longest prefix of `text` with at most `max` characters that ends at a word boundary
/// (trailing whitespace trimmed). A first word longer than `max` is cut at `max` characters.
pub(crate) fn word_prefix(text: &str, max: usize) -> &str {
    let cut = match text.char_indices().nth(max) {
        Some((i, _)) => i,
        None => return text.trim_end(),
    };
    let (head, rest) = text.split_at(cut);
    // Cutting just before whitespace already ends a word.
    if rest.starts_with(char::is_whitespace) {
        return head.trim_end();
    }
    match head.rfind(char::is_whitespace).and_then(|i| head.get(..i)).map(str::trim_end) {
        Some(h) if !h.is_empty() => h,
        _ => head.trim_end(),
    }
}

/// `text` on one line (runs of whitespace become one space), cut to at most `max` characters at
/// a word boundary, with "…" when cut.
pub(crate) fn snippet(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let head = word_prefix(&flat, max);
    if head.len() < flat.len() { format!("{head}\u{2026}") } else { flat }
}

/// A quote taken from the start of `source` (when known), shown short: a quote that stops before
/// `source` does, or one longer than `max`, ends at a word boundary with "…".
pub(crate) fn quote_excerpt(quote: &str, source: Option<&str>, max: usize) -> String {
    let flat = quote.split_whitespace().collect::<Vec<_>>().join(" ");
    let more = source.is_some_and(|s| {
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        s.len() > flat.len() && s.starts_with(&flat)
    });
    let mid_word = source.is_some_and(|s| {
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        s.get(flat.len()..).and_then(|r| r.chars().next()).is_some_and(|c| !c.is_whitespace())
    });
    if !more || flat.is_empty() {
        return snippet(&flat, max);
    }
    // The quote stops inside the source: drop a half word and say there's more.
    let head = if mid_word {
        match flat.rfind(' ') {
            Some(i) => flat.get(..i).unwrap_or(&flat),
            None => flat.as_str(),
        }
    } else {
        flat.as_str()
    };
    format!("{}\u{2026}", word_prefix(head, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_at_word_boundaries_and_char_boundaries() {
        assert_eq!(word_prefix("hello world", 20), "hello world");
        assert_eq!(word_prefix("hello world again", 8), "hello");
        assert_eq!(word_prefix("hello world again", 11), "hello world");
        assert_eq!(word_prefix("supercalifragilistic", 5), "super");
        assert_eq!(word_prefix("ééé ééé", 5), "ééé");
        assert_eq!(word_prefix("", 5), "");
        assert_eq!(word_prefix("   ab", 3), "");
        assert_eq!(snippet("Storage is\n\n local first and fast", 18), "Storage is local\u{2026}");
        assert_eq!(snippet("short", 18), "short");
        assert_eq!(snippet(&"漢".repeat(100), 10), format!("{}\u{2026}", "漢".repeat(10)));
    }

    #[test]
    fn quotes_end_on_a_word_with_an_ellipsis() {
        let src = "The app is a thin client over the engine";
        // Cut mid-word ("thi"): the half word goes.
        assert_eq!(quote_excerpt("The app is a thi", Some(src), 80), "The app is a\u{2026}");
        // Cut at a word end: kept, with an ellipsis.
        assert_eq!(quote_excerpt("The app is a", Some(src), 80), "The app is a\u{2026}");
        // The whole block: no ellipsis.
        assert_eq!(quote_excerpt(src, Some(src), 80), src);
        // Outdated (the block is gone or changed): only the length matters.
        assert_eq!(quote_excerpt("The app is a thin", None, 12), "The app is a\u{2026}");
        assert_eq!(quote_excerpt("The app", Some("Other text"), 80), "The app");
        assert_eq!(quote_excerpt("", Some(src), 80), "");
    }
}
