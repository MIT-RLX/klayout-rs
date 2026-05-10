//! Tokenizer for LEF/DEF text files.
//!
//! Both formats share the same lexical structure:
//! * Whitespace-separated tokens
//! * `;` terminates statements
//! * `(` `)` delimit coordinate pairs
//! * `+` `-` are statement-level prefixes (DEF) or part of names
//! * `# ...` to end-of-line are comments
//! * Strings in double quotes are single tokens
//!
//! Numbers are recognized as a special case (decimal with optional sign
//! and exponent) but everything else is a `Word`.

use crate::error::{LefError, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Word(String),
    Number(f64),
    String(String),
    Semicolon,
    OpenParen,
    CloseParen,
    Plus,
    Minus,
}

pub struct Tokenizer<'a> {
    src: &'a [u8],
    pos: usize,
    pub line: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self {
            src,
            pos: 0,
            line: 1,
        }
    }

    pub fn next_token(&mut self) -> Result<Option<Token>> {
        self.skip_whitespace_and_comments();
        if self.pos >= self.src.len() {
            return Ok(None);
        }
        let c = self.src[self.pos];
        Ok(Some(match c {
                b';' => {
                    self.pos += 1;
                    Token::Semicolon
                }
                b'(' => {
                    self.pos += 1;
                    Token::OpenParen
                }
                b')' => {
                    self.pos += 1;
                    Token::CloseParen
                }
                b'"' => Token::String(self.read_string()?),
                b'+' if !self.next_starts_number() => {
                    self.pos += 1;
                    Token::Plus
                }
                b'-' if !self.next_starts_number() => {
                    self.pos += 1;
                    Token::Minus
                }
            _ => self.read_word_or_number()?,
        }))
    }

    fn skip_whitespace_and_comments(&mut self) {
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c == b'\n' {
                self.line += 1;
                self.pos += 1;
            } else if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if c == b'#' {
                // line comment
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn next_starts_number(&self) -> bool {
        let p = self.pos + 1;
        if p >= self.src.len() {
            return false;
        }
        matches!(self.src[p], b'0'..=b'9' | b'.')
    }

    fn read_string(&mut self) -> Result<String> {
        // pos is at the opening quote
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.src.len() && self.src[self.pos] != b'"' {
            if self.src[self.pos] == b'\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
        if self.pos >= self.src.len() {
            return Err(LefError::UnterminatedString { line: self.line });
        }
        let s = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| LefError::InvalidUtf8)?
            .to_string();
        self.pos += 1; // consume closing quote
        Ok(s)
    }

    fn read_word_or_number(&mut self) -> Result<Token> {
        let start = self.pos;
        // Numbers: optional sign + digits with optional . and e
        let first = self.src[self.pos];
        let is_numeric_start = matches!(first, b'0'..=b'9' | b'.')
            || ((first == b'+' || first == b'-') && self.next_starts_number());
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c.is_ascii_whitespace() || c == b';' || c == b'(' || c == b')' || c == b'#' {
                break;
            }
            self.pos += 1;
        }
        let word = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| LefError::InvalidUtf8)?
            .to_string();
        if is_numeric_start {
            if let Ok(n) = word.parse::<f64>() {
                return Ok(Token::Number(n));
            }
        }
        Ok(Token::Word(word))
    }
}

/// Convenience: drain all tokens from `src` into a Vec.
pub fn tokenize(src: &[u8]) -> Result<Vec<Token>> {
    let mut tk = Tokenizer::new(src);
    let mut out = Vec::new();
    while let Some(t) = tk.next_token()? {
        out.push(t);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_statement() {
        let toks = tokenize(b"VERSION 5.7 ;").unwrap();
        assert_eq!(toks.len(), 3);
        assert!(matches!(&toks[0], Token::Word(s) if s == "VERSION"));
        assert!(matches!(&toks[1], Token::Number(n) if (*n - 5.7).abs() < 1e-9));
        assert_eq!(toks[2], Token::Semicolon);
    }

    #[test]
    fn coordinates() {
        let toks = tokenize(b"( 100 200 ) ( 300 400 )").unwrap();
        assert_eq!(toks.len(), 8);
        assert_eq!(toks[0], Token::OpenParen);
        assert!(matches!(&toks[1], Token::Number(n) if *n == 100.0));
    }

    #[test]
    fn quoted_string() {
        let toks = tokenize(b"BUSBITCHARS \"[]\" ;").unwrap();
        assert_eq!(toks.len(), 3);
        assert!(matches!(&toks[1], Token::String(s) if s == "[]"));
    }

    #[test]
    fn comment_skipped() {
        let toks = tokenize(b"# this is a comment\nVERSION 5.7 ;").unwrap();
        assert_eq!(toks.len(), 3);
    }

    #[test]
    fn def_component_dash() {
        let toks = tokenize(b"- u1 INV + PLACED").unwrap();
        assert_eq!(toks.len(), 5);
        assert_eq!(toks[0], Token::Minus);
        assert!(matches!(&toks[1], Token::Word(s) if s == "u1"));
        assert_eq!(toks[3], Token::Plus);
    }

    #[test]
    fn negative_number_in_paren() {
        let toks = tokenize(b"( -100 200 )").unwrap();
        assert_eq!(toks.len(), 4);
        assert!(matches!(&toks[1], Token::Number(n) if *n == -100.0));
    }
}
