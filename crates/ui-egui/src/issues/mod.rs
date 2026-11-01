//! Tracker views: the JQL issue list, the board, issue detail and the create dialog.

mod board;
mod create;
mod detail;
mod list;

// UI-6
#[cfg(test)]
pub(crate) use board::OTHER_STATUSES;
pub(crate) use board::{board, board_columns, date_range, days_left, sprint_board, state_chip};
pub(crate) use create::{create_dialog, new_form};
pub(crate) use detail::detail;
pub(crate) use list::list;
