//! Loading the module tree (spec 3.1): starting from the entry file, every
//! `mod` declared in a `.vr` module is found, read, and parsed or imported,
//! to any depth.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use varyk_syntax::{FileId, Item, ModDecl, SourceFile, Span};

use crate::diagnostics::{Diagnostic, codes};
use crate::interop::{ImportError, ImportedModule, import_rust_module_in};

use super::{
    Dependencies, Module, ModuleId, ModuleKind, duplicate, parse, reserved_type_name,
    std_module_taken, std_type_taken, varyk_prefix_taken,
};

/// Loads the file behind every `mod` declared in every `.vr` module of
/// `modules` (the entry module first), appending each child to `modules`
/// breadth-first (and, for `.rs` modules, their imported functions to
/// `imports`). A `mod` resolves in its declaring module's
/// [`Module::dir`]. Returns false if any module file had syntax errors,
/// which stops resolution. `dependencies` are the package's, whose crates
/// a `.rs` module may name (`None` for a single file); `dependency` says
/// the package is a dependency of the build, whose `.vr` files win over
/// `.rs` files of the same module (M5b2 spec 3).
pub(super) fn load_modules(
    dependencies: Option<Dependencies<'_>>,
    dependency: bool,
    sources: &mut Vec<SourceFile>,
    modules: &mut Vec<Module>,
    imports: &mut Vec<(ModuleId, ImportedModule)>,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let mut syntax_ok = true;
    let mut next = 0;
    while next < modules.len() {
        let parent = &modules[next];
        next += 1;
        let ModuleKind::Varyk(program) = &parent.kind else {
            continue;
        };
        let mod_decls: Vec<ModDecl> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Mod(decl) => Some(decl.clone()),
                _ => None,
            })
            .collect();
        let parent_id = parent.id;
        let dir = parent.dir.clone();
        let place = place(modules, parent_id);
        let mut declared: HashMap<&str, Span> = HashMap::new();
        for decl in &mod_decls {
            let name = decl.name.name.as_str();
            if !check_name(decl, parent_id == ModuleId(0), &mut declared, diagnostics) {
                continue;
            }
            let Some(path) = find_file(decl, &dir, &place, dependency, diagnostics) else {
                continue;
            };
            match load_one(
                decl,
                path,
                dependencies,
                sources,
                modules.len(),
                imports,
                diagnostics,
            ) {
                Loaded::Module(kind, file) => modules.push(Module {
                    id: ModuleId(modules.len() as u32),
                    name: name.to_string(),
                    parent: Some(parent_id),
                    is_pub: decl.is_pub,
                    kind,
                    file,
                    dir: dir.join(name),
                }),
                Loaded::SyntaxErrors => syntax_ok = false,
                Loaded::Failed => {}
            }
        }
    }
    syntax_ok
}

/// V0104 for `mod main;` and `mod lib;`, and for `mod bin;` in the crate
/// root (`at_root`); V0103 for a reserved or repeated name; false when
/// `decl` should not be loaded.
fn check_name<'d>(
    decl: &'d ModDecl,
    at_root: bool,
    declared: &mut HashMap<&'d str, Span>,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let name = decl.name.name.as_str();
    // `main.rs` is the crate root and `lib.rs` would become a second
    // crate of its own under cargo. On a case-insensitive file system
    // (the macOS and Windows default) `Main.rs` is the same file, so the
    // names are compared in any capitalization.
    if name.eq_ignore_ascii_case("main") || name.eq_ignore_ascii_case("lib") {
        let why = if name.eq_ignore_ascii_case("main") {
            "`src/main.rs` is where the generated program starts"
        } else {
            "cargo builds `src/lib.rs` as a library of its own"
        };
        diagnostics.push(
            Diagnostic::new(
                codes::V0104,
                decl.span,
                format!(
                    "a module cannot be named `{name}`, in any capitalization; use another name"
                ),
            )
            .with_note(match name.to_ascii_lowercase() {
                lower if lower == name => why.to_string(),
                lower => format!(
                    "{why}, and many file systems treat `{name}.rs` and `{lower}.rs` as one file"
                ),
            }),
        );
        return false;
    }
    // Cargo builds every file directly in `src/bin/` as a program of its
    // own, so a root module `bin` would break the build.
    if at_root && name.eq_ignore_ascii_case("bin") {
        diagnostics.push(
            Diagnostic::new(
                codes::V0104,
                decl.span,
                format!(
                    "a module of the entry file cannot be named `{name}`, in any \
                     capitalization; use another name"
                ),
            )
            .with_note("cargo treats the files in `src/bin/` as extra programs"),
        );
        return false;
    }
    let reserved = reserved_type_name(name, decl.name.span)
        .or_else(|| std_module_taken(name, decl.name.span))
        .or_else(|| varyk_prefix_taken(name, decl.name.span));
    if let Some(diagnostic) = reserved {
        diagnostics.push(diagnostic);
        return false;
    }
    if let Some(&first) = declared.get(name) {
        diagnostics.push(duplicate(name, decl.name.span, first));
        return false;
    }
    declared.insert(name, decl.name.span);
    true
}

/// Where a module's children live, for a diagnostic: "next to the entry
/// file" for the root, else "in `shop/cart/`".
fn place(modules: &[Module], id: ModuleId) -> String {
    let mut names = Vec::new();
    let mut current = &modules[id.0 as usize];
    while let Some(parent) = current.parent {
        names.push(current.name.as_str());
        current = &modules[parent.0 as usize];
    }
    if names.is_empty() {
        return "next to the entry file".to_string();
    }
    names.reverse();
    format!("in `{}/`", names.join("/"))
}

/// The one file for `mod name;` in `dir`: `name.vr`, `name.rs`, or
/// `name/mod.vr`. None of them, or more than one, is V0104 at the `mod`;
/// except that in a `dependency` a `.vr` file wins over `name.rs`, which
/// is what a publisher's assembly put beside it (M5b2 spec 3).
fn find_file(
    decl: &ModDecl,
    dir: &std::path::Path,
    place: &str,
    dependency: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<PathBuf> {
    let name = &decl.name.name;
    let candidates = [
        format!("{name}.vr"),
        format!("{name}.rs"),
        format!("{name}/mod.vr"),
    ];
    let mut found: Vec<&String> = candidates
        .iter()
        .filter(|candidate| dir.join(candidate).is_file())
        .collect();
    if dependency && found.iter().any(|file| file.ends_with(".vr")) {
        found.retain(|file| file.ends_with(".vr"));
    }
    match found.as_slice() {
        [one] => Some(dir.join(one)),
        [] => {
            diagnostics.push(Diagnostic::new(
                codes::V0104,
                decl.span,
                format!(
                    "no file for module `{name}`: expected `{name}.vr`, `{name}.rs`, or \
                     `{name}/mod.vr` {place}"
                ),
            ));
            None
        }
        several => {
            let files: Vec<String> = several.iter().map(|f| format!("`{f}`")).collect();
            let files = match files.as_slice() {
                [a, b] => format!("both {a} and {b}"),
                _ => format!("all of {}", files.join(", ")),
            };
            diagnostics.push(Diagnostic::new(
                codes::V0104,
                decl.span,
                format!("module `{name}` is ambiguous: {files} exist {place}"),
            ));
            None
        }
    }
}

enum Loaded {
    Module(ModuleKind, FileId),
    /// The file was read but had syntax errors (already reported).
    SyntaxErrors,
    /// The file could not be read or imported (already reported).
    Failed,
}

/// Reads `path` for `decl`, pushing it onto `sources`, and parses it (a
/// `.vr` file) or imports it (a `.rs` file, whose functions are recorded
/// in `imports` under the module id `id` it will get).
fn load_one(
    decl: &ModDecl,
    path: PathBuf,
    dependencies: Option<Dependencies<'_>>,
    sources: &mut Vec<SourceFile>,
    id: usize,
    imports: &mut Vec<(ModuleId, ImportedModule)>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Loaded {
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                codes::V0104,
                decl.span,
                format!("cannot read `{}`: {err}", path.display()),
            ));
            return Loaded::Failed;
        }
    };
    let file = FileId(sources.len() as u32);
    sources.push(SourceFile::new(file, path.clone(), text));
    let source = sources.last().expect("just pushed");

    if path.extension().is_some_and(|ext| ext == "vr") {
        return match parse(source) {
            Ok(program) => Loaded::Module(ModuleKind::Varyk(program), file),
            Err(errors) => {
                diagnostics.extend(errors);
                Loaded::SyntaxErrors
            }
        };
    }
    let crates = dependencies.map(|dependencies| dependencies.crates);
    match import_rust_module_in(&source.text, crates) {
        Ok(mut imported) => {
            imported.file = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            // `Error`, `Task`, and `Shared` are the standard types' names
            // (M5a spec 2.10, milestone 5b1 spec 2.8).
            let types = imported
                .structs
                .iter()
                .map(|s| (&s.name, &s.span))
                .chain(imported.enums.iter().map(|e| (&e.name, &e.span)));
            for (name, span) in types {
                let span = Span::new(file, span.start as u32, span.end as u32);
                diagnostics.extend(std_type_taken(name, span));
            }
            imports.push((ModuleId(id as u32), imported));
            Loaded::Module(ModuleKind::Rust(source.text.clone()), file)
        }
        Err(error) => {
            let entry = sources
                .iter()
                .find(|source| source.path.extension().is_some_and(|ext| ext == "vr"))
                .map(|source| source.path.clone())
                .unwrap_or_default();
            diagnostics.push(import_failure(
                &error,
                decl,
                &path,
                &entry,
                file,
                dependencies,
            ));
            Loaded::Failed
        }
    }
}

/// V0104 for the `.rs` file `path` (`file`), declared by `decl`, that
/// cannot be imported: at `decl` when it does not parse, else at the name
/// in the file that the generated crate cannot have. `dependencies`:
/// `Some` when the file belongs to a package, which has a `Cargo.toml`
/// to add a dependency to. `entry` is the program's entry `.vr` file.
fn import_failure(
    error: &ImportError,
    decl: &ModDecl,
    path: &Path,
    entry: &Path,
    file: FileId,
    dependencies: Option<Dependencies<'_>>,
) -> Diagnostic {
    let shown = path.display();
    let (message, span, note) = match error {
        ImportError::Parse(message) => {
            return Diagnostic::new(
                codes::V0104,
                decl.span,
                format!("cannot parse `{shown}`: {message}"),
            );
        }
        ImportError::Crate { name, span } => {
            let dev = |dependencies: Dependencies<'_>| {
                dependencies
                    .dev
                    .iter()
                    .any(|key| key.replace('-', "_") == *name)
            };
            let message = if dependencies.is_none() {
                let entry = entry.file_name().unwrap_or_default().to_string_lossy();
                format!(
                    "`{shown}` uses the crate `{name}`, but a single file has no dependencies; \
                     make a package with `varyk init`, move `{entry}` and its module files into \
                     its `src/` (replacing the `src/main.vr` it wrote), and add `{name}` under \
                     `[dependencies]`"
                )
            } else if dependencies.is_some_and(dev) {
                format!(
                    "`{shown}` uses the crate `{name}`, which is only in `[dev-dependencies]`: \
                     cargo's tests may use it, but a `.rs` module cannot; move it to `[dependencies]` \
                     in Cargo.toml"
                )
            } else {
                format!(
                    "`{shown}` uses the crate `{name}`, which is not in `[dependencies]`; add it \
                     under `[dependencies]` in Cargo.toml"
                )
            };
            (message, span, None)
        }
        ImportError::NestedModule { name, span } => {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            (
                format!(
                    "`{shown}` declares a module `{name}`; a `.rs` module cannot have \
                     submodules here, so move the code into `{file_name}`"
                ),
                span,
                Some("only the `.rs` file itself is copied into the generated crate"),
            )
        }
        ImportError::Include { name, span } => (
            format!("`{shown}` uses `{name}!`, which Varyk cannot follow"),
            span,
            Some(
                "only the `.rs` file itself is copied into the generated crate, so the file it names would be missing",
            ),
        ),
    };
    let span = Span::new(file, span.start as u32, span.end as u32);
    let diagnostic = Diagnostic::new(codes::V0104, span, message);
    match note {
        Some(note) => diagnostic.with_note(note),
        None => diagnostic,
    }
}
