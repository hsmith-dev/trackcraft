//! The command registry, one module per area.

mod file;
mod issue;
mod wiki;

use crate::CommandSpec;

pub fn all() -> Vec<CommandSpec> {
    let mut v = Vec::new();
    v.extend(file::commands());
    v.extend(issue::commands());
    v.extend(wiki::commands());
    v
}
