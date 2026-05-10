use thiserror::Error;

pub type Result<T> = std::result::Result<T, LefError>;

#[derive(Debug, Error)]
pub enum LefError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid UTF-8")]
    InvalidUtf8,

    #[error("unterminated string starting on line {line}")]
    UnterminatedString { line: usize },

    #[error("unexpected token at line {line}: {got}")]
    UnexpectedToken { line: usize, got: String },

    #[error("expected {expected} at line {line}, got {got}")]
    Expected {
        line: usize,
        expected: &'static str,
        got: String,
    },

    #[error("unexpected end of input")]
    UnexpectedEof,

    #[error("missing required field {0}")]
    MissingField(&'static str),

    #[error("unknown macro '{0}' referenced in DEF")]
    UnknownMacro(String),

    #[error("layer '{0}' not in library")]
    UnknownLayer(String),
}
