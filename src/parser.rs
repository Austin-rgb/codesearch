//! Parser for the signature query language.
//!
//! ```text
//! query   := params "->" type
//! params  := "(" [ type { "," type } [ "," ] ] ")"     // parenthesised list (0..n)
//!          | type                                      // exactly one parameter, no parens
//! type    := "*"                                       // wildcard
//!          | "(" ")"                                   // unit
//!          | "(" type ")"                              // grouping
//!          | "(" type "," [ type { "," type } [ "," ] ] ")"   // tuple
//!          | "&" [ lifetime ] [ "mut" ] type
//!          | "[" type "]"  |  "[" type ";" ( number | ident | "*" ) "]"
//!          | ( "impl" | "dyn" ) bound { "+" bound }
//!          | path [ "<" arg { "," arg } [ "," ] ">" ]
//! path    := ident { "::" ident }
//! ```
//!
//! A query that starts with `(` is always read as a parameter list, so
//! `(A, B) -> C` has two parameters and `((A, B)) -> C` has one tuple parameter.
//! Lifetimes are accepted and ignored, like in the extractor.

use crate::types::{Query, TypeExpr};
use std::fmt;

/// A malformed query. `Display` renders the message with a caret under the problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    /// Byte offset into `input`.
    pub position: usize,
    pub input: String,
    pub hint: Option<String>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let column = match self.input.get(..self.position) {
            Some(prefix) => prefix.chars().count(),
            None => self.input.chars().count(),
        };
        writeln!(f, "invalid query: {}", self.message)?;
        writeln!(f, "  {}", self.input)?;
        write!(f, "  {}^", " ".repeat(column))?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  hint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

fn make_error(
    input: &str,
    position: usize,
    message: impl Into<String>,
    hint: Option<&str>,
) -> ParseError {
    ParseError {
        message: message.into(),
        position,
        input: input.to_string(),
        hint: hint.map(str::to_string),
    }
}

// ---------------------------------------------------------------- lexer

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Number(String),
    Lifetime,
    LParen,
    RParen,
    Lt,
    Gt,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Amp,
    Star,
    Plus,
    Arrow,
    PathSep,
    Eof,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Ident(s) | Tok::Number(s) => format!("`{s}`"),
            Tok::Lifetime => "a lifetime".to_string(),
            Tok::LParen => "`(`".to_string(),
            Tok::RParen => "`)`".to_string(),
            Tok::Lt => "`<`".to_string(),
            Tok::Gt => "`>`".to_string(),
            Tok::LBracket => "`[`".to_string(),
            Tok::RBracket => "`]`".to_string(),
            Tok::Comma => "`,`".to_string(),
            Tok::Semi => "`;`".to_string(),
            Tok::Amp => "`&`".to_string(),
            Tok::Star => "`*`".to_string(),
            Tok::Plus => "`+`".to_string(),
            Tok::Arrow => "`->`".to_string(),
            Tok::PathSep => "`::`".to_string(),
            Tok::Eof => "end of query".to_string(),
        }
    }
}

struct Token {
    tok: Tok,
    pos: usize,
}

fn lex(input: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some(&(pos, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let mut text = String::new();
            while let Some(&(_, ch)) = chars.peek() {
                if ch.is_alphanumeric() || ch == '_' {
                    text.push(ch);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token { tok: Tok::Ident(text), pos });
            continue;
        }

        if c.is_ascii_digit() {
            let mut text = String::new();
            while let Some(&(_, ch)) = chars.peek() {
                if ch.is_alphanumeric() || ch == '_' {
                    text.push(ch);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token { tok: Tok::Number(text), pos });
            continue;
        }

        chars.next();
        let tok = match c {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            ',' => Tok::Comma,
            ';' => Tok::Semi,
            '&' => Tok::Amp,
            '*' => Tok::Star,
            '+' => Tok::Plus,
            '-' => {
                if matches!(chars.peek(), Some(&(_, '>'))) {
                    chars.next();
                    Tok::Arrow
                } else {
                    return Err(make_error(
                        input,
                        pos,
                        "unexpected `-`; did you mean `->`?",
                        None,
                    ));
                }
            }
            ':' => {
                if matches!(chars.peek(), Some(&(_, ':'))) {
                    chars.next();
                    Tok::PathSep
                } else {
                    return Err(make_error(
                        input,
                        pos,
                        "unexpected `:`; paths use `::`",
                        None,
                    ));
                }
            }
            '\'' => {
                let mut named = false;
                while let Some(&(_, ch)) = chars.peek() {
                    if ch.is_alphanumeric() || ch == '_' {
                        named = true;
                        chars.next();
                    } else {
                        break;
                    }
                }
                if !named {
                    return Err(make_error(input, pos, "expected a lifetime name after `'`", None));
                }
                Tok::Lifetime
            }
            other => {
                return Err(make_error(
                    input,
                    pos,
                    format!("unexpected character `{other}`"),
                    None,
                ));
            }
        };
        tokens.push(Token { tok, pos });
    }

    tokens.push(Token {
        tok: Tok::Eof,
        pos: input.len(),
    });
    Ok(tokens)
}

// --------------------------------------------------------------- parser

struct Parser<'a> {
    input: &'a str,
    toks: Vec<Token>,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }

    fn pos(&self) -> usize {
        self.toks[self.i].pos
    }

    /// Consumes and returns the current token. Never moves past `Eof`.
    fn bump(&mut self) -> Tok {
        let tok = self.toks[self.i].tok.clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        tok
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == tok {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        let is_keyword = matches!(self.peek(), Tok::Ident(s) if s == keyword);
        if is_keyword {
            self.bump();
        }
        is_keyword
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        make_error(self.input, self.pos(), message, None)
    }

    fn error_with_hint(&self, message: impl Into<String>, hint: &str) -> ParseError {
        make_error(self.input, self.pos(), message, Some(hint))
    }

    fn expect(&mut self, tok: Tok, what: &str) -> Result<(), ParseError> {
        if self.eat(&tok) {
            Ok(())
        } else {
            Err(self.error(format!(
                "expected {what}, found {}",
                self.peek().describe()
            )))
        }
    }

    fn skip_lifetime(&mut self) {
        if *self.peek() == Tok::Lifetime {
            self.bump();
        }
    }

    fn parse_param_list(&mut self) -> Result<Vec<TypeExpr>, ParseError> {
        self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        loop {
            if self.eat(&Tok::RParen) {
                break;
            }
            params.push(self.parse_type()?);
            if self.eat(&Tok::Comma) {
                continue;
            }
            if self.eat(&Tok::RParen) {
                break;
            }
            return Err(self.error(format!(
                "expected `,` or `)`, found {}",
                self.peek().describe()
            )));
        }
        Ok(params)
    }

    fn parse_type(&mut self) -> Result<TypeExpr, ParseError> {
        match self.peek().clone() {
            Tok::Star => {
                self.bump();
                Ok(TypeExpr::Wildcard)
            }
            Tok::Amp => {
                self.bump();
                self.skip_lifetime();
                let mutable = self.eat_keyword("mut");
                let inner = self.parse_type()?;
                Ok(TypeExpr::Ref {
                    mutable,
                    inner: Box::new(inner),
                })
            }
            Tok::LParen => self.parse_paren_type(),
            Tok::LBracket => self.parse_bracket_type(),
            Tok::Ident(name) if name == "impl" || name == "dyn" => {
                self.bump();
                let bounds = self.parse_bounds()?;
                Ok(TypeExpr::TraitObject {
                    dyn_: name == "dyn",
                    bounds,
                })
            }
            Tok::Ident(_) => self.parse_path(),
            other => Err(self.error(format!("expected a type, found {}", other.describe()))),
        }
    }

    /// After `(`: unit, a grouped type, or a tuple.
    fn parse_paren_type(&mut self) -> Result<TypeExpr, ParseError> {
        self.bump(); // `(`
        if self.eat(&Tok::RParen) {
            return Ok(TypeExpr::Unit);
        }
        let first = self.parse_type()?;
        if self.eat(&Tok::RParen) {
            return Ok(first);
        }
        if *self.peek() != Tok::Comma {
            return Err(self.error(format!(
                "expected `,` or `)`, found {}",
                self.peek().describe()
            )));
        }
        let mut items = vec![first];
        while self.eat(&Tok::Comma) {
            if *self.peek() == Tok::RParen {
                break;
            }
            items.push(self.parse_type()?);
        }
        self.expect(Tok::RParen, "`)` to close the tuple")?;
        Ok(TypeExpr::Tuple(items))
    }

    /// After `[`: slice or array.
    fn parse_bracket_type(&mut self) -> Result<TypeExpr, ParseError> {
        self.bump(); // `[`
        let inner = self.parse_type()?;
        if self.eat(&Tok::Semi) {
            let len = match self.peek().clone() {
                Tok::Number(n) | Tok::Ident(n) => {
                    self.bump();
                    Some(n)
                }
                Tok::Star => {
                    self.bump();
                    None
                }
                other => {
                    return Err(self.error(format!(
                        "expected an array length, found {}",
                        other.describe()
                    )));
                }
            };
            self.expect(Tok::RBracket, "`]`")?;
            Ok(TypeExpr::Array(Box::new(inner), len))
        } else {
            self.expect(Tok::RBracket, "`]`")?;
            Ok(TypeExpr::Slice(Box::new(inner)))
        }
    }

    fn parse_bounds(&mut self) -> Result<Vec<TypeExpr>, ParseError> {
        let mut bounds = Vec::new();
        loop {
            if *self.peek() == Tok::Lifetime {
                self.bump();
            } else {
                bounds.push(self.parse_path()?);
            }
            if !self.eat(&Tok::Plus) {
                break;
            }
        }
        if bounds.is_empty() {
            return Err(self.error("expected a trait name"));
        }
        Ok(bounds)
    }

    fn parse_path(&mut self) -> Result<TypeExpr, ParseError> {
        let mut segments = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Ident(name) => {
                    self.bump();
                    segments.push(name);
                }
                other => {
                    return Err(self.error(format!(
                        "expected a type name, found {}",
                        other.describe()
                    )));
                }
            }
            if !self.eat(&Tok::PathSep) {
                break;
            }
        }

        let mut args = Vec::new();
        if self.eat(&Tok::Lt) {
            loop {
                if *self.peek() == Tok::Lifetime {
                    self.bump();
                } else {
                    args.push(self.parse_type()?);
                }
                if self.eat(&Tok::Comma) {
                    if *self.peek() == Tok::Gt {
                        break;
                    }
                    continue;
                }
                break;
            }
            self.expect(Tok::Gt, "`>` to close the generic arguments")?;
        }
        Ok(TypeExpr::Path { segments, args })
    }
}

/// Parses a signature query such as `(Username, Password) -> Result<Session, AuthError>`.
pub fn parse_query(input: &str) -> Result<Query, ParseError> {
    let toks = lex(input)?;
    let mut parser = Parser { input, toks, i: 0 };

    if *parser.peek() == Tok::Eof {
        return Err(make_error(
            input,
            0,
            "the query is empty",
            Some("write a signature such as \"(Username, Password) -> Session\""),
        ));
    }

    let params = if *parser.peek() == Tok::LParen {
        parser.parse_param_list()?
    } else {
        vec![parser.parse_type()?]
    };

    if *parser.peek() == Tok::Comma {
        return Err(parser.error_with_hint(
            "multiple parameters must be wrapped in parentheses",
            "write \"(A, B) -> C\" instead of \"A, B -> C\"",
        ));
    }

    if !parser.eat(&Tok::Arrow) {
        return Err(parser.error_with_hint(
            format!(
                "expected `->` after the parameter list, found {}",
                parser.peek().describe()
            ),
            "queries look like \"(ParamType, ...) -> ReturnType\"",
        ));
    }

    let ret = parser.parse_type()?;

    if *parser.peek() != Tok::Eof {
        return Err(parser.error(format!(
            "unexpected {} after the return type",
            parser.peek().describe()
        )));
    }

    Ok(Query { params, ret })
}
