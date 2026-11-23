use std::fmt;

/// One error type for the whole core: a message meant to be shown to the user.
#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// `err!("No layer with id \"{id}\"")` builds an `Error`.
#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => { $crate::Error(format!($($arg)*)) };
}
