use std::fmt;

/// A message, and optionally an HTTP status. Every failing `varyk-std`
/// call returns one.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    message: String,
    status: Option<u16>,
}

impl Error {
    /// An error with no status.
    pub fn new(message: String) -> Error {
        Error {
            message,
            status: None,
        }
    }

    /// An error with an HTTP status. A status means the message was
    /// written for the client: an HTTP server sends an error with a status
    /// from 400 to 599 as that status, with the message as the body of its
    /// response, and sends any other status as a 500 with a fixed body.
    /// Only Varyk code sets one, with `Error::with_status` or the
    /// constructors of `varyk-http` over it; no official package's Rust
    /// calls this, and Rust that does must never give it text that came
    /// from a database, a file, or any other failure the client should not
    /// see.
    pub fn with_status(status: u16, message: String) -> Error {
        Error {
            message,
            status: Some(status),
        }
    }

    /// The status, when one was set.
    pub fn status(&self) -> Option<u16> {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_the_message() {
        let e = Error::new("boom".to_string());
        assert_eq!(e.to_string(), "boom");
        assert_eq!(e.message(), "boom");
    }

    #[test]
    fn equality_compares_messages() {
        assert_eq!(Error::new("a".into()), Error::new("a".into()));
        assert_ne!(Error::new("a".into()), Error::new("b".into()));
        assert_eq!(Error::new("a".into()).clone(), Error::new("a".into()));
    }

    #[test]
    fn new_has_no_status() {
        assert_eq!(Error::new("a".into()).status(), None);
    }

    #[test]
    fn with_status_keeps_the_status_and_the_message() {
        let e = Error::with_status(404, "no such user".into());
        assert_eq!(e.status(), Some(404));
        assert_eq!(e.message(), "no such user");
    }

    #[test]
    fn clone_and_equality_include_the_status() {
        let e = Error::with_status(400, "a".into());
        assert_eq!(e.clone(), e);
        assert_eq!(e.clone().status(), Some(400));
        assert_ne!(e, Error::new("a".into()));
        assert_ne!(e, Error::with_status(401, "a".into()));
    }

    #[test]
    fn display_is_the_message_alone_with_a_status() {
        let e = Error::with_status(404, "gone".into());
        assert_eq!(e.to_string(), "gone");
    }
}
