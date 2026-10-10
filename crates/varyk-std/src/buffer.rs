//! `Bytes` (spec 2.3): an immutable run of bytes, base64 in JSON. Every
//! one `varyk-std` makes is in the `bytes` crate's shared form
//! (`bytes::Bytes::from_owner`), so a clone only adds to a reference
//! count and never allocates.

use crate::Error;
use base64::Engine;
use base64::display::Base64Display;
use base64::engine::general_purpose::STANDARD;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;

const NOT_BASE64: &str = "the text is not standard base64 with padding";

/// An immutable run of bytes; `.clone()` copies the handle to a shared
/// buffer, not the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Bytes(bytes::Bytes);

impl Bytes {
    /// The UTF-8 bytes of `text`, a copy.
    pub fn from_text(text: &str) -> Bytes {
        Bytes::owning(text.as_bytes().to_vec())
    }

    /// Reads standard base64 with padding (RFC 4648 section 4), strictly:
    /// another alphabet, missing padding, or a stray character is an error.
    pub fn from_base64(text: &str) -> Result<Bytes, Error> {
        match STANDARD.decode(text) {
            Ok(bytes) => Ok(Bytes::owning(bytes)),
            Err(_) => Err(Error::new(NOT_BASE64.to_string())),
        }
    }

    /// The bytes as text, a copy; an error when they are not UTF-8.
    pub fn to_text(&self) -> Result<String, Error> {
        match std::str::from_utf8(&self.0) {
            Ok(text) => Ok(text.to_string()),
            Err(e) => Err(Error::new(format!(
                "the bytes are not UTF-8 text: byte {} starts no character",
                e.valid_up_to()
            ))),
        }
    }

    /// Standard base64 with padding.
    pub fn to_base64(&self) -> String {
        STANDARD.encode(&self.0)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn owning(bytes: Vec<u8>) -> Bytes {
        Bytes(bytes::Bytes::from_owner(bytes))
    }
}

/// Wraps the handle, without copying the bytes, so a clone only adds to a
/// reference count whichever way the buffer was made.
impl From<bytes::Bytes> for Bytes {
    fn from(bytes: bytes::Bytes) -> Bytes {
        Bytes(bytes::Bytes::from_owner(bytes))
    }
}

impl From<Bytes> for bytes::Bytes {
    fn from(bytes: Bytes) -> bytes::Bytes {
        bytes.0
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Base64, as JSON writes it (spec 5).
impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&Base64Display::new(&self.0, &STANDARD), f)
    }
}

/// A string of standard base64 with padding; to a serializer that is not
/// human-readable, the raw bytes.
impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(&Base64Display::new(&self.0, &STANDARD))
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}

/// From a base64 string, or from raw bytes, as a database package hands
/// over a binary column; JSON never gives raw bytes for a string.
impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Bytes, D::Error> {
        deserializer.deserialize_str(BytesVisitor)
    }
}

struct BytesVisitor;

impl Visitor<'_> for BytesVisitor {
    type Value = Bytes;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("standard base64 with padding")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Bytes, E> {
        Bytes::from_base64(text).map_err(E::custom)
    }

    fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Bytes, E> {
        Ok(Bytes::owning(bytes.to_vec()))
    }

    /// Takes the buffer without copying it.
    fn visit_byte_buf<E: de::Error>(self, bytes: Vec<u8>) -> Result<Bytes, E> {
        Ok(Bytes::owning(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::{BorrowedBytesDeserializer, BytesDeserializer, Error as ValueError};

    fn base64(text: &str) -> Result<Vec<u8>, String> {
        Bytes::from_base64(text)
            .map(|b| b.as_ref().to_vec())
            .map_err(|e| e.to_string())
    }

    #[test]
    fn base64_is_standard_with_padding_both_ways() {
        assert_eq!(Bytes::from_text("hi?>").to_base64(), "aGk/Pg==");
        assert_eq!(base64("aGk/Pg=="), Ok(b"hi?>".to_vec()));
        assert_eq!(base64(""), Ok(Vec::new()));
        assert_eq!(Bytes::from_text("").to_base64(), "");
    }

    #[test]
    fn missing_padding_the_url_alphabet_and_a_stray_character_are_refused() {
        for text in ["aGk/Pg", "aGk_Pg==", "aGk/P g==", "aGk/Pg==!", "aGk/Ph=="] {
            assert_eq!(base64(text), Err(NOT_BASE64.to_string()), "{text}");
        }
    }

    #[test]
    fn to_text_refuses_bytes_that_are_not_utf_8() {
        assert_eq!(Bytes::from_text("héllo").to_text(), Ok("héllo".to_string()));
        let raw = Bytes::from(bytes::Bytes::from_static(b"ok\xff"));
        assert_eq!(
            raw.to_text().map_err(|e| e.to_string()),
            Err("the bytes are not UTF-8 text: byte 2 starts no character".to_string())
        );
    }

    #[test]
    fn len_and_is_empty_count_bytes() {
        assert_eq!(Bytes::from_text("héllo").len(), 6);
        assert!(!Bytes::from_text("a").is_empty());
        assert!(Bytes::from_text("").is_empty());
    }

    #[test]
    fn a_clone_shares_the_buffer() {
        let b = Bytes::from_text("shared");
        let c = b.clone();
        assert_eq!(b, c);
        assert_eq!(b.as_ref().as_ptr(), c.as_ref().as_ptr());
        let decoded = Bytes::from_base64("c2hhcmVk");
        let pointers = decoded
            .as_ref()
            .map(|d| (d.as_ref().as_ptr(), d.clone().as_ref().as_ptr()));
        assert!(matches!(pointers, Ok((a, b)) if a == b));
    }

    #[test]
    fn a_handed_over_buffer_is_not_copied() {
        let handed = bytes::Bytes::from(b"body".to_vec());
        let at = handed.as_ptr();
        let b = Bytes::from(handed);
        assert_eq!(b.as_ref().as_ptr(), at);
        assert_eq!(b.clone().as_ref().as_ptr(), at);
        assert_eq!(bytes::Bytes::from(b).as_ptr(), at);
    }

    #[test]
    fn debug_writes_base64() {
        assert_eq!(format!("{:?}", Bytes::from_text("hi")), "aGk=");
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Row {
        data: Bytes,
    }

    #[test]
    fn json_writes_and_reads_base64() {
        let row = Row {
            data: Bytes::from_text("hi?>"),
        };
        assert_eq!(crate::json::stringify(&row), r#"{"data":"aGk/Pg=="}"#);
        assert_eq!(crate::json::parse::<Row>(r#"{"data":"aGk/Pg=="}"#), Ok(row));
        match crate::json::parse::<Row>(r#"{"data":"aGk_Pg=="}"#) {
            Err(e) => assert!(e.message().contains(NOT_BASE64), "{e}"),
            Ok(row) => panic!("read {row:?}"),
        }
        assert!(crate::json::parse::<Row>(r#"{"data":[104,105]}"#).is_err());
    }

    #[test]
    fn raw_bytes_read_as_they_are() {
        let read = Bytes::deserialize(BytesDeserializer::<ValueError>::new(b"\x00\xff"));
        assert_eq!(read.map(|b| b.as_ref().to_vec()), Ok(vec![0, 0xff]));
        let read = Bytes::deserialize(BorrowedBytesDeserializer::<ValueError>::new(b"ab"));
        assert_eq!(read.map(|b| b.as_ref().to_vec()), Ok(b"ab".to_vec()));
        let buffer = vec![1, 2, 3];
        let at = buffer.as_ptr();
        let owned = BytesVisitor.visit_byte_buf::<ValueError>(buffer);
        assert_eq!(owned.map(|b| b.as_ref().as_ptr()), Ok(at));
    }

    #[test]
    fn serializes_as_base64_when_readable_and_in_json() {
        use serde_test::{Configure, Token, assert_ser_tokens};
        assert_eq!(
            crate::json::stringify(&Bytes::from_text("hi?>")),
            "\"aGk/Pg==\""
        );
        assert_ser_tokens(
            &Bytes::from_text("hi?>").readable(),
            &[Token::Str("aGk/Pg==")],
        );
    }

    #[test]
    fn serializes_as_raw_bytes_when_compact() {
        use serde_test::{Configure, Token, assert_ser_tokens};
        assert_ser_tokens(
            &Bytes::from_text("hi?>").compact(),
            &[Token::Bytes(b"hi?>")],
        );
    }
}
