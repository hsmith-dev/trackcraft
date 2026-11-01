//! JQL (L1): lexer, parser and evaluator for the Jira Query Language.
//!
//! Behaviour comes from Atlassian's public JQL reference and black-box observation, never from
//! Atlassian code. The evaluator is data-agnostic: anything implementing [`Record`] can be
//! searched, so the model crate stays free of query logic.
//!
//! Supported today: `AND`/`OR`/`NOT`, parentheses, `= != > >= < <= ~ !~ IN NOT IN IS IS NOT`,
//! `EMPTY`/`NULL`, quoted field names, `ORDER BY f [ASC|DESC], …`, the functions
//! `currentUser()`, `now()`, `startOfDay()`, `endOfDay()`, and relative dates (`-7d`, `2w`, `-4h`).
//! Not yet: `WAS`, `CHANGED`, history functions, `cf[…]` custom fields (tracked in `docs/gaps.md`).
//!
//! Hostile input: query length, nesting and list sizes are capped; nothing here panics.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod eval;
mod lex;
mod parse;

pub use eval::{Context, FieldValue, Record, matches, sort};
pub use parse::parse;

/// Longest query accepted, in bytes.
pub const MAX_QUERY_LEN: usize = 64 * 1024;
/// Deepest `(`/`NOT` nesting accepted.
pub const MAX_DEPTH: usize = 64;
/// Most values in one `IN (…)` list.
pub const MAX_LIST: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JqlError {
    #[error("the query is longer than {MAX_QUERY_LEN} bytes")]
    TooLong,
    #[error("the query nests deeper than {MAX_DEPTH} levels")]
    TooDeep,
    #[error("an IN list holds more than {MAX_LIST} values")]
    ListTooLong,
    #[error("unterminated string starting at character {0}")]
    UnterminatedString(usize),
    #[error("unexpected character '{0}' at position {1}")]
    UnexpectedChar(char, usize),
    #[error("expected {expected} but found {found}")]
    Expected { expected: &'static str, found: String },
    #[error("unknown function {0}()")]
    UnknownFunction(String),
    #[error("operator {op} can't be used with {value}")]
    BadOperand { op: &'static str, value: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub filter: Option<Expr>,
    pub order: Vec<(String, Dir)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Clause { field: String, op: Op, value: Operand },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    NotContains,
    In,
    NotIn,
    Is,
    IsNot,
}

impl Op {
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Eq => "=",
            Op::Ne => "!=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Contains => "~",
            Op::NotContains => "!~",
            Op::In => "IN",
            Op::NotIn => "NOT IN",
            Op::Is => "IS",
            Op::IsNot => "IS NOT",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Value(String),
    List(Vec<Operand>),
    Empty,
    Func(String, Vec<String>),
}
