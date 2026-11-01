//! The data directory: one `NAME.track` file per hosted workspace, loaded at start and saved
//! atomically (`trackcraft_store::save`) after changes.

use std::path::{Path, PathBuf};

use trackcraft_api::{Hosted, Hub};
use trackcraft_engine::Session;
use trackcraft_engine::model::Workspace;

use crate::ServerError;

/// Longest workspace name.
pub const MAX_NAME: usize = 64;

/// A workspace name is its file stem and its URL segment (`/w/{name}`): 1–64 of `a-z 0-9 - _`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn workspace_path(data: &Path, name: &str) -> PathBuf {
    data.join(format!("{name}.track"))
}

/// Every `*.track` file in `data` with a valid name, sorted.
pub fn list(data: &Path) -> Result<Vec<(String, PathBuf)>, ServerError> {
    let mut out = Vec::new();
    let dir = std::fs::read_dir(data).map_err(|e| ServerError::io(data, e))?;
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("track") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && valid_name(stem)
        {
            out.push((stem.to_string(), path.clone()));
        }
    }
    out.sort();
    Ok(out)
}

/// Loads every workspace in `data` into `hub`. A file that fails to load stops the start-up
/// (serving a team without one of its workspaces would be worse than not starting).
pub fn load_all(data: &Path, hub: &Hub) -> Result<usize, ServerError> {
    let found = list(data)?;
    for (name, path) in &found {
        let ws = trackcraft_store::load(path).map_err(|e| ServerError::Store(e.to_string()))?;
        let user = ws.users.keys().next().cloned().unwrap_or_default();
        let mut s = Session::new(ws, &user);
        s.path = Some(path.clone());
        hub.add(Hosted::new(name, s, Some(path.clone())));
    }
    Ok(found.len())
}

/// Creates `data/NAME.track`: the sample workspace, or an empty one (with `user` as its first
/// member, if given). Refuses to overwrite.
pub fn create(data: &Path, name: &str, sample: bool, user: Option<&str>) -> Result<PathBuf, ServerError> {
    if !valid_name(name) {
        return Err(ServerError::Config(format!("workspace names are 1–{MAX_NAME} characters of a-z, 0-9, - and _")));
    }
    std::fs::create_dir_all(data).map_err(|e| ServerError::io(data, e))?;
    let path = workspace_path(data, name);
    if path.exists() {
        return Err(ServerError::Config(format!("{} already exists", path.display())));
    }
    let mut ws = if sample { Session::sample().ws } else { Workspace::new(name) };
    if let Some(u) = user.filter(|u| ws.user(u).is_err()) {
        ws.add_user(u, u).map_err(|e| ServerError::Config(e.to_string()))?;
    }
    trackcraft_store::save(&ws, &path).map_err(|e| ServerError::Store(e.to_string()))?;
    Ok(path)
}

/// Sets a user's workspace role in `data/NAME.track` (`trackcraft-server user role`). Without a
/// name, the only workspace in `data`. Edits the file, so run it while the server is stopped (a
/// running server would save over it); online, admins use `access.setWorkspaceRole`. Refuses to
/// demote the last admin, like the command.
pub fn set_role(data: &Path, name: Option<&str>, user: &str, role: &str) -> Result<PathBuf, ServerError> {
    let role = trackcraft_engine::model::WorkspaceRole::parse(role).ok_or_else(|| ServerError::Config("--role is admin, member or viewer".into()))?;
    let path = match name {
        Some(n) if valid_name(n) => workspace_path(data, n),
        Some(_) => return Err(ServerError::Config(format!("workspace names are 1–{MAX_NAME} characters of a-z, 0-9, - and _"))),
        None => match list(data)?.as_slice() {
            [(_, p)] => p.clone(),
            [] => return Err(ServerError::Config(format!("no workspaces in {}", data.display()))),
            _ => return Err(ServerError::Config("several workspaces: pass --workspace NAME".into())),
        },
    };
    let mut ws = trackcraft_store::load(&path).map_err(|e| ServerError::Store(e.to_string()))?;
    ws.set_workspace_role(user, role).map_err(|e| ServerError::Config(e.to_string()))?;
    trackcraft_store::save(&ws, &path).map_err(|e| ServerError::Store(e.to_string()))?;
    Ok(path)
}

/// Saves every workspace whose revision moved since its last save. Returns how many were written.
/// The session is locked only to copy the workspace; the write happens after.
pub fn flush(hub: &Hub) -> Result<usize, ServerError> {
    let mut saved = 0;
    let mut first_err = None;
    for h in hub.all() {
        let Some(path) = h.path().map(Path::to_path_buf) else { continue };
        let (rev, ws) = {
            let s = h.lock();
            if s.revision() == h.saved_revision() {
                continue;
            }
            (s.revision(), s.ws.clone())
        };
        match trackcraft_store::save(&ws, &path) {
            Ok(()) => {
                h.mark_saved(rev);
                saved += 1;
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(ServerError::Store(e.to_string()));
                }
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(saved),
    }
}
