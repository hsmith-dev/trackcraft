//! UI-37: may the acting user run a command? Views use [`can`] to disable the controls they
//! couldn't use, with [`denied`] as the tooltip.
//!
//! The answer is `access.check`'s (both call `Session::authorize`; calling it directly skips the
//! filtered workspace copy a query costs under enforcement). Answers are cached until the
//! workspace revision or the acting user changes, so asking every frame is cheap. A local file
//! doesn't enforce permissions (one person owns it), so everything is allowed there; a server
//! workspace (remote mode) or an enforcing session asks.

use std::collections::HashMap;

use serde_json::Value;

use crate::TrackApp;

/// Cached answers at most (a frame asks a handful; hostile params can't grow it without bound).
const CAP: usize = 4096;

/// Answers for one workspace revision and acting user.
#[derive(Debug, Default)]
pub(crate) struct CanCache {
    stamp: Option<(u64, String)>,
    answers: HashMap<String, bool>,
}

/// Permissions apply: the session enforces them, or the workspace lives on a server.
pub(crate) fn enforced(app: &TrackApp) -> bool {
    app.session.is_enforcing() || app.is_remote()
}

/// Whether the acting user may run `command` with `params` (always true when nothing enforces).
pub(crate) fn can(app: &mut TrackApp, command: &str, params: &Value) -> bool {
    if !enforced(app) {
        return true;
    }
    let stamp = (app.session.revision(), app.session.user.clone());
    if app.can.stamp.as_ref() != Some(&stamp) {
        app.can.answers.clear();
        app.can.stamp = Some(stamp);
    }
    let key = format!("{command}\u{1f}{params}");
    if let Some(a) = app.can.answers.get(&key) {
        return *a;
    }
    let a = app.session.authorize(&app.session.user, command, params).is_ok();
    if app.can.answers.len() < CAP {
        app.can.answers.insert(key, a);
    }
    a
}

/// The tooltip on a control the user may not use: "You don't have permission to {what}".
pub(crate) fn denied(what: &str) -> String {
    format!("You don't have permission to {what}")
}
