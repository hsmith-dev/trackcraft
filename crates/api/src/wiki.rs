//! Confluence Cloud REST v2 subset (`/wiki/api/v2/…`): spaces, pages, versions, children and
//! footer comments, with cursor pagination (`cursor`, `limit`, `_links.next` and a `Link` header).
//!
//! **Bodies.** `body-format=atlas_doc_format` returns ADF (a JSON string in `value`, as the public
//! API does). TrackCraft does not store Confluence storage XHTML: `body-format=storage` returns the
//! page as **Markdown** text in `value`. On input, a `storage` value that contains markup is
//! converted to plain paragraphs (tags dropped, entities decoded — lossy); a `storage` value
//! without tags is read as Markdown, so a GET-then-PUT round trip keeps formatting.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get, routing::post};
use serde_json::{Value, json};
use trackcraft_core::Id;
use trackcraft_engine::model::{Doc, Page, PageComment, Space, Timestamp, Workspace};

use crate::ApiState;
use crate::ctx::{Body, Ctx, P, Q, pct};
use crate::error::{ApiError, Flavor};
use crate::ids;

/// Default and largest `limit` (the public API's are 25 and 250).
pub const DEFAULT_LIMIT: usize = 25;
pub const MAX_LIMIT: usize = 250;
const C: Flavor = Flavor::Confluence;

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/wiki/api/v2/spaces", get(spaces))
        .route("/wiki/api/v2/spaces/{id}", get(space))
        .route("/wiki/api/v2/spaces/{id}/pages", get(space_pages))
        .route("/wiki/api/v2/pages", get(pages).post(page_create))
        .route("/wiki/api/v2/pages/{id}", get(page_get).put(page_update).delete(page_delete))
        .route("/wiki/api/v2/pages/{id}/versions", get(versions))
        .route("/wiki/api/v2/pages/{id}/children", get(children))
        .route("/wiki/api/v2/pages/{id}/footer-comments", get(footer_comments))
        .route("/wiki/api/v2/footer-comments", post(footer_comment_create))
}

fn cdate(t: &Timestamp) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Storage,
    Adf,
}

fn format(q: &Q) -> Result<Option<Format>, ApiError> {
    match q.get("body-format").map(str::to_ascii_lowercase).as_deref() {
        None | Some("") => Ok(None),
        Some("storage") => Ok(Some(Format::Storage)),
        Some("atlas_doc_format") => Ok(Some(Format::Adf)),
        Some(other) => Err(ApiError::bad(
            C,
            format!("body-format '{}' is not supported: use storage or atlas_doc_format", other.chars().take(40).collect::<String>()),
        )),
    }
}

fn body_out(doc: &Doc, f: Format) -> Value {
    match f {
        Format::Adf => json!({"atlas_doc_format": {"representation": "atlas_doc_format", "value": doc.to_adf().to_string()}}),
        Format::Storage => json!({"storage": {"representation": "storage", "value": doc.to_markdown()}}),
    }
}

/// A request `body` (`{"representation", "value"}`, or nested under `storage`/`atlas_doc_format`)
/// → the engine's `body` param (ADF object or Markdown string), and the format it came in.
fn body_in(b: Option<&Value>) -> Result<Option<(Value, Format)>, ApiError> {
    let Some(b) = b.filter(|b| !b.is_null()) else { return Ok(None) };
    let (repr, value) = match (b.get("representation").and_then(Value::as_str), b.get("value")) {
        (Some(r), Some(v)) => (r.to_string(), v.clone()),
        _ => match (b.get("storage"), b.get("atlas_doc_format")) {
            (Some(s), _) => ("storage".to_string(), s.get("value").cloned().unwrap_or(Value::Null)),
            (None, Some(a)) => ("atlas_doc_format".to_string(), a.get("value").cloned().unwrap_or(Value::Null)),
            _ => return Err(ApiError::bad(C, "body needs \"representation\" and \"value\"")),
        },
    };
    match repr.as_str() {
        "atlas_doc_format" => {
            let adf = match value {
                Value::String(s) => serde_json::from_str::<Value>(&s).map_err(|e| ApiError::bad(C, format!("body value is not ADF JSON: {e}")))?,
                v @ Value::Object(_) => v,
                _ => return Err(ApiError::bad(C, "body value must be an ADF document")),
            };
            Ok(Some((adf, Format::Adf)))
        }
        "storage" => {
            let s = value.as_str().ok_or_else(|| ApiError::bad(C, "body value must be a string"))?;
            if s.contains('<') { Ok(Some((Doc::plain(&storage_text(s)).to_adf(), Format::Storage))) } else { Ok(Some((json!(s), Format::Storage))) }
        }
        other => Err(ApiError::bad(
            C,
            format!("body representation '{}' is not supported: use storage or atlas_doc_format", other.chars().take(40).collect::<String>()),
        )),
    }
}

/// Storage XHTML → text: block tags become paragraph breaks, `<br>` a line break, other tags are
/// dropped and the common entities decoded.
pub fn storage_text(s: &str) -> String {
    const BLOCKS: &[&str] = &["p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr", "blockquote", "pre", "table", "ul", "ol"];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        out.push_str(&entities(rest.get(..lt).unwrap_or_default()));
        let after = rest.get(lt..).unwrap_or_default();
        let Some(gt) = after.find('>') else {
            rest = "";
            break;
        };
        let tag = after.get(1..gt).unwrap_or_default().trim().trim_start_matches('/').to_ascii_lowercase();
        let name: String = tag.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if name == "br" {
            out.push('\n');
        } else if BLOCKS.contains(&name.as_str()) && !out.ends_with("\n\n") && !out.is_empty() {
            out.push_str("\n\n");
        }
        rest = after.get(gt.saturating_add(1)..).unwrap_or_default();
    }
    out.push_str(&entities(rest));
    out.trim().to_string()
}

fn entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or_default());
        let after = rest.get(amp..).unwrap_or_default();
        let end = after.get(..12.min(after.len())).and_then(|w| w.find(';'));
        let decoded = end.and_then(|e| {
            let name = after.get(1..e)?;
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" | "#39" => '\'',
                "nbsp" => ' ',
                n if n.starts_with("#x") || n.starts_with("#X") => char::from_u32(u32::from_str_radix(n.get(2..)?, 16).ok()?)?,
                n if n.starts_with('#') => char::from_u32(n.get(1..)?.parse().ok()?)?,
                _ => return None,
            };
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = after.get(e.saturating_add(1)..).unwrap_or_default();
            }
            None => {
                out.push('&');
                rest = after.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

fn space_json(s: &Space) -> Value {
    json!({
        "id": ids::key_id(s.key.as_str()).to_string(),
        "key": s.key.as_str(),
        "name": s.name,
        "type": "global",
        "status": "current",
        "homepageId": s.homepage.0.to_string(),
        "description": null,
        "icon": null,
        "_links": {"webui": format!("/spaces/{}", s.key.as_str())},
    })
}

fn version_json(number: u32, author: &str, at: &Timestamp, message: &str) -> Value {
    json!({"createdAt": cdate(at), "message": message, "number": number, "minorEdit": false, "authorId": author})
}

fn page_json(p: &Page, f: Option<Format>) -> Value {
    let creator = p.history.first().map(|v| v.author.as_str()).unwrap_or(&p.author);
    let mut v = json!({
        "id": p.id.0.to_string(),
        "status": "current",
        "title": p.title,
        "spaceId": ids::key_id(p.space.as_str()).to_string(),
        "parentId": p.parent.map(|i| i.0.to_string()),
        "parentType": p.parent.map(|_| "page"),
        "position": p.position,
        "authorId": creator,
        "ownerId": creator,
        "lastOwnerId": null,
        "createdAt": cdate(&p.created),
        "version": version_json(p.version, &p.author, &p.updated, &p.message),
        "_links": {
            "webui": format!("/spaces/{}/pages/{}", p.space.as_str(), p.id.0),
            "editui": format!("/pages/resumedraft.action?draftId={}", p.id.0),
            "tinyui": format!("/x/{}", p.id.0),
        },
    });
    if let (Some(f), Some(o)) = (f, v.as_object_mut()) {
        o.insert("body".into(), body_out(&p.body, f));
    }
    v
}

fn comment_json(page: &Page, c: &PageComment, f: Option<Format>) -> Value {
    let number = if c.edited.is_some() { 2 } else { 1 };
    let mut v = json!({
        "id": c.id.0.to_string(),
        "status": "current",
        "title": format!("Re: {}", page.title),
        "pageId": page.id.0.to_string(),
        "version": version_json(number, &c.author, &c.edited.unwrap_or(c.created), ""),
        "_links": {"webui": format!("/spaces/{}/pages/{}?focusedCommentId={}", page.space.as_str(), page.id.0, c.id.0)},
    });
    if let Some(o) = v.as_object_mut() {
        if let Some(p) = c.parent {
            o.insert("parentCommentId".into(), json!(p.0.to_string()));
        }
        if let Some(f) = f {
            o.insert("body".into(), body_out(&c.body, f));
        }
    }
    v
}

/// One page of `items` with `_links.next` (relative to the workspace base) and a `Link` header.
fn cursor_page(base: &str, path: &str, q: &Q, items: Vec<Value>) -> Result<Response, ApiError> {
    let start = match q.get("cursor").filter(|c| !c.is_empty()) {
        Some(c) => c.strip_prefix('o').and_then(|n| n.parse::<usize>().ok()).ok_or_else(|| ApiError::bad(C, "invalid cursor"))?,
        None => 0,
    };
    let limit = q.num("limit", DEFAULT_LIMIT, MAX_LIMIT, C)?.max(1);
    let total = items.len();
    let page: Vec<Value> = items.into_iter().skip(start).take(limit).collect();
    let end = start.saturating_add(page.len());
    let mut links = json!({"base": format!("{base}/wiki")});
    let mut next_link = None;
    if end < total {
        let mut qs: Vec<String> = q.0.iter().filter(|(k, _)| k != "cursor" && k != "limit").map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect();
        qs.push(format!("limit={limit}"));
        qs.push(format!("cursor=o{end}"));
        let next = format!("{path}?{}", qs.join("&"));
        if let Some(o) = links.as_object_mut() {
            o.insert("next".into(), json!(next));
        }
        next_link = Some(next);
    }
    let mut r = Json(json!({"results": page, "_links": links})).into_response();
    if let Some(n) = next_link
        && let Ok(h) = HeaderValue::from_str(&format!("<{n}>; rel=\"next\""))
    {
        r.headers_mut().insert(header::LINK, h);
    }
    Ok(r)
}

fn page_id(path: &P) -> Result<u64, ApiError> {
    path.get("id").trim().parse().map_err(|_| ApiError::bad(C, "the page id must be a number"))
}

fn find_page(ws: &Workspace, id: u64) -> Result<&Page, ApiError> {
    ws.page(Id(id)).map_err(|_| ApiError::not_found(C, format!("no page with id {id}")))
}

async fn spaces(cx: Ctx, q: Q) -> Result<Response, ApiError> {
    let base = cx.base.clone();
    cx.run(move |tx| {
        let (want_ids, keys) = (q.list("ids"), q.list("keys"));
        let status_ok = q.get("status").is_none_or(|s| s == "current");
        let type_ok = q.get("type").is_none_or(|t| t == "global");
        let mut all: Vec<&Space> = tx
            .ws()
            .spaces
            .values()
            .filter(|_| status_ok && type_ok)
            .filter(|s| want_ids.is_empty() || want_ids.contains(&ids::key_id(s.key.as_str()).to_string()))
            .filter(|s| keys.is_empty() || keys.iter().any(|k| k.eq_ignore_ascii_case(s.key.as_str())))
            .collect();
        match q.get("sort") {
            Some("-name") => all.sort_by(|a, b| b.name.cmp(&a.name)),
            Some("name") => all.sort_by(|a, b| a.name.cmp(&b.name)),
            Some("-key") => all.sort_by(|a, b| b.key.cmp(&a.key)),
            _ => {}
        }
        cursor_page(&base, "/wiki/api/v2/spaces", &q, all.into_iter().map(space_json).collect())
    })
    .await
}

async fn space(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let id = path.get("id");
    cx.run(move |tx| {
        let ws = tx.ws();
        let k = ids::space_key(ws, &id).ok_or_else(|| ApiError::not_found(C, "no such space"))?;
        let s = ws.space(&k).map_err(|e| ApiError::not_found(C, e.to_string()))?;
        Ok(Json(space_json(s)).into_response())
    })
    .await
}

/// Pages filtered and sorted by the shared `GET /pages` parameters.
fn select_pages<'a>(ws: &'a Workspace, q: &Q, space: Option<&str>) -> Result<Vec<&'a Page>, ApiError> {
    let want_ids = q.list("id");
    let mut space_keys: Vec<String> = q.list("space-id").iter().filter_map(|s| ids::space_key(ws, s)).collect();
    if let Some(s) = space {
        space_keys = vec![s.to_string()];
    } else if !q.list("space-id").is_empty() && space_keys.is_empty() {
        return Ok(vec![]);
    }
    let title = q.get("title");
    let status_ok = q.list("status").is_empty() || q.list("status").iter().any(|s| s == "current");
    let mut all: Vec<&Page> = ws
        .pages
        .values()
        .filter(|_| status_ok)
        .filter(|p| want_ids.is_empty() || want_ids.contains(&p.id.0.to_string()))
        .filter(|p| space_keys.is_empty() || space_keys.iter().any(|k| k == p.space.as_str()))
        .filter(|p| title.is_none_or(|t| p.title == t))
        .collect();
    match q.get("sort").unwrap_or("id") {
        "id" => {}
        "-id" => all.reverse(),
        "title" => all.sort_by(|a, b| a.title.cmp(&b.title)),
        "-title" => all.sort_by(|a, b| b.title.cmp(&a.title)),
        "created-date" => all.sort_by_key(|p| p.created),
        "-created-date" => all.sort_by_key(|p| std::cmp::Reverse(p.created)),
        "modified-date" => all.sort_by_key(|p| p.updated),
        "-modified-date" => all.sort_by_key(|p| std::cmp::Reverse(p.updated)),
        other => return Err(ApiError::bad(C, format!("sort '{}' is not supported", other.chars().take(40).collect::<String>()))),
    }
    Ok(all)
}

async fn pages(cx: Ctx, q: Q) -> Result<Response, ApiError> {
    let base = cx.base.clone();
    let f = format(&q)?;
    cx.run(move |tx| {
        let ws = tx.ws();
        let all = select_pages(ws, &q, None)?;
        cursor_page(&base, "/wiki/api/v2/pages", &q, all.into_iter().map(|p| page_json(p, f)).collect())
    })
    .await
}

async fn space_pages(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), path.get("id"));
    let f = format(&q)?;
    cx.run(move |tx| {
        let ws = tx.ws();
        let k = ids::space_key(ws, &id).ok_or_else(|| ApiError::not_found(C, "no such space"))?;
        let all = select_pages(ws, &q, Some(&k))?;
        let path = format!("/wiki/api/v2/spaces/{}/pages", pct(&id));
        cursor_page(&base, &path, &q, all.into_iter().map(|p| page_json(p, f)).collect())
    })
    .await
}

async fn page_create(cx: Ctx, Body(b): Body) -> Result<Response, ApiError> {
    cx.run(move |tx| {
        if b.get("status").and_then(Value::as_str).is_some_and(|s| s != "current") {
            return Err(ApiError::bad(C, "only status \"current\" is supported (drafts are not)"));
        }
        let ws = tx.ws();
        let sid = b
            .get("spaceId")
            .and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string())))
            .ok_or_else(|| ApiError::bad(C, "spaceId is required"))?;
        let space = ids::space_key(ws, &sid)
            .ok_or_else(|| ApiError::not_found(C, format!("no space with id {}", sid.chars().take(40).collect::<String>())))?;
        let title = b.get("title").and_then(Value::as_str).ok_or_else(|| ApiError::bad(C, "title is required"))?;
        let mut params = json!({"space": space, "title": title});
        if let Some(p) = b.get("parentId").filter(|p| !p.is_null()) {
            params["parent"] = p.clone();
        }
        let (body, fmt) = match body_in(b.get("body"))? {
            Some((v, f)) => (Some(v), f),
            None => (None, Format::Adf),
        };
        if let Some(v) = body {
            params["body"] = v;
        }
        let r = tx.run("page.create", params)?;
        let id = r.get("id").and_then(Value::as_u64).unwrap_or_default();
        let ws = tx.ws();
        Ok(Json(page_json(find_page(ws, id)?, Some(fmt))).into_response())
    })
    .await
}

async fn page_get(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let id = page_id(&path)?;
    let f = format(&q)?;
    let version = match q.get("version") {
        None | Some("") => None,
        Some(v) => Some(v.trim().parse::<u32>().map_err(|_| ApiError::bad(C, "version must be a number"))?),
    };
    cx.run(move |tx| {
        let ws = tx.ws();
        let p = find_page(ws, id)?;
        let mut v = page_json(p, f);
        if let Some(n) = version.filter(|n| *n != p.version) {
            let old = p.history.iter().find(|h| h.version == n).ok_or_else(|| ApiError::not_found(C, format!("page {id} has no version {n}")))?;
            v["title"] = json!(old.title);
            v["version"] = version_json(old.version, &old.author, &old.at, &old.message);
            if let Some(f) = f {
                v["body"] = body_out(&old.body, f);
            }
        }
        Ok(Json(v).into_response())
    })
    .await
}

async fn page_update(cx: Ctx, path: P, Body(b): Body) -> Result<Response, ApiError> {
    let id = page_id(&path)?;
    cx.run(move |tx| {
        if b.get("status").and_then(Value::as_str).is_some_and(|s| s != "current") {
            return Err(ApiError::bad(C, "only status \"current\" is supported (drafts are not)"));
        }
        let number = b
            .get("version")
            .and_then(|v| v.get("number"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| ApiError::bad(C, "version.number is required (the current version + 1)"))?;
        let base_version = number.checked_sub(1).ok_or_else(|| ApiError::bad(C, "version.number must be at least 2"))?;
        let current = find_page(tx.ws(), id)?;
        if current.version != base_version {
            return Err(ApiError::new(
                C,
                StatusCode::CONFLICT,
                format!("Version must be incremented on update. Current version is: {}", current.version),
            ));
        }
        let mut params = json!({"id": id, "baseVersion": base_version});
        if let Some(t) = b.get("title").and_then(Value::as_str).filter(|t| *t != current.title) {
            params["title"] = json!(t);
        }
        let (body, fmt) = match body_in(b.get("body"))? {
            Some((v, f)) => (Some(v), f),
            None => (None, Format::Adf),
        };
        if let Some(v) = body {
            params["body"] = v;
        }
        if let Some(m) = b.get("version").and_then(|v| v.get("message")).and_then(Value::as_str) {
            params["message"] = json!(m);
        }
        if params.get("title").is_none() && params.get("body").is_none() {
            return Err(ApiError::bad(C, "nothing to change: send a new title or body"));
        }
        tx.run("page.edit", params)?;
        let ws = tx.ws();
        Ok(Json(page_json(find_page(ws, id)?, Some(fmt))).into_response())
    })
    .await
}

async fn page_delete(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let id = page_id(&path)?;
    cx.run(move |tx| {
        find_page(tx.ws(), id)?;
        tx.run("page.delete", json!({"id": id}))?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

async fn versions(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), page_id(&path)?);
    cx.run(move |tx| {
        let r = tx.run("page.history", json!({"id": id}))?;
        let title = find_page(tx.ws(), id)?.title.clone();
        let mut all: Vec<Value> = r
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                let at = v
                    .get("when")
                    .and_then(Value::as_str)
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| t.to_utc())
                    .unwrap_or_default();
                let mut out = version_json(
                    v.get("number").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or_default(),
                    v.get("by").and_then(Value::as_str).unwrap_or_default(),
                    &at,
                    v.get("message").and_then(Value::as_str).unwrap_or_default(),
                );
                out["page"] = json!({"id": id.to_string(), "title": title});
                out
            })
            .collect();
        if q.get("sort") == Some("modified-date") {
            all.reverse();
        }
        cursor_page(&base, &format!("/wiki/api/v2/pages/{id}/versions"), &q, all)
    })
    .await
}

async fn children(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), page_id(&path)?);
    cx.run(move |tx| {
        let ws = tx.ws();
        find_page(ws, id)?;
        let all: Vec<Value> = ws
            .children(Id(id))
            .iter()
            .map(|c| json!({"id": c.id.0.to_string(), "status": "current", "title": c.title, "spaceId": ids::key_id(c.space.as_str()).to_string(), "childPosition": c.position}))
            .collect();
        cursor_page(&base, &format!("/wiki/api/v2/pages/{id}/children"), &q, all)
    })
    .await
}

async fn footer_comments(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), page_id(&path)?);
    let f = format(&q)?;
    cx.run(move |tx| {
        let p = find_page(tx.ws(), id)?;
        let mut all: Vec<Value> = p.comments.iter().filter(|c| c.anchor.is_none() && c.parent.is_none()).map(|c| comment_json(p, c, f)).collect();
        if q.get("sort") == Some("-created-date") {
            all.reverse();
        }
        cursor_page(&base, &format!("/wiki/api/v2/pages/{id}/footer-comments"), &q, all)
    })
    .await
}

fn num_or_str(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

async fn footer_comment_create(cx: Ctx, Body(b): Body) -> Result<Response, ApiError> {
    cx.run(move |tx| {
        let parent = num_or_str(b.get("parentCommentId"));
        let page = match (num_or_str(b.get("pageId")), parent) {
            (Some(p), _) => p,
            (None, Some(c)) => tx
                .ws()
                .pages
                .values()
                .find(|p| p.comments.iter().any(|x| x.id.0 == c))
                .map(|p| p.id.0)
                .ok_or_else(|| ApiError::not_found(C, format!("no comment with id {c}")))?,
            (None, None) => return Err(ApiError::bad(C, "pageId (or parentCommentId) is required")),
        };
        let (body, fmt) = body_in(b.get("body"))?.ok_or_else(|| ApiError::bad(C, "body is required"))?;
        let mut params = json!({"id": page, "body": body});
        if let Some(c) = parent {
            params["parent"] = json!(c);
        }
        let r = tx.run("page.comment", params)?;
        let cid = r.get("comment").and_then(Value::as_u64).unwrap_or_default();
        let p = find_page(tx.ws(), page)?;
        let c = p.comments.iter().find(|c| c.id.0 == cid).ok_or_else(|| ApiError::internal(C, "the new comment is missing"))?;
        Ok((StatusCode::CREATED, Json(comment_json(p, c, Some(fmt)))).into_response())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_to_text() {
        assert_eq!(storage_text("<p>Hello &amp; <b>bye</b></p><p>Two<br/>lines</p>"), "Hello & bye\n\nTwo\nlines");
        assert_eq!(storage_text("a &#65;&#x42; &bogus; & <unclosed"), "a AB &bogus; &");
        assert_eq!(storage_text(""), "");
        assert_eq!(entities("&#xFFFFFFFF;"), "&#xFFFFFFFF;");
    }
}
