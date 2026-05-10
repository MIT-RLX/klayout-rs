//! Liberty tokenizer.

use crate::{LibertyError, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// An identifier or unquoted value (alphanumeric + underscore + . / + -).
    Ident(String),
    /// A quoted string (no enclosing quotes).
    String(String),
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `,`
    Comma,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
}

pub struct Tokenizer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn next_token(&mut self) -> Result<Option<Token>> {
        loop {
            self.skip_whitespace_and_comments()?;
            if self.pos >= self.src.len() {
                return Ok(None);
            }
            let b = self.src[self.pos];
            match b {
                b'(' => {
                    self.pos += 1;
                    return Ok(Some(Token::LParen));
                }
                b')' => {
                    self.pos += 1;
                    return Ok(Some(Token::RParen));
                }
                b'{' => {
                    self.pos += 1;
                    return Ok(Some(Token::LBrace));
                }
                b'}' => {
                    self.pos += 1;
                    return Ok(Some(Token::RBrace));
                }
                b',' => {
                    self.pos += 1;
                    return Ok(Some(Token::Comma));
                }
                b':' => {
                    self.pos += 1;
                    return Ok(Some(Token::Colon));
                }
                b';' => {
                    self.pos += 1;
                    return Ok(Some(Token::Semicolon));
                }
                b'"' => {
                    return self.read_string().map(Some);
                }
                b'\\' => {
                    // Line continuation `\`+newline — skip whitespace then continue.
                    self.pos += 1;
                    continue;
                }
                _ => {
                    return self.read_ident().map(Some);
                }
            }
        }
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<()> {
        loop {
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos + 1 < self.src.len()
                && self.src[self.pos] == b'/'
                && self.src[self.pos + 1] == b'*'
            {
                self.pos += 2;
                while self.pos + 1 < self.src.len() {
                    if self.src[self.pos] == b'*' && self.src[self.pos + 1] == b'/' {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
                continue;
            }
            // Liberty has no `//` line comments per spec, but tools sometimes
            // emit them — tolerate.
            if self.pos + 1 < self.src.len()
                && self.src[self.pos] == b'/'
                && self.src[self.pos + 1] == b'/'
            {
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
        Ok(())
    }

    fn read_string(&mut self) -> Result<Token> {
        // Already at `"`.
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.src.len() && self.src[self.pos] != b'"' {
            // Allow embedded backslash-newline continuation.
            if self.src[self.pos] == b'\\' && self.pos + 1 < self.src.len() {
                self.pos += 2;
                continue;
            }
            self.pos += 1;
        }
        if self.pos >= self.src.len() {
            return Err(LibertyError::UnexpectedEof);
        }
        let s = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| LibertyError::InvalidUtf8)?
            .to_string();
        self.pos += 1; // skip closing quote
        Ok(Token::String(s))
    }

    fn read_ident(&mut self) -> Result<Token> {
        let start = self.pos;
        while self.pos < self.src.len() && is_ident_byte(self.src[self.pos]) {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(LibertyError::UnexpectedToken {
                pos: self.pos,
                msg: format!("char '{}'", self.src[self.pos] as char),
            });
        }
        let s = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| LibertyError::InvalidUtf8)?
            .to_string();
        Ok(Token::Ident(s))
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || b == b'_'
        || b == b'.'
        || b == b'-'
        || b == b'+'
        || b == b'/'
        || b == b'!'
        || b == b'['
        || b == b']'
        || b == b'\''
}
