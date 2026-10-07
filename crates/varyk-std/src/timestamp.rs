//! `Time` (spec 2.1): one point in time in UTC, to the microsecond, from
//! the year 0000 to the year 9999. The `time` crate reads and writes
//! RFC 3339; it never appears in the public API.

use crate::Error;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Microseconds since 1970-01-01T00:00:00Z of 0000-01-01T00:00:00Z.
const FIRST: i64 = -62_167_219_200_000_000;
/// Microseconds since 1970-01-01T00:00:00Z of 9999-12-31T23:59:59.999999Z.
const LAST: i64 = 253_402_300_799_999_999;
const MICROS: i64 = 1_000_000;
const RANGE: &str = "is out of range for a time, which runs from the year 0000 to the year 9999";
const EXAMPLE: &str = "a time like 2026-10-07T12:00:00Z";

/// A point in time in UTC, held as microseconds since
/// 1970-01-01T00:00:00Z, always inside the range.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Time(i64);

impl Time {
    /// The current time, cut to the microsecond. A clock set before 1970
    /// gives a negative time; one outside the range gives its nearest end.
    pub fn now() -> Time {
        Time::at(SystemTime::now())
    }

    fn at(reading: SystemTime) -> Time {
        let micros = match reading.duration_since(UNIX_EPOCH) {
            Ok(after) => i128::try_from(after.as_micros()).unwrap_or(i128::MAX),
            // Cut down, as `from_iso` cuts a fraction: one nanosecond
            // before 1970 is in the microsecond before it.
            Err(before) => {
                let nanos = i128::try_from(before.duration().as_nanos()).unwrap_or(i128::MAX);
                -(nanos.saturating_add(999) / 1000)
            }
        };
        Time(
            match i64::try_from(micros.clamp(i128::from(FIRST), i128::from(LAST))) {
                Ok(micros) => micros,
                Err(_) => LAST,
            },
        )
    }

    /// Reads RFC 3339, converting an offset to UTC and cutting digits
    /// past the sixth (spec 2.1).
    pub fn from_iso(text: &str) -> Result<Time, Error> {
        let not_a_time = || Error::new(format!("`{text}` is not {EXAMPLE}"));
        if !shaped(text.as_bytes()) {
            return Err(not_a_time());
        }
        let read = match OffsetDateTime::parse(text, &Rfc3339) {
            Ok(read) => read,
            Err(_) => return Err(not_a_time()),
        };
        let micros = read.unix_timestamp_nanos().div_euclid(1000);
        match i64::try_from(micros) {
            Ok(micros) if (FIRST..=LAST).contains(&micros) => Ok(Time(micros)),
            _ => Err(Error::new(format!("`{text}` {RANGE}"))),
        }
    }

    /// Seconds since 1970-01-01T00:00:00Z.
    pub fn from_unix(seconds: i64) -> Result<Time, Error> {
        match seconds.checked_mul(MICROS) {
            Some(micros) if (FIRST..=LAST).contains(&micros) => Ok(Time(micros)),
            _ => Err(Error::new(format!(
                "`{seconds}` seconds since 1970 {RANGE}"
            ))),
        }
    }

    /// Microseconds since 1970-01-01T00:00:00Z.
    pub fn from_unix_micros(micros: i64) -> Result<Time, Error> {
        if (FIRST..=LAST).contains(&micros) {
            Ok(Time(micros))
        } else {
            Err(Error::new(format!(
                "`{micros}` microseconds since 1970 {RANGE}"
            )))
        }
    }

    /// The written form, UTC with `Z`, as `{}` prints it.
    pub fn to_iso(self) -> String {
        self.to_string()
    }

    /// Seconds since 1970, rounded down.
    pub fn to_unix(self) -> i64 {
        self.0.div_euclid(MICROS)
    }

    pub fn to_unix_micros(self) -> i64 {
        self.0
    }

    /// `seconds` later, or earlier when negative.
    pub fn add_seconds(self, seconds: i64) -> Result<Time, Error> {
        match seconds
            .checked_mul(MICROS)
            .and_then(|micros| self.0.checked_add(micros))
        {
            Some(micros) if (FIRST..=LAST).contains(&micros) => Ok(Time(micros)),
            _ => Err(Error::new(format!(
                "{self} plus `{seconds}` seconds {RANGE}"
            ))),
        }
    }

    /// `self` minus `earlier` in whole seconds, rounded toward zero. Inside
    /// the range the difference cannot overflow.
    pub fn seconds_since(self, earlier: Time) -> i64 {
        (self.0 - earlier.0) / MICROS
    }
}

/// Spec 2.1's shape, which the `time` crate's parser is looser about:
/// `YYYY-MM-DD`, `T` or `t`, `HH:MM:SS` with no leap second, one to nine
/// fraction digits, and `Z`, `z`, or `+HH:MM` or `-HH:MM`.
fn shaped(text: &[u8]) -> bool {
    let at = |i: usize, want: &[u8]| text.get(i).is_some_and(|c| want.contains(c));
    let digit = |i: usize| text.get(i).is_some_and(u8::is_ascii_digit);
    let fixed = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 18]
        .into_iter()
        .all(digit)
        && at(4, b"-")
        && at(7, b"-")
        && at(10, b"Tt")
        && at(13, b":")
        && at(16, b":")
        && at(17, b"012345");
    if !fixed {
        return false;
    }
    let mut rest = text.get(19..).unwrap_or_default();
    if let Some(fraction) = rest.strip_prefix(b".") {
        let digits = fraction.iter().take_while(|c| c.is_ascii_digit()).count();
        if !(1..=9).contains(&digits) {
            return false;
        }
        rest = fraction.get(digits..).unwrap_or_default();
    }
    match rest {
        [b'Z' | b'z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => [h1, h2, m1, m2].into_iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let written = OffsetDateTime::from_unix_timestamp_nanos(i128::from(self.0) * 1000)
            .map_err(|e| e.to_string())
            .and_then(|t| t.format(&Rfc3339).map_err(|e| e.to_string()));
        match written {
            Ok(text) => f.write_str(&text),
            // Unreachable: every `Time` is inside the range RFC 3339
            // writes. Write the error rather than fail, which would panic
            // in `to_string`.
            Err(e) => f.write_str(&e),
        }
    }
}

/// The written form, as `{}` prints it (spec 5).
impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// As `from_iso`.
impl FromStr for Time {
    type Err = Error;

    fn from_str(text: &str) -> Result<Time, Error> {
        Time::from_iso(text)
    }
}

/// The written form, as a string.
impl Serialize for Time {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// From a string only, as `from_iso` reads it: a number is not a time.
impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Time, D::Error> {
        deserializer.deserialize_str(TimeVisitor)
    }
}

struct TimeVisitor;

impl Visitor<'_> for TimeVisitor {
    type Value = Time;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(EXAMPLE)
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Time, E> {
        Time::from_iso(text).map_err(E::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// The written form of what `from_iso` reads, or its error.
    fn iso(text: &str) -> String {
        match Time::from_iso(text) {
            Ok(t) => t.to_iso(),
            Err(e) => e.to_string(),
        }
    }

    fn shown(time: Result<Time, Error>) -> String {
        match time {
            Ok(t) => t.to_string(),
            Err(e) => e.to_string(),
        }
    }

    fn not_a_time(text: &str) -> String {
        format!("`{text}` is not a time like 2026-10-07T12:00:00Z")
    }

    const FIRST: i64 = -62_167_219_200_000_000;
    const LAST: i64 = 253_402_300_799_999_999;

    #[test]
    fn an_offset_is_converted_to_utc() {
        assert_eq!(iso("2026-10-07T14:30:00+02:30"), "2026-10-07T12:00:00Z");
        assert_eq!(iso("2026-10-07T23:00:00-01:00"), "2026-10-08T00:00:00Z");
        assert_eq!(
            Time::from_iso("2026-10-07T14:00:00+02:00"),
            Time::from_iso("2026-10-07T12:00:00Z")
        );
    }

    #[test]
    fn digits_past_the_sixth_are_cut() {
        assert_eq!(
            iso("2026-10-07T12:00:00.123456789Z"),
            "2026-10-07T12:00:00.123456Z"
        );
        assert_eq!(
            iso("1969-12-31T23:59:59.9999999Z"),
            "1969-12-31T23:59:59.999999Z"
        );
        assert_eq!(iso("2026-10-07T12:00:00.5Z"), "2026-10-07T12:00:00.5Z");
    }

    #[test]
    fn a_lowercase_t_and_z_read() {
        assert_eq!(iso("2026-10-07t12:00:00z"), "2026-10-07T12:00:00Z");
    }

    #[test]
    fn the_range_runs_from_the_year_0000_to_the_year_9999() {
        assert_eq!(
            Time::from_iso("0000-01-01T00:00:00Z").map(Time::to_unix_micros),
            Ok(FIRST)
        );
        assert_eq!(
            Time::from_iso("9999-12-31T23:59:59.999999Z").map(Time::to_unix_micros),
            Ok(LAST)
        );
    }

    #[test]
    fn an_offset_that_leaves_the_range_is_an_error_naming_the_text() {
        for text in ["9999-12-31T23:30:00-01:00", "0000-01-01T00:30:00+01:00"] {
            assert_eq!(
                iso(text),
                format!(
                    "`{text}` is out of range for a time, which runs from the year 0000 to the year 9999"
                )
            );
        }
        assert_eq!(iso("9999-12-31T23:30:00Z"), "9999-12-31T23:30:00Z");
        assert_eq!(iso("0000-01-01T00:30:00Z"), "0000-01-01T00:30:00Z");
        assert_eq!(iso("9999-12-31T22:30:00-01:00"), "9999-12-31T23:30:00Z");
        assert_eq!(iso("0000-01-01T01:30:00+01:00"), "0000-01-01T00:30:00Z");
    }

    #[test]
    fn other_forms_are_refused_with_the_example() {
        assert_eq!(
            iso("2026-10-07 12:00"),
            "`2026-10-07 12:00` is not a time like 2026-10-07T12:00:00Z"
        );
        for text in [
            "2016-12-31T23:59:60Z",
            "2026-10-07 12:00:00Z",
            "2026-10-07T12:00:00.1234567891Z",
            "20261007T120000Z",
            "2026-W41-3T12:00:00Z",
            "2026-280T12:00:00Z",
            "2026-10-07T12:00:00",
            "2026-10-07T12:00:00.Z",
            "2026-10-07T12:00:00+0200",
            "2026-13-01T00:00:00Z",
            "2026-10-07T12:00:00Z ",
            "",
        ] {
            assert_eq!(iso(text), not_a_time(text));
        }
    }

    #[test]
    fn writing_drops_a_fraction_of_zero_and_trailing_zeros() {
        assert_eq!(shown(Time::from_unix(0)), "1970-01-01T00:00:00Z");
        assert_eq!(
            shown(Time::from_unix_micros(1_500_000)),
            "1970-01-01T00:00:01.5Z"
        );
        assert_eq!(
            shown(Time::from_unix_micros(123_456)),
            "1970-01-01T00:00:00.123456Z"
        );
        assert_eq!(
            shown(Time::from_unix_micros(120_000)),
            "1970-01-01T00:00:00.12Z"
        );
        assert_eq!(
            Time::from_unix_micros(-500_000).map(|t| format!("{t:?}")),
            Ok("1969-12-31T23:59:59.5Z".to_string())
        );
    }

    #[test]
    fn from_unix_takes_seconds_inside_the_range() {
        assert_eq!(
            shown(Time::from_unix(-62_167_219_200)),
            "0000-01-01T00:00:00Z"
        );
        assert_eq!(
            shown(Time::from_unix(253_402_300_799)),
            "9999-12-31T23:59:59Z"
        );
        for n in [-62_167_219_201, 253_402_300_800, i64::MAX, i64::MIN] {
            assert_eq!(
                shown(Time::from_unix(n)),
                format!(
                    "`{n}` seconds since 1970 is out of range for a time, which runs from the year 0000 to the year 9999"
                )
            );
        }
    }

    #[test]
    fn from_unix_micros_takes_microseconds_inside_the_range() {
        assert_eq!(shown(Time::from_unix_micros(FIRST)), "0000-01-01T00:00:00Z");
        assert_eq!(
            shown(Time::from_unix_micros(LAST)),
            "9999-12-31T23:59:59.999999Z"
        );
        for n in [FIRST - 1, LAST + 1, i64::MIN, i64::MAX] {
            assert_eq!(
                shown(Time::from_unix_micros(n)),
                format!(
                    "`{n}` microseconds since 1970 is out of range for a time, which runs from the year 0000 to the year 9999"
                )
            );
        }
    }

    #[test]
    fn add_seconds_stays_inside_the_range() {
        assert_eq!(
            shown(Time::from_unix(60).and_then(|t| t.add_seconds(-120))),
            "1969-12-31T23:59:00Z"
        );
        assert_eq!(
            shown(Time::from_unix(253_402_300_798).and_then(|t| t.add_seconds(1))),
            "9999-12-31T23:59:59Z"
        );
        let range = "is out of range for a time, which runs from the year 0000 to the year 9999";
        assert_eq!(
            shown(Time::from_unix_micros(LAST).and_then(|t| t.add_seconds(1))),
            format!("9999-12-31T23:59:59.999999Z plus `1` seconds {range}")
        );
        assert_eq!(
            shown(Time::from_unix_micros(FIRST).and_then(|t| t.add_seconds(-1))),
            format!("0000-01-01T00:00:00Z plus `-1` seconds {range}")
        );
        assert_eq!(
            shown(Time::from_unix(0).and_then(|t| t.add_seconds(i64::MAX))),
            format!("1970-01-01T00:00:00Z plus `{}` seconds {range}", i64::MAX)
        );
        assert_eq!(
            shown(Time::from_unix(0).and_then(|t| t.add_seconds(i64::MIN))),
            format!("1970-01-01T00:00:00Z plus `{}` seconds {range}", i64::MIN)
        );
    }

    #[test]
    fn to_unix_rounds_down() {
        let unix = |n| Time::from_unix_micros(n).map(Time::to_unix);
        assert_eq!(unix(1_999_999), Ok(1));
        assert_eq!(unix(0), Ok(0));
        assert_eq!(unix(-1), Ok(-1));
        assert_eq!(unix(-1_000_000), Ok(-1));
        assert_eq!(unix(-1_000_001), Ok(-2));
        assert_eq!(unix(FIRST), Ok(-62_167_219_200));
        assert_eq!(Time::from_unix_micros(-7).map(Time::to_unix_micros), Ok(-7));
    }

    #[test]
    fn seconds_since_rounds_toward_zero() {
        let since = |a, b| {
            Time::from_unix_micros(a)
                .and_then(|a| Time::from_unix_micros(b).map(|b| a.seconds_since(b)))
        };
        assert_eq!(since(1_500_000, 0), Ok(1));
        assert_eq!(since(0, 1_500_000), Ok(-1));
        assert_eq!(since(-500_000, 0), Ok(0));
        assert_eq!(since(LAST, FIRST), Ok(315_569_519_999));
        assert_eq!(since(FIRST, LAST), Ok(-315_569_519_999));
    }

    #[test]
    fn times_order_by_instant() {
        let early = Time::from_iso("2026-10-07T12:00:00+01:00");
        let late = Time::from_iso("2026-10-07T11:30:00Z");
        assert_eq!(early.and_then(|e| late.map(|l| e < l)), Ok(true));
    }

    #[test]
    fn now_is_cut_to_the_microsecond() {
        let micros = |reading: SystemTime| match reading.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_micros(),
            Err(_) => 0,
        };
        let before = micros(SystemTime::now());
        let now = Time::now().to_unix_micros();
        let after = micros(SystemTime::now());
        assert!(before <= now as u128 && now as u128 <= after, "{now}");
    }

    #[test]
    fn a_clock_before_1970_gives_a_negative_time_and_one_past_the_range_its_end() {
        let at = |reading: Option<SystemTime>| reading.map(|r| Time::at(r).to_unix_micros());
        assert_eq!(
            at(UNIX_EPOCH.checked_add(Duration::from_nanos(1_999))),
            Some(1)
        );
        assert_eq!(
            at(UNIX_EPOCH.checked_sub(Duration::from_micros(1_500))),
            Some(-1_500)
        );
        assert_eq!(
            at(UNIX_EPOCH.checked_sub(Duration::from_nanos(1))),
            Some(-1)
        );
        assert_eq!(
            at(UNIX_EPOCH.checked_add(Duration::from_secs(300_000_000_000))),
            Some(LAST)
        );
        assert_eq!(
            at(UNIX_EPOCH.checked_sub(Duration::from_secs(100_000_000_000))),
            Some(FIRST)
        );
    }

    #[test]
    fn from_str_and_parse_read_as_from_iso() {
        assert_eq!(
            "2026-10-07T12:00:00Z".parse::<Time>(),
            Time::from_iso("2026-10-07T12:00:00Z")
        );
        assert_eq!(
            crate::parse::<Time>("x").map_err(|e| e.to_string()),
            Err(not_a_time("x"))
        );
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Row {
        at: Time,
    }

    #[test]
    fn json_writes_and_reads_the_written_form() {
        let row = Time::from_iso("2026-10-07T12:00:00.25Z").map(|at| Row { at });
        assert_eq!(
            row.as_ref().map(crate::json::stringify),
            Ok(r#"{"at":"2026-10-07T12:00:00.25Z"}"#.to_string())
        );
        assert_eq!(
            crate::json::parse::<Row>(r#"{"at":"2026-10-07T14:00:00.25+02:00"}"#),
            row
        );
    }

    #[test]
    fn json_refuses_a_number_or_a_bad_text() {
        match crate::json::parse::<Row>(r#"{"at":1759838400}"#) {
            Err(e) => assert!(
                e.message()
                    .contains("expected a time like 2026-10-07T12:00:00Z"),
                "{e}"
            ),
            Ok(row) => panic!("read {row:?}"),
        }
        match crate::json::parse::<Row>(r#"{"at":"2026-10-07"}"#) {
            Err(e) => assert!(e.message().contains(&not_a_time("2026-10-07")), "{e}"),
            Ok(row) => panic!("read {row:?}"),
        }
    }
}
