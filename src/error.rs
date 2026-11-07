use std::fmt;

/// An error with the Ruby class name `qm.log` records for it.
#[derive(Debug)]
pub struct QmError {
    pub class: &'static str,
    pub message: String,
}

/// The class of the error a write raises, before touching anything, when only Ruby can write the value faithfully.
pub const UNEMITTABLE: &str = "Unemittable";

pub type Res<T> = Result<T, QmError>;

impl QmError {
    pub fn new(class: &'static str, message: impl Into<String>) -> Self {
        QmError { class, message: message.into() }
    }

    pub fn argument(message: impl Into<String>) -> Self {
        Self::new("ArgumentError", message)
    }
}

impl fmt::Display for QmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<std::io::Error> for QmError {
    fn from(e: std::io::Error) -> Self {
        QmError::new("IOError", e.to_string())
    }
}

impl From<serde_json::Error> for QmError {
    fn from(e: serde_json::Error) -> Self {
        QmError::new("JSON::ParserError", e.to_string())
    }
}

impl From<serde_yaml::Error> for QmError {
    fn from(e: serde_yaml::Error) -> Self {
        QmError::new("Psych::SyntaxError", e.to_string())
    }
}
