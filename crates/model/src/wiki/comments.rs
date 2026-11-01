//! Page comments: footer comments with one level of replies, and inline comments anchored to a
//! text selection. An inline anchor stores the quoted text with its block and character offsets;
//! when the page changes the anchor is looked up again, and a comment whose quote is gone is
//! reported as outdated (kept, never lost).

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, Timestamp};
use trackcraft_markup::Doc;

use crate::{ModelError, Result, Workspace};

/// Most comments (including replies) on one page.
pub const MAX_PAGE_COMMENTS: usize = 5000;
/// Longest comment body, in characters of plain text.
pub const MAX_COMMENT_CHARS: usize = 32_000;
/// Longest quoted selection of an inline comment, in characters.
pub const MAX_QUOTE_CHARS: usize = 2000;

/// Where an inline comment points: top-level block `block`, characters `start..end` of that
/// block's plain text ([`trackcraft_markup::Block::plain_text`]), which read `quote` when the
/// comment was made on page version `version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineAnchor {
    pub block: u32,
    pub start: u32,
    pub end: u32,
    pub quote: String,
    #[serde(default)]
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageComment {
    pub id: Id,
    pub author: String,
    pub body: Doc,
    pub created: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<Timestamp>,
    /// The comment this replies to (always a top-level comment).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<InlineAnchor>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
}

/// What `page.comment` adds.
#[derive(Debug, Clone, Default)]
pub struct NewPageComment {
    pub body: Doc,
    pub parent: Option<Id>,
    pub anchor: Option<InlineAnchor>,
}

/// The current position of an anchor: `(block, start, end)` in characters.
pub type AnchorLocation = (u32, u32, u32);

fn check_body(body: &Doc) -> Result<()> {
    let text = body.plain_text();
    if text.trim().is_empty() {
        return Err(ModelError::Invalid("a comment can't be empty".into()));
    }
    if text.chars().count() > MAX_COMMENT_CHARS {
        return Err(ModelError::Invalid(format!("a comment is at most {MAX_COMMENT_CHARS} characters")));
    }
    Ok(())
}

/// Characters `start..end` of `s`, or `None` when out of range.
fn char_slice(s: &str, start: usize, end: usize) -> Option<&str> {
    if start > end {
        return None;
    }
    let mut idx = s.char_indices().map(|(i, _)| i).chain(std::iter::once(s.len()));
    let a = idx.nth(start)?;
    let b = if end == start { a } else { idx.nth(end - start - 1)? };
    s.get(a..b)
}

/// Finds an anchor in `doc`: at its stored offsets when the text there still matches, else the
/// first occurrence of the quote in the same block, else in any block. `None` = outdated.
pub fn locate_anchor(doc: &Doc, a: &InlineAnchor) -> Option<AnchorLocation> {
    if a.quote.is_empty() {
        return None;
    }
    let len = u32::try_from(a.quote.chars().count()).ok()?;
    let texts: Vec<String> = doc.blocks.iter().map(|b| b.plain_text()).collect();
    let block = usize::try_from(a.block).ok()?;
    if let Some(t) = texts.get(block)
        && char_slice(t, a.start as usize, a.end as usize) == Some(a.quote.as_str())
    {
        return Some((a.block, a.start, a.end));
    }
    let find = |bi: usize, t: &str| -> Option<AnchorLocation> {
        let byte = t.find(&a.quote)?;
        let start = u32::try_from(t.get(..byte)?.chars().count()).ok()?;
        Some((u32::try_from(bi).ok()?, start, start.checked_add(len)?))
    };
    if let Some(found) = texts.get(block).and_then(|t| find(block, t)) {
        return Some(found);
    }
    texts.iter().enumerate().find_map(|(bi, t)| find(bi, t))
}

impl Workspace {
    fn page_comment(&self, page: Id, comment: Id) -> Result<&PageComment> {
        self.page(page)?.comments.iter().find(|c| c.id == comment).ok_or(ModelError::NoSuchComment(comment))
    }

    /// Adds a footer comment, a reply (`parent`) or an inline comment (`anchor`, which must
    /// match the page's current text).
    pub fn add_page_comment(&mut self, page: Id, new: NewPageComment, author: &str, now: Timestamp) -> Result<Id> {
        let author = self.user(author)?.id.clone();
        check_body(&new.body)?;
        let p = self.page(page)?;
        if p.comments.len() >= MAX_PAGE_COMMENTS {
            return Err(ModelError::Invalid(format!("a page holds at most {MAX_PAGE_COMMENTS} comments")));
        }
        if let Some(parent) = new.parent {
            let pc = self.page_comment(page, parent)?;
            if pc.parent.is_some() {
                return Err(ModelError::Invalid("replies are one level deep: reply to the top comment of the thread".into()));
            }
            if new.anchor.is_some() {
                return Err(ModelError::Invalid("a reply can't have its own anchor; it follows its thread".into()));
            }
        }
        let anchor = match new.anchor {
            Some(mut a) => {
                if a.quote.chars().count() > MAX_QUOTE_CHARS {
                    return Err(ModelError::Invalid(format!("an inline comment quotes at most {MAX_QUOTE_CHARS} characters")));
                }
                if a.start >= a.end {
                    return Err(ModelError::Invalid("an inline comment needs a non-empty selection (start < end)".into()));
                }
                let block = usize::try_from(a.block).ok().and_then(|b| p.body.blocks.get(b));
                let block = block.ok_or_else(|| ModelError::Invalid(format!("the page has no block {}", a.block)))?;
                let text = block.plain_text();
                match char_slice(&text, a.start as usize, a.end as usize) {
                    Some(q) if q == a.quote => {}
                    Some(_) => return Err(ModelError::Invalid("the quoted text doesn't match the page at that selection".into())),
                    None => return Err(ModelError::Invalid("the selection is outside the block's text".into())),
                }
                a.version = p.version;
                Some(a)
            }
            None => None,
        };
        let id = self.ids.next_id();
        let p = self.pages.get_mut(&page).ok_or(ModelError::NoSuchPage(page))?;
        p.comments.push(PageComment {
            id,
            author: author.clone(),
            body: new.body,
            created: now,
            edited: None,
            parent: new.parent,
            anchor,
            resolved: false,
            resolved_by: None,
        });
        self.notify_page_comment(page, id, &author, now); // M4.5
        Ok(id)
    }

    fn own_comment_mut(&mut self, page: Id, comment: Id, user: &str) -> Result<&mut PageComment> {
        let c = self.pages.get_mut(&page).ok_or(ModelError::NoSuchPage(page))?.comments.iter_mut().find(|c| c.id == comment);
        let c = c.ok_or(ModelError::NoSuchComment(comment))?;
        if c.author != user {
            return Err(ModelError::Invalid("only the comment's author can change it".into()));
        }
        Ok(c)
    }

    /// Replaces a comment's body. Only its author may.
    pub fn edit_page_comment(&mut self, page: Id, comment: Id, body: Doc, user: &str, now: Timestamp) -> Result<()> {
        check_body(&body)?;
        let c = self.own_comment_mut(page, comment, user)?;
        c.body = body;
        c.edited = Some(now);
        Ok(())
    }

    /// Deletes a comment and its replies. Only its author may. Returns the removed ids.
    pub fn delete_page_comment(&mut self, page: Id, comment: Id, user: &str) -> Result<Vec<Id>> {
        self.own_comment_mut(page, comment, user)?;
        let p = self.pages.get_mut(&page).ok_or(ModelError::NoSuchPage(page))?;
        let gone: Vec<Id> = p.comments.iter().filter(|c| c.id == comment || c.parent == Some(comment)).map(|c| c.id).collect();
        p.comments.retain(|c| !gone.contains(&c.id));
        Ok(gone)
    }

    /// Resolves or reopens an inline comment thread (by its top comment). Anyone may.
    pub fn resolve_page_comment(&mut self, page: Id, comment: Id, resolved: bool, user: &str) -> Result<()> {
        let user = self.user(user)?.id.clone();
        let c = self.pages.get_mut(&page).ok_or(ModelError::NoSuchPage(page))?.comments.iter_mut().find(|c| c.id == comment);
        let c = c.ok_or(ModelError::NoSuchComment(comment))?;
        if c.parent.is_some() {
            return Err(ModelError::Invalid("resolve the thread's top comment, not a reply".into()));
        }
        if c.anchor.is_none() {
            return Err(ModelError::Invalid("only inline comments are resolved".into()));
        }
        c.resolved = resolved;
        c.resolved_by = resolved.then_some(user);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_slices() {
        assert_eq!(char_slice("héllo", 1, 3), Some("él"));
        assert_eq!(char_slice("héllo", 0, 5), Some("héllo"));
        assert_eq!(char_slice("héllo", 5, 5), Some(""));
        assert_eq!(char_slice("héllo", 4, 9), None);
        assert_eq!(char_slice("héllo", 3, 2), None);
        assert_eq!(char_slice("", 0, 0), Some(""));
    }

    #[test]
    fn anchors_relocate_or_go_outdated() {
        let a = InlineAnchor { block: 0, start: 4, end: 9, quote: "quick".into(), version: 1 };
        assert_eq!(locate_anchor(&Doc::from_markdown("the quick fox"), &a), Some((0, 4, 9)));
        assert_eq!(locate_anchor(&Doc::from_markdown("a very quick fox"), &a), Some((0, 7, 12)));
        assert_eq!(locate_anchor(&Doc::from_markdown("intro\n\nthe quick fox"), &a), Some((1, 4, 9)));
        assert_eq!(locate_anchor(&Doc::from_markdown("the slow fox"), &a), None);
        let far = InlineAnchor { block: u32::MAX, start: u32::MAX, end: u32::MAX, quote: "x".into(), version: 0 };
        assert_eq!(locate_anchor(&Doc::default(), &far), None);
    }
}
