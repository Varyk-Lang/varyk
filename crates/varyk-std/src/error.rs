use std::fmt;

/// A message. Every failing `varyk-std` call returns one.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    message: String,
}

impl Error {
    pub fn new(message: String) -> Error {
        Error { message }
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
}
