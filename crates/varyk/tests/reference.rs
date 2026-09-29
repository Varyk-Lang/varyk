//! Reference coverage (M4 spec 7, the minimum form of the test M1 §7
//! promised): `docs/language.md` mentions, in its code, every keyword and
//! reserved word of the lexer, every built-in type name, every call of the
//! standard table, and every diagnostic code.

use std::collections::HashSet;
use std::path::Path;

/// The file's text and every identifier written in its code: fenced
/// blocks and inline `code` spans, so a word of prose such as "in" or "as"
/// does not count as the keyword.
fn reference() -> (String, HashSet<String>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/language.md");
    let text = std::fs::read_to_string(&path).expect("read docs/language.md");
    let mut code = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if fenced {
            code.push_str(line);
            code.push('\n');
        } else {
            for span in line.split('`').skip(1).step_by(2) {
                code.push_str(span);
                code.push('\n');
            }
        }
    }
    let words = code
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    (text, words)
}

fn missing<'a>(names: impl IntoIterator<Item = &'a str>, words: &HashSet<String>) -> Vec<&'a str> {
    names
        .into_iter()
        .filter(|name| !words.contains(*name))
        .collect()
}

#[test]
fn the_reference_shows_every_keyword_and_reserved_word() {
    let (_, words) = reference();
    let keywords = varyk_syntax::KEYWORDS.iter().map(|(word, _)| *word);
    let reserved = varyk_syntax::RESERVED_KEYWORDS.iter().copied();
    assert_eq!(
        missing(keywords.chain(reserved), &words),
        Vec::<&str>::new()
    );
}

#[test]
fn the_reference_shows_every_built_in_type_name() {
    let (_, words) = reference();
    let names = varyk::types::BUILTIN_TYPE_NAMES.iter().copied();
    assert_eq!(missing(names, &words), Vec::<&str>::new());
}

#[test]
fn the_reference_shows_every_call_of_the_standard_table() {
    let (_, words) = reference();
    let names = varyk::builtins::TABLE.iter().map(|row| row.name);
    assert_eq!(missing(names, &words), Vec::<&str>::new());
}

#[test]
fn the_reference_lists_every_diagnostic_code() {
    let (text, _) = reference();
    let absent: Vec<&str> = varyk::diagnostics::codes::ALL
        .iter()
        .copied()
        .filter(|code| !text.contains(code))
        .collect();
    assert_eq!(absent, Vec::<&str>::new());
}
