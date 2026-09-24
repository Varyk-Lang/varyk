//! Source files and the line index that maps a byte offset to a one-based
//! line and column.

use std::path::PathBuf;

use crate::span::FileId;

/// Maps byte offsets into a source text to one-based line and column
/// positions. Built once from the text, from the byte offset where each
/// line starts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LineIndex {
    /// Byte offset of the start of each line, `line_starts[0] == 0`.
    line_starts: Vec<u32>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push((offset + 1) as u32);
            }
        }
        Self { line_starts }
    }

    /// Converts a byte offset to a one-based `(line, column)` pair by
    /// finding the last line start at or before `offset`.
    fn line_col(&self, offset: u32) -> (u32, u32) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(insertion_point) => insertion_point - 1,
        };
        let column = offset - self.line_starts[line] + 1;
        ((line + 1) as u32, column)
    }
}

/// A single source file: its id, path, full text, and a line index built
/// once so byte offsets convert to line/column cheaply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub id: FileId,
    pub path: PathBuf,
    pub text: String,
    line_index: LineIndex,
}

impl SourceFile {
    pub fn new(id: FileId, path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        let text = text.into();
        let line_index = LineIndex::new(&text);
        Self {
            id,
            path: path.into(),
            text,
            line_index,
        }
    }

    /// Converts a byte offset into this file's text to a one-based
    /// `(line, column)` pair.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        self.line_index.line_col(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_is_one_one() {
        let file = SourceFile::new(FileId(0), "test.vr", "let x;\n");
        assert_eq!(file.line_col(0), (1, 1));
        assert_eq!(file.line_col(4), (1, 5));
    }

    #[test]
    fn second_line_offsets_convert_through_the_line_index() {
        // Byte offsets:
        // "let x;\n" -> 0..=6, second line starts at byte 7.
        // "let y;\n" -> second line is "let y;\n"
        let file = SourceFile::new(FileId(0), "test.vr", "let x;\nlet y;\n");
        // 'l' of the second "let" is at offset 7: line 2, column 1.
        assert_eq!(file.line_col(7), (2, 1));
        // 'y' of the second line is at offset 11: line 2, column 5.
        assert_eq!(file.line_col(11), (2, 5));
    }

    #[test]
    fn third_line_after_two_newlines() {
        let file = SourceFile::new(FileId(0), "test.vr", "a\nb\nc");
        assert_eq!(file.line_col(4), (3, 1));
    }
}
