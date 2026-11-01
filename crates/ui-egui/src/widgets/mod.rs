//! Small shared pieces used across views. Each draws with the theme [`Tokens`](crate::theme::Tokens)
//! and adds nothing to the accessibility tree beyond its text.

mod avatar;
mod card;
mod empty_state;
mod field_row;
mod issue_row;
mod pill;

pub(crate) use avatar::{name_of, who};
pub(crate) use card::card;
pub(crate) use empty_state::empty_state;
pub(crate) use field_row::field_row;
pub(crate) use issue_row::{Col, RowCx, issue_row};
pub(crate) use pill::pill;
