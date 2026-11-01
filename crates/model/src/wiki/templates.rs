//! Page templates: a few built-in ones (our own text, written for TrackCraft) and templates
//! people save for the whole workspace or for one space.
//!
//! A template has a suggested page title and a Markdown body. `{date}` in either is replaced by
//! the creation date (`YYYY-MM-DD`) when a page is made from it.

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, ProjectKey, Timestamp};
use trackcraft_markup::{Block, Doc, Inline};

use crate::{MAX_TITLE, ModelError, Result, Workspace, title};

/// Most user templates in one workspace.
pub const MAX_TEMPLATES: usize = 500;
/// Longest template body, in bytes of Markdown.
pub const MAX_TEMPLATE_BYTES: usize = 256 * 1024;
/// Longest template description, in characters.
pub const MAX_TEMPLATE_DESCRIPTION: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub id: Id,
    /// `None` = available in every space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<ProjectKey>,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Suggested page title (may be empty).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub body: Doc,
    pub author: String,
    pub created: Timestamp,
}

/// A template that ships with TrackCraft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinTemplate {
    /// Stable id (`meeting-notes`).
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

/// The built-in templates. All text here is original to TrackCraft.
pub const BUILTIN_TEMPLATES: &[BuiltinTemplate] = &[
    BuiltinTemplate {
        key: "meeting-notes",
        name: "Meeting notes",
        description: "Who came, what was talked through, and who is doing what next.",
        title: "Meeting {date}",
        body: "## Why we met\n\nOne or two sentences on the purpose of this meeting.\n\n## Who was there\n\n- \n\n## What we talked about\n\n1. First topic — the key points raised\n2. Second topic\n\n## What we agreed\n\n- \n\n## Follow-ups\n\n- [ ] Owner — task — due date",
    },
    BuiltinTemplate {
        key: "decision-record",
        name: "Decision record",
        description: "Capture one decision: the situation, the options weighed and the choice made.",
        title: "Decision: ",
        body: "**Status:** proposed · **Decided on:** {date} · **Decided by:** \n\n## Situation\n\nWhat forces this decision now? Include the constraints that matter.\n\n## Options weighed\n\n1. **Option one** — what it costs, what it gives us\n2. **Option two** — what it costs, what it gives us\n\n## Our choice\n\nThe option picked and the main reason for it.\n\n## What follows\n\nWhat becomes easier, what becomes harder, and what we will revisit later.",
    },
    BuiltinTemplate {
        key: "how-to",
        name: "How-to guide",
        description: "Step-by-step instructions for one task, with what you need before starting.",
        title: "How to ",
        body: "A short line on what this guide helps you get done.\n\n## Before you start\n\n- Access or tools you need\n\n## Steps\n\n1. Do the first thing.\n2. Do the next thing.\n3. Check that it worked.\n\n## If something goes wrong\n\nCommon problems and how to get past them.\n\n## See also\n\n- ",
    },
    BuiltinTemplate {
        key: "retrospective",
        name: "Retrospective",
        description: "Look back on a stretch of work: what helped, what got in the way, what to try.",
        title: "Retrospective {date}",
        body: "**Period covered:** \n\n**Participants:** \n\n## What helped us\n\n- \n\n## What slowed us down\n\n- \n\n## What we want to try next\n\n- [ ] Experiment — owner\n\n## Mood check\n\nA sentence or two on how the team feels about the period.",
    },
    BuiltinTemplate {
        key: "project-plan",
        name: "Project plan",
        description: "Set out goals, scope, milestones and risks for a piece of work.",
        title: "Project plan: ",
        body: "**Owner:** · **Target date:** · **Last reviewed:** {date}\n\n## Goal\n\nThe outcome we want, stated so we can tell when we've reached it.\n\n## In scope / out of scope\n\n- In: \n- Out: \n\n## Milestones\n\n1. Milestone — target date\n2. Milestone — target date\n\n## Risks and open questions\n\n- \n\n## Related issues\n\n- ",
    },
];

/// A template found by `page.create`'s `template` param.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTemplate {
    pub title: String,
    pub body: Doc,
}

/// Replaces `{date}` with the day of `now`.
pub fn expand_placeholders(s: &str, now: Timestamp) -> String {
    s.replace("{date}", &now.format("%Y-%m-%d").to_string())
}

/// Expands placeholders in every text run of a document, keeping its structure (and any nodes
/// Markdown can't express) intact.
pub fn expand_doc(doc: &Doc, now: Timestamp) -> Doc {
    let mut d = doc.clone();
    d.blocks.iter_mut().for_each(|b| expand_block(b, now, 0));
    d
}

fn expand_block(b: &mut Block, now: Timestamp, depth: usize) {
    if depth > trackcraft_markup::MAX_DEPTH {
        return;
    }
    match b {
        Block::Paragraph(i) | Block::Heading { content: i, .. } => {
            for inl in i.iter_mut() {
                if let Inline::Text { text, .. } = inl {
                    *text = expand_placeholders(text, now);
                }
            }
        }
        Block::BulletList(items) | Block::OrderedList(items) => {
            items.iter_mut().flatten().for_each(|b| expand_block(b, now, depth.saturating_add(1)));
        }
        Block::Quote(bs) => bs.iter_mut().for_each(|b| expand_block(b, now, depth.saturating_add(1))),
        Block::CodeBlock { .. } | Block::Rule | Block::Unknown(_) => {}
    }
}

pub fn builtin_template(key_or_name: &str) -> Option<&'static BuiltinTemplate> {
    let k = key_or_name.trim();
    BUILTIN_TEMPLATES.iter().find(|t| t.key.eq_ignore_ascii_case(k) || t.name.eq_ignore_ascii_case(k))
}

/// What `template.create` saves.
#[derive(Debug, Clone, Default)]
pub struct NewTemplate {
    pub space: Option<String>,
    pub name: String,
    pub description: String,
    pub title: String,
    pub body: Doc,
}

impl Workspace {
    pub fn create_template(&mut self, new: NewTemplate, author: &str, now: Timestamp) -> Result<Id> {
        let author = self.user(author)?.id.clone();
        let space = match new.space.as_deref() {
            Some(s) => Some(self.space(s)?.key.clone()),
            None => None,
        };
        let name = title(&new.name, "the template name")?;
        if builtin_template(&name).is_some() {
            return Err(ModelError::Duplicate(format!("a built-in template named '{name}'")));
        }
        if self.templates.values().any(|t| t.space == space && t.name.eq_ignore_ascii_case(&name)) {
            return Err(ModelError::Duplicate(format!("a template named '{name}' here")));
        }
        if self.templates.len() >= MAX_TEMPLATES {
            return Err(ModelError::Invalid(format!("a workspace holds at most {MAX_TEMPLATES} templates")));
        }
        let t = new.title.trim();
        if t.chars().count() > MAX_TITLE || t.chars().any(char::is_control) {
            return Err(ModelError::Invalid(format!("the template's page title is one line of at most {MAX_TITLE} characters")));
        }
        if new.body.to_markdown().len() > MAX_TEMPLATE_BYTES {
            return Err(ModelError::Invalid(format!("a template body is at most {} KiB", MAX_TEMPLATE_BYTES / 1024)));
        }
        let description: String = new.description.trim().chars().take(MAX_TEMPLATE_DESCRIPTION).collect();
        let id = self.ids.next_id();
        self.templates.insert(id, Template { id, space, name, description, title: t.to_string(), body: new.body, author, created: now });
        Ok(id)
    }

    pub fn delete_template(&mut self, id: Id) -> Result<()> {
        self.templates.remove(&id).map(|_| ()).ok_or_else(|| ModelError::Invalid(format!("no template with id {id}")))
    }

    /// Finds a template for a new page in `space`: a user template id (number or numeric
    /// string) or a built-in template's id or name. Placeholders are expanded.
    pub fn resolve_template(&self, reference: &str, space: &str, now: Timestamp) -> Result<ResolvedTemplate> {
        let r = reference.trim();
        if let Ok(n) = r.parse::<u64>() {
            let t = self.templates.get(&Id(n)).ok_or_else(|| ModelError::Invalid(format!("no template with id {n}")))?;
            if let Some(ts) = &t.space
                && ts != &self.space(space)?.key
            {
                return Err(ModelError::Invalid(format!("template '{}' belongs to space {ts}", t.name)));
            }
            let body = expand_doc(&t.body, now);
            return Ok(ResolvedTemplate { title: expand_placeholders(&t.title, now), body });
        }
        let Some(b) = builtin_template(r) else {
            // A user template by name: the space's own first, then workspace-wide ones.
            let key = self.space(space)?.key.clone();
            let named = |s: Option<&ProjectKey>| self.templates.values().find(|t| t.space.as_ref() == s && t.name.eq_ignore_ascii_case(r));
            let t = named(Some(&key)).or_else(|| named(None)).ok_or_else(|| {
                let short: String = r.chars().take(80).collect();
                ModelError::Invalid(format!("no template '{short}'"))
            })?;
            let body = expand_doc(&t.body, now);
            return Ok(ResolvedTemplate { title: expand_placeholders(&t.title, now), body });
        };
        Ok(ResolvedTemplate { title: expand_placeholders(b.title, now), body: expand_doc(&Doc::from_markdown(b.body), now) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_well_formed() {
        for b in BUILTIN_TEMPLATES {
            assert!(builtin_template(b.key).is_some() && builtin_template(b.name).is_some());
            assert!(!Doc::from_markdown(b.body).is_empty(), "{}", b.key);
        }
        assert!(builtin_template("nope").is_none());
    }
}
