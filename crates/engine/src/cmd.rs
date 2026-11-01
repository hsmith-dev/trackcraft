//! The command registry, one module per area.

mod access;
mod agile;
mod fields; // M2.4
mod file;
mod import;
mod issue;
mod notify;
pub(crate) mod report; // M2.5
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
    v.extend(access::commands());
    v.extend(report::commands()); // M2.5
    v.extend(fields::commands()); // M2.4
    v
}
