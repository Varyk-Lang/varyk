//! `varyk-syntax`: spans, tokens, lexer, parser, and AST for Varyk.
//!
//! This crate carries no semantic analysis, so a formatter or a language
//! server can depend on it without pulling in the type checker or borrow
//! analysis.

mod ast;
mod error;
mod lexer;
mod parser;
mod source;
mod span;
mod token;

pub use ast::{
    BinaryOp, Block, EnumDecl, EnumVariant, Expr, ExprKind, FieldDecl, ForHead, Function, Ident,
    ImplBlock, Item, MatchArm, ModDecl, Param, Pattern, Program, SelfMode, Stmt, StructDecl,
    SubPattern, TypeExpr, UnaryOp, VariantPattern,
};
pub use error::{FixIt, SyntaxError, V0001, V0002, V0003, V0010, V0011, V0012};
pub use lexer::lex;
pub use parser::parse;
pub use source::SourceFile;
pub use span::{FileId, Span};
pub use token::{Token, TokenKind};

/// Lexes `file` then parses it, merging the lexer's errors first. If the
/// lexer reported anything, parsing is skipped: a file with lexical errors
/// has no reliable token stream to parse from, so its `Program` is empty
/// and only the lexer's errors are returned. The compiler's `resolve`
/// module calls this for every file it loads.
pub fn parse_source(file: &SourceFile) -> (Program, Vec<SyntaxError>) {
    let (tokens, lex_errors) = lex(file);
    if !lex_errors.is_empty() {
        return (Program { items: Vec::new() }, lex_errors);
    }
    parse(&tokens, file.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Every `.vr` file under `examples/`, found recursively so the
    /// per-language-feature example directories (`interop/`, `modules/`)
    /// are covered too.
    fn example_files() -> Vec<PathBuf> {
        let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut files = Vec::new();
        collect_vr_files(&examples_dir, &mut files);
        files
    }

    fn collect_vr_files(dir: &Path, files: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("failed to read directory {dir:?}: {e}"));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|e| panic!("bad directory entry: {e}"))
                .path();
            if path.is_dir() {
                collect_vr_files(&path, files);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("vr") {
                files.push(path);
            }
        }
    }

    /// The first end-to-end check of the grammar: every example file in
    /// the repository must parse with zero errors.
    #[test]
    fn every_example_file_parses_with_zero_errors() {
        let files = example_files();
        assert!(
            !files.is_empty(),
            "found no .vr files under examples/; check the path"
        );
        for path in files {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {path:?}: {e}"));
            let file = SourceFile::new(FileId(0), path.clone(), text);
            let (_program, errors) = parse_source(&file);
            assert!(
                errors.is_empty(),
                "unexpected errors parsing {path:?}: {errors:?}"
            );
        }
    }
}
