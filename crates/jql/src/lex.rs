//! Tokens. Keywords are case-insensitive; words stay as written so field names keep their case.

use crate::JqlError;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A bare word: field name, keyword, unquoted value (`ATLAS-1`, `-7d`, `High`).
    Word(String),
    /// A quoted string (`"In Progress"`).
    Str(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Word(w) => format!("'{w}'"),
            Tok::Str(s) => format!("\"{s}\""),
            Tok::Op(o) => format!("'{o}'"),
            Tok::LParen => "'('".into(),
            Tok::RParen => "')'".into(),
            Tok::Comma => "','".into(),
        }
    }
    /// True when this is the bare keyword `kw` (case-insensitive).
    pub fn is_kw(&self, kw: &str) -> bool {
        matches!(self, Tok::Word(w) if w.eq_ignore_ascii_case(kw))
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '@' | '/' | ':' | '+' | '[' | ']')
}

pub fn lex(src: &str) -> Result<Vec<Tok>, JqlError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(&c) = chars.get(i) {
        let next = chars.get(i.saturating_add(1)).copied();
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '"' | '\'' => {
                let start = i;
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(JqlError::UnterminatedString(start)),
                        Some('\\') => {
                            if let Some(&e) = chars.get(i + 1) {
                                s.push(e);
                            }
                            i += 2;
                        }
                        Some(&q) if q == c => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Tok::Str(s));
            }
            '!' if next == Some('=') => {
                out.push(Tok::Op("!="));
                i += 2;
            }
            '!' if next == Some('~') => {
                out.push(Tok::Op("!~"));
                i += 2;
            }
            '>' | '<' if next == Some('=') => {
                out.push(Tok::Op(if c == '>' { ">=" } else { "<=" }));
                i += 2;
            }
            '>' => {
                out.push(Tok::Op(">"));
                i += 1;
            }
            '<' => {
                out.push(Tok::Op("<"));
                i += 1;
            }
            '=' => {
                out.push(Tok::Op("="));
                i += 1;
            }
            '~' => {
                out.push(Tok::Op("~"));
                i += 1;
            }
            c if is_word_char(c) => {
                let start = i;
                while chars.get(i).is_some_and(|&c| is_word_char(c)) {
                    i += 1;
                }
                out.push(Tok::Word(chars.get(start..i).unwrap_or_default().iter().collect()));
            }
            other => return Err(JqlError::UnexpectedChar(other, i)),
        }
    }
    Ok(out)
}
