//! The `.env` reader (spec 2.5): `KEY=value` lines, read once from the
//! current directory, never written into the process environment.

use crate::Error;
use std::path::Path;
use std::sync::OnceLock;

type Entries = Vec<(String, String)>;

/// The current directory's `.env`, read on first use. A malformed file is
/// kept as its `Error` so `env::parse` can report it.
pub(crate) fn cached() -> &'static Result<Entries, Error> {
    static CACHE: OnceLock<Result<Entries, Error>> = OnceLock::new();
    CACHE.get_or_init(|| load(Path::new(".")))
}

/// Reads `dir/.env`. A missing file is no entries.
pub(crate) fn load(dir: &Path) -> Result<Entries, Error> {
    match std::fs::read_to_string(dir.join(".env")) {
        Ok(text) => parse_text(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Error::new(format!("cannot read .env: {e}"))),
    }
}

fn parse_text(text: &str) -> Result<Entries, Error> {
    let mut entries = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match split_line(line) {
            Some(entry) => entries.push(entry),
            None => {
                return Err(Error::new(format!(
                    ".env line {} is not KEY=value: `{line}`",
                    index + 1
                )));
            }
        }
    }
    Ok(entries)
}

fn split_line(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let valid = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return None;
    }
    Some((key.to_string(), unquote(value.trim()).to_string()))
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(text: &str) -> Vec<(String, String)> {
        match parse_text(text) {
            Ok(v) => v,
            Err(e) => vec![("error".to_string(), e.to_string())],
        }
    }

    fn kv(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn plain_lines() {
        assert_eq!(pairs("A=1\nB=two\n"), vec![kv("A", "1"), kv("B", "two")]);
    }

    #[test]
    fn blank_and_comment_lines_are_skipped() {
        assert_eq!(pairs("\n# note\n  \nA=1\n  # more\n"), vec![kv("A", "1")]);
    }

    #[test]
    fn quotes_are_stripped() {
        assert_eq!(
            pairs("A='x y'\nB=\"p q\"\nC=\"\"\nD='it\"s'"),
            vec![
                kv("A", "x y"),
                kv("B", "p q"),
                kv("C", ""),
                kv("D", "it\"s")
            ]
        );
    }

    #[test]
    fn a_value_keeps_everything_after_the_first_equals() {
        assert_eq!(
            pairs("DATABASE_URL=postgres://h/db?sslmode=require"),
            vec![kv("DATABASE_URL", "postgres://h/db?sslmode=require")]
        );
    }

    #[test]
    fn crlf_lines() {
        assert_eq!(pairs("A=1\r\nB=2\r\n"), vec![kv("A", "1"), kv("B", "2")]);
    }

    #[test]
    fn export_is_a_malformed_line() {
        assert_eq!(
            pairs("A=1\nexport PORT=1"),
            vec![kv("error", ".env line 2 is not KEY=value: `export PORT=1`")]
        );
    }

    #[test]
    fn a_line_without_equals_is_malformed() {
        assert_eq!(
            pairs("\nJUST_A_WORD"),
            vec![kv("error", ".env line 2 is not KEY=value: `JUST_A_WORD`")]
        );
    }

    #[test]
    fn a_missing_file_is_no_entries() {
        let dir = std::env::temp_dir().join("varyk-std-no-such-dir-for-dotenv");
        assert_eq!(load(&dir), Ok(vec![]));
    }

    #[test]
    fn load_reads_the_file_in_the_directory() {
        let dir = std::env::temp_dir().join(format!("varyk-std-dotenv-{}", std::process::id()));
        assert!(std::fs::create_dir_all(&dir).is_ok());
        assert!(std::fs::write(dir.join(".env"), "A=1\n").is_ok());
        assert_eq!(load(&dir), Ok(vec![kv("A", "1")]));
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }
}
