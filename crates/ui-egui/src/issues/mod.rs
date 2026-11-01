//! Tracker views: the JQL issue list, the board, issue detail and the create dialog.

mod board;
mod create;
mod detail;
mod fields; // M2.4
mod list;

// UI-6
#[cfg(test)]
pub(crate) use board::OTHER_STATUSES;
pub(crate) use board::{board, board_columns, date_range, days_left, sprint_board, state_chip};
pub(crate) use create::{create_dialog, new_form};
pub(crate) use detail::detail;
pub(crate) use detail::{ActivityTab, Polish}; // UI-9
pub(crate) use fields::{CreateExtra, short_name}; // M2.4
pub(crate) use list::list;
