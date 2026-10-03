//! Logging setup (spec 2.6): `start()` installs a tracing subscriber that
//! writes to stderr, at the level `LOG` names, as text or JSON lines.

use crate::dotenv;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Sets up logging from `LOG` and `LOG_FORMAT`, read from the process
/// environment and then the `.env` in the current directory. Never fails:
/// a bad `LOG` value or an unreadable `.env` is one warning line, and
/// logging goes on at `info`. Calling it twice keeps the first setup.
pub fn start() {
    let entries = match dotenv::cached() {
        Ok(entries) => entries.as_slice(),
        Err(_) => &[],
    };
    let mut setup = settings(&process_variable, entries);
    if let Err(e) = dotenv::cached() {
        setup.warnings.push(format!("ignoring the .env file: {e}"));
    }
    let format = LineFormat {
        json: setup.json,
        clock: SystemTime::now,
    };
    let installed = tracing_subscriber::fmt()
        .with_max_level(setup.level)
        .with_writer(std::io::stderr)
        .event_format(format)
        .try_init();
    // Written straight to stderr, not through the subscriber, so that
    // `LOG=error` or `LOG=off` cannot hide a warning about the setup.
    if installed.is_ok() {
        for warning in &setup.warnings {
            eprint!(
                "{}",
                line(setup.json, &rfc3339(SystemTime::now()), "WARN", warning)
            );
        }
    }
}

/// A process variable; one that is set but is not valid text is read
/// with its bad bytes replaced, so it still wins over `.env` and a bad
/// `LOG` still warns.
fn process_variable(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(value)) => Some(value.to_string_lossy().into_owned()),
    }
}

struct Settings {
    level: LevelFilter,
    json: bool,
    warnings: Vec<String>,
}

/// The pure half of `start`: what `LOG` and `LOG_FORMAT` say, first from
/// the lookup and then from the `.env` entries.
fn settings(lookup: &dyn Fn(&str) -> Option<String>, entries: &[(String, String)]) -> Settings {
    let find = |key: &str| {
        lookup(key).or_else(|| {
            entries
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        })
    };
    let mut warnings = Vec::new();
    let level = match find("LOG") {
        None => LevelFilter::INFO,
        Some(text) => match level_from(&text) {
            Some(level) => level,
            None => {
                warnings.push(format!(
                    "LOG is `{text}`, not one of debug, info, warn, error, off; using info"
                ));
                LevelFilter::INFO
            }
        },
    };
    let json = find("LOG_FORMAT").as_deref() == Some("json");
    Settings {
        level,
        json,
        warnings,
    }
}

fn level_from(text: &str) -> Option<LevelFilter> {
    match text.to_ascii_lowercase().as_str() {
        "debug" => Some(LevelFilter::DEBUG),
        "info" => Some(LevelFilter::INFO),
        "warn" => Some(LevelFilter::WARN),
        "error" => Some(LevelFilter::ERROR),
        "off" => Some(LevelFilter::OFF),
        _ => None,
    }
}

/// One line per event: `<time> <LEVEL> <message>`, or a JSON object with
/// exactly `time`, `level`, and `message`. The event's other fields follow
/// the message as `name=value`. No target, no colour.
struct LineFormat {
    json: bool,
    clock: fn() -> SystemTime,
}

/// The event's `message` field, and every other field as `name=value`.
#[derive(Default)]
struct Message {
    message: String,
    fields: Vec<String>,
}

impl Message {
    /// The message, then the fields, separated by spaces.
    fn text(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if !self.message.is_empty() {
            parts.push(&self.message);
        }
        parts.extend(self.fields.iter().map(String::as_str));
        parts.join(" ")
    }
}

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }
}

impl<S, N> FormatEvent<S, N> for LineFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut message = Message::default();
        event.record(&mut message);
        let time = rfc3339((self.clock)());
        let level = event.metadata().level().as_str();
        write!(writer, "{}", line(self.json, &time, level, &message.text()))
    }
}

/// One log line with its newline, as text or as a JSON object.
fn line(json: bool, time: &str, level: &str, message: &str) -> String {
    if json {
        format!(
            "{{\"time\":{},\"level\":{},\"message\":{}}}\n",
            json_string(time),
            json_string(level),
            json_string(message)
        )
    } else {
        format!("{time} {level} {message}\n")
    }
}

fn json_string(text: &str) -> String {
    match serde_json::to_string(text) {
        Ok(quoted) => quoted,
        // Unreachable: a string always serializes.
        Err(_) => "\"\"".to_string(),
    }
}

/// `2026-09-30T12:34:56.789Z`, UTC, milliseconds.
fn rfc3339(time: SystemTime) -> String {
    let (secs, millis) = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs(), d.subsec_millis()),
        Err(_) => (0, 0),
    };
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| owned.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    fn level_of(pairs: &[(&str, &str)]) -> LevelFilter {
        settings(&lookup_from(pairs), &[]).level
    }

    #[test]
    fn unset_log_is_info() {
        assert_eq!(level_of(&[]), LevelFilter::INFO);
        assert!(settings(&lookup_from(&[]), &[]).warnings.is_empty());
    }

    #[test]
    fn log_is_case_insensitive() {
        assert_eq!(level_of(&[("LOG", "debug")]), LevelFilter::DEBUG);
        assert_eq!(level_of(&[("LOG", "WARN")]), LevelFilter::WARN);
        assert_eq!(level_of(&[("LOG", "Error")]), LevelFilter::ERROR);
        assert_eq!(level_of(&[("LOG", "off")]), LevelFilter::OFF);
    }

    #[test]
    fn a_bad_log_value_warns_once_and_uses_info() {
        let s = settings(&lookup_from(&[("LOG", "loud")]), &[]);
        assert_eq!(s.level, LevelFilter::INFO);
        assert_eq!(s.warnings.len(), 1);
        assert!(s.warnings[0].contains("loud"));
    }

    #[test]
    fn dotenv_is_the_second_source() {
        let entries = vec![("LOG".to_string(), "error".to_string())];
        assert_eq!(
            settings(&lookup_from(&[]), &entries).level,
            LevelFilter::ERROR
        );
        let env = lookup_from(&[("LOG", "debug")]);
        assert_eq!(settings(&env, &entries).level, LevelFilter::DEBUG);
    }

    #[test]
    fn log_format_json_chooses_json_lines() {
        assert!(settings(&lookup_from(&[("LOG_FORMAT", "json")]), &[]).json);
        assert!(!settings(&lookup_from(&[("LOG_FORMAT", "text")]), &[]).json);
        assert!(!settings(&lookup_from(&[("LOG_FORMAT", "JSON")]), &[]).json);
        assert!(!settings(&lookup_from(&[]), &[]).json);
    }

    #[test]
    fn times_are_rfc3339_utc() {
        let at = |secs: u64, millis: u32| UNIX_EPOCH + Duration::new(secs, millis * 1_000_000);
        assert_eq!(rfc3339(at(0, 0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(at(951_782_400, 5)), "2000-02-29T00:00:00.005Z");
        assert_eq!(rfc3339(at(1_790_771_696, 789)), "2026-09-30T12:34:56.789Z");
    }

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if let Ok(mut inner) = self.0.lock() {
                inner.extend_from_slice(buf);
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn fixed_clock() -> SystemTime {
        UNIX_EPOCH + Duration::new(1_790_771_696, 789_000_000)
    }

    fn logged(json: bool, level: LevelFilter, emit: impl FnOnce()) -> String {
        let capture = Capture(Arc::new(Mutex::new(Vec::new())));
        let sink = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(level)
            .with_writer(move || sink.clone())
            .event_format(LineFormat {
                json,
                clock: fixed_clock,
            })
            .finish();
        tracing::subscriber::with_default(subscriber, emit);
        let bytes = capture.0.lock().map(|b| b.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn a_text_line_is_time_level_message() {
        let out = logged(false, LevelFilter::DEBUG, || {
            tracing::info!("hello {}", 7);
            tracing::warn!("careful");
            tracing::error!("boom");
            tracing::debug!("detail");
        });
        assert_eq!(
            out,
            "2026-09-30T12:34:56.789Z INFO hello 7\n\
             2026-09-30T12:34:56.789Z WARN careful\n\
             2026-09-30T12:34:56.789Z ERROR boom\n\
             2026-09-30T12:34:56.789Z DEBUG detail\n"
        );
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn a_json_line_has_exactly_time_level_message() {
        let out = logged(true, LevelFilter::INFO, || {
            tracing::info!("say \"hi\"\nnow");
        });
        assert_eq!(out.lines().count(), 1);
        let value: serde_json::Value = match serde_json::from_str(out.trim_end()) {
            Ok(v) => v,
            Err(e) => panic!("not JSON: {e}: {out}"),
        };
        let keys: Vec<&String> = match value.as_object() {
            Some(o) => o.keys().collect(),
            None => panic!("not an object"),
        };
        assert_eq!(keys, ["level", "message", "time"]);
        assert_eq!(value["time"], "2026-09-30T12:34:56.789Z");
        assert_eq!(value["level"], "INFO");
        assert_eq!(value["message"], "say \"hi\"\nnow");
    }

    #[test]
    fn a_fields_only_event_shows_its_fields() {
        let out = logged(false, LevelFilter::DEBUG, || {
            tracing::debug!(summary = "select 1", rows = 1_u64);
        });
        assert_eq!(
            out,
            "2026-09-30T12:34:56.789Z DEBUG summary=select 1 rows=1\n"
        );
        let json = logged(true, LevelFilter::DEBUG, || {
            tracing::debug!(summary = "select 1", rows = 1_u64);
        });
        assert!(
            json.contains("\"message\":\"summary=select 1 rows=1\""),
            "{json}"
        );
    }

    #[test]
    fn a_message_and_fields_show_both() {
        let out = logged(false, LevelFilter::INFO, || {
            tracing::info!(code = 7_u64, "hello");
        });
        assert_eq!(out, "2026-09-30T12:34:56.789Z INFO hello code=7\n");
    }

    #[test]
    fn events_below_the_level_are_dropped() {
        let out = logged(false, LevelFilter::WARN, || {
            tracing::info!("quiet");
            tracing::error!("loud");
        });
        assert_eq!(out, "2026-09-30T12:34:56.789Z ERROR loud\n");
        let off = logged(false, LevelFilter::OFF, || tracing::error!("x"));
        assert_eq!(off, "");
    }

    #[test]
    fn a_setup_warning_line_has_the_chosen_format() {
        assert_eq!(
            line(false, "T", "WARN", "LOG is `x`"),
            "T WARN LOG is `x`\n"
        );
        assert_eq!(
            line(true, "T", "WARN", "bad"),
            "{\"time\":\"T\",\"level\":\"WARN\",\"message\":\"bad\"}\n"
        );
    }
}
