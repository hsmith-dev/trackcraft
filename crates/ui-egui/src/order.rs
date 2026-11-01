//! Rewriting a JQL query's `ORDER BY` clause for sortable list headers. The filter part is kept
//! byte for byte; only the clause after a top-level `ORDER BY` (outside quotes and parentheses,
//! any case) is replaced.

/// Splits `jql` at its top-level `ORDER BY`: (filter, order clause). Both are trimmed; the order
/// clause is `None` when the query has none.
pub(crate) fn split_order_by(jql: &str) -> (&str, Option<&str>) {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut prev: Option<char> = None;
    for (i, c) in jql.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            prev = Some(c);
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            'o' | 'O' if depth == 0 && !prev.is_some_and(ident_char) => {
                if let Some(order) = jql.get(i..).and_then(order_clause) {
                    return (jql.get(..i).unwrap_or_default().trim(), Some(order));
                }
            }
            _ => {}
        }
        prev = Some(c);
    }
    (jql.trim(), None)
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// `rest` starts at a candidate `order`: the text after `ORDER BY`, if that's what it is.
fn order_clause(rest: &str) -> Option<&str> {
    let after = rest.get(..5).filter(|w| w.eq_ignore_ascii_case("order")).and_then(|_| rest.get(5..))?;
    let trimmed = after.trim_start();
    if trimmed.len() == after.len() {
        return None; // `orderly`, `order_by`…
    }
    let by = trimmed.get(..2).filter(|w| w.eq_ignore_ascii_case("by")).and_then(|_| trimmed.get(2..))?;
    if by.chars().next().is_some_and(ident_char) {
        return None;
    }
    Some(by.trim())
}

/// The query's first sort key: (field, lower-cased and unquoted; descending?).
pub(crate) fn current_sort(jql: &str) -> Option<(String, bool)> {
    let (_, order) = split_order_by(jql);
    let first = order?.split(',').next()?.trim();
    let (field, dir) = match first.rsplit_once(char::is_whitespace) {
        Some((f, d)) if d.eq_ignore_ascii_case("asc") || d.eq_ignore_ascii_case("desc") => (f.trim(), d),
        _ => (first, "asc"),
    };
    let field = field.trim_matches(|c| c == '"' || c == '\'').to_lowercase();
    if field.is_empty() { None } else { Some((field, dir.eq_ignore_ascii_case("desc"))) }
}

/// `jql` sorted by `field`: ascending, or descending when it already sorts by `field` ascending.
pub(crate) fn sort_by(jql: &str, field: &str) -> String {
    let bare = field.trim_matches(|c| c == '"' || c == '\'').to_lowercase(); // M2.4: quoted custom field names
    let desc = current_sort(jql).is_some_and(|(f, d)| f == bare && !d);
    let dir = if desc { "DESC" } else { "ASC" };
    let (filter, _) = split_order_by(jql);
    if filter.is_empty() { format!("ORDER BY {field} {dir}") } else { format!("{filter} ORDER BY {field} {dir}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_at_top_level_order_by() {
        assert_eq!(split_order_by("project = LUME ORDER BY priority, updated DESC"), ("project = LUME", Some("priority, updated DESC")));
        assert_eq!(split_order_by("order by key"), ("", Some("key")));
        assert_eq!(split_order_by("status = Done\n  Order\tBy created"), ("status = Done", Some("created")));
        assert_eq!(split_order_by("project = LUME"), ("project = LUME", None));
        assert_eq!(split_order_by("  "), ("", None));
    }

    #[test]
    fn ignores_order_by_inside_quotes_parens_and_words() {
        assert_eq!(split_order_by("summary ~ \"order by me\""), ("summary ~ \"order by me\"", None));
        assert_eq!(split_order_by("summary ~ 'it\\'s order by' ORDER BY key"), ("summary ~ 'it\\'s order by'", Some("key")));
        assert_eq!(split_order_by("(text ~ x order by y)"), ("(text ~ x order by y)", None));
        assert_eq!(split_order_by("reorder by = 1"), ("reorder by = 1", None));
        assert_eq!(split_order_by("orderly = 1 AND x.order by"), ("orderly = 1 AND x.order by", None));
        assert_eq!(split_order_by("x = 1 ORDER BYE"), ("x = 1 ORDER BYE", None));
        // Unbalanced or hostile input never panics.
        assert_eq!(split_order_by("\"unterminated ORDER BY key"), ("\"unterminated ORDER BY key", None));
        assert_eq!(split_order_by(")) ORDER BY é"), ("))", Some("é")));
        assert_eq!(split_order_by("é ORDER"), ("é ORDER", None));
    }

    #[test]
    fn sort_toggles_direction_and_keeps_the_filter() {
        assert_eq!(current_sort("x = 1 ORDER BY Priority DESC, key"), Some(("priority".into(), true)));
        assert_eq!(current_sort("ORDER BY \"story points\""), Some(("story points".into(), false)));
        assert_eq!(current_sort("x = 1"), None);
        assert_eq!(sort_by("project = LUME ORDER BY priority", "key"), "project = LUME ORDER BY key ASC");
        assert_eq!(sort_by("project = LUME ORDER BY key", "key"), "project = LUME ORDER BY key DESC");
        assert_eq!(sort_by("project = LUME ORDER BY key DESC", "key"), "project = LUME ORDER BY key ASC");
        assert_eq!(sort_by("", "updated"), "ORDER BY updated ASC");
        assert_eq!(sort_by("summary ~ \"order by\"", "status"), "summary ~ \"order by\" ORDER BY status ASC");
    }
}
