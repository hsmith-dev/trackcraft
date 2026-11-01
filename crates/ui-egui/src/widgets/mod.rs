//! Small shared pieces used across views. Each draws with the theme [`Tokens`](crate::theme::Tokens)
//! and adds nothing to the accessibility tree beyond its text.

mod avatar;
mod card;
mod empty_state;
mod field_row;
mod issue_row;
mod pill;
mod truncated; // UI-6

pub(crate) use avatar::{name_of, who};
pub(crate) use card::card;
pub(crate) use empty_state::empty_state;
pub(crate) use field_row::field_row;
pub(crate) use issue_row::{Col, RowCx, issue_row};
pub(crate) use pill::pill;
pub(crate) use truncated::link_truncated; // UI-6
mod issue_picker; // UI-9
mod label_editor; // UI-9
mod number_field; // UI-9
mod user_picker; // UI-9
pub(crate) use issue_picker::{Candidate, issue_picker, short}; // UI-9
pub(crate) use label_editor::{label_editor, remove_button}; // UI-9
pub(crate) use number_field::number_field; // UI-9
pub(crate) use user_picker::{Picked, user_picker}; // UI-9
mod snippet; // UI-fixes
pub(crate) use snippet::{quote_excerpt, snippet, word_prefix}; // UI-fixes
