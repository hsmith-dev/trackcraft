//! Recursive-descent parser. Precedence: `NOT` > `AND` > `OR`, as in Jira.

use crate::lex::{Tok, lex};
use crate::{Dir, Expr, JqlError, MAX_DEPTH, MAX_LIST, MAX_QUERY_LEN, Op, Operand, Query};

/// Parses a full query. An empty query (or one with only `ORDER BY`) matches everything.
pub fn parse(src: &str) -> Result<Query, JqlError> {
    if src.len() > MAX_QUERY_LEN {
        return Err(JqlError::TooLong);
    }
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let filter = if p.peek().is_none() || p.at_order_by() { None } else { Some(p.or()?) };
    let mut order = Vec::new();
    if p.at_order_by() {
        p.pos += 2;
        loop {
            let field = p.field_name()?;
            let dir = if p.peek().is_some_and(|t| t.is_kw("DESC")) {
                p.pos += 1;
                Dir::Desc
            } else {
                if p.peek().is_some_and(|t| t.is_kw("ASC")) {
                    p.pos += 1;
                }
                Dir::Asc
            };
            order.push((field, dir));
            if p.peek() == Some(&Tok::Comma) {
                p.pos += 1;
            } else {
                break;
            }
        }
    }
    if let Some(t) = p.peek() {
        return Err(JqlError::Expected { expected: "AND, OR, ORDER BY or the end of the query", found: t.describe() });
    }
    Ok(Query { filter, order })
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.pos.saturating_add(n))
    }
    fn found(&self) -> String {
        self.peek().map_or_else(|| "the end of the query".to_string(), Tok::describe)
    }
    fn at_order_by(&self) -> bool {
        self.peek().is_some_and(|t| t.is_kw("ORDER")) && self.peek_at(1).is_some_and(|t| t.is_kw("BY"))
    }
    fn enter(&mut self) -> Result<(), JqlError> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_DEPTH { Err(JqlError::TooDeep) } else { Ok(()) }
    }

    fn or(&mut self) -> Result<Expr, JqlError> {
        let mut lhs = self.and()?;
        while self.peek().is_some_and(|t| t.is_kw("OR")) {
            self.pos += 1;
            lhs = Expr::Or(Box::new(lhs), Box::new(self.and()?));
        }
        Ok(lhs)
    }

    fn and(&mut self) -> Result<Expr, JqlError> {
        let mut lhs = self.unary()?;
        while self.peek().is_some_and(|t| t.is_kw("AND")) {
            self.pos += 1;
            lhs = Expr::And(Box::new(lhs), Box::new(self.unary()?));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, JqlError> {
        self.enter()?;
        let e = if self.peek().is_some_and(|t| t.is_kw("NOT")) {
            self.pos += 1;
            Expr::Not(Box::new(self.unary()?))
        } else if self.peek() == Some(&Tok::LParen) {
            self.pos += 1;
            let e = self.or()?;
            if self.peek() != Some(&Tok::RParen) {
                return Err(JqlError::Expected { expected: "')'", found: self.found() });
            }
            self.pos += 1;
            e
        } else {
            self.clause()?
        };
        self.depth = self.depth.saturating_sub(1);
        Ok(e)
    }

    fn field_name(&mut self) -> Result<String, JqlError> {
        match self.peek() {
            Some(Tok::Word(w)) | Some(Tok::Str(w)) => {
                let w = w.clone();
                self.pos += 1;
                Ok(w)
            }
            _ => Err(JqlError::Expected { expected: "a field name", found: self.found() }),
        }
    }

    fn op(&mut self) -> Result<Op, JqlError> {
        let t = self.peek().cloned();
        let op = match &t {
            Some(Tok::Op("=")) => Op::Eq,
            Some(Tok::Op("!=")) => Op::Ne,
            Some(Tok::Op(">")) => Op::Gt,
            Some(Tok::Op(">=")) => Op::Ge,
            Some(Tok::Op("<")) => Op::Lt,
            Some(Tok::Op("<=")) => Op::Le,
            Some(Tok::Op("~")) => Op::Contains,
            Some(Tok::Op("!~")) => Op::NotContains,
            Some(t) if t.is_kw("IN") => Op::In,
            Some(t) if t.is_kw("NOT") && self.peek_at(1).is_some_and(|t| t.is_kw("IN")) => {
                self.pos += 1;
                Op::NotIn
            }
            Some(t) if t.is_kw("IS") => {
                if self.peek_at(1).is_some_and(|t| t.is_kw("NOT")) {
                    self.pos += 1;
                    Op::IsNot
                } else {
                    Op::Is
                }
            }
            _ => return Err(JqlError::Expected { expected: "an operator (=, !=, ~, IN, IS…)", found: self.found() }),
        };
        self.pos += 1;
        Ok(op)
    }

    fn clause(&mut self) -> Result<Expr, JqlError> {
        let field = self.field_name()?;
        let op = self.op()?;
        let value = match op {
            Op::In | Op::NotIn => self.list()?,
            _ => self.operand()?,
        };
        match (&op, &value) {
            (Op::Is | Op::IsNot, v) if *v != Operand::Empty => {
                return Err(JqlError::BadOperand { op: op.as_str(), value: "anything but EMPTY".into() });
            }
            (Op::In | Op::NotIn, _) | (Op::Is | Op::IsNot, _) => {}
            (_, Operand::List(_)) => return Err(JqlError::BadOperand { op: op.as_str(), value: "a list".into() }),
            _ => {}
        }
        Ok(Expr::Clause { field, op, value })
    }

    fn list(&mut self) -> Result<Operand, JqlError> {
        if self.peek() != Some(&Tok::LParen) {
            // `status IN currentUser()`-style single operands are accepted as a one-item list.
            return Ok(Operand::List(vec![self.operand()?]));
        }
        self.pos += 1;
        let mut items = Vec::new();
        if self.peek() == Some(&Tok::RParen) {
            self.pos += 1;
            return Ok(Operand::List(items));
        }
        loop {
            if items.len() >= MAX_LIST {
                return Err(JqlError::ListTooLong);
            }
            items.push(self.operand()?);
            match self.peek() {
                Some(Tok::Comma) => self.pos += 1,
                Some(Tok::RParen) => {
                    self.pos += 1;
                    return Ok(Operand::List(items));
                }
                _ => return Err(JqlError::Expected { expected: "',' or ')'", found: self.found() }),
            }
        }
    }

    fn operand(&mut self) -> Result<Operand, JqlError> {
        match self.peek().cloned() {
            Some(Tok::Str(s)) => {
                self.pos += 1;
                Ok(Operand::Value(s))
            }
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("EMPTY") || w.eq_ignore_ascii_case("NULL") => {
                self.pos += 1;
                Ok(Operand::Empty)
            }
            Some(Tok::Word(w)) if self.peek_at(1) == Some(&Tok::LParen) => {
                self.pos += 2;
                let mut args = Vec::new();
                while let Some(t) = self.peek().cloned() {
                    match t {
                        Tok::RParen => break,
                        Tok::Comma => self.pos += 1,
                        Tok::Word(a) | Tok::Str(a) => {
                            if args.len() >= MAX_LIST {
                                return Err(JqlError::ListTooLong);
                            }
                            args.push(a);
                            self.pos += 1;
                        }
                        other => return Err(JqlError::Expected { expected: "a function argument", found: other.describe() }),
                    }
                }
                if self.peek() != Some(&Tok::RParen) {
                    return Err(JqlError::Expected { expected: "')'", found: self.found() });
                }
                self.pos += 1;
                Ok(Operand::Func(w, args))
            }
            Some(Tok::Word(w)) => {
                self.pos += 1;
                Ok(Operand::Value(w))
            }
            _ => Err(JqlError::Expected { expected: "a value", found: self.found() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_queries() {
        let q =
            parse(r#"project = ATLAS AND status IN ("To Do", "In Progress") AND assignee = currentUser() ORDER BY priority DESC, created"#).unwrap();
        assert_eq!(q.order, vec![("priority".into(), Dir::Desc), ("created".into(), Dir::Asc)]);
        assert!(matches!(q.filter, Some(Expr::And(..))));
        assert!(parse("").unwrap().filter.is_none());
        assert!(parse("ORDER BY key").unwrap().filter.is_none());
        assert!(parse("assignee IS NOT EMPTY").is_ok());
        assert!(parse("NOT (labels = a OR labels = b)").is_ok());
        assert!(parse("\"Story Points\" >= 3").is_ok());
        assert!(parse("created >= -7d").is_ok());
    }

    #[test]
    fn rejects_bad_queries_without_panicking() {
        for bad in ["project", "project =", "a = 1 AND", "(a = 1", "a IS 3", "a = (1,2)", "a ! b", "\"open", "a IN (1,", "a = 1 b", "f(", "a = f(1"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(parse(&"(".repeat(500)), Err(JqlError::TooDeep));
        assert_eq!(parse(&"NOT ".repeat(500)), Err(JqlError::TooDeep));
        assert_eq!(parse(&"a".repeat(MAX_QUERY_LEN + 1)), Err(JqlError::TooLong));
    }
}
