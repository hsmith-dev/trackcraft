//! Confluence space exports.
//!
//! **HTML export** (a directory or .zip): `index.html` (the space overview with its page tree)
//! and one `.html` file per page. Each page file carries its title (`#title-text`, "Space :
//! Title"), breadcrumbs (`#breadcrumbs`: the space, then each ancestor page), its body
//! (`#main-content`) and a metadata line ("Created by …, last modified by … on …").
//! The page tree in `index.html` gives order and hierarchy; breadcrumbs fill in pages the tree
//! doesn't list.
//!
//! **Storage format** (one page's XHTML body, `<ac:structured-macro>`…): one page, its macros
//! kept verbatim as unknown blocks (code macros become code blocks).

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::Path;

use trackcraft_core::{Id, ProjectKey};
use trackcraft_markup::Doc;
use trackcraft_model::{MAX_PAGE_DEPTH, NewPage, Timestamp, Workspace};

use crate::html::{Converter, Element, parse};
use crate::jira::parse_date;
use crate::people::{People, Person};
use crate::{Actor, ImportError, ImportReport, MAX_ENTRIES, MAX_PAGES, MAX_UNZIPPED_BYTES, Result, check_size, clip, text_of, transact};

/// Largest single file read from an export.
const MAX_ENTRY_BYTES: u64 = 16 << 20;

/// Where a Confluence export comes from.
#[derive(Debug, Clone)]
pub enum ConfluenceSource {
    /// The export's files: (path relative to the export, contents).
    Files(Vec<(String, Vec<u8>)>),
    /// The export as a .zip archive.
    Zip(Vec<u8>),
    /// One page in storage format.
    Storage { title: String, xhtml: String },
}

/// Imports a Confluence export. `space` picks the space key (otherwise it's read from the
/// export or derived from the space name).
pub fn confluence_html(ws: &mut Workspace, source: ConfluenceSource, space: Option<&str>, actor: Actor<'_>) -> Result<ImportReport> {
    let space = match space.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(ProjectKey::parse(&s.to_ascii_uppercase()).map_err(|e| ImportError::Malformed(format!("space key '{}': {e}", clip(s, 40))))?),
        None => None,
    };
    match source {
        ConfluenceSource::Storage { title, xhtml } => {
            check_size(xhtml.len(), "the page")?;
            let key = space.ok_or_else(|| ImportError::Malformed("a storage-format page needs a space key (--space KEY)".into()))?;
            transact(ws, actor, |w, report| {
                let (root, truncated) = parse(&xhtml);
                if truncated {
                    report.warn("the page has too many elements; the end was cut off");
                }
                let mut c = Converter::new(&xhtml);
                let body = c.doc(&root);
                note_dropped(&c, report);
                let home = ensure_space(w, &key, key.as_str(), actor, report)?;
                let page = Page { title: title.clone(), body, author: None, at: None, parent: None, order: 0 };
                let mut people = People::default();
                place(w, &key, home, &page, None, &mut people, actor, report);
                Ok(())
            })
        }
        ConfluenceSource::Zip(bytes) => {
            check_size(bytes.len(), "the .zip file")?;
            let files = unzip(&bytes)?;
            export(ws, files, space, actor)
        }
        ConfluenceSource::Files(files) => export(ws, files, space, actor),
    }
}

/// Imports a Confluence export from a path: a directory, a `.zip`, or a single `.html`/`.xml`
/// file (an HTML page, or storage-format XHTML).
pub fn confluence_path(ws: &mut Workspace, path: &Path, space: Option<&str>, actor: Actor<'_>) -> Result<ImportReport> {
    let meta = std::fs::metadata(path).map_err(|e| ImportError::Io(format!("{}: {e}", path.display())))?;
    if meta.is_dir() {
        return confluence_html(ws, ConfluenceSource::Files(read_dir(path)?), space, actor);
    }
    let bytes = crate::read_file(path)?;
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return confluence_html(ws, ConfluenceSource::Zip(bytes), space, actor);
    }
    let text = text_of(&bytes);
    let lower: String = text.chars().take(4096).collect::<String>().to_ascii_lowercase();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "page.html".into());
    if text.contains("<ac:") || text.contains("<ri:") || !(lower.contains("<html") || lower.contains("<body")) {
        let title = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported page".into());
        return confluence_html(ws, ConfluenceSource::Storage { title, xhtml: text }, space, actor);
    }
    confluence_html(ws, ConfluenceSource::Files(vec![(name, bytes)]), space, actor)
}

/// A zip entry name that stays inside the export: relative, no `..`, no drive letters.
pub(crate) fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\0')
        && !name.starts_with('/')
        && !name.starts_with('\\')
        && !name.contains(':')
        && !name.split(['/', '\\']).any(|c| c == "..")
}

fn is_html(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".html") || l.ends_with(".htm")
}

fn unzip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ImportError::Malformed(format!("not a readable .zip: {e}")))?;
    if z.len() > MAX_ENTRIES {
        return Err(ImportError::TooLarge(format!("the .zip has {} entries; the limit is {MAX_ENTRIES}", z.len())));
    }
    for name in z.file_names() {
        if !safe_name(name) {
            return Err(ImportError::Unsafe(format!("the .zip holds an unsafe path '{}'", clip(name, 80))));
        }
    }
    let mut out = Vec::new();
    let mut total: u64 = 0;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| ImportError::Malformed(format!("zip entry {i}: {e}")))?;
        let name = f.name().to_string();
        if f.is_dir() {
            continue;
        }
        if f.enclosed_name().is_none() || !safe_name(&name) {
            return Err(ImportError::Unsafe(format!("the .zip holds an unsafe path '{}'", clip(&name, 80))));
        }
        if !is_html(&name) {
            out.push((name, Vec::new()));
            continue;
        }
        let mut buf = Vec::new();
        (&mut f).take(MAX_ENTRY_BYTES + 1).read_to_end(&mut buf).map_err(|e| ImportError::Malformed(format!("{}: {e}", clip(&name, 80))))?;
        let len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if len > MAX_ENTRY_BYTES {
            return Err(ImportError::TooLarge(format!("{} unpacks to more than {MAX_ENTRY_BYTES} bytes", clip(&name, 80))));
        }
        total = total.saturating_add(len);
        if total > MAX_UNZIPPED_BYTES {
            return Err(ImportError::TooLarge(format!("the .zip unpacks to more than {MAX_UNZIPPED_BYTES} bytes")));
        }
        out.push((name, buf));
    }
    Ok(out)
}

fn read_dir(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize)];
    let mut seen = 0usize;
    let mut total = 0usize;
    while let Some((dir, prefix, depth)) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| ImportError::Io(format!("{}: {e}", dir.display())))?;
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return Err(ImportError::TooLarge(format!("the export has more than {MAX_ENTRIES} files")));
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if ft.is_dir() {
                if depth < 6 {
                    stack.push((entry.path(), format!("{name}/"), depth + 1));
                }
            } else if ft.is_file() {
                if !is_html(&name) {
                    out.push((name, Vec::new()));
                    continue;
                }
                let bytes = crate::read_file(&entry.path())?;
                total = total.saturating_add(bytes.len());
                if u64::try_from(total).unwrap_or(u64::MAX) > MAX_UNZIPPED_BYTES {
                    return Err(ImportError::TooLarge(format!("the export holds more than {MAX_UNZIPPED_BYTES} bytes of pages")));
                }
                out.push((name, bytes));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// One page read from the export.
#[derive(Debug, Clone)]
struct Page {
    title: String,
    body: Doc,
    author: Option<String>,
    at: Option<Timestamp>,
    /// The parent page's file name (`None` = top level).
    parent: Option<String>,
    order: usize,
}

/// `Page_123.html#x?y`, `./a%20b.html` → the file name it points at, relative to the root.
fn target(href: &str) -> Option<String> {
    let h = href.trim();
    if h.contains("://") || h.starts_with("mailto:") || h.starts_with('#') {
        return None;
    }
    let h = h.split(['#', '?']).next().unwrap_or_default().trim_start_matches("./");
    let mut out = Vec::new();
    let bytes = h.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%'
            && let Some(v) = h.get(i + 1..i + 3).and_then(|x| u8::from_str_radix(x, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b);
        i += 1;
    }
    let s = String::from_utf8_lossy(&out).into_owned();
    (!s.is_empty()).then_some(s)
}

/// "Created by Ann Lee, last modified by Bo Ng on Mar 21, 2024" → (last author, date).
fn metadata(text: &str) -> (Option<String>, Option<Timestamp>) {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let date = t.rsplit_once(" on ").and_then(|(_, d)| parse_date(d.trim_end_matches('.')));
    let who = |marker: &str| {
        t.find(marker).map(|p| {
            let rest = t.get(p + marker.len()..).unwrap_or_default();
            let end = [rest.find(','), rest.find(" on ")].into_iter().flatten().min().unwrap_or(rest.len());
            rest.get(..end).unwrap_or_default().trim().to_string()
        })
    };
    let author = who("last modified by ").or_else(|| who("Created by ")).filter(|a| !a.is_empty());
    (author, date)
}

/// Strips "Space name : " from a page title.
fn page_title(raw: &str, space_name: Option<&str>) -> String {
    let t = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(sp) = space_name
        && let Some(rest) = t.strip_prefix(&format!("{sp} : "))
    {
        return rest.to_string();
    }
    t
}

/// A space key from a space name: initials of a multi-word name, else its first letters.
fn derive_key(name: &str) -> Option<ProjectKey> {
    let words: Vec<&str> = name.split_whitespace().filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_alphabetic())).collect();
    let candidate: String = if words.len() > 1 {
        words.iter().filter_map(|w| w.chars().next()).map(|c| c.to_ascii_uppercase()).take(10).collect()
    } else {
        name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).take(10).collect()
    };
    ProjectKey::parse(&candidate).or_else(|_| ProjectKey::parse("IMPORT")).ok()
}

fn note_dropped(c: &Converter<'_>, report: &mut ImportReport) {
    if c.images > 0 {
        report.warn(format!("{} image(s) kept as '[image: name]' text (attachments aren't imported yet)", c.images));
    }
    if c.macros > 0 {
        report.warn(format!("{} macro(s) kept verbatim as unknown blocks", c.macros));
    }
}

fn export(ws: &mut Workspace, files: Vec<(String, Vec<u8>)>, space: Option<ProjectKey>, actor: Actor<'_>) -> Result<ImportReport> {
    // The export's root: the folder of the shallowest index.html (zips wrap it in a folder).
    let index_path = files
        .iter()
        .filter(|(n, _)| n.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case("index.html")))
        .min_by_key(|(n, _)| n.matches('/').count())
        .map(|(n, _)| n.clone());
    let root = index_path.as_deref().and_then(|p| p.rsplit_once('/')).map(|(d, _)| format!("{d}/")).unwrap_or_default();
    let mut index: Option<String> = None;
    let mut pages_src: BTreeMap<String, String> = BTreeMap::new();
    let mut attachments = 0usize;
    for (name, bytes) in &files {
        let Some(rel) = name.strip_prefix(&root) else { continue };
        if !is_html(rel) {
            attachments += 1;
            continue;
        }
        if rel.eq_ignore_ascii_case("index.html") {
            index = Some(text_of(bytes));
        } else if !rel.contains('/') {
            pages_src.insert(rel.to_string(), text_of(bytes));
        }
    }
    drop(files);
    if pages_src.is_empty() && index.is_none() {
        return Err(ImportError::Malformed("no index.html or page .html files found".into()));
    }
    for s in pages_src.values() {
        check_size(s.len(), "a page")?;
    }
    if pages_src.len() > MAX_PAGES {
        return Err(ImportError::TooLarge(format!("the export has {} pages; the limit is {MAX_PAGES}", pages_src.len())));
    }
    transact(ws, actor, |w, report| {
        if attachments > 0 {
            report.warn(format!("{attachments} non-page file(s) (attachments, images, styles) were not imported"));
        }
        // Space name and key, and the page tree from index.html.
        let mut space_name: Option<String> = None;
        let mut space_key = space;
        let mut tree: HashMap<String, (Option<String>, usize)> = HashMap::new();
        let mut index_body: Option<Doc> = None;
        if let Some(src) = &index {
            let (root_el, _) = parse(src);
            space_name = root_el
                .by_id("title-text")
                .map(Element::text)
                .or_else(|| root_el.find(&|e| e.name == "title").map(Element::text))
                .filter(|t| !t.is_empty());
            if space_key.is_none() {
                let mut rows = Vec::new();
                root_el.find_all(&|e| e.name == "tr", &mut rows);
                space_key = rows.iter().find_map(|r| {
                    let cells: Vec<&Element> = r.child_elements().filter(|c| c.name == "td" || c.name == "th").collect();
                    match cells.as_slice() {
                        [k, v, ..] if k.text().eq_ignore_ascii_case("key") => ProjectKey::parse(&v.text().to_ascii_uppercase()).ok(),
                        _ => None,
                    }
                });
            }
            let mut counter = 0usize;
            walk_tree(&root_el, None, &pages_src, &mut tree, &mut counter, 0);
            // A space description in the overview, if any.
            if let Some(main) = root_el.by_id("main-content") {
                let mut c = Converter::new(src);
                let d = c.doc(main);
                if !d.is_empty() && tree.is_empty() {
                    index_body = Some(d);
                }
            }
        }
        // Pages.
        let mut pages: BTreeMap<String, Page> = BTreeMap::new();
        let mut images = 0usize;
        let mut macros = 0usize;
        for (n, (file, src)) in pages_src.iter().enumerate() {
            let (el, truncated) = parse(src);
            if truncated {
                report.warn(format!("{file}: too many elements; the end of the page was cut off"));
            }
            let raw_title = el
                .by_id("title-text")
                .map(Element::text)
                .or_else(|| el.find(&|e| e.name == "title").map(Element::text))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| file.trim_end_matches(".html").replace(['_', '-'], " "));
            if space_name.is_none() {
                space_name = el.by_id("breadcrumbs").and_then(|b| b.find(&|e| e.name == "a")).map(Element::text).filter(|t| !t.is_empty());
            }
            let title = page_title(&raw_title, space_name.as_deref());
            let body_el = el
                .by_id("main-content")
                .or_else(|| el.find(&|e| e.has_class("wiki-content")))
                .or_else(|| el.find(&|e| e.name == "body"))
                .unwrap_or(&el);
            let mut c = Converter::new(src);
            let body = c.doc(body_el);
            images += c.images;
            macros += c.macros;
            let (author, at) = el.find(&|e| e.has_class("page-metadata")).map(|m| metadata(&m.text())).unwrap_or((None, None));
            let crumbs: Vec<String> = el
                .by_id("breadcrumbs")
                .map(|b| {
                    let mut links = Vec::new();
                    b.find_all(&|e| e.name == "a", &mut links);
                    links.iter().filter_map(|a| a.attr("href")).filter_map(target).collect()
                })
                .unwrap_or_default();
            let crumb_parent = crumbs.iter().rev().find(|c| pages_src.contains_key(*c) && *c != file).cloned();
            let (parent, order) = match tree.get(file) {
                Some((p, o)) => (p.clone(), *o),
                None => (crumb_parent, usize::MAX / 2 + n),
            };
            pages.insert(file.clone(), Page { title, body, author, at, parent, order });
        }
        if images > 0 {
            report.warn(format!("{images} image(s) kept as '[image: name]' text (attachments aren't imported yet)"));
        }
        if macros > 0 {
            report.warn(format!("{macros} macro(s) kept verbatim as unknown blocks"));
        }
        let name = space_name.clone().unwrap_or_else(|| "Imported space".into());
        let key = space_key.or_else(|| derive_key(&name)).ok_or_else(|| ImportError::Malformed("no usable space key: pass one".into()))?;
        let home = ensure_space(w, &key, &name, actor, report)?;
        if let Some(d) = index_body
            && w.page(home).is_ok_and(|p| p.version == 1 && p.body.is_empty())
        {
            let _ = w.set_imported_page(home, None, d, None, actor.now, actor.now);
        }

        // The one top-level page becomes the space's homepage (when that's still blank).
        let tops: Vec<&String> = pages.iter().filter(|(_, p)| p.parent.is_none()).map(|(f, _)| f).collect();
        let mut ids: HashMap<String, Id> = HashMap::new();
        let mut people = People::default();
        if let [only] = tops.as_slice()
            && w.page(home).is_ok_and(|p| p.version == 1 && p.body.is_empty())
            && let Some(p) = pages.get(*only)
        {
            let author = p.author.as_ref().and_then(|a| people.resolve(w, &Person::named(a), report));
            let at = p.at.unwrap_or(actor.now);
            let title = unique_title(w, &key, &p.title, Some(home));
            match w.set_imported_page(home, Some(&title), p.body.clone(), author.as_deref(), at, at) {
                Ok(()) => {
                    report.pages += 1;
                    ids.insert((*only).clone(), home);
                }
                Err(e) => report.warn(format!("{only}: {e}")),
            }
        }
        // Parents before children, siblings in tree order.
        let depth = |f: &String| {
            let mut d = 0usize;
            let mut at = pages.get(f).and_then(|p| p.parent.clone());
            while let Some(p) = at {
                d += 1;
                if d > MAX_PAGE_DEPTH + 1 {
                    break;
                }
                at = pages.get(&p).and_then(|x| x.parent.clone());
            }
            d
        };
        let mut order: Vec<(usize, usize, &String)> = pages.iter().map(|(f, p)| (depth(f), p.order, f)).collect();
        order.sort();
        for (_, _, file) in order {
            if ids.contains_key(file) {
                continue;
            }
            let Some(p) = pages.get(file) else { continue };
            let parent = match &p.parent {
                Some(pf) => match ids.get(pf) {
                    Some(id) => Some(*id),
                    None => {
                        report.warn(format!("{file}: its parent page wasn't imported; placed under the space's homepage"));
                        None
                    }
                },
                None => None,
            };
            if let Some(id) = place(w, &key, home, p, parent, &mut people, actor, report) {
                ids.insert(file.clone(), id);
            }
        }
        Ok(())
    })
}

/// Records index.html's page tree: each `li` whose link points at a page file is that page;
/// its parent is the nearest enclosing such `li`.
fn walk_tree(
    el: &Element,
    parent: Option<&String>,
    pages: &BTreeMap<String, String>,
    tree: &mut HashMap<String, (Option<String>, usize)>,
    counter: &mut usize,
    depth: usize,
) {
    if depth > crate::MAX_HTML_DEPTH {
        return;
    }
    for c in el.child_elements() {
        if c.name == "li" {
            let link = c
                .child_elements()
                .find_map(|x| if x.name == "a" { Some(x) } else { x.child_elements().find(|y| y.name == "a") })
                .and_then(|a| a.attr("href"))
                .and_then(target)
                .filter(|t| pages.contains_key(t));
            if let Some(file) = link {
                if !tree.contains_key(&file) && parent != Some(&file) {
                    *counter += 1;
                    tree.insert(file.clone(), (parent.cloned(), *counter));
                }
                walk_tree(c, Some(&file), pages, tree, counter, depth + 1);
                continue;
            }
        }
        walk_tree(c, parent, pages, tree, counter, depth + 1);
    }
}

/// The space (created if missing) and its homepage.
fn ensure_space(w: &mut Workspace, key: &ProjectKey, name: &str, actor: Actor<'_>, report: &mut ImportReport) -> Result<Id> {
    if !w.spaces.contains_key(key) {
        let name = crate::clip(&name.split_whitespace().collect::<Vec<_>>().join(" "), trackcraft_model::MAX_TITLE);
        let name = if name.is_empty() { key.to_string() } else { name };
        w.create_space(key.as_str(), &name, actor.user.trim(), actor.now).map_err(|e| ImportError::Model(format!("space {key}: {e}")))?;
        report.spaces.push(key.to_string());
    }
    w.space(key.as_str()).map(|s| s.homepage).map_err(|e| ImportError::Model(e.to_string()))
}

/// A title not yet used in the space: "Title", "Title (2)", "Title (3)"…
fn unique_title(w: &Workspace, key: &ProjectKey, title: &str, except: Option<Id>) -> String {
    let base = {
        let t: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        let t = if t.is_empty() { "Untitled".to_string() } else { t };
        crate::clip(&t, trackcraft_model::MAX_TITLE - 8)
    };
    let taken = |t: &str| w.pages.values().any(|p| &p.space == key && Some(p.id) != except && p.title.eq_ignore_ascii_case(t));
    if !taken(&base) {
        return base;
    }
    (2..10_000).map(|n| format!("{base} ({n})")).find(|t| !taken(t)).unwrap_or(base)
}

#[allow(clippy::too_many_arguments)]
fn place(
    w: &mut Workspace,
    key: &ProjectKey,
    home: Id,
    p: &Page,
    parent: Option<Id>,
    people: &mut People,
    actor: Actor<'_>,
    report: &mut ImportReport,
) -> Option<Id> {
    if report.pages >= MAX_PAGES {
        return None;
    }
    let title = unique_title(w, key, &p.title, None);
    if !title.eq_ignore_ascii_case(p.title.trim()) {
        report.warn(format!("page '{}' was renamed '{title}' (titles are unique in a space)", clip(&p.title, 60)));
    }
    let author = p.author.as_ref().and_then(|a| people.resolve(w, &Person::named(a), report)).unwrap_or_else(|| actor.user.trim().to_string());
    let at = p.at.unwrap_or(actor.now);
    let mut new = NewPage { space: key.to_string(), parent: Some(parent.unwrap_or(home)), title, body: p.body.clone() };
    let id = match w.create_page(new.clone(), &author, at) {
        Ok(id) => id,
        Err(e) => {
            // Too deep (or another space's parent): put it under the homepage instead.
            report.warn(format!("page '{}': {e}; placed under the space's homepage", clip(&p.title, 60)));
            new.parent = Some(home);
            match w.create_page(new, &author, at) {
                Ok(id) => id,
                Err(e) => {
                    report.warn(format!("page '{}' not imported: {e}", clip(&p.title, 60)));
                    return None;
                }
            }
        }
    };
    report.pages += 1;
    Some(id)
}
