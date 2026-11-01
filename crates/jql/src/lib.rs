//! JQL (L1): lexer, parser and evaluator for the Jira Query Language.
//!
//! Behaviour comes from Atlassian's public JQL reference and black-box observation, never from
//! Atlassian code. The evaluator is data-agnostic: anything implementing [`Record`] can be
//! searched, so the model crate stays free of query logic.
//!
//! Supported today: `AND`/`OR`/`NOT`, parentheses, `= != > >= < <= ~ !~ IN NOT IN IS IS NOT`,
//! `EMPTY`/`NULL`, quoted field names, `ORDER BY f [ASC|DESC], …`, the functions
//! `currentUser()`, `now()`, `startOfDay()`, `endOfDay()`, and relative dates (`-7d`, `2w`, `-4h`).
//!
//! History operators (from the public JQL reference): `WAS`, `WAS NOT`, `WAS IN (…)`,
//! `WAS NOT IN (…)` and `CHANGED`, with the predicates `AFTER`, `BEFORE`, `ON`, `DURING (a, b)`,
//! `BY user|(users…)`, and for `CHANGED` also `FROM value` and `TO value`. A record opts in per
//! field through [`Record::history`]; fields without history give a clear error.
//! Not yet: history functions, `cf[…]` custom fields (tracked in `docs/gaps.md`).
//!
//! Hostile input: query length, nesting and list sizes are capped; nothing here panics.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod eval;
mod lex;
mod parse;

pub use eval::{Context, FieldChange, FieldHistory, FieldValue, Record, matches, sort};
pub use parse::parse;

/// Longest query accepted, in bytes.
pub const MAX_QUERY_LEN: usize = 64 * 1024;
/// Deepest `(`/`NOT` nesting accepted.
pub const MAX_DEPTH: usize = 64;
/// Most values in one `IN (…)` list.
pub const MAX_LIST: usize = 10_000;
/// Most predicates (`AFTER`, `BY`, `FROM`…) on one `WAS`/`CHANGED` clause.
pub const MAX_PREDICATES: usize = 16;

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
    #[error("a WAS/CHANGED clause takes at most {MAX_PREDICATES} predicates")]
    TooManyPredicates,
    #[error("'{0}' is not a date (use 2026-10-31, 2026-10-31 14:00, -7d or a date function)")]
    BadDate(String),
    #[error("the {op} operator is not supported for field '{field}'")]
    HistoryUnsupported { op: &'static str, field: String },
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
    Clause {
        field: String,
        op: Op,
        value: Operand,
    },
    /// `field WAS …` / `field CHANGED …`.
    History(History),
}

/// A history clause: `status WAS "In Progress" BY ann AFTER -7d`, `status CHANGED FROM a TO b`.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    pub field: String,
    pub op: HistoryOp,
    /// The value for `WAS`/`WAS NOT` (a value or `EMPTY`), the list for `WAS [NOT] IN`; `None`
    /// for `CHANGED`.
    pub value: Option<Operand>,
    pub predicates: Vec<Predicate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryOp {
    Was,
    WasNot,
    WasIn,
    WasNotIn,
    Changed,
}

impl HistoryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryOp::Was => "WAS",
            HistoryOp::WasNot => "WAS NOT",
            HistoryOp::WasIn => "WAS IN",
            HistoryOp::WasNotIn => "WAS NOT IN",
            HistoryOp::Changed => "CHANGED",
        }
    }
}

/// A `WAS`/`CHANGED` predicate. `By` holds a list (`BY ann` is a one-item list).
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    After(Operand),
    Before(Operand),
    On(Operand),
    During(Operand, Operand),
    By(Vec<Operand>),
    From(Operand),
    To(Operand),
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
