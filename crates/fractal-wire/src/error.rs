//! Stable structured operation errors shared across runtime boundaries.

use std::fmt;

use serde_json::Value;

#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    pub details: serde_json::Map<String, Value>,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: serde_json::Map::new(),
        }
    }

    /// SQLite connection contention is not a malformed claim or projection failure.
    pub fn is_sqlite_contention(&self) -> bool {
        matches!(self.code, "database-busy" | "database-locked")
    }

    pub fn with_detail(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.details.insert(name.into(), value.into());
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
