//! The wiki: spaces and their page trees, with full version history.

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, ProjectKey, Timestamp};
use trackcraft_markup::Doc;

use crate::{MAX_PAGE_DEPTH, MAX_TITLE, ModelError, Result, Workspace, labels, title};

mod comments;
mod templates;

pub use comments::{AnchorLocation, InlineAnchor, MAX_COMMENT_CHARS, MAX_PAGE_COMMENTS, MAX_QUOTE_CHARS, NewPageComment, PageComment, locate_anchor};
pub use templates::{
    BUILTIN_TEMPLATES, BuiltinTemplate, MAX_TEMPLATE_BYTES, MAX_TEMPLATES, NewTemplate, ResolvedTemplate, Template, builtin_template, expand_doc,
    expand_placeholders,
};

/// Most pages one `page.copy` duplicates.
pub const MAX_COPY: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub key: ProjectKey,
    pub name: String,
    pub description: Doc,
    /// The space's overview page, created with the space.
    pub homepage: Id,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageVersion {
    pub version: u32,
    pub title: String,
    pub body: Doc,
    pub author: String,
    pub at: Timestamp,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: Id,
    pub space: ProjectKey,
    pub parent: Option<Id>,
    /// Position among siblings (lower first).
    pub position: u32,
    pub title: String,
    pub body: Doc,
    pub labels: Vec<String>,
    pub version: u32,
    pub author: String,
    pub created: Timestamp,
    pub updated: Timestamp,
    /// The change message of the current version.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// Earlier versions, oldest first (the current one is the page itself).
    pub history: Vec<PageVersion>,
    /// Footer and inline comments, oldest first (added after schema 1; older files have none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<PageComment>,
}

/// What `page.edit` changes; `None` keeps the current value.
#[derive(Debug, Clone, Default)]
pub struct PageEdit {
    pub title: Option<String>,
    pub body: Option<Doc>,
    pub base_version: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct NewPage {
    pub space: String,
    pub parent: Option<Id>,
    pub title: String,
    pub body: Doc,
}

/// Where `page.move` puts a page among its new siblings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    #[default]
    Last,
    Before(Id),
    After(Id),
}

/// What `page.copy` duplicates and where to.
#[derive(Debug, Clone, Default)]
pub struct CopyPage {
    pub id: Id,
    pub parent: Option<Id>,
    pub space: Option<String>,
    pub title: Option<String>,
    pub children: bool,
}

impl Workspace {
    pub fn space(&self, key: &str) -> Result<&Space> {
        let k = ProjectKey::parse(&key.to_ascii_uppercase()).map_err(|_| ModelError::NoSuchSpace(key.to_string()))?;
        self.spaces.get(&k).ok_or_else(|| ModelError::NoSuchSpace(key.to_string()))
    }

    pub fn page(&self, id: Id) -> Result<&Page> {
        self.pages.get(&id).ok_or(ModelError::NoSuchPage(id))
    }

    pub fn create_space(&mut self, key: &str, name: &str, author: &str, now: Timestamp) -> Result<ProjectKey> {
        let key = ProjectKey::parse(key).map_err(|e| ModelError::Invalid(e.to_string()))?;
        if self.spaces.contains_key(&key) {
            return Err(ModelError::Duplicate(format!("space {key}")));
        }
        let name = title(name, "the space name")?;
        let author = self.user(author)?.id.clone();
        let homepage = self.ids.next_id();
        self.pages.insert(homepage, new_page(homepage, key.clone(), None, 0, name.clone(), Doc::default(), author, now));
        self.spaces.insert(key.clone(), Space { key: key.clone(), name, description: Doc::default(), homepage });
        Ok(key)
    }

    pub fn create_page(&mut self, new: NewPage, author: &str, now: Timestamp) -> Result<Id> {
        let author = self.user(author)?.id.clone();
        let space = self.space(&new.space)?;
        let skey = space.key.clone();
        let parent = new.parent.unwrap_or(space.homepage);
        let parent_page = self.page(parent)?;
        if parent_page.space != skey {
            return Err(ModelError::Invalid("the parent page is in another space".into()));
        }
        if self.depth(parent) >= MAX_PAGE_DEPTH {
            return Err(ModelError::Invalid(format!("pages can nest at most {MAX_PAGE_DEPTH} levels deep")));
        }
        let t = title(&new.title, "the page title")?;
        self.check_unique_title(&skey, &t, None)?;
        let position = u32::try_from(self.children(parent).len()).unwrap_or(u32::MAX);
        let id = self.ids.next_id();
        self.pages.insert(id, new_page(id, skey, Some(parent), position, t, new.body, author, now));
        Ok(id)
    }

    /// Saves a new version. `edit.base_version` is the version the editor started from; a
    /// mismatch means someone else saved in between (the caller merges or asks).
    pub fn edit_page(&mut self, id: Id, edit: PageEdit, author: &str, now: Timestamp) -> Result<u32> {
        let author = self.user(author)?.id.clone();
        let page = self.page(id)?;
        if let Some(b) = edit.base_version
            && b != page.version
        {
            return Err(ModelError::StaleVersion { expected: b, actual: page.version });
        }
        let t = match edit.title.as_deref() {
            Some(t) => {
                let t = title(t, "the page title")?;
                self.check_unique_title(&page.space, &t, Some(id))?;
                t
            }
            None => page.title.clone(),
        };
        let p = self.pages.get_mut(&id).ok_or(ModelError::NoSuchPage(id))?;
        p.history.push(PageVersion {
            version: p.version,
            title: p.title.clone(),
            body: p.body.clone(),
            author: p.author.clone(),
            at: p.updated,
            message: std::mem::take(&mut p.message),
        });
        p.version = p.version.saturating_add(1);
        p.title = t;
        if let Some(b) = edit.body {
            p.body = b;
        }
        p.author = author;
        p.updated = now;
        p.message = edit.message.chars().take(1000).collect();
        Ok(p.version)
    }

    /// Restores an earlier version as a new version (history is never rewritten).
    pub fn restore_page(&mut self, id: Id, version: u32, author: &str, now: Timestamp) -> Result<u32> {
        let page = self.page(id)?;
        let old = page
            .history
            .iter()
            .find(|v| v.version == version)
            .ok_or_else(|| ModelError::Invalid(format!("page {id} has no earlier version {version}")))?
            .clone();
        let edit = PageEdit { title: Some(old.title), body: Some(old.body), base_version: None, message: format!("Restored version {version}") };
        self.edit_page(id, edit, author, now)
    }

    pub fn set_page_labels(&mut self, id: Id, l: &[String]) -> Result<()> {
        let l = labels(l)?;
        let p = self.pages.get_mut(&id).ok_or(ModelError::NoSuchPage(id))?;
        p.labels = l;
        Ok(())
    }

    /// Moves a page (and its subtree) under `new_parent`, as its last child.
    pub fn move_page(&mut self, id: Id, new_parent: Id) -> Result<()> {
        self.move_page_to(id, Some(new_parent), Placement::Last)
    }

    /// Moves a page (and its subtree) to `placement`: last child of `parent`, or just before /
    /// after a sibling (whose parent becomes the page's parent; `parent`, if also given, must
    /// agree). Sibling positions are renumbered `0..n` in both the old and new parent. The
    /// parent may be in another space: the subtree moves there with its comments and history,
    /// unless one of its titles is already taken in that space.
    pub fn move_page_to(&mut self, id: Id, parent: Option<Id>, placement: Placement) -> Result<()> {
        let sibling = match placement {
            Placement::Before(s) | Placement::After(s) => Some(s),
            Placement::Last => None,
        };
        let new_parent = match (sibling, parent) {
            (Some(s), p) => {
                if s == id {
                    return Err(ModelError::Invalid("a page can't be placed next to itself".into()));
                }
                let sp = self.page(s)?.parent.ok_or_else(|| ModelError::Invalid("a space's home page has no siblings".into()))?;
                if p.is_some_and(|p| p != sp) {
                    return Err(ModelError::Invalid("the sibling page isn't a child of that parent".into()));
                }
                sp
            }
            (None, Some(p)) => p,
            (None, None) => return Err(ModelError::Invalid("say where to move the page: parent, before or after".into())),
        };
        let old_parent = self.page(id)?.parent;
        self.check_move(id, new_parent)?;
        let mut order: Vec<Id> = self.children(new_parent).iter().map(|p| p.id).filter(|c| *c != id).collect();
        let at = match placement {
            Placement::Last => order.len(),
            Placement::Before(s) => order.iter().position(|c| *c == s).unwrap_or(order.len()),
            Placement::After(s) => order.iter().position(|c| *c == s).map_or(order.len(), |i| i.saturating_add(1)),
        };
        order.insert(at.min(order.len()), id);
        if let Some(p) = self.pages.get_mut(&id) {
            p.parent = Some(new_parent);
        }
        // Across spaces the whole subtree goes along, with its comments and history.
        let target_space = self.page(new_parent)?.space.clone();
        if self.page(id)?.space != target_space {
            for m in self.subtree(id) {
                if let Some(p) = self.pages.get_mut(&m) {
                    p.space = target_space.clone();
                }
            }
        }
        self.renumber(&order);
        if let Some(op) = old_parent
            && op != new_parent
        {
            let rest: Vec<Id> = self.children(op).iter().map(|p| p.id).collect();
            self.renumber(&rest);
        }
        Ok(())
    }

    fn renumber(&mut self, order: &[Id]) {
        for (i, c) in order.iter().enumerate() {
            if let Some(p) = self.pages.get_mut(c) {
                p.position = u32::try_from(i).unwrap_or(u32::MAX);
            }
        }
    }

    /// Copies a page (and, with `children`, its whole subtree) under `parent` — or, when that
    /// is `None`, beside the original (its homepage-less root copies go under the space home).
    /// `space` picks another space's home as the parent. Copied titles that clash get
    /// " (copy)", " (copy 2)"…; a given `new_title` must be free. Copies start at version 1
    /// with no comments. Returns the new root's id.
    pub fn copy_page(&mut self, req: CopyPage, author: &str, now: Timestamp) -> Result<Id> {
        let author = self.user(author)?.id.clone();
        let src = self.page(req.id)?;
        let parent = match (req.parent, req.space.as_deref()) {
            (Some(p), Some(s)) => {
                let key = self.space(s)?.key.clone();
                if self.page(p)?.space != key {
                    return Err(ModelError::Invalid("the parent page is in another space".into()));
                }
                p
            }
            (Some(p), None) => p,
            (None, Some(s)) => self.space(s)?.homepage,
            (None, None) => match src.parent {
                Some(p) => p,
                None => self.space(src.space.as_str())?.homepage,
            },
        };
        let target_space = self.page(parent)?.space.clone();
        let ids: Vec<Id> = if req.children { self.subtree(req.id) } else { vec![req.id] };
        if ids.len() > MAX_COPY {
            return Err(ModelError::Invalid(format!("a copy takes at most {MAX_COPY} pages")));
        }
        let height = if req.children { self.height(req.id) } else { 0 };
        if self.depth(parent).saturating_add(height) >= MAX_PAGE_DEPTH {
            return Err(ModelError::Invalid(format!("pages can nest at most {MAX_PAGE_DEPTH} levels deep")));
        }
        let root_title = match req.title.as_deref() {
            Some(t) => {
                let t = title(t, "the page title")?;
                self.check_unique_title(&target_space, &t, None)?;
                Some(t)
            }
            None => None,
        };
        // Snapshot the sources first, so copying into the page's own subtree terminates.
        let sources: Vec<Page> = ids.iter().filter_map(|i| self.pages.get(i).cloned()).collect();
        let mut map: std::collections::BTreeMap<Id, Id> = std::collections::BTreeMap::new();
        let mut root = None;
        for s in &sources {
            let (new_parent, position) = if s.id == req.id {
                (parent, u32::try_from(self.children(parent).len()).unwrap_or(u32::MAX))
            } else {
                (s.parent.and_then(|p| map.get(&p).copied()).unwrap_or(parent), s.position)
            };
            let t = match (&root_title, s.id == req.id) {
                (Some(t), true) => t.clone(),
                _ => self.free_title(&target_space, &s.title)?,
            };
            let id = self.ids.next_id();
            let mut page = new_page(id, target_space.clone(), Some(new_parent), position, t, s.body.clone(), author.clone(), now);
            page.labels = s.labels.clone();
            self.pages.insert(id, page);
            map.insert(s.id, id);
            root.get_or_insert(id);
        }
        root.ok_or(ModelError::NoSuchPage(req.id))
    }

    /// `base` if no page in `space` has it, else `base (copy)`, `base (copy 2)`…
    fn free_title(&self, space: &ProjectKey, base: &str) -> Result<String> {
        let taken = |t: &str| self.pages.values().any(|p| &p.space == space && p.title.eq_ignore_ascii_case(t));
        if !taken(base) {
            return Ok(base.to_string());
        }
        for n in 1..=10_000u32 {
            let suffix = if n == 1 { " (copy)".to_string() } else { format!(" (copy {n})") };
            let keep = MAX_TITLE.saturating_sub(suffix.chars().count());
            let t = format!("{}{suffix}", base.chars().take(keep).collect::<String>().trim_end());
            if !taken(&t) {
                return Ok(t);
            }
        }
        Err(ModelError::Invalid(format!("too many copies of '{}'", base.chars().take(40).collect::<String>())))
    }

    /// A version of a page: the current one or one from its history.
    pub fn page_version(&self, id: Id, version: u32) -> Result<(String, Doc)> {
        let p = self.page(id)?;
        if p.version == version {
            return Ok((p.title.clone(), p.body.clone()));
        }
        p.history
            .iter()
            .find(|v| v.version == version)
            .map(|v| (v.title.clone(), v.body.clone()))
            .ok_or_else(|| ModelError::Invalid(format!("page {id} has no version {version}")))
    }

    fn check_move(&self, id: Id, new_parent: Id) -> Result<()> {
        let page = self.page(id)?;
        if page.parent.is_none() {
            return Err(ModelError::Invalid("a space's home page can't be moved".into()));
        }
        let target = self.page(new_parent)?;
        if new_parent == id || self.ancestors(new_parent).contains(&id) {
            return Err(ModelError::Invalid("a page can't move under itself".into()));
        }
        if self.depth(new_parent).saturating_add(self.height(id)) >= MAX_PAGE_DEPTH {
            return Err(ModelError::Invalid(format!("pages can nest at most {MAX_PAGE_DEPTH} levels deep")));
        }
        if target.space != page.space {
            self.check_move_titles(id, &target.space)?;
        }
        Ok(())
    }

    /// A subtree moving to `space` must not bring a title that a page there already has
    /// (titles are unique per space): the error names the first clash.
    fn check_move_titles(&self, id: Id, space: &ProjectKey) -> Result<()> {
        let moving = self.subtree(id);
        for m in &moving {
            let Some(mp) = self.pages.get(m) else { continue };
            if let Some(clash) = self.pages.values().find(|p| &p.space == space && !moving.contains(&p.id) && p.title.eq_ignore_ascii_case(&mp.title))
            {
                return Err(ModelError::Duplicate(format!(
                    "a page titled '{}' in space {space} (page {}); rename '{}' before moving it there",
                    clash.title, clash.id, mp.title
                )));
            }
        }
        Ok(())
    }

    /// Deletes a page and its subtree. The space's home page can't be deleted.
    pub fn delete_page(&mut self, id: Id) -> Result<Vec<Id>> {
        if self.page(id)?.parent.is_none() {
            return Err(ModelError::Invalid("a space's home page can't be deleted".into()));
        }
        let gone = self.subtree(id);
        for g in &gone {
            self.pages.remove(g);
        }
        Ok(gone)
    }

    /// Children of a page, in order.
    pub fn children(&self, id: Id) -> Vec<&Page> {
        let mut c: Vec<&Page> = self.pages.values().filter(|p| p.parent == Some(id)).collect();
        c.sort_by_key(|p| (p.position, p.id));
        c
    }

    /// Ancestors, nearest first. Bounded, so a corrupt cycle in a loaded file can't hang.
    pub fn ancestors(&self, id: Id) -> Vec<Id> {
        let mut out = Vec::new();
        let mut cur = self.pages.get(&id).and_then(|p| p.parent);
        while let Some(c) = cur {
            if out.len() > MAX_PAGE_DEPTH * 4 || out.contains(&c) {
                break;
            }
            out.push(c);
            cur = self.pages.get(&c).and_then(|p| p.parent);
        }
        out
    }

    fn depth(&self, id: Id) -> usize {
        self.ancestors(id).len()
    }

    fn height(&self, id: Id) -> usize {
        let sub = self.subtree(id);
        sub.iter().map(|s| self.depth(*s)).max().unwrap_or(0).saturating_sub(self.depth(id))
    }

    /// The page and all its descendants (breadth first).
    pub fn subtree(&self, id: Id) -> Vec<Id> {
        let mut out = vec![id];
        let mut i = 0;
        while let Some(&cur) = out.get(i) {
            for c in self.children(cur) {
                if !out.contains(&c.id) {
                    out.push(c.id);
                }
            }
            i += 1;
        }
        out
    }

    fn check_unique_title(&self, space: &ProjectKey, t: &str, except: Option<Id>) -> Result<()> {
        // Titles are unique within a space, as in Confluence.
        if self.pages.values().any(|p| &p.space == space && Some(p.id) != except && p.title.eq_ignore_ascii_case(t)) {
            return Err(ModelError::Duplicate(format!("a page titled '{t}' in space {space}")));
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn new_page(id: Id, space: ProjectKey, parent: Option<Id>, position: u32, title: String, body: Doc, author: String, now: Timestamp) -> Page {
    Page {
        id,
        space,
        parent,
        position,
        title,
        body,
        labels: vec![],
        version: 1,
        author,
        created: now,
        updated: now,
        message: String::new(),
        history: vec![],
        comments: vec![],
    }
}
