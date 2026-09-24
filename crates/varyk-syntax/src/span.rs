//! Source spans: a file id plus a byte range within that file.

/// Identifies a source file within a compilation. A small newtype over
/// `u32` so file ids stay cheap to copy and compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

/// A byte range `[start, end)` within a single source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        Self { file, start, end }
    }
}
