//! Tracker views: the JQL issue list, the board, issue detail and the create dialog.

mod board;
mod create;
mod detail;
mod list;

pub(crate) use board::{board, board_columns};
pub(crate) use create::{create_dialog, new_form};
pub(crate) use detail::detail;
pub(crate) use list::list;
