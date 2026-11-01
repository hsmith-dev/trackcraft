//! Imports: Jira CSV / JSON exports and Confluence space exports (`trackcraft-import`).

use serde_json::Value;

use crate::{CmdResult, CommandSpec, Session, params};
use trackcraft_import::{Actor, Kind, import_path};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("workspace.import", "Import…", "File › Import", import).params(
            r#"{"kind": "jiraCsv" | "jiraJson" | "confluence", "path": "export.csv" (Confluence: a folder, .zip or one page), "space": "KEY" (Confluence, optional)}"#,
        ),
    ]
}

fn import(s: &mut Session, p: &Value) -> CmdResult {
    let kind = params::str(p, "kind")?;
    let kind = Kind::parse(kind).ok_or("\"kind\" must be jiraCsv, jiraJson or confluence")?;
    let path = std::path::PathBuf::from(params::str(p, "path")?);
    let space = params::opt_str(p, "space")?;
    let now = s.now();
    let user = s.user.clone();
    let known: Vec<String> = s.ws.users.keys().cloned().collect();
    let report = import_path(&mut s.ws, kind, &path, space, Actor { user: &user, now }).map_err(|e| e.to_string())?;
    // People an import adds are members, not admins.
    let added: Vec<String> = s.ws.users.keys().filter(|u| !known.contains(u)).cloned().collect();
    for u in added {
        s.ws.set_workspace_role(&u, trackcraft_model::WorkspaceRole::Member).map_err(|e| e.to_string())?;
    }
    Ok(report.to_json())
}
