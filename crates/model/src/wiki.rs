//! The wiki: spaces and their page trees, with full version history.

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, ProjectKey, Timestamp};
use trackcraft_markup::Doc;

use crate::{MAX_PAGE_DEPTH, ModelError, Result, Workspace, labels, title};

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

    /// Moves a page (and its subtree) under `new_parent` in the same space.
    pub fn move_page(&mut self, id: Id, new_parent: Id) -> Result<()> {
        let page = self.page(id)?;
        if page.parent.is_none() {
            return Err(ModelError::Invalid("a space's home page can't be moved".into()));
        }
        let target = self.page(new_parent)?;
        if target.space != page.space {
            return Err(ModelError::Invalid("pages move between spaces with page.copy (not yet available)".into()));
        }
        if new_parent == id || self.ancestors(new_parent).contains(&id) {
            return Err(ModelError::Invalid("a page can't move under itself".into()));
        }
        if self.depth(new_parent).saturating_add(self.height(id)) >= MAX_PAGE_DEPTH {
            return Err(ModelError::Invalid(format!("pages can nest at most {MAX_PAGE_DEPTH} levels deep")));
        }
        let position = u32::try_from(self.children(new_parent).len()).unwrap_or(u32::MAX);
        let p = self.pages.get_mut(&id).ok_or(ModelError::NoSuchPage(id))?;
        p.parent = Some(new_parent);
        p.position = position;
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
    }
}
