//! [`Writer`]: builds one generated `.rs` file line by line, recording the
//! Varyk span each generated line came from (spec 2.3, 5), so the driver
//! can map a rustc error in that file back to the Varyk source that
//! produced it.

use varyk_syntax::Span;

/// One generated file: its path relative to the crate root, its full
/// text, and a line-level source map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub text: String,
    /// `lines[i]` is the Varyk span the zero-based generated line `i` came
    /// from; `None` for a structural line (a brace, a blank line, a `mod`
    /// line, or an attribute).
    pub lines: Vec<Option<Span>>,
    /// `marks[i]` is what the zero-based generated line `i` belongs to,
    /// when a rustc error there needs words of its own.
    pub marks: Vec<Option<Mark>>,
}

/// What a generated line belongs to, for the words of a thread-safety
/// error rustc reports there (milestone 5b4 spec 7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A route's adapter or the call registering it.
    Route,
    /// An `App::new` call, which shares the state between threads.
    State,
}

/// Builds a [`GeneratedFile`] one line at a time, so every physical line
/// of output carries the span of the Varyk node that produced it.
#[derive(Debug, Default)]
pub struct Writer {
    path: String,
    text: String,
    lines: Vec<Option<Span>>,
    marks: Vec<Option<Mark>>,
}

impl Writer {
    /// A writer for the file at `path` (relative to the crate root, e.g.
    /// `src/main.rs`).
    pub fn new(path: impl Into<String>) -> Self {
        Writer {
            path: path.into(),
            text: String::new(),
            lines: Vec::new(),
            marks: Vec::new(),
        }
    }

    /// Appends one line: `indent` levels of four-space indentation, then
    /// `text`, then a newline. `span` is the Varyk node this line came
    /// from, or `None` for a structural line.
    pub fn line(&mut self, indent: usize, text: &str, span: Option<Span>) {
        self.marked_line(indent, text, span, None);
    }

    /// [`Writer::line`], the line belonging to `mark`.
    pub fn marked_line(
        &mut self,
        indent: usize,
        text: &str,
        span: Option<Span>,
        mark: Option<Mark>,
    ) {
        for _ in 0..indent {
            self.text.push_str("    ");
        }
        self.text.push_str(text);
        self.text.push('\n');
        self.lines.push(span);
        self.marks.push(mark);
    }

    /// Consumes the writer, producing the finished file.
    pub fn finish(self) -> GeneratedFile {
        GeneratedFile {
            path: self.path,
            text: self.text,
            lines: self.lines,
            marks: self.marks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use varyk_syntax::FileId;

    fn span(start: u32, end: u32) -> Span {
        Span::new(FileId(0), start, end)
    }

    #[test]
    fn records_one_span_per_line_and_none_for_structural_lines() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() {", Some(span(0, 10)));
        writer.line(1, "let x = 1;", Some(span(15, 25)));
        writer.line(0, "}", None);

        let file = writer.finish();

        assert_eq!(file.path, "src/main.rs");
        assert_eq!(file.text, "fn main() {\n    let x = 1;\n}\n");
        assert_eq!(
            file.lines,
            vec![Some(span(0, 10)), Some(span(15, 25)), None]
        );
    }

    #[test]
    fn a_lines_index_is_the_zero_based_generated_line_number() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "one", None);
        writer.line(0, "two", Some(span(3, 6)));
        writer.line(2, "three", None);

        let file = writer.finish();

        assert_eq!(
            file.text.lines().collect::<Vec<_>>(),
            vec!["one", "two", "        three"]
        );
        assert_eq!(file.lines[0], None);
        assert_eq!(file.lines[1], Some(span(3, 6)));
        assert_eq!(file.lines[2], None);
    }

    #[test]
    fn indentation_is_four_spaces_per_level() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(3, "deep", None);

        assert_eq!(writer.finish().text, "            deep\n");
    }
}
