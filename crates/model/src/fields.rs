//! M2.4: per-project custom fields, components and versions (releases), and their values on
//! issues.
//!
//! The concepts follow Atlassian's public documentation of custom fields, project components
//! and versions; the kinds, limits and rules are TrackCraft's own. Everything a project defines
//! lives in one [`ProjectFields`] (the workspace's `fields` map, keyed by project); issues hold
//! values by id (`Issue::custom`, `components`, `fix_versions`, `affects_versions`), so renaming a
//! definition never touches issues. Ids come from the workspace counter (replays stay
//! deterministic), so `cf[ID]` and `customfield_ID` are unique across the workspace.
//!
//! History: a value change records the field's name (`Customer`), and components and versions
//! record `Component`, `Fix Version` and `Version` (the names Jira's changelog uses), with names
//! as display values. Deleting a definition, or an option of a select field, removes it from
//! every issue and records that on each.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, IssueKey, ProjectKey, Timestamp};

use crate::{Issue, ModelError, Result, StatusCategory, Workspace, clip, title};

/// Most custom fields one project defines.
pub const MAX_FIELDS: usize = 200;
/// Most options of one select or multi-select field.
pub const MAX_OPTIONS: usize = 500;
/// Most components one project defines.
pub const MAX_COMPONENTS: usize = 1000;
/// Most versions one project defines.
pub const MAX_VERSIONS: usize = 2000;
/// Most components, fix versions, affects versions or multi-select values on one issue.
pub const MAX_PER_ISSUE: usize = 100;
/// Longest text value (characters; Jira documents 32,767 for paragraph fields).
pub const MAX_TEXT_VALUE: usize = 32_767;
/// Longest URL value.
pub const MAX_URL: usize = 2048;
/// Longest description of a field, component or version.
pub const MAX_DEF_DESCRIPTION: usize = 1000;
/// Largest number a number field holds (either sign).
pub const MAX_NUMBER: f64 = 1e15;

/// History field names for components and versions (Jira's changelog uses these).
pub const HISTORY_COMPONENT: &str = "Component";
pub const HISTORY_FIX_VERSION: &str = "Fix Version";
pub const HISTORY_AFFECTS_VERSION: &str = "Version";

/// JQL names of system fields: a custom field can't take one (its JQL name would be ambiguous).
pub const RESERVED_FIELD_NAMES: &[&str] = &[
    "project",
    "key",
    "issuekey",
    "id",
    "summary",
    "description",
    "comment",
    "status",
    "statuscategory",
    "type",
    "issuetype",
    "priority",
    "assignee",
    "reporter",
    "labels",
    "parent",
    "story points",
    "storypoints",
    "created",
    "createddate",
    "updated",
    "updateddate",
    "resolved",
    "resolutiondate",
    "rank",
    "sprint",
    "issuelinktype",
    "component",
    "components",
    "fixversion",
    "fixversions",
    "affectedversion",
    "affectsversion",
    "affectsversions",
    "versions",
    "text",
];

/// What a custom field holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Number,
    /// A calendar date (`YYYY-MM-DD`).
    Date,
    Select {
        options: Vec<String>,
    },
    MultiSelect {
        options: Vec<String>,
    },
    /// A workspace user (by id).
    User,
    Url,
}

/// The kind names commands take (`"select"`, `"multiSelect"`…).
pub const FIELD_KINDS: [&str; 7] = ["text", "number", "date", "select", "multiSelect", "user", "url"];

impl FieldKind {
    pub fn name(&self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Number => "number",
            FieldKind::Date => "date",
            FieldKind::Select { .. } => "select",
            FieldKind::MultiSelect { .. } => "multiSelect",
            FieldKind::User => "user",
            FieldKind::Url => "url",
        }
    }

    /// The options of a select or multi-select field (empty for other kinds).
    pub fn options(&self) -> &[String] {
        match self {
            FieldKind::Select { options } | FieldKind::MultiSelect { options } => options,
            _ => &[],
        }
    }

    /// A kind by name (case-insensitive; `multiselect`, `multi-select` too), with its options.
    pub fn parse(name: &str, options: &[String]) -> Result<FieldKind> {
        let n = name.trim().to_ascii_lowercase().replace(['-', '_', ' '], "");
        let kind = match n.as_str() {
            "text" | "string" => FieldKind::Text,
            "number" | "float" => FieldKind::Number,
            "date" => FieldKind::Date,
            "select" | "option" => FieldKind::Select { options: check_options(options)? },
            "multiselect" | "multioption" => FieldKind::MultiSelect { options: check_options(options)? },
            "user" => FieldKind::User,
            "url" => FieldKind::Url,
            _ => return Err(ModelError::Invalid(format!("'{}' is not a field type (use one of {})", clip(name, 40), FIELD_KINDS.join(", ")))),
        };
        if kind.options().is_empty() && matches!(kind, FieldKind::Select { .. } | FieldKind::MultiSelect { .. }) {
            return Err(ModelError::Invalid("a select field needs at least one option".into()));
        }
        Ok(kind)
    }
}

/// Options: titles, unique (case-insensitive), at most [`MAX_OPTIONS`].
fn check_options(options: &[String]) -> Result<Vec<String>> {
    if options.len() > MAX_OPTIONS {
        return Err(ModelError::Invalid(format!("a field has at most {MAX_OPTIONS} options")));
    }
    let mut out: Vec<String> = Vec::with_capacity(options.len());
    for o in options {
        if o.trim().is_empty() {
            continue;
        }
        let o = title(o, "an option")?;
        if out.iter().any(|x| x.eq_ignore_ascii_case(&o)) {
            return Err(ModelError::Duplicate(format!("option '{}'", clip(&o, 40))));
        }
        out.push(o);
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDef {
    pub id: Id,
    pub name: String,
    pub kind: FieldKind,
    /// New issues must set it, and it can't be cleared.
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A custom field's value on an issue (the variant matches the field's kind).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum FieldValue {
    Text(String),
    Number(f64),
    Date(NaiveDate),
    Select(String),
    MultiSelect(Vec<String>),
    User(String),
    Url(String),
}

impl FieldValue {
    /// The value as one line of text (history entries, lists): numbers without a needless
    /// `.0`, dates as `YYYY-MM-DD`, multi-select values comma-separated.
    pub fn display(&self) -> String {
        match self {
            FieldValue::Text(s) | FieldValue::Select(s) | FieldValue::User(s) | FieldValue::Url(s) => s.clone(),
            FieldValue::Number(n) => number_text(*n),
            FieldValue::Date(d) => d.format("%Y-%m-%d").to_string(),
            FieldValue::MultiSelect(v) => v.join(", "),
        }
    }

    /// Parses typed text for a field of `kind` (what editors, the CLI and imports have).
    /// Empty text is `None` (clear the value). Multi-select values are comma-separated. The
    /// result still goes through validation when set.
    pub fn from_text(kind: &FieldKind, raw: &str) -> Result<Option<FieldValue>> {
        let s = raw.trim();
        if s.is_empty() {
            return Ok(None);
        }
        Ok(Some(match kind {
            FieldKind::Text => FieldValue::Text(raw.to_string()),
            FieldKind::Number => {
                FieldValue::Number(s.replace(',', "").parse::<f64>().map_err(|_| ModelError::Invalid(format!("'{}' is not a number", clip(s, 40))))?)
            }
            FieldKind::Date => FieldValue::Date(parse_day(s)?),
            FieldKind::Select { .. } => FieldValue::Select(s.to_string()),
            FieldKind::MultiSelect { .. } => {
                FieldValue::MultiSelect(s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect())
            }
            FieldKind::User => FieldValue::User(s.to_string()),
            FieldKind::Url => FieldValue::Url(s.to_string()),
        }))
    }
}

/// `YYYY-MM-DD`, years 1000–9999.
pub fn parse_day(s: &str) -> Result<NaiveDate> {
    let s = s.trim();
    let bad = || ModelError::Invalid(format!("'{}' is not a date (use YYYY-MM-DD)", clip(s, 40)));
    if s.len() != 10 {
        return Err(bad());
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| bad())?;
    use chrono::Datelike;
    if !(1000..=9999).contains(&d.year()) {
        return Err(bad());
    }
    Ok(d)
}

fn number_text(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{n:.0}") } else { n.to_string() }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    pub id: Id,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub id: Id,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<NaiveDate>,
    #[serde(default)]
    pub released: bool,
    #[serde(default)]
    pub archived: bool,
}

/// Everything one project defines.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFields {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<Version>,
}

impl ProjectFields {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.components.is_empty() && self.versions.is_empty()
    }
    pub fn field(&self, id: Id) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.id == id)
    }
    pub fn component(&self, id: Id) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }
    pub fn version(&self, id: Id) -> Option<&Version> {
        self.versions.iter().find(|v| v.id == id)
    }
    /// A field by `customfield_N`, `cf[N]`, `N` or name (case-insensitive).
    pub fn find_field(&self, r: &str) -> Option<&FieldDef> {
        let r = r.trim();
        if let Some(id) = field_id_of(r) {
            return self.field(id);
        }
        self.fields.iter().find(|f| f.name.eq_ignore_ascii_case(r))
    }
    /// A component by name (case-insensitive), else by id.
    pub fn find_component(&self, r: &str) -> Option<&Component> {
        let r = r.trim();
        self.components.iter().find(|c| c.name.eq_ignore_ascii_case(r)).or_else(|| r.parse().ok().and_then(|n| self.component(Id(n))))
    }
    /// A version by name (case-insensitive), else by id.
    pub fn find_version(&self, r: &str) -> Option<&Version> {
        let r = r.trim();
        self.versions.iter().find(|v| v.name.eq_ignore_ascii_case(r)).or_else(|| r.parse().ok().and_then(|n| self.version(Id(n))))
    }
}

/// The id in `customfield_N`, `cf[N]` or `N`.
pub fn field_id_of(r: &str) -> Option<Id> {
    let r = r.trim();
    let lower = r.to_ascii_lowercase();
    let digits = lower.strip_prefix("customfield_").or_else(|| lower.strip_prefix("cf[").and_then(|x| x.strip_suffix(']'))).unwrap_or(&lower);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().map(Id)
}

/// A field definition change (`None` = leave alone).
#[derive(Debug, Clone, Default)]
pub struct FieldEdit {
    pub name: Option<String>,
    /// New option list (select kinds only). Values on issues that lose their option are cleared.
    pub options: Option<Vec<String>>,
    pub required: Option<bool>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ComponentEdit {
    pub name: Option<String>,
    /// `Some(None)` clears the lead.
    pub lead: Option<Option<String>>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewVersion {
    pub name: String,
    pub description: String,
    pub start: Option<NaiveDate>,
    pub release_date: Option<NaiveDate>,
    pub released: bool,
}

#[derive(Debug, Clone, Default)]
pub struct VersionEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    pub start: Option<Option<NaiveDate>>,
    pub release_date: Option<Option<NaiveDate>>,
    pub released: Option<bool>,
    pub archived: Option<bool>,
}

/// Values set when an issue is created or imported (no history: they are its first values).
#[derive(Debug, Clone, Default)]
pub struct IssueFields {
    pub custom: Vec<(Id, FieldValue)>,
    pub components: Vec<Id>,
    pub fix_versions: Vec<Id>,
    pub affects_versions: Vec<Id>,
}

impl IssueFields {
    pub fn is_empty(&self) -> bool {
        self.custom.is_empty() && self.components.is_empty() && self.fix_versions.is_empty() && self.affects_versions.is_empty()
    }
}

/// Which version list of an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionList {
    Fix,
    Affects,
}

impl VersionList {
    fn history(self) -> &'static str {
        match self {
            VersionList::Fix => HISTORY_FIX_VERSION,
            VersionList::Affects => HISTORY_AFFECTS_VERSION,
        }
    }
    fn of(self, i: &Issue) -> &Vec<Id> {
        match self {
            VersionList::Fix => &i.fix_versions,
            VersionList::Affects => &i.affects_versions,
        }
    }
    fn of_mut(self, i: &mut Issue) -> &mut Vec<Id> {
        match self {
            VersionList::Fix => &mut i.fix_versions,
            VersionList::Affects => &mut i.affects_versions,
        }
    }
}

fn description(s: &str, what: &str) -> Result<String> {
    let s = s.trim();
    if s.chars().count() > MAX_DEF_DESCRIPTION {
        return Err(ModelError::Invalid(format!("{what} is longer than {MAX_DEF_DESCRIPTION} characters")));
    }
    if s.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ModelError::Invalid(format!("{what} can't contain control characters")));
    }
    Ok(s.to_string())
}

fn names(ids: &[Id], name: impl Fn(Id) -> Option<String>) -> Option<String> {
    let v: Vec<String> = ids.iter().filter_map(|id| name(*id)).collect();
    if v.is_empty() { None } else { Some(v.join(", ")) }
}

type Items = Vec<(String, Option<String>, Option<String>)>;

impl Workspace {
    /// What `project` defines (`None` when nothing yet).
    pub fn project_fields(&self, project: &ProjectKey) -> Option<&ProjectFields> {
        self.fields.get(project)
    }

    fn defs_mut(&mut self, project: &str) -> Result<(ProjectKey, &mut ProjectFields)> {
        let key = self.project(project)?.key.clone();
        Ok((key.clone(), self.fields.entry(key).or_default()))
    }

    /// The project defining field `id`, and the field.
    pub fn find_field(&self, id: Id) -> Option<(&ProjectKey, &FieldDef)> {
        self.fields.iter().find_map(|(k, d)| d.field(id).map(|f| (k, f)))
    }
    pub fn find_component(&self, id: Id) -> Option<(&ProjectKey, &Component)> {
        self.fields.iter().find_map(|(k, d)| d.component(id).map(|c| (k, c)))
    }
    pub fn find_version(&self, id: Id) -> Option<(&ProjectKey, &Version)> {
        self.fields.iter().find_map(|(k, d)| d.version(id).map(|v| (k, v)))
    }

    fn no_field(id: Id) -> ModelError {
        ModelError::Invalid(format!("no field with id {id}"))
    }
    fn no_component(id: Id) -> ModelError {
        ModelError::Invalid(format!("no component with id {id}"))
    }
    fn no_version(id: Id) -> ModelError {
        ModelError::Invalid(format!("no version with id {id}"))
    }

    /// Records `items` on every issue of `project` that `change` touched (it returns the
    /// history items for one issue, empty when untouched).
    fn cascade(&mut self, project: &ProjectKey, author: &str, now: Timestamp, mut change: impl FnMut(&mut Issue) -> Items) -> usize {
        let mut touched: Vec<(IssueKey, Items)> = Vec::new();
        for i in self.issues.values_mut().filter(|i| &i.key.project == project) {
            let items = change(i);
            if !items.is_empty() {
                i.updated = now;
                touched.push((i.key.clone(), items));
            }
        }
        let n = touched.len();
        for (k, items) in touched {
            let items = items.iter().map(|(f, a, b)| (f.as_str(), a.clone(), b.clone())).collect();
            self.record(&k, author, now, items);
        }
        n
    }

    // ---- custom field definitions ----

    fn check_field_name(&self, project: &ProjectKey, name: &str, except: Option<Id>) -> Result<String> {
        let name = title(name, "the field name")?;
        let lower = name.to_ascii_lowercase();
        if RESERVED_FIELD_NAMES.contains(&lower.as_str()) || field_id_of(&name).is_some() || lower.starts_with("cf[") {
            return Err(ModelError::Invalid(format!("'{}' is a system field's name: choose another", clip(&name, 40))));
        }
        if self.fields.get(project).is_some_and(|d| d.fields.iter().any(|f| Some(f.id) != except && f.name.eq_ignore_ascii_case(&name))) {
            return Err(ModelError::Duplicate(format!("field '{}'", clip(&name, 40))));
        }
        Ok(name)
    }

    /// Adds a custom field to a project.
    pub fn create_field(&mut self, project: &str, name: &str, kind: FieldKind, required: bool, desc: &str) -> Result<Id> {
        let key = self.project(project)?.key.clone();
        let name = self.check_field_name(&key, name, None)?;
        let desc = description(desc, "the description")?;
        if self.fields.get(&key).is_some_and(|d| d.fields.len() >= MAX_FIELDS) {
            return Err(ModelError::Invalid(format!("project {key} has the most custom fields allowed ({MAX_FIELDS})")));
        }
        let id = self.ids.next_id();
        let (_, d) = self.defs_mut(key.as_str())?;
        d.fields.push(FieldDef { id, name, kind, required, description: desc });
        Ok(id)
    }

    /// Changes a field's name, options, required flag or description. Issues whose value used an
    /// option that is gone lose it (recorded in their history).
    pub fn edit_field(&mut self, id: Id, edit: FieldEdit, author: &str, now: Timestamp) -> Result<()> {
        let (key, def) = self.find_field(id).map(|(k, f)| (k.clone(), f.clone())).ok_or_else(|| Self::no_field(id))?;
        let name = match &edit.name {
            Some(n) => self.check_field_name(&key, n, Some(id))?,
            None => def.name.clone(),
        };
        let desc = edit.description.as_deref().map(|d| description(d, "the description")).transpose()?;
        let kind = match (&edit.options, &def.kind) {
            (None, k) => k.clone(),
            (Some(o), k @ (FieldKind::Select { .. } | FieldKind::MultiSelect { .. })) => FieldKind::parse(k.name(), o)?,
            (Some(_), k) => return Err(ModelError::Invalid(format!("a {} field has no options", k.name()))),
        };
        let required = edit.required.unwrap_or(def.required);
        let options = kind.options().to_vec();
        let has = |o: &str| options.iter().find(|x| x.eq_ignore_ascii_case(o)).cloned();
        let field_name = name.clone();
        self.cascade(&key, author, now, |i| {
            let Some(v) = i.custom.get(&id).cloned() else { return vec![] };
            let next = match &v {
                FieldValue::Select(s) => has(s).map(FieldValue::Select),
                FieldValue::MultiSelect(list) => {
                    let kept: Vec<String> = list.iter().filter_map(|s| has(s)).collect();
                    if kept.is_empty() { None } else { Some(FieldValue::MultiSelect(kept)) }
                }
                _ => Some(v.clone()),
            };
            if next.as_ref() == Some(&v) {
                return vec![];
            }
            match &next {
                Some(n) => i.custom.insert(id, n.clone()),
                None => i.custom.remove(&id),
            };
            vec![(field_name.clone(), Some(v.display()), next.map(|n| n.display()))]
        });
        let d = self.fields.get_mut(&key).and_then(|d| d.fields.iter_mut().find(|f| f.id == id)).ok_or_else(|| Self::no_field(id))?;
        d.name = name;
        d.kind = kind;
        d.required = required;
        if let Some(desc) = desc {
            d.description = desc;
        }
        Ok(())
    }

    /// Deletes a field and its values on every issue (recorded). Returns how many issues lost one.
    pub fn delete_field(&mut self, id: Id, author: &str, now: Timestamp) -> Result<usize> {
        let (key, name) = self.find_field(id).map(|(k, f)| (k.clone(), f.name.clone())).ok_or_else(|| Self::no_field(id))?;
        let n = self.cascade(&key, author, now, |i| match i.custom.remove(&id) {
            Some(v) => vec![(name.clone(), Some(v.display()), None)],
            None => vec![],
        });
        if let Some(d) = self.fields.get_mut(&key) {
            d.fields.retain(|f| f.id != id);
        }
        self.fields.retain(|_, d| !d.is_empty());
        Ok(n)
    }

    // ---- components ----

    fn check_named<'a>(existing: impl Iterator<Item = (Id, &'a str)>, name: &str, what: &str, except: Option<Id>) -> Result<String> {
        let name = title(name, &format!("the {what} name"))?;
        for (id, n) in existing {
            if Some(id) != except && n.eq_ignore_ascii_case(&name) {
                return Err(ModelError::Duplicate(format!("{what} '{}'", clip(&name, 40))));
            }
        }
        Ok(name)
    }

    pub fn create_component(&mut self, project: &str, name: &str, lead: Option<&str>, desc: &str) -> Result<Id> {
        let key = self.project(project)?.key.clone();
        let existing = self.fields.get(&key).map(|d| d.components.clone()).unwrap_or_default();
        if existing.len() >= MAX_COMPONENTS {
            return Err(ModelError::Invalid(format!("project {key} has the most components allowed ({MAX_COMPONENTS})")));
        }
        let name = Self::check_named(existing.iter().map(|c| (c.id, c.name.as_str())), name, "component", None)?;
        let lead = self.check_user(lead)?;
        let desc = description(desc, "the description")?;
        let id = self.ids.next_id();
        let (_, d) = self.defs_mut(key.as_str())?;
        d.components.push(Component { id, name, lead, description: desc });
        Ok(id)
    }

    pub fn edit_component(&mut self, id: Id, edit: ComponentEdit) -> Result<()> {
        let key = self.find_component(id).map(|(k, _)| k.clone()).ok_or_else(|| Self::no_component(id))?;
        let existing = self.fields.get(&key).map(|d| d.components.clone()).unwrap_or_default();
        let name = edit
            .name
            .as_deref()
            .map(|n| Self::check_named(existing.iter().map(|c| (c.id, c.name.as_str())), n, "component", Some(id)))
            .transpose()?;
        let lead = match edit.lead {
            None => None,
            Some(l) => Some(self.check_user(l.as_deref())?),
        };
        let desc = edit.description.as_deref().map(|d| description(d, "the description")).transpose()?;
        let c = self.fields.get_mut(&key).and_then(|d| d.components.iter_mut().find(|c| c.id == id)).ok_or_else(|| Self::no_component(id))?;
        if let Some(n) = name {
            c.name = n;
        }
        if let Some(l) = lead {
            c.lead = l;
        }
        if let Some(d) = desc {
            c.description = d;
        }
        Ok(())
    }

    /// Deletes a component and removes it from every issue (recorded).
    pub fn delete_component(&mut self, id: Id, author: &str, now: Timestamp) -> Result<usize> {
        let key = self.find_component(id).map(|(k, _)| k.clone()).ok_or_else(|| Self::no_component(id))?;
        let lookup = self.fields.get(&key).cloned().unwrap_or_default();
        let name = |x: Id| lookup.component(x).map(|c| c.name.clone());
        let n = self.cascade(&key, author, now, |i| {
            if !i.components.contains(&id) {
                return vec![];
            }
            let from = names(&i.components, name);
            i.components.retain(|c| *c != id);
            vec![(HISTORY_COMPONENT.to_string(), from, names(&i.components, name))]
        });
        if let Some(d) = self.fields.get_mut(&key) {
            d.components.retain(|c| c.id != id);
        }
        self.fields.retain(|_, d| !d.is_empty());
        Ok(n)
    }

    // ---- versions ----

    pub fn create_version(&mut self, project: &str, new: NewVersion) -> Result<Id> {
        let key = self.project(project)?.key.clone();
        let existing = self.fields.get(&key).map(|d| d.versions.clone()).unwrap_or_default();
        if existing.len() >= MAX_VERSIONS {
            return Err(ModelError::Invalid(format!("project {key} has the most versions allowed ({MAX_VERSIONS})")));
        }
        let name = Self::check_named(existing.iter().map(|v| (v.id, v.name.as_str())), &new.name, "version", None)?;
        let desc = description(&new.description, "the description")?;
        check_span(new.start, new.release_date)?;
        let id = self.ids.next_id();
        let (_, d) = self.defs_mut(key.as_str())?;
        d.versions.push(Version {
            id,
            name,
            description: desc,
            start: new.start,
            release_date: new.release_date,
            released: new.released,
            archived: false,
        });
        Ok(id)
    }

    pub fn edit_version(&mut self, id: Id, edit: VersionEdit) -> Result<()> {
        let (key, v) = self.find_version(id).map(|(k, v)| (k.clone(), v.clone())).ok_or_else(|| Self::no_version(id))?;
        let existing = self.fields.get(&key).map(|d| d.versions.clone()).unwrap_or_default();
        let name =
            edit.name.as_deref().map(|n| Self::check_named(existing.iter().map(|v| (v.id, v.name.as_str())), n, "version", Some(id))).transpose()?;
        let desc = edit.description.as_deref().map(|d| description(d, "the description")).transpose()?;
        let start = edit.start.unwrap_or(v.start);
        let release_date = edit.release_date.unwrap_or(v.release_date);
        check_span(start, release_date)?;
        let x = self.fields.get_mut(&key).and_then(|d| d.versions.iter_mut().find(|v| v.id == id)).ok_or_else(|| Self::no_version(id))?;
        if let Some(n) = name {
            x.name = n;
        }
        if let Some(d) = desc {
            x.description = d;
        }
        x.start = start;
        x.release_date = release_date;
        if let Some(r) = edit.released {
            x.released = r;
        }
        if let Some(a) = edit.archived {
            x.archived = a;
        }
        Ok(())
    }

    /// Marks a version released on `date` (default: today). Unresolved issues fixed in it move
    /// to `move_unresolved_to` (another unreleased version of the project) when given.
    /// Returns how many issues moved.
    pub fn release_version(
        &mut self,
        id: Id,
        date: Option<NaiveDate>,
        move_unresolved_to: Option<Id>,
        author: &str,
        now: Timestamp,
    ) -> Result<usize> {
        let (key, v) = self.find_version(id).map(|(k, v)| (k.clone(), v.clone())).ok_or_else(|| Self::no_version(id))?;
        if v.released {
            return Err(ModelError::Invalid(format!("version {} is already released", clip(&v.name, 40))));
        }
        let date = date.unwrap_or_else(|| now.date_naive());
        check_span(v.start, Some(date))?;
        let mut moved = 0;
        if let Some(to) = move_unresolved_to {
            let target = self.fields.get(&key).and_then(|d| d.version(to)).ok_or_else(|| Self::no_version(to))?;
            if to == id || target.released {
                return Err(ModelError::Invalid("unresolved issues move to another unreleased version of the project".into()));
            }
            let unresolved: Vec<IssueKey> = self
                .issues
                .values()
                .filter(|i| i.key.project == key && i.fix_versions.contains(&id) && self.status_category(i) != StatusCategory::Done)
                .map(|i| i.key.clone())
                .collect();
            for k in &unresolved {
                let list: Vec<Id> = self.issues.get(k).map(|i| i.fix_versions.clone()).unwrap_or_default();
                let mut next: Vec<Id> = list.into_iter().filter(|x| *x != id).collect();
                if !next.contains(&to) {
                    next.push(to);
                }
                self.set_versions(&k.to_string(), VersionList::Fix, &next, author, now)?;
            }
            moved = unresolved.len();
        }
        let x = self.fields.get_mut(&key).and_then(|d| d.versions.iter_mut().find(|v| v.id == id)).ok_or_else(|| Self::no_version(id))?;
        x.released = true;
        x.release_date = Some(date);
        Ok(moved)
    }

    /// Deletes a version and removes it from every issue's fix and affects versions (recorded).
    pub fn delete_version(&mut self, id: Id, author: &str, now: Timestamp) -> Result<usize> {
        let key = self.find_version(id).map(|(k, _)| k.clone()).ok_or_else(|| Self::no_version(id))?;
        let lookup = self.fields.get(&key).cloned().unwrap_or_default();
        let name = |x: Id| lookup.version(x).map(|v| v.name.clone());
        let n = self.cascade(&key, author, now, |i| {
            let mut items = Vec::new();
            for list in [VersionList::Fix, VersionList::Affects] {
                if list.of(i).contains(&id) {
                    let from = names(list.of(i), name);
                    list.of_mut(i).retain(|v| *v != id);
                    items.push((list.history().to_string(), from, names(list.of(i), name)));
                }
            }
            items
        });
        if let Some(d) = self.fields.get_mut(&key) {
            d.versions.retain(|v| v.id != id);
        }
        self.fields.retain(|_, d| !d.is_empty());
        Ok(n)
    }

    /// `(done, total)`: issues with `id` as a fix version, and how many are in a Done status.
    pub fn version_progress(&self, id: Id) -> (usize, usize) {
        let mut done = 0;
        let mut total = 0;
        for i in self.issues.values().filter(|i| i.fix_versions.contains(&id)) {
            total += 1;
            if self.status_category(i) == StatusCategory::Done {
                done += 1;
            }
        }
        (done, total)
    }

    // ---- values on issues ----

    /// Validates (and canonicalises) `v` for field `def`: the variant matches the kind, select
    /// values are options (their stored spelling), numbers are finite and in range, users
    /// exist, URLs are http(s), text is capped.
    pub fn check_value(&self, def: &FieldDef, v: FieldValue) -> Result<FieldValue> {
        let bad = |why: String| ModelError::Invalid(format!("{}: {why}", clip(&def.name, 60)));
        let option = |o: &str| def.kind.options().iter().find(|x| x.eq_ignore_ascii_case(o.trim())).cloned();
        Ok(match (&def.kind, v) {
            (FieldKind::Text, FieldValue::Text(s)) => {
                if s.chars().count() > MAX_TEXT_VALUE {
                    return Err(bad(format!("text is longer than {MAX_TEXT_VALUE} characters")));
                }
                if s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                    return Err(bad("text can't contain control characters".into()));
                }
                FieldValue::Text(s)
            }
            (FieldKind::Number, FieldValue::Number(n)) => {
                if !n.is_finite() || n.abs() > MAX_NUMBER {
                    return Err(bad("a number must be finite and at most 10^15".into()));
                }
                FieldValue::Number(n)
            }
            (FieldKind::Date, FieldValue::Date(d)) => FieldValue::Date(parse_day(&d.format("%Y-%m-%d").to_string())?),
            (FieldKind::Select { .. }, FieldValue::Select(s)) => {
                FieldValue::Select(option(&s).ok_or_else(|| bad(format!("'{}' is not an option", clip(&s, 40))))?)
            }
            (FieldKind::MultiSelect { .. }, FieldValue::MultiSelect(list)) => {
                if list.len() > MAX_PER_ISSUE {
                    return Err(bad(format!("at most {MAX_PER_ISSUE} values")));
                }
                let mut out: Vec<String> = Vec::new();
                for s in &list {
                    let o = option(s).ok_or_else(|| bad(format!("'{}' is not an option", clip(s, 40))))?;
                    if !out.contains(&o) {
                        out.push(o);
                    }
                }
                FieldValue::MultiSelect(out)
            }
            (FieldKind::User, FieldValue::User(u)) => {
                FieldValue::User(self.user(u.trim()).map_err(|_| bad(format!("no user {}", clip(&u, 40))))?.id.clone())
            }
            (FieldKind::Url, FieldValue::Url(u)) => {
                let u = u.trim().to_string();
                let lower = u.to_ascii_lowercase();
                if u.chars().count() > MAX_URL
                    || !(lower.starts_with("http://") || lower.starts_with("https://"))
                    || u.len() <= "https://".len()
                    || u.chars().any(|c| c.is_whitespace() || c.is_control())
                {
                    return Err(bad("a URL starts with http:// or https://, has no spaces and is at most 2048 characters".into()));
                }
                FieldValue::Url(u)
            }
            (k, v) => return Err(bad(format!("a {} field can't hold a {} value", k.name(), value_kind(&v)))),
        })
    }

    fn issue_project_fields(&self, key: &str) -> Result<(IssueKey, ProjectFields)> {
        let i = self.issue(key)?;
        Ok((i.key.clone(), self.fields.get(&i.key.project).cloned().unwrap_or_default()))
    }

    /// Sets (`Some`) or clears (`None`) custom field `field` on an issue. A required field
    /// can't be cleared. Records the field's name in the history.
    pub fn set_custom(&mut self, key: &str, field: Id, value: Option<FieldValue>, author: &str, now: Timestamp) -> Result<()> {
        let (k, defs) = self.issue_project_fields(key)?;
        let def = defs.field(field).ok_or_else(|| ModelError::Invalid(format!("project {} has no field with id {field}", k.project)))?;
        let value = value.map(|v| self.check_value(def, v)).transpose()?;
        if value.is_none() && def.required {
            return Err(ModelError::Invalid(format!("{} is required", clip(&def.name, 60))));
        }
        let i = self.issues.get_mut(&k).ok_or_else(|| ModelError::NoSuchIssue(k.to_string()))?;
        let from = match &value {
            Some(v) => i.custom.insert(field, v.clone()),
            None => i.custom.remove(&field),
        };
        if from != value {
            i.updated = now;
        }
        self.record(&k, author, now, vec![(def.name.as_str(), from.map(|v| v.display()), value.map(|v| v.display()))]);
        Ok(())
    }

    fn check_ids(defs: &ProjectFields, project: &ProjectKey, ids: &[Id], what: &str, known: impl Fn(&ProjectFields, Id) -> bool) -> Result<Vec<Id>> {
        if ids.len() > MAX_PER_ISSUE {
            return Err(ModelError::Invalid(format!("an issue has at most {MAX_PER_ISSUE} {what}s")));
        }
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if !known(defs, *id) {
                return Err(ModelError::Invalid(format!("project {project} has no {what} with id {id}")));
            }
            if !out.contains(id) {
                out.push(*id);
            }
        }
        Ok(out)
    }

    /// Replaces an issue's components (ids of its project's components).
    pub fn set_components(&mut self, key: &str, ids: &[Id], author: &str, now: Timestamp) -> Result<()> {
        let (k, defs) = self.issue_project_fields(key)?;
        let ids = Self::check_ids(&defs, &k.project, ids, "component", |d, id| d.component(id).is_some())?;
        let name = |x: Id| defs.component(x).map(|c| c.name.clone());
        let i = self.issues.get_mut(&k).ok_or_else(|| ModelError::NoSuchIssue(k.to_string()))?;
        let from = std::mem::replace(&mut i.components, ids.clone());
        if from != ids {
            i.updated = now;
        }
        self.record(&k, author, now, vec![(HISTORY_COMPONENT, names(&from, name), names(&ids, name))]);
        Ok(())
    }

    /// Replaces an issue's fix or affects versions (ids of its project's versions).
    pub fn set_versions(&mut self, key: &str, list: VersionList, ids: &[Id], author: &str, now: Timestamp) -> Result<()> {
        let (k, defs) = self.issue_project_fields(key)?;
        let ids = Self::check_ids(&defs, &k.project, ids, "version", |d, id| d.version(id).is_some())?;
        let name = |x: Id| defs.version(x).map(|v| v.name.clone());
        let i = self.issues.get_mut(&k).ok_or_else(|| ModelError::NoSuchIssue(k.to_string()))?;
        let from = std::mem::replace(list.of_mut(i), ids.clone());
        if from != ids {
            i.updated = now;
        }
        self.record(&k, author, now, vec![(list.history(), names(&from, name), names(&ids, name))]);
        Ok(())
    }

    /// Sets the first values of a new (or imported) issue, without history entries. Validates
    /// like the setters.
    pub fn init_issue_fields(&mut self, key: &str, values: IssueFields) -> Result<()> {
        let (k, defs) = self.issue_project_fields(key)?;
        let components = Self::check_ids(&defs, &k.project, &values.components, "component", |d, id| d.component(id).is_some())?;
        let fix = Self::check_ids(&defs, &k.project, &values.fix_versions, "version", |d, id| d.version(id).is_some())?;
        let affects = Self::check_ids(&defs, &k.project, &values.affects_versions, "version", |d, id| d.version(id).is_some())?;
        let mut custom = BTreeMap::new();
        for (id, v) in values.custom {
            let def = defs.field(id).ok_or_else(|| ModelError::Invalid(format!("project {} has no field with id {id}", k.project)))?;
            custom.insert(id, self.check_value(def, v)?);
        }
        let i = self.issues.get_mut(&k).ok_or_else(|| ModelError::NoSuchIssue(k.to_string()))?;
        i.components = components;
        i.fix_versions = fix;
        i.affects_versions = affects;
        i.custom.extend(custom);
        Ok(())
    }

    /// Names of the issue's required fields that have no value.
    pub fn missing_required(&self, key: &str) -> Result<Vec<String>> {
        let i = self.issue(key)?;
        Ok(self
            .fields
            .get(&i.key.project)
            .map(|d| d.fields.iter().filter(|f| f.required && !i.custom.contains_key(&f.id)).map(|f| f.name.clone()).collect())
            .unwrap_or_default())
    }
}

fn value_kind(v: &FieldValue) -> &'static str {
    match v {
        FieldValue::Text(_) => "text",
        FieldValue::Number(_) => "number",
        FieldValue::Date(_) => "date",
        FieldValue::Select(_) => "select",
        FieldValue::MultiSelect(_) => "multiSelect",
        FieldValue::User(_) => "user",
        FieldValue::Url(_) => "url",
    }
}

fn check_span(start: Option<NaiveDate>, end: Option<NaiveDate>) -> Result<()> {
    use chrono::Datelike;
    for d in [start, end].into_iter().flatten() {
        if !(1000..=9999).contains(&d.year()) {
            return Err(ModelError::Invalid("dates are between the years 1000 and 9999".into()));
        }
    }
    match (start, end) {
        (Some(s), Some(e)) if e < s => Err(ModelError::Invalid("the release date is before the start date".into())),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "fields_tests.rs"]
mod tests;
