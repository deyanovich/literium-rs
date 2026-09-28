//! Crate-wide error type. The Perl implementation signals errors
//! with `die`/`croak` messages caught at the CLI boundary; this
//! port carries the same messages in a string-shaped error so the
//! two CLIs stay behaviorally comparable.

use std::fmt;

#[derive(Debug, Clone)]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

/// `err!(...)` — the `die`/`croak` analogue: an `Err(Error)` with
/// a formatted message.
#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => {
        return Err($crate::error::Error::new(format!($($arg)*)))
    };
}
