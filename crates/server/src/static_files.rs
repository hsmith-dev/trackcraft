//! Serves a static directory (the web client's `dist/web`) next to the API: `trackcraft-server
//! serve --web DIR` (M4.4, `docs/web.md`).
//!
//! Only `GET`/`HEAD` requests that no API route matched reach it, so API routes always win, and
//! API-looking paths (`/api`, `/rest`, `/wiki`, `/w/…`) keep their JSON 404s. `/` and directories
//! serve `index.html`; an unknown path without a file extension also gets the root `index.html`
//! (the app's own routes), while a missing `x.wasm` is a plain 404.
//!
//! Hostile paths are refused: the path is percent-decoded, `.`/`..`/dot-file segments,
//! backslashes and NULs are rejected, and the resolved file is canonicalized and must lie inside
//! the canonical root (so a symlink can't lead out). Files over [`MAX_FILE_BYTES`] aren't served.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use trackcraft_api::{ApiError, Flavor};

use crate::ServerError;

/// Largest file served (the web client's `.wasm` is well under it).
pub const MAX_FILE_BYTES: u64 = 64 << 20;
/// Longest request path looked at.
const MAX_PATH_BYTES: usize = 2048;
/// Most path segments looked at.
const MAX_SEGMENTS: usize = 32;

/// A directory served as a static site.
#[derive(Debug, Clone)]
pub struct StaticDir {
    root: PathBuf,
}

impl StaticDir {
    /// The directory `dir`, which must exist (it is canonicalized now; files are resolved
    /// against the canonical root on each request).
    pub fn new(dir: &Path) -> Result<Self, ServerError> {
        let root = dir.canonicalize().map_err(|e| ServerError::io(dir, e))?;
        if !root.is_dir() {
            return Err(ServerError::Config(format!("--web {}: not a directory", dir.display())));
        }
        Ok(StaticDir { root })
    }

    /// The canonical root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file a request path names, if it is a servable file inside the root. Blocking.
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let rel = clean_path(path)?;
        let mut p = self.root.join(rel);
        if p.is_dir() {
            p.push("index.html");
        }
        let canon = p.canonicalize().ok()?;
        if !canon.starts_with(&self.root) {
            return None;
        }
        let meta = std::fs::metadata(&canon).ok()?;
        (meta.is_file() && meta.len() <= MAX_FILE_BYTES).then_some(canon)
    }

    /// The response for `method path`: the file, the index fallback, or a 404.
    pub async fn respond(self: Arc<Self>, method: Method, uri: Uri) -> Response {
        let path = uri.path().to_string();
        if is_api_path(&path) || (method != Method::GET && method != Method::HEAD) {
            let shown: String = path.chars().take(200).collect();
            return ApiError::not_found(Flavor::of(&path), format!("no such endpoint: {shown}")).into_response();
        }
        let head = method == Method::HEAD;
        let dir = Arc::clone(&self);
        let found = tokio::task::spawn_blocking(move || {
            let file = dir.resolve(&path).or_else(|| if wants_index(&path) { dir.resolve("/") } else { None })?;
            let bytes = std::fs::read(&file).ok()?;
            // The size can change between the check and the read.
            (bytes.len() as u64 <= MAX_FILE_BYTES).then_some((file, bytes))
        })
        .await
        .ok()
        .flatten();
        match found {
            Some((file, bytes)) => file_response(&file, bytes, head),
            None => (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], "not found").into_response(),
        }
    }
}

/// `router` with `dir` served for every request no route matched. API routes keep precedence.
pub fn with_web(router: Router, dir: Option<&Path>) -> Router {
    let Some(dir) = dir else { return router };
    match StaticDir::new(dir) {
        Ok(d) => {
            let d = Arc::new(d);
            router.fallback(move |method: Method, uri: Uri| Arc::clone(&d).respond(method, uri))
        }
        Err(e) => {
            eprintln!("trackcraft-server: not serving the web client: {e}");
            router
        }
    }
}

/// Paths that belong to the API surfaces: they never serve files.
fn is_api_path(path: &str) -> bool {
    ["/api", "/rest", "/wiki", "/w"].iter().any(|p| path == *p || path.strip_prefix(p).is_some_and(|r| r.starts_with('/')))
}

/// An unknown path whose last segment has no extension is one of the app's own pages.
fn wants_index(path: &str) -> bool {
    !path.rsplit('/').next().unwrap_or_default().contains('.')
}

/// The relative file path a URL path names, or `None` if it is hostile.
fn clean_path(path: &str) -> Option<PathBuf> {
    if path.len() > MAX_PATH_BYTES {
        return None;
    }
    let decoded = percent_decode(path)?;
    if decoded.contains('\\') || decoded.contains('\0') {
        return None;
    }
    let mut rel = PathBuf::new();
    for (i, seg) in decoded.split('/').filter(|s| !s.is_empty()).enumerate() {
        if i >= MAX_SEGMENTS || seg.starts_with('.') || seg.contains(':') {
            return None;
        }
        rel.push(seg);
    }
    Some(rel)
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The `Content-Type` for a file name.
pub fn mime(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" | "md" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// Trunk names built files `name-<16 hex>…`: those never change and can be cached for good.
fn is_hashed(path: &Path) -> bool {
    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    stem.split(['-', '_', '.']).any(|part| part.len() == 16 && part.bytes().all(|c| c.is_ascii_hexdigit()))
}

fn file_response(file: &Path, bytes: Vec<u8>, head: bool) -> Response {
    let cache = if is_hashed(file) && !mime(file).starts_with("text/html") { "public, max-age=31536000, immutable" } else { "no-cache" };
    let len = bytes.len();
    let body = if head { Body::empty() } else { Body::from(bytes) };
    let mut r = (StatusCode::OK, body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime(file)));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    r
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;
    use trackcraft_api::{Hosted, Hub, NullSink};
    use trackcraft_engine::Session;

    use super::*;
    use crate::{Options, Server, TokenIdentity};

    struct Site {
        dir: PathBuf,
        outside: PathBuf,
        router: Router,
    }

    impl Drop for Site {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
            let _ = std::fs::remove_file(&self.outside);
        }
    }

    fn site(name: &str) -> Site {
        let base = std::env::temp_dir().join(format!("tc-static-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("dist");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("index.html"), "<!DOCTYPE html><title>TrackCraft</title>").unwrap();
        std::fs::write(dir.join("trackcraft-web-0123456789abcdef_bg.wasm"), b"\0asm\x01\0\0\0").unwrap();
        std::fs::write(dir.join("trackcraft-web-0123456789abcdef.js"), "export default 1;").unwrap();
        std::fs::write(dir.join("sub/index.html"), "sub").unwrap();
        std::fs::write(dir.join(".secret"), "dot").unwrap();
        let outside = base.join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, dir.join("link.txt")).unwrap();
        let hub = Arc::new(Hub::new());
        hub.add(Hosted::new("lab", Session::sample(), None));
        let opts = Options { log_requests: false, web: Some(dir.clone()), ..Options::default() };
        let server = Server::new(hub, Arc::new(TokenIdentity::new(vec![])), Arc::new(NullSink), opts);
        Site { router: server.router(), dir: base, outside }
    }

    async fn fetch(router: &Router, method: &str, path: &str) -> (StatusCode, String, Vec<u8>) {
        let req = Request::builder().method(method).uri(path).body(Body::empty()).unwrap();
        let r = router.clone().oneshot(req).await.unwrap();
        let ct = r.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let status = r.status();
        (status, ct, to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec())
    }

    #[tokio::test]
    async fn serves_index_and_wasm_with_mime_types() {
        let s = site("mime");
        let (st, ct, body) = fetch(&s.router, "GET", "/").await;
        assert_eq!((st, ct.as_str()), (StatusCode::OK, "text/html; charset=utf-8"));
        assert!(String::from_utf8(body).unwrap().contains("TrackCraft"));
        let (st, ct, body) = fetch(&s.router, "GET", "/trackcraft-web-0123456789abcdef_bg.wasm").await;
        assert_eq!((st, ct.as_str()), (StatusCode::OK, "application/wasm"));
        assert_eq!(body, b"\0asm\x01\0\0\0");
        let (st, ct, _) = fetch(&s.router, "GET", "/trackcraft-web-0123456789abcdef.js").await;
        assert_eq!((st, ct.as_str()), (StatusCode::OK, "text/javascript; charset=utf-8"));
        let (st, _, body) = fetch(&s.router, "HEAD", "/").await;
        assert_eq!(st, StatusCode::OK);
        assert!(body.is_empty());
        let (_, _, body) = fetch(&s.router, "GET", "/sub/").await;
        assert_eq!(body, b"sub");
    }

    #[tokio::test]
    async fn caches_hashed_files_only() {
        let s = site("cache");
        let req = |p: &str| Request::builder().uri(p).body(Body::empty()).unwrap();
        let r = s.router.clone().oneshot(req("/trackcraft-web-0123456789abcdef_bg.wasm")).await.unwrap();
        assert_eq!(r.headers()[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
        let r = s.router.clone().oneshot(req("/index.html")).await.unwrap();
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(r.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    }

    #[tokio::test]
    async fn index_fallback_for_app_paths_only() {
        let s = site("fallback");
        let (st, ct, _) = fetch(&s.router, "GET", "/board/LUME").await;
        assert_eq!((st, ct.as_str()), (StatusCode::OK, "text/html; charset=utf-8"));
        let (st, _, _) = fetch(&s.router, "GET", "/missing.wasm").await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn api_routes_take_precedence() {
        let s = site("api");
        let (st, ct, body) = fetch(&s.router, "GET", "/api/health").await;
        assert_eq!(st, StatusCode::OK);
        assert!(ct.starts_with("application/json"));
        assert!(String::from_utf8(body).unwrap().contains("\"ok\""));
        // Unknown API paths keep their JSON 404, never index.html.
        for p in ["/api/nope", "/rest/api/3/nope", "/wiki/api/v2/nope", "/w/lab/nope"] {
            let (st, ct, _) = fetch(&s.router, "GET", p).await;
            assert_eq!(st, StatusCode::NOT_FOUND, "{p}");
            assert!(ct.starts_with("application/json"), "{p}: {ct}");
        }
        let (st, ct, _) = fetch(&s.router, "POST", "/index.html").await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(ct.starts_with("application/json"));
    }

    #[tokio::test]
    async fn traversal_is_refused() {
        let s = site("traversal");
        for p in [
            "/../outside.txt",
            "/%2e%2e/outside.txt",
            "/%2E%2E%2Foutside.txt",
            "/sub/..%2f..%2foutside.txt",
            "/..%5coutside.txt",
            "/.secret",
            "/%00",
            "/link.txt",
            "/%zz",
            "/C:/Windows/win.ini",
        ] {
            let (st, _, body) = fetch(&s.router, "GET", p).await;
            let text = String::from_utf8_lossy(&body);
            assert!(!text.contains("secret") && text != "dot", "{p} leaked: {text}");
            // Either refused, or (extensionless) the app's index — never the file.
            assert!(st == StatusCode::NOT_FOUND || text.contains("TrackCraft"), "{p}: {st}");
        }
        assert!(clean_path("/a/../b").is_none());
        assert!(clean_path(&"/a".repeat(MAX_SEGMENTS + 1)).is_none());
        assert_eq!(clean_path("/a/b.js").unwrap(), PathBuf::from("a/b.js"));
    }

    #[test]
    fn missing_dir_is_an_error_and_mime_table() {
        assert!(StaticDir::new(Path::new("/definitely/not/here/tc")).is_err());
        assert_eq!(mime(Path::new("a.WASM")), "application/wasm");
        assert_eq!(mime(Path::new("a.css")), "text/css; charset=utf-8");
        assert_eq!(mime(Path::new("a")), "application/octet-stream");
        assert!(is_hashed(Path::new("trackcraft-web-0123456789abcdef_bg.wasm")));
        assert!(!is_hashed(Path::new("index.html")));
    }
}
