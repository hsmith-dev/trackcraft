//! The command registry, one module per area.

mod agile;
mod file;
mod import;
mod issue;
mod notify;
mod wiki;

use crate::CommandSpec;

pub fn all() -> Vec<CommandSpec> {
    let mut v = Vec::new();
    v.extend(file::commands());
    v.extend(issue::commands());
    v.extend(agile::commands());
    v.extend(wiki::commands());
    v.extend(import::commands());
    v.extend(notify::commands()); // M4.5
    v
}
