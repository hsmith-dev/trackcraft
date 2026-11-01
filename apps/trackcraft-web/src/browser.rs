//! Browser APIs through `web-sys`: downloads, the file picker, `localStorage`, the leave-page
//! guard and the server check. Every JS call is a wasm-bindgen binding; there is no handwritten JS.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

fn js(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

/// The page's query string (`?sample&webgl`), or empty.
pub fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

/// Whether the query string has `key` (`?sample`, `?a=1&sample`).
pub fn has_flag(key: &str) -> bool {
    web_sys::UrlSearchParams::new_with_str(&query()).is_ok_and(|p| p.has(key))
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// The stored value under `key`, if the browser allows storage and has one.
pub fn stored(key: &str) -> Option<String> {
    local_storage()?.get_item(key).ok()?
}

/// Stores `value` under `key`; fails when storage is unavailable or full.
pub fn store(key: &str, value: &str) -> Result<(), String> {
    local_storage().ok_or("browser storage is unavailable")?.set_item(key, value).map_err(js)
}

/// Removes `key` from storage (nothing to do when storage is unavailable).
pub fn remove(key: &str) -> Result<(), String> {
    match local_storage() {
        Some(s) => s.remove_item(key).map_err(js),
        None => Ok(()),
    }
}

/// Starts a download of `bytes` as `name` (a Blob behind an object URL and a clicked `<a download>`).
pub fn download(name: &str, bytes: &[u8], mime: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor".to_string())?;
    a.set_href(&url);
    a.set_download(name);
    a.click();
    // The click has handed the Blob to the download; the URL is no longer needed.
    if let Err(e) = web_sys::Url::revoke_object_url(&url) {
        log::warn!("could not release the download URL: {}", js(e));
    }
    Ok(())
}

/// What a picked file delivered: its name and text, or why it couldn't be read.
pub type Picked = Rc<RefCell<Option<Result<(String, String), String>>>>;

/// Opens the browser's file picker for one `.track` file (an `<input type=file>` made here and
/// never added to the page). The file's text arrives in `into`, then `ctx` repaints. Files over
/// `max_bytes` are refused before they're read. Cancelling delivers nothing.
pub fn pick_text_file(accept: &str, max_bytes: f64, into: Picked, ctx: egui::Context) -> Result<(), String> {
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let input: web_sys::HtmlInputElement = document.create_element("input").map_err(js)?.dyn_into().map_err(|_| "not an input".to_string())?;
    input.set_type("file");
    input.set_accept(accept);
    let picker = input.clone();
    let on_change = Closure::once_into_js(move || {
        let Some(file) = picker.files().and_then(|f| f.get(0)) else { return };
        let name = file.name();
        if file.size() > max_bytes {
            *into.borrow_mut() = Some(Err(format!("{name} is larger than {} MB", (max_bytes / 1_048_576.0).round())));
            ctx.request_repaint();
            return;
        }
        wasm_bindgen_futures::spawn_local(async move {
            let read = JsFuture::from(file.text()).await;
            *into.borrow_mut() = Some(match read {
                Ok(text) => text.as_string().map(|t| (name.clone(), t)).ok_or_else(|| format!("{name} isn't text")),
                Err(e) => Err(format!("couldn't read {name}: {}", js(e))),
            });
            ctx.request_repaint();
        });
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    input.click();
    Ok(())
}

/// Asks before the page is left while `unsafe_to_leave` is set (work not kept anywhere).
pub fn guard_unload(unsafe_to_leave: Rc<Cell<bool>>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let on_unload = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |e: web_sys::BeforeUnloadEvent| {
        if unsafe_to_leave.get() {
            e.prevent_default();
            e.set_return_value("unsaved");
        }
    });
    window.add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref()).map_err(js)?;
    // The listener lives as long as the page.
    on_unload.forget();
    Ok(())
}

/// Checks whether the page was served by a TrackCraft server: `GET api/health` relative to the
/// page (so a server mounted under a prefix is found too), same origin, no credentials. Sets
/// `found` to the server's version when it answers like one.
pub fn detect_server(found: Rc<RefCell<Option<String>>>, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        match health().await {
            Ok(Some(version)) => {
                *found.borrow_mut() = Some(version);
                ctx.request_repaint();
            }
            Ok(None) => {}
            Err(e) => log::info!("trackcraft-web: no TrackCraft server here ({e})"),
        }
    });
}

async fn health() -> Result<Option<String>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let req = web_sys::Request::new_with_str_and_init("api/health", &init).map_err(js)?;
    let resp: web_sys::Response =
        JsFuture::from(window.fetch_with_request(&req)).await.map_err(js)?.dyn_into().map_err(|_| "not a response".to_string())?;
    if !resp.ok() {
        return Ok(None);
    }
    let text = JsFuture::from(resp.text().map_err(js)?).await.map_err(js)?;
    // A health answer is tiny; anything long isn't one.
    Ok(text.as_string().filter(|t| t.len() <= 4096).and_then(|t| crate::logic::parse_health(&t)))
}
