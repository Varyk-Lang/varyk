//! `Uuid` (spec 2.2): a 128-bit identifier. The `uuid` crate makes and
//! reads them; it never appears in the public API.

use crate::{Error, Time};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;
use uuid::{ContextV7, Timestamp};

/// Orders the version 7 ids of one process.
static CONTEXT: Mutex<ContextV7> = Mutex::new(ContextV7::new());

const EXAMPLE: &str = "a Uuid like 01890a5d-ac96-774b-bcce-b302099a8057";

/// A 128-bit identifier, written in the lowercase hyphenated form.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uuid(uuid::Uuid);

impl Uuid {
    /// A new version 7 id: time-ordered, ordered by creation within one
    /// process. It shows when it was made to anyone who reads it;
    /// `Uuid::v4()` does not. Panics only when the operating system cannot
    /// give random bytes.
    // A default id would hide a random one behind `Default`.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Uuid {
        Uuid::v7()
    }

    /// The same as `Uuid::new()`.
    pub fn v7() -> Uuid {
        Uuid::v7_at(Time::now())
    }

    /// A new random version 4 id. Panics only when the operating system
    /// cannot give random bytes.
    pub fn v4() -> Uuid {
        Uuid(uuid::Uuid::new_v4())
    }

    /// A version 7 id made at `reading`, a time before 1970 taken as 0,
    /// since the id holds unsigned milliseconds since 1970. Never `uuid`'s
    /// `now_v7`, which panics on a clock before 1970.
    fn v7_at(reading: Time) -> Uuid {
        let micros = u64::try_from(reading.to_unix_micros()).unwrap_or(0);
        let seconds = micros / 1_000_000;
        let nanos = u32::try_from(micros % 1_000_000 * 1000).unwrap_or(0);
        Uuid(uuid::Uuid::new_v7(Timestamp::from_unix(
            &CONTEXT, seconds, nanos,
        )))
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Uuid {
        Uuid(uuid::Uuid::from_bytes(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }
}

/// Only the 36-character hyphenated form, hex digits in either case; the
/// `uuid` crate also reads the simple, braced, and URN forms.
impl FromStr for Uuid {
    type Err = Error;

    fn from_str(text: &str) -> Result<Uuid, Error> {
        let not_an_id = || Error::new(format!("`{text}` is not {EXAMPLE}"));
        if text.len() != 36 {
            return Err(not_an_id());
        }
        match uuid::Uuid::parse_str(text) {
            Ok(id) => Ok(Uuid(id)),
            Err(_) => Err(not_an_id()),
        }
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.hyphenated(), f)
    }
}

/// The written form, as `{}` prints it (spec 5).
impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The written form, as a string.
impl Serialize for Uuid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// From a string, as `parse` reads it, or from exactly 16 raw bytes, as a
/// database package hands over a binary column.
impl<'de> Deserialize<'de> for Uuid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
        deserializer.deserialize_str(UuidVisitor)
    }
}

struct UuidVisitor;

impl Visitor<'_> for UuidVisitor {
    type Value = Uuid;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(EXAMPLE)
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Uuid, E> {
        text.parse().map_err(E::custom)
    }

    // `visit_byte_buf` comes here too, serde's default.
    fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Uuid, E> {
        match <[u8; 16]>::try_from(bytes) {
            Ok(bytes) => Ok(Uuid::from_bytes(bytes)),
            Err(_) => Err(E::invalid_length(bytes.len(), &"16 bytes")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::{BorrowedBytesDeserializer, BytesDeserializer, Error as ValueError};

    const ID: &str = "01890a5d-ac96-774b-bcce-b302099a8057";

    fn version(id: Uuid) -> u8 {
        id.as_bytes()[6] >> 4
    }

    fn read(text: &str) -> String {
        match text.parse::<Uuid>() {
            Ok(id) => id.to_string(),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn new_and_v7_make_version_7_and_v4_version_4() {
        assert_eq!(version(Uuid::new()), 7);
        assert_eq!(version(Uuid::v7()), 7);
        assert_eq!(version(Uuid::v4()), 4);
        assert_ne!(Uuid::v4(), Uuid::v4());
    }

    #[test]
    fn version_7_ids_made_in_a_row_are_ordered() {
        let ids: Vec<[u8; 16]> = (0..1000).map(|_| *Uuid::new().as_bytes()).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn ids_from_a_clock_before_1970_are_ordered() {
        let ordered = Time::from_unix(-86_400).map(|before| {
            let first = *Uuid::v7_at(before).as_bytes();
            let second = *Uuid::v7_at(before).as_bytes();
            first < second
        });
        assert_eq!(ordered, Ok(true));
    }

    #[test]
    fn only_the_hyphenated_form_reads_in_either_case() {
        assert_eq!(read(ID), ID);
        assert_eq!(read(&ID.to_uppercase()), ID);
        for text in [
            "01890a5dac96774bbcceb302099a8057",
            "{01890a5d-ac96-774b-bcce-b302099a8057}",
            "urn:uuid:01890a5d-ac96-774b-bcce-b302099a8057",
            "01890a5d-ac96-774b-bcce-b302099a805g",
            "01890a5d-ac96-774b-bcce-b302099a80577",
            "01890a5dxac96-774b-bcce-b302099a8057",
            "",
        ] {
            assert_eq!(
                read(text),
                format!("`{text}` is not a Uuid like {ID}"),
                "{text}"
            );
        }
        assert_eq!(
            crate::parse::<Uuid>(ID).map(|id| id.to_string()),
            Ok(ID.to_string())
        );
    }

    #[test]
    fn written_lowercase_hyphenated_by_display_and_debug() {
        let id = Uuid::from_bytes([0xAB; 16]);
        assert_eq!(id.to_string(), "abababab-abab-abab-abab-abababababab");
        assert_eq!(format!("{id:?}"), "abababab-abab-abab-abab-abababababab");
        assert_eq!(id.as_bytes(), &[0xAB; 16]);
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Row {
        id: Uuid,
    }

    #[test]
    fn json_writes_and_reads_the_written_form() {
        let row = ID.parse().map(|id| Row { id });
        let text = format!(r#"{{"id":"{ID}"}}"#);
        assert_eq!(row.as_ref().map(crate::json::stringify), Ok(text.clone()));
        assert_eq!(crate::json::parse::<Row>(&text), row);
        assert!(crate::json::parse::<Row>(r#"{"id":"nope"}"#).is_err());
        assert!(crate::json::parse::<Row>(r#"{"id":7}"#).is_err());
    }

    #[test]
    fn sixteen_raw_bytes_read_and_fifteen_do_not() {
        let raw: [u8; 16] = *Uuid::v4().as_bytes();
        let read = |bytes: &[u8]| {
            Uuid::deserialize(BytesDeserializer::<ValueError>::new(bytes))
                .map(|id| *id.as_bytes())
                .map_err(|e| e.to_string())
        };
        assert_eq!(read(&raw), Ok(raw));
        assert!(read(&raw[..15]).is_err());
        let borrowed = Uuid::deserialize(BorrowedBytesDeserializer::<ValueError>::new(&raw));
        assert_eq!(borrowed.map(|id| *id.as_bytes()), Ok(raw));
        let owned = UuidVisitor.visit_byte_buf::<ValueError>(raw.to_vec());
        assert_eq!(owned.map(|id| *id.as_bytes()), Ok(raw));
        assert!(
            UuidVisitor
                .visit_byte_buf::<ValueError>(vec![0; 17])
                .is_err()
        );
    }
}
