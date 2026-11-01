//! People in an export (account ids, display names, emails) → workspace users.
//!
//! A person matches an existing user by email, then display name (both case-insensitive), then
//! by id = account id. Otherwise a user is created with a stable id: the email's local part,
//! else the display name lowercased with spaces as dots, else `acct-<account id>`; a clash with
//! a different existing user adds `-2`, `-3`….

use std::collections::HashMap;

use trackcraft_model::Workspace;

use crate::{ImportReport, clip};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Person {
    pub account: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
}

impl Person {
    pub fn named(name: &str) -> Person {
        Person { name: clean(name), ..Person::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.account.is_none() && self.name.is_none() && self.email.is_none()
    }
}

/// Trims, drops empty values and the placeholders exports use for "nobody".
pub fn clean(s: &str) -> Option<String> {
    let s = clip(s, 255).replace(|c: char| c.is_control(), " ");
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("unassigned") || s.eq_ignore_ascii_case("null") {
        return None;
    }
    Some(s.to_string())
}

#[derive(Default)]
pub struct People {
    /// `a:`/`e:`/`n:` keys → user id.
    known: HashMap<String, String>,
    /// What the export taught us about each account id.
    names: HashMap<String, (Option<String>, Option<String>)>,
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if (c.is_whitespace() || c == '.') && !out.ends_with('.') && !out.is_empty() {
            out.push('.');
        }
        if out.chars().count() >= 64 {
            break;
        }
    }
    out.trim_matches('.').to_string()
}

impl People {
    /// Records an account id's name and email, so a later mention by id alone finds them.
    pub fn learn(&mut self, p: &Person) {
        if let Some(a) = &p.account {
            let e = self.names.entry(a.clone()).or_default();
            if e.0.is_none() {
                e.0.clone_from(&p.name);
            }
            if e.1.is_none() {
                e.1.clone_from(&p.email);
            }
        }
    }

    fn remember(&mut self, p: &Person, id: &str) {
        if let Some(a) = &p.account {
            self.known.insert(format!("a:{a}"), id.to_string());
        }
        if let Some(e) = &p.email {
            self.known.insert(format!("e:{}", e.to_lowercase()), id.to_string());
        }
        if let Some(n) = &p.name {
            self.known.insert(format!("n:{}", n.to_lowercase()), id.to_string());
        }
    }

    /// The user id for a person, creating the user when needed (`None` for nobody).
    pub fn resolve(&mut self, ws: &mut Workspace, p: &Person, report: &mut ImportReport) -> Option<String> {
        let mut p = p.clone();
        if let Some((n, e)) = p.account.as_ref().and_then(|a| self.names.get(a)) {
            if p.name.is_none() {
                p.name.clone_from(n);
            }
            if p.email.is_none() {
                p.email.clone_from(e);
            }
        }
        if p.is_empty() {
            return None;
        }
        let keys = [
            p.account.as_ref().map(|a| format!("a:{a}")),
            p.email.as_ref().map(|e| format!("e:{}", e.to_lowercase())),
            p.name.as_ref().map(|n| format!("n:{}", n.to_lowercase())),
        ];
        if let Some(id) = keys.iter().flatten().find_map(|k| self.known.get(k)).cloned() {
            self.remember(&p, &id);
            return Some(id);
        }
        let existing = ws
            .users
            .values()
            .find(|u| p.email.as_ref().is_some_and(|e| u.email.as_ref().is_some_and(|ue| ue.eq_ignore_ascii_case(e))))
            .or_else(|| ws.users.values().find(|u| p.name.as_ref().is_some_and(|n| u.display_name.eq_ignore_ascii_case(n))))
            .or_else(|| p.account.as_ref().and_then(|a| ws.users.get(a)))
            .map(|u| u.id.clone());
        if let Some(id) = existing {
            self.remember(&p, &id);
            return Some(id);
        }
        let base = p
            .email
            .as_ref()
            .and_then(|e| e.split('@').next())
            .map(slug)
            .filter(|s| !s.is_empty())
            .or_else(|| p.name.as_deref().map(slug).filter(|s| !s.is_empty()))
            .or_else(|| p.account.as_deref().map(|a| format!("acct-{}", slug(a))).filter(|s| s.len() > 5))
            .unwrap_or_else(|| "user".to_string());
        let mut id = base.clone();
        let mut n = 1u32;
        while ws.users.contains_key(&id) {
            n = n.saturating_add(1);
            id = format!("{base}-{n}");
            if n > 10_000 {
                report.warn(format!("couldn't find a free user id for {base}"));
                return None;
            }
        }
        let display = p.name.clone().or_else(|| p.email.clone()).or_else(|| p.account.clone()).unwrap_or_else(|| id.clone());
        if let Err(e) = ws.add_user(&id, &display) {
            report.warn(format!("user {}: {e}", clip(&display, 60)));
            return None;
        }
        if let Some(e) = &p.email
            && ws.edit_user(&id, None, Some(Some(e))).is_err()
        {
            report.warn(format!("user {id}: '{}' is not an email address; left out", clip(e, 60)));
        }
        report.users.push(id.clone());
        self.remember(&p, &id);
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_and_matching() {
        let mut ws = Workspace::new("t");
        ws.add_user("ann", "Ann Lee").unwrap();
        let mut people = People::default();
        let mut r = ImportReport::default();
        assert_eq!(people.resolve(&mut ws, &Person::named("ann lee"), &mut r).as_deref(), Some("ann"));
        let bo = Person { account: Some("5b10:ac".into()), name: Some("Bo Ng".into()), email: None };
        people.learn(&bo);
        let by_id = Person { account: Some("5b10:ac".into()), ..Person::default() };
        assert_eq!(people.resolve(&mut ws, &by_id, &mut r).as_deref(), Some("bo.ng"));
        assert_eq!(people.resolve(&mut ws, &bo, &mut r).as_deref(), Some("bo.ng"));
        let cy = Person { email: Some("Cy@Example.com".into()), name: Some("Cy".into()), account: None };
        assert_eq!(people.resolve(&mut ws, &cy, &mut r).as_deref(), Some("cy"));
        assert_eq!(ws.user("cy").unwrap().email.as_deref(), Some("Cy@Example.com"));
        // A known display name matches.
        let other = Person { name: Some("Cy".into()), email: Some("cy@elsewhere.org".into()), account: None };
        assert_eq!(people.resolve(&mut ws, &other, &mut r).as_deref(), Some("cy"));
        let mut fresh = People::default();
        let other2 = Person { name: Some("Ann  Lee!".into()), email: Some("ann@other.org".into()), account: None };
        assert_eq!(fresh.resolve(&mut ws, &other2, &mut r).as_deref(), Some("ann-2"));
        assert_eq!(people.resolve(&mut ws, &Person::named("  "), &mut r), None);
        assert_eq!(people.resolve(&mut ws, &Person::named("Unassigned"), &mut r), None);
        let weird = Person::named("\u{0}\u{0}");
        assert_eq!(people.resolve(&mut ws, &weird, &mut r), None);
        assert_eq!(r.users, vec!["bo.ng", "cy", "ann-2"]);
    }
}
