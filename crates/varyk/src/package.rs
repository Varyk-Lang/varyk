//! Varyk packages (M3 spec 2.1, 2.2): a `Cargo.toml` whose crate root is
//! `src/main.vr` (a binary) or `src/lib.vr` (a library). This module owns
//! everything TOML: finding the manifest, reading it, and rejecting what
//! Varyk does not support in it.

use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};
use varyk_syntax::{FileId, SourceFile, Span};

use crate::diagnostics::{Diagnostic, codes};

/// What a package's crate root makes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `src/main.vr`: must define `main`, can be run.
    Binary,
    /// `src/lib.vr`: must not define `main`, cannot be run.
    Library,
}

/// A loaded package.
#[derive(Clone, Debug)]
pub struct Package {
    /// The directory holding `Cargo.toml`, as found (possibly relative, or
    /// empty for the current directory).
    pub root: PathBuf,
    pub name: String,
    pub version: String,
    pub kind: Kind,
    /// `src/main.vr` or `src/lib.vr` under `root`.
    pub entry: PathBuf,
    /// `[dependencies]` as written, relative `path` values made absolute.
    pub dependencies: toml::Table,
    /// The names under `[dev-dependencies]`, which only tests may use.
    pub dev_dependencies: Vec<String>,
    /// The whole `Cargo.toml`, parsed once, as written.
    pub manifest: toml::Table,
    /// The workspace this package is a member of, as cargo finds it (see
    /// [`workspace_root`]): its directory and parsed manifest. Cargo reads
    /// the lock file, `[profile.*]`, and the resolver from there, so the
    /// crates Varyk builds do too.
    pub workspace: Option<(PathBuf, toml::Table)>,
    /// The `Cargo.lock` cargo would use: the workspace root's for a
    /// member, else the one beside `Cargo.toml`, when there is one.
    pub lock: Option<PathBuf>,
    /// Where V0404 points in the manifest.
    pub std_spans: StdSpans,
}

/// Places in `Cargo.toml` that [`check_std_dependency`] points at.
#[derive(Clone, Copy, Debug)]
pub struct StdSpans {
    /// `[dependencies]` when the manifest has it, else `[package]`: where a
    /// missing dependency is reported.
    pub anchor: Span,
    /// The `varyk-std` key, when the manifest has one.
    pub entry: Option<Span>,
}

/// The nearest `Cargo.toml` at or above `start`: first along `start` as
/// written (so a relative `start` gives a relative manifest path), then
/// along its absolute form.
pub fn find(start: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(start).ok();
    start
        .ancestors()
        .chain(absolute.iter().flat_map(|path| path.ancestors()))
        .map(|dir| dir.join("Cargo.toml"))
        .find(|manifest| manifest.is_file())
}

/// The root file of the package at `root`: `src/main.vr` if it exists,
/// else `src/lib.vr` if it exists, else `None`.
pub fn root_of(root: &Path) -> Option<(Kind, PathBuf)> {
    [(Kind::Binary, "src/main.vr"), (Kind::Library, "src/lib.vr")]
        .into_iter()
        .map(|(kind, file)| (kind, root.join(file)))
        .find(|(_, file)| file.is_file())
}

/// Reads and checks the manifest at `manifest` (spec 2.2), registering it
/// in `sources` with the next `FileId` so its diagnostics render like any
/// other. Never runs cargo.
pub fn load(manifest: &Path, sources: &mut Vec<SourceFile>) -> Result<Package, Vec<Diagnostic>> {
    let file = FileId(sources.len() as u32);
    let (text, unreadable) = match fs::read_to_string(manifest) {
        Ok(text) => (text, None),
        Err(err) => (String::new(), Some(err)),
    };
    sources.push(SourceFile::new(file, manifest.to_path_buf(), text));
    let text = &sources.last().expect("just pushed").text;
    let at = |range: Range<usize>| Span::new(file, range.start as u32, range.end as u32);
    let start = at(0..0);
    if let Some(err) = unreadable {
        return Err(vec![Diagnostic::new(
            codes::V0403,
            start,
            format!("this `Cargo.toml` cannot be read: {err}"),
        )]);
    }
    let doc = match DeTable::parse(text) {
        Ok(doc) => doc.into_inner(),
        Err(err) => return Err(vec![malformed(&err, &at)]),
    };
    let root = manifest.parent().unwrap_or(Path::new("")).to_path_buf();

    let mut diagnostics = Vec::new();
    let Some((package_key, package)) = table(&doc, "package") else {
        return Err(vec![
            Diagnostic::new(
                codes::V0403,
                start,
                "this `Cargo.toml` has no `[package]` table",
            )
            .with_note("add `[package]` with the package's `name`, `version`, and `edition`"),
        ]);
    };
    // Varyk reads a fixed set of `Cargo.toml` keys, the ones a small
    // service needs; anything else is refused as not supported yet, so an
    // unknown key can never pass `check` and surprise `build` or `publish`.
    for (key, _) in doc
        .iter()
        .filter(|(key, _)| !TOP_LEVEL_KEYS.contains(&key.get_ref().as_ref()))
    {
        let name: &str = key.get_ref().as_ref();
        let diagnostic = if TARGET_TABLES.contains(&name) {
            let header = if name == "lib" {
                "`[lib]`".to_string()
            } else {
                format!("`[[{name}]]`")
            };
            Diagnostic::new(
                codes::V0402,
                at(key.span()),
                format!("{header} is not supported yet"),
            )
            .with_note(
                "a Varyk package is one program from `src/main.vr` or one library from \
                 `src/lib.vr`, which cargo finds by its defaults; target tables are not read",
            )
        } else {
            Diagnostic::new(
                codes::V0401,
                at(key.span()),
                format!("`{name}` is not supported in `Cargo.toml` yet"),
            )
            .with_note(
                "Varyk reads a fixed set of Cargo.toml tables; the language reference lists them",
            )
        };
        diagnostics.push(diagnostic);
    }
    for (key, value) in package.iter() {
        let name: &str = key.get_ref().as_ref();
        // `x.workspace = true` is inheritance, refused as V0401 below; the
        // three keys with rules of their own report their own shape.
        let inherited = matches!(value.get_ref(), DeValue::Table(table) if entry(table, "workspace").is_some())
            || matches!(name, "name" | "version" | "edition");
        match PACKAGE_KEYS.iter().find(|(known, _)| *known == name) {
            None => diagnostics.push(
                Diagnostic::new(
                    codes::V0401,
                    at(key.span()),
                    format!("`{name}` is not supported under `[package]` yet"),
                )
                .with_note(
                    "Varyk reads a fixed set of `[package]` keys; the language reference lists them",
                ),
            ),
            Some((_, shape)) if !inherited && !shape.accepts(value.get_ref()) => diagnostics.push(
                Diagnostic::new(
                    codes::V0403,
                    at(key.span()),
                    format!("`{name}` must be {}", shape.describe()),
                )
                .with_note("cargo would refuse this manifest; write the value as cargo expects"),
            ),
            _ => {}
        }
    }
    // Every top-level key cargo reads as a table must be one; anything
    // else is a manifest cargo refuses, whichever way the value is spelled.
    for name in TABLE_KEYS {
        if let Some((key, value)) = entry(&doc, name) {
            if !matches!(value.get_ref(), DeValue::Table(_)) {
                diagnostics.push(
                    Diagnostic::new(
                        codes::V0403,
                        at(key.span()),
                        format!("`{name}` must be a table, `[{name}]`"),
                    )
                    .with_note(
                        "cargo would refuse this manifest; write the table as cargo expects",
                    ),
                );
            }
        }
    }
    for (key, value) in package {
        if key.get_ref() != "metadata" {
            check_not_inherited(key.get_ref(), value, &at, &mut diagnostics);
        }
    }
    // `rust-version` is a version without the patch part being required
    // (`1.85`), and without pre-release or build parts.
    if let Some((_, value)) = entry(package, "rust-version") {
        if let DeValue::String(version) = value.get_ref() {
            if !is_partial_version(version) {
                diagnostics.push(
                    Diagnostic::new(
                        codes::V0403,
                        at(value.span()),
                        format!("`{version}` is not a Rust version"),
                    )
                    .with_note("cargo needs `MAJOR.MINOR` or `MAJOR.MINOR.PATCH`, such as `1.85`"),
                );
            }
        }
    }
    if let Some((links, _)) = entry(package, "links") {
        diagnostics.push(
            Diagnostic::new(codes::V0401, at(links.span()), "`links` cannot be used yet")
                .with_note(
                    "a package that `links` a native library needs a build script, and the crate \
                 `varyk publish` assembles has none",
                ),
        );
    }
    let name_value = entry(package, "name").map(|(_, value)| value);
    // Where a diagnostic about the package as a whole points: its `name`,
    // else the `[package]` header.
    let package_span = at(name_value.map_or(package_key.span(), |value| value.span()));
    let name = match name_value.map(|value| value.get_ref()) {
        Some(DeValue::String(name)) => {
            if let Some(problem) = name_problem(name) {
                diagnostics.push(
                    Diagnostic::new(
                        codes::V0403,
                        package_span,
                        format!("this package cannot be called `{name}`: {problem}"),
                    )
                    .with_note(
                        "the name is also the program's name and its build directory's, so it \
                         follows the rules of Cargo, Rust's build tool",
                    ),
                );
            }
            name.to_string()
        }
        Some(_) => {
            diagnostics.push(Diagnostic::new(
                codes::V0403,
                package_span,
                "the package's `name` must be text in quotes",
            ));
            String::new()
        }
        None => {
            diagnostics.push(Diagnostic::new(
                codes::V0403,
                package_span,
                "`[package]` needs a `name`",
            ));
            String::new()
        }
    };
    // Cargo defaults a missing version to 0.0.0; anything present must be
    // one it accepts, or `build` and `publish` fail on the generated
    // manifest after `check` passed.
    let version = match entry(package, "version").map(|(_, value)| value) {
        None => "0.0.0".to_string(),
        Some(value) => match value.get_ref() {
            DeValue::String(version) => {
                if !is_version(version) {
                    diagnostics.push(
                        Diagnostic::new(
                            codes::V0403,
                            at(value.span()),
                            format!("`{version}` is not a version"),
                        )
                        .with_note(
                            "cargo needs `MAJOR.MINOR.PATCH`, such as `0.1.0`, with an optional \
                             `-pre` or `+build` part",
                        ),
                    );
                }
                version.to_string()
            }
            // `version.workspace = true`, already V0401.
            DeValue::Table(table) if entry(table, "workspace").is_some() => "0.0.0".to_string(),
            _ => {
                diagnostics.push(Diagnostic::new(
                    codes::V0403,
                    at(value.span()),
                    "the package's `version` must be text in quotes, such as \"0.1.0\"",
                ));
                "0.0.0".to_string()
            }
        },
    };
    match entry(package, "edition") {
        Some((_, value)) => match value.get_ref() {
            DeValue::String(edition) if edition == "2024" => {}
            // `edition.workspace`, already V0401.
            DeValue::Table(table) if entry(table, "workspace").is_some() => {}
            _ => diagnostics.push(
                Diagnostic::new(
                    codes::V0400,
                    at(value.span()),
                    "this package must use edition \"2024\"",
                )
                .with_note(
                    "Varyk generates Rust 2024 code; write `edition = \"2024\"` under `[package]`",
                ),
            ),
        },
        None => diagnostics.push(
            Diagnostic::new(
                codes::V0400,
                at(package_key.span()),
                "this package does not say its edition; it must be \"2024\"",
            )
            .with_note(
                "Varyk generates Rust 2024 code; write `edition = \"2024\"` under `[package]`",
            ),
        ),
    }
    for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some((_, dependencies)) = table(&doc, kind) {
            for (key, value) in dependencies {
                check_not_inherited(key.get_ref(), value, &at, &mut diagnostics);
            }
        }
    }
    // `[lints]` applies to the whole crate, the generated Rust included,
    // which carries only `allow`s for warnings and cannot opt out of a
    // `deny`; and a custom build script would have to run in the crate
    // `varyk build` makes and the one `publish` assembles, which run none.
    if let Some((key, _)) = entry(&doc, "lints") {
        diagnostics.push(
            Diagnostic::new(codes::V0401, at(key.span()), "`[lints]` cannot be used yet")
                .with_note(
                    "lints in `Cargo.toml` apply to the generated Rust too; put `#[deny(..)]` or \
                     `#[warn(..)]` attributes in the `.rs` file instead",
                ),
        );
    }
    // `build` may only be left out or name the `build.rs` that `init`
    // writes: another script would not run in the crates Varyk builds, and
    // `false` would keep plain `cargo build` from running the include stub.
    if let Some((key, value)) = entry(package, "build") {
        let (message, note) = match value.get_ref() {
            DeValue::String(script) if script == "build.rs" => (None, ""),
            DeValue::String(_) => (
                Some("a custom build script cannot be used yet"),
                "the crate `varyk build` makes and the one `varyk publish` assembles run no \
                 build script; the `build.rs` that `varyk init` writes only includes the \
                 generated Rust",
            ),
            DeValue::Boolean(false) => (
                Some("`build` cannot be turned off"),
                "plain `cargo build` needs the `build.rs` that `varyk init` writes to include \
                 the generated Rust; leave `build` out",
            ),
            _ => (None, ""),
        };
        if let Some(message) = message {
            diagnostics
                .push(Diagnostic::new(codes::V0401, at(key.span()), message).with_note(note));
        }
    }
    if let Some((_, targets)) = table(&doc, "target") {
        for (_, target) in targets {
            if let DeValue::Table(target) = target.get_ref() {
                for kind in ["dev-dependencies", "build-dependencies"] {
                    if let Some((_, dependencies)) = table(target, kind) {
                        for (key, value) in dependencies {
                            check_not_inherited(key.get_ref(), value, &at, &mut diagnostics);
                        }
                    }
                }
                if let Some((key, _)) = entry(target, "dependencies") {
                    diagnostics.push(
                        Diagnostic::new(
                            codes::V0401,
                            at(key.span()),
                            "dependencies for only some platforms are not supported yet",
                        )
                        .with_note(
                            "these are in a `[target.'cfg(..)'.dependencies]` table; list them under \
                             `[dependencies]` instead",
                        ),
                    );
                }
            }
        }
    }
    // `[patch]` (and cargo's older `[replace]`) does not reach the hidden
    // crate yet, and inside a workspace cargo reads it from the root
    // manifest, whose members Varyk does not work out; an override that
    // silently did not apply would be worse than an error, so both are
    // refused for now.
    for table_name in OVERRIDE_TABLES {
        if let Some((key, _)) = entry(&doc, table_name) {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0401,
                    at(key.span()),
                    format!("`[{table_name}]` cannot be used yet"),
                )
                .with_note(
                    "Varyk builds the package in a crate of its own, where the override would \
                     not apply; support for `[patch]` is planned",
                ),
            );
        }
    }
    // A package with its own `[workspace]` table is its own root; cargo
    // looks no further up. One that names its root with `[package]
    // workspace = "path"` belongs to that one, wherever it is.
    let named_root = match entry(package, "workspace").map(|(_, value)| value.get_ref()) {
        Some(DeValue::String(dir)) => Some(root.join(dir.as_ref())),
        _ => None,
    };
    let workspace = match named_root {
        Some(dir) => {
            // Cargo refuses a package whose `workspace` names no workspace
            // root; so does `check`, before the key is dropped from the
            // isolated manifest (whether that root lists this package is
            // cargo's to say, at the package root).
            let table = fs::read_to_string(dir.join("Cargo.toml"))
                .ok()
                .and_then(|text| text.parse::<toml::Table>().ok())
                .filter(|table| table.get("workspace").is_some_and(|w| w.is_table()));
            let Some(table) = table else {
                diagnostics.push(
                    Diagnostic::new(
                        codes::V0403,
                        package_span,
                        format!(
                            "`workspace` names `{}`, which has no workspace manifest",
                            dir.display()
                        ),
                    )
                    .with_note(
                        "`[package] workspace` must point at a directory whose `Cargo.toml` \
                         has a `[workspace]` table",
                    ),
                );
                return Err(diagnostics);
            };
            Some((dir, table))
        }
        None if entry(&doc, "workspace").is_some() => None,
        None => {
            match workspace_root(&root) {
                // Cargo reads that manifest on the way up and refuses the
                // package when its `workspace` is not a table; so does `check`.
                Some((dir, table)) if !table["workspace"].is_table() => {
                    diagnostics.push(
                    Diagnostic::new(
                        codes::V0403,
                        package_span,
                        format!(
                            "the enclosing `{}` has a `workspace` that is not a table",
                            dir.join("Cargo.toml").display()
                        ),
                    )
                    .with_note("cargo would refuse this package; fix that manifest or move the package"),
                );
                    return Err(diagnostics);
                }
                found => found,
            }
        }
    };
    if let Some((dir, _)) = workspace.as_ref().filter(|(_, table)| has_override(table)) {
        diagnostics.push(
            Diagnostic::new(
                codes::V0401,
                package_span,
                format!(
                    "`[patch]` or `[replace]` in the enclosing workspace `{}` cannot be used yet",
                    dir.join("Cargo.toml").display()
                ),
            )
            .with_note(
                "Varyk builds the package in a crate of its own, where the workspace's override \
                 would not apply; support for `[patch]` is planned",
            ),
        );
    }
    let has_main = root.join("src/main.vr").is_file();
    let has_lib = root.join("src/lib.vr").is_file();
    // Cargo would also build what it finds by convention, and the crates
    // Varyk builds carry only the generated tree; rather than drop those
    // targets silently, they are refused. A Varyk package is one program or
    // one library.
    for (path, what) in [
        ("src/bin", "programs"),
        ("examples", "examples"),
        ("tests", "tests"),
        ("benches", "benches"),
    ] {
        if root.join(path).is_dir() {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0401,
                    package_span,
                    format!("`{path}/` is not supported yet"),
                )
                .with_note(format!(
                    "cargo would build the {what} in it as part of the package, and the crates \
                     `varyk build` makes and `varyk publish` assembles would not carry them; \
                     move them out of the package"
                )),
            );
        }
    }
    let (other_root, own_root) = if has_main {
        ("src/lib.rs", "src/main.vr")
    } else {
        ("src/main.rs", "src/lib.vr")
    };
    if (has_main || has_lib) && root.join(other_root).is_file() {
        diagnostics.push(
            Diagnostic::new(
                codes::V0401,
                package_span,
                format!("`{other_root}` beside `{own_root}` is not supported yet"),
            )
            .with_note(
                "cargo would build it as a second target of the package; a Varyk package is one \
                 program or one library",
            ),
        );
    }
    let kind = match (has_main, has_lib) {
        (true, false) => Kind::Binary,
        (false, true) => Kind::Library,
        (both, _) => {
            let message = if both {
                "this package has both `src/main.vr` and `src/lib.vr`; keep one"
            } else {
                "a `Cargo.toml` was found but the package has no `src/main.vr` or `src/lib.vr`"
            };
            diagnostics.push(
                Diagnostic::new(codes::V0403, package_span, message).with_note(
                    "a Varyk package is a program started from `src/main.vr` or a library \
                     in `src/lib.vr`",
                ),
            );
            Kind::Binary
        }
    };
    if has_main && !has_lib {
        if let Some(reserved) = program_name_problem(&name) {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0403,
                    package_span,
                    format!(
                        "this program cannot be called `{name}`: Cargo, Rust's build tool, keeps \
                         its own `{reserved}` folder where it would put the program"
                    ),
                )
                .with_note(
                    "pick another name, or make the package a library (`src/lib.vr`), which \
                     may use this name",
                ),
            );
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let manifest = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(err) => return Err(vec![malformed(&err, &at)]),
    };
    let std_spans = StdSpans {
        anchor: at(table_key_span(&doc, "dependencies").unwrap_or_else(|| package_key.span())),
        entry: table(&doc, "dependencies")
            .and_then(|(_, dependencies)| entry(dependencies, "varyk-std"))
            .map(|(key, _)| at(key.span())),
    };
    let dependencies = match manifest.get("dependencies") {
        Some(toml::Value::Table(dependencies)) => absolutize(dependencies.clone(), &root),
        _ => toml::Table::new(),
    };
    let dev_dependencies = match manifest.get("dev-dependencies") {
        Some(toml::Value::Table(dev)) => dev.keys().cloned().collect(),
        _ => Vec::new(),
    };
    // Cargo keeps one lock per workspace, at the root.
    let lock_dir = workspace
        .as_ref()
        .map_or(root.as_path(), |(dir, _)| dir.as_path());
    let lock = Some(lock_dir.join("Cargo.lock")).filter(|lock| lock.is_file());
    Ok(Package {
        entry: root.join(match kind {
            Kind::Binary => "src/main.vr",
            Kind::Library => "src/lib.vr",
        }),
        root,
        name,
        version,
        kind,
        dependencies,
        dev_dependencies,
        manifest,
        workspace,
        lock,
        std_spans,
    })
}

/// The span of the key `name` in `doc`, when there is one.
fn table_key_span(doc: &DeTable<'_>, name: &str) -> Option<Range<usize>> {
    entry(doc, name).map(|(key, _)| key.span())
}

/// The `varyk-std` checks of spec 5.1 (V0404), for a program that uses
/// `varyk-std` (`uses_std`); `compiler_version` is the compiler's own.
/// `varyk-std` must be in `[dependencies]` as a crates.io requirement on
/// the compiler's minor version, and a locked `varyk-std` must be no older
/// than the compiler. Never runs cargo.
pub fn check_std_dependency(
    package: &Package,
    uses_std: bool,
    compiler_version: &str,
) -> Vec<Diagnostic> {
    let Some(compiler) = version_triple(compiler_version).filter(|_| uses_std) else {
        return Vec::new();
    };
    let line = format!(
        "write `varyk-std = \"{}.{}.{}\"` under `[dependencies]` in `Cargo.toml`",
        compiler.0, compiler.1, compiler.2
    );
    let spans = package.std_spans;
    let at = spans.entry.unwrap_or(spans.anchor);
    let refuse =
        |message: &str| vec![Diagnostic::new(codes::V0404, at, message).with_note(line.clone())];
    match package.dependencies.get("varyk-std") {
        None => return refuse("this program needs `varyk-std`, which `Cargo.toml` does not list"),
        Some(toml::Value::String(requirement)) => {
            if !requirement_matches(requirement, compiler) {
                return refuse(&format!(
                    "`varyk-std` must be version {}.{}, the version of this compiler",
                    compiler.0, compiler.1
                ));
            }
        }
        Some(toml::Value::Table(dependency)) => {
            for (key, why) in [
                (
                    "path",
                    "`varyk-std` must come from crates.io, not from a path",
                ),
                ("git", "`varyk-std` must come from crates.io, not from git"),
                ("optional", "`varyk-std` cannot be optional"),
                ("package", "`varyk-std` cannot be renamed"),
            ] {
                // `optional = false` is an ordinary dependency.
                let harmless = key == "optional"
                    && matches!(dependency.get(key), Some(toml::Value::Boolean(false)));
                if dependency.contains_key(key) && !harmless {
                    return refuse(why);
                }
            }
            match dependency.get("version") {
                Some(toml::Value::String(requirement))
                    if requirement_matches(requirement, compiler) => {}
                Some(_) => {
                    return refuse(&format!(
                        "`varyk-std` must be version {}.{}, the version of this compiler",
                        compiler.0, compiler.1
                    ));
                }
                None => return refuse("`varyk-std` must say which version to use"),
            }
        }
        Some(_) => return refuse("`varyk-std` must be a version, such as the one below"),
    }
    if locked_older(package, compiler) {
        return vec![
            Diagnostic::new(
                codes::V0404,
                at,
                "`Cargo.lock` holds an older `varyk-std` than this compiler",
            )
            .with_note("run `cargo update -p varyk-std`"),
        ];
    }
    Vec::new()
}

/// `MAJOR.MINOR.PATCH`, ignoring any pre-release or build part.
fn version_triple(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}

/// Whether `requirement` is `MAJOR.MINOR` or `MAJOR.MINOR.PATCH`, with an
/// optional leading `^`, on the compiler's major and minor.
fn requirement_matches(requirement: &str, compiler: (u64, u64, u64)) -> bool {
    let text = requirement.strip_prefix('^').unwrap_or(requirement);
    let numbers: Vec<Option<u64>> = text
        .split('.')
        .map(|part| {
            let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
            if digits { part.parse().ok() } else { None }
        })
        .collect();
    matches!(
        numbers.as_slice(),
        [Some(major), Some(minor)] | [Some(major), Some(minor), Some(_)]
            if (*major, *minor) == (compiler.0, compiler.1)
    )
}

/// Whether `Cargo.lock` locks a `varyk-std` older than `compiler`.
fn locked_older(package: &Package, compiler: (u64, u64, u64)) -> bool {
    let Some(lock) = package
        .lock
        .as_ref()
        .and_then(|lock| fs::read_to_string(lock).ok())
        .and_then(|text| text.parse::<toml::Table>().ok())
    else {
        return false;
    };
    let Some(toml::Value::Array(locked)) = lock.get("package") else {
        return false;
    };
    locked.iter().any(|entry| {
        entry.get("name").and_then(toml::Value::as_str) == Some("varyk-std")
            && entry
                .get("version")
                .and_then(toml::Value::as_str)
                .and_then(version_triple)
                .is_some_and(|version| version < compiler)
    })
}

/// V0403 for a manifest that is not valid TOML, at the parser's span,
/// with its reason.
fn malformed(err: &toml::de::Error, at: &impl Fn(Range<usize>) -> Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0403,
        at(err.span().unwrap_or(0..0)),
        format!(
            "this `Cargo.toml` is not valid TOML: {}",
            err.message().trim_end()
        ),
    )
}

/// Why `name` cannot name a package, if it cannot: Cargo's rule (ASCII
/// letters, digits, `-`, `_`, not starting with a digit), which also keeps
/// it a single directory name for [`Package::hidden_crate_dir`]; and
/// `cache`, which is the shared target directory beside the hidden crate,
/// and `package`, where `varyk publish` assembles its crate.
pub(crate) fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("a name cannot be empty")
    } else if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Some("a name can only use letters, digits, `-`, and `_`")
    } else if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        Some("a name must start with a letter or `_`")
    } else if name.eq_ignore_ascii_case("cache") {
        Some("Varyk keeps its build cache in `target/varyk/cache`")
    } else if name.eq_ignore_ascii_case("package") {
        Some(
            "Varyk builds a package in `target/varyk/<name>` and assembles `varyk publish`'s \
             crate in `target/varyk/package`, so the two would share a directory",
        )
    } else {
        None
    }
}

/// The folder of Cargo's own that a program (a binary package) called
/// `name` would collide with, if any: Cargo forbids a binary named `deps`,
/// `examples`, `build`, or `incremental`, since the binary sits beside
/// those folders. Compared ignoring ASCII case, because on a filesystem
/// that ignores case (macOS, Windows) `Deps` collides with `deps` too. A
/// library may use these names.
pub(crate) fn program_name_problem(name: &str) -> Option<&'static str> {
    ["deps", "examples", "build", "incremental"]
        .into_iter()
        .find(|reserved| name.eq_ignore_ascii_case(reserved))
}

/// Every top-level key `check` accepts; the rest is V0401 (a target
/// table, V0402).
const TOP_LEVEL_KEYS: [&str; 12] = [
    "package",
    "workspace",
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "target",
    "features",
    "profile",
    "patch",
    "replace",
    "lints",
    "badges",
];

/// Cargo's target tables: none is supported, since a Varyk package has
/// exactly the one root cargo finds by its defaults.
const TARGET_TABLES: [&str; 5] = ["lib", "bin", "example", "test", "bench"];

/// Every `[package]` key `check` accepts, with the shape of its value;
/// any other key is V0401, a wrong shape V0403. The target-layout keys
/// (`autobins` and the other `auto*` flags, `default-run`) are left out on
/// purpose: a Varyk package has one root, found by cargo's defaults.
const PACKAGE_KEYS: [(&str, Shape); 22] = [
    ("name", Shape::Text),
    ("version", Shape::Text),
    ("edition", Shape::Text),
    ("rust-version", Shape::Text),
    ("authors", Shape::Texts),
    ("description", Shape::Text),
    ("documentation", Shape::Text),
    ("readme", Shape::TextOrFlag),
    ("homepage", Shape::Text),
    ("repository", Shape::Text),
    ("license", Shape::Text),
    ("license-file", Shape::Text),
    ("keywords", Shape::Texts),
    ("categories", Shape::Texts),
    ("workspace", Shape::Text),
    ("build", Shape::TextOrFlag),
    ("links", Shape::Text),
    ("exclude", Shape::Texts),
    ("include", Shape::Texts),
    ("publish", Shape::FlagOrTexts),
    ("metadata", Shape::Table),
    ("resolver", Shape::Text),
];

/// The value shapes cargo accepts for a `[package]` key.
#[derive(Clone, Copy)]
enum Shape {
    Text,
    TextOrFlag,
    Texts,
    FlagOrTexts,
    Table,
}

impl Shape {
    fn accepts(self, value: &DeValue) -> bool {
        let texts = |value: &DeValue| {
            matches!(value, DeValue::Array(items)
                if items.iter().all(|item| matches!(item.get_ref(), DeValue::String(_))))
        };
        match self {
            Shape::Text => matches!(value, DeValue::String(_)),
            Shape::TextOrFlag => matches!(value, DeValue::String(_) | DeValue::Boolean(_)),
            Shape::Texts => texts(value),
            Shape::FlagOrTexts => matches!(value, DeValue::Boolean(_)) || texts(value),
            Shape::Table => matches!(value, DeValue::Table(_)),
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Shape::Text => "text in quotes",
            Shape::TextOrFlag => "text in quotes, or `true` or `false`",
            Shape::Texts => "a list of texts in quotes",
            Shape::FlagOrTexts => "`true`, `false`, or a list of texts in quotes",
            Shape::Table => "a table",
        }
    }
}

/// The top-level keys cargo reads as tables (`lib` and the target arrays
/// are checked with the targets).
const TABLE_KEYS: [&str; 12] = [
    "package",
    "workspace",
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "target",
    "features",
    "profile",
    "patch",
    "replace",
    "lints",
    "badges",
];

/// The dependency-override tables cargo reads from a package or a
/// workspace root: neither reaches the hidden crate yet.
const OVERRIDE_TABLES: [&str; 2] = ["patch", "replace"];

/// Whether `manifest` has one of the [`OVERRIDE_TABLES`].
fn has_override(manifest: &toml::Table) -> bool {
    OVERRIDE_TABLES
        .iter()
        .any(|table| manifest.get(*table).is_some())
}

/// The workspace the package at `root` belongs to, as cargo finds it
/// when the package does not name one: the nearest ancestor directory
/// whose `Cargo.toml` has a `[workspace]` table and does not `exclude`
/// the package (cargo skips one that does and keeps looking up), with
/// that manifest parsed. `None` when there is none or it cannot be read.
/// Whether that root lists the package as a member is cargo's to say.
fn workspace_root(root: &Path) -> Option<(PathBuf, toml::Table)> {
    let package_dir = fs::canonicalize(root).ok()?;
    let excludes = |dir: &Path, table: &toml::Table| {
        table
            .get("workspace")
            .and_then(|workspace| workspace.get("exclude"))
            .and_then(|exclude| exclude.as_array())
            .is_some_and(|excluded| {
                excluded.iter().filter_map(|e| e.as_str()).any(|e| {
                    fs::canonicalize(dir.join(e)).is_ok_and(|e| package_dir.starts_with(e))
                })
            })
    };
    package_dir
        .ancestors()
        .skip(1)
        .map(|dir| dir.join("Cargo.toml"))
        .filter_map(|manifest| {
            let table: toml::Table = fs::read_to_string(&manifest).ok()?.parse().ok()?;
            Some((manifest, table))
        })
        .find(|(manifest, table)| {
            table.get("workspace").is_some()
                && !manifest.parent().is_some_and(|dir| excludes(dir, table))
        })
        .map(|(manifest, table)| {
            (
                manifest.parent().expect("a manifest path").to_path_buf(),
                table,
            )
        })
}

/// Whether `text` is a `rust-version` cargo accepts: one to three numbers
/// (`1`, `1.85`, `1.85.0`), digits without a leading zero, no pre-release
/// or build part.
fn is_partial_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    (1..=3).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
                && part.parse::<u64>().is_ok()
        })
}

/// Whether `text` is a version cargo accepts: `MAJOR.MINOR.PATCH` in
/// digits without leading zeros, then optionally `-` and dot-separated
/// pre-release identifiers and `+` and dot-separated build identifiers,
/// each of ASCII letters, digits, and `-`.
fn is_version(text: &str) -> bool {
    let (rest, build) = match text.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (text, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    // A number: digits without a leading zero. The three core numbers
    // must also fit cargo's `u64`; a pre-release number has no bound.
    let number = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let numeric = |part: &str| number(part) && part.parse::<u64>().is_ok();
    // A pre-release identifier made of digits is a number and may not
    // have a leading zero; a build identifier may (cargo 1.85 rejects
    // `1.2.3-01` and accepts `1.2.3+007`).
    let identifiers = |list: &str, numbers: bool| {
        list.split('.').all(|id| {
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && (!numbers || !id.bytes().all(|b| b.is_ascii_digit()) || number(id))
        })
    };
    let mut parts = core.split('.');
    let three = parts.by_ref().take(3).filter(|part| numeric(part)).count() == 3;
    three
        && parts.next().is_none()
        && pre.is_none_or(|pre| identifiers(pre, true))
        && build.is_none_or(|build| identifiers(build, false))
}

/// V0401 when `value`, the manifest's `key`, is a table naming
/// `workspace` (inheriting from a workspace, spec 2.2).
fn check_not_inherited(
    key: &str,
    value: &Spanned<DeValue>,
    at: &impl Fn(Range<usize>) -> Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let DeValue::Table(value) = value.get_ref() else {
        return;
    };
    if let Some((workspace, _)) = entry(value, "workspace") {
        diagnostics.push(
            Diagnostic::new(
                codes::V0401,
                at(workspace.span()),
                format!("`{key}` cannot be taken from the workspace yet; write it out here"),
            )
            .with_note(
                "`workspace = true` inherits a setting from a Cargo workspace, which Varyk does \
                 not support yet",
            ),
        );
    }
}

/// The entry `key` of `table`, with its key and value spans.
fn entry<'t, 'i>(
    table: &'t DeTable<'i>,
    key: &str,
) -> Option<(&'t Spanned<DeString<'i>>, &'t Spanned<DeValue<'i>>)> {
    table.iter().find(|(name, _)| name.get_ref() == key)
}

/// The entry `key` of `table` when it is a table itself.
fn table<'t, 'i>(
    table: &'t DeTable<'i>,
    key: &str,
) -> Option<(&'t Spanned<DeString<'i>>, &'t DeTable<'i>)> {
    let (key, value) = entry(table, key)?;
    match value.get_ref() {
        DeValue::Table(inner) => Some((key, inner)),
        _ => None,
    }
}

/// `dependencies` with every relative `path` made absolute against
/// `root`, so the hidden crate (elsewhere) finds the same crates.
pub(crate) fn absolutize(mut dependencies: toml::Table, root: &Path) -> toml::Table {
    let base = if root.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf())
    };
    for (_, value) in dependencies.iter_mut() {
        let Some(path) = value.get_mut("path") else {
            continue;
        };
        let Some(relative) = path.as_str().filter(|path| Path::new(path).is_relative()) else {
            continue;
        };
        *path = toml::Value::String(base.join(relative).to_string_lossy().into_owned());
    }
    dependencies
}

impl Package {
    /// The spec 2.1 rule for a file argument: look for `Cargo.toml`
    /// upward from `file`; if `file` is that package's `src/main.vr` or
    /// `src/lib.vr`, the package (loaded, its entry spelled as `file`);
    /// anything else is `None`, single-file mode.
    pub fn for_file(
        file: &Path,
        sources: &mut Vec<SourceFile>,
    ) -> Option<Result<Package, Vec<Diagnostic>>> {
        let manifest = find(file)?;
        let root = manifest.parent().unwrap_or(Path::new(""));
        let file_canonical = file.canonicalize().ok()?;
        let is_root = ["src/main.vr", "src/lib.vr"]
            .iter()
            .any(|name| root.join(name).canonicalize().ok().as_ref() == Some(&file_canonical));
        if !is_root {
            return None;
        }
        Some(load(&manifest, sources).map(|mut package| {
            package.entry = file.to_path_buf();
            package
        }))
    }

    /// The manifest of every crate Varyk builds from this package, the
    /// hidden one under `target/varyk/<name>` and the one `varyk publish`
    /// assembles alike, so the two cannot disagree: the package's own
    /// `Cargo.toml` with `[package] build = false` (neither crate runs a
    /// build script), the keys that describe the source layout dropped
    /// (`include`, `exclude`: the tree is a different layout, and a
    /// filter written for `.vr` sources would hide the generated
    /// `src/main.rs` or `src/lib.rs`; `workspace`, which points at a root
    /// these crates must not join, and which cargo refuses beside a
    /// `[workspace]` table), `[dependencies]` as
    /// [`Self::dependencies`] (paths absolute), relative `path`s in the
    /// other dependency tables, at the top level and under each
    /// `[target.'cfg(..)']`, made absolute the same way, and an empty
    /// `[workspace]` table so an enclosing workspace does not claim the
    /// crate. Everything else (`rust-version`, `[features]`,
    /// `[profile.*]`, `license`, ...) is cargo's to read, as written.
    pub fn isolated_manifest(&self) -> toml::Table {
        let mut manifest = self.manifest.clone();
        if let toml::Value::Table(package_table) = manifest
            .entry("package")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        {
            package_table.insert("build".to_string(), toml::Value::Boolean(false));
            for key in ["include", "exclude", "workspace"] {
                package_table.remove(key);
            }
        }
        manifest.insert(
            "dependencies".to_string(),
            toml::Value::Table(self.dependencies.clone()),
        );
        fn absolutize_auxiliary(owner: &mut toml::Table, root: &Path) {
            for key in ["dev-dependencies", "build-dependencies"] {
                if let Some(toml::Value::Table(table)) = owner.get_mut(key) {
                    *table = absolutize(std::mem::take(table), root);
                }
            }
        }
        absolutize_auxiliary(&mut manifest, &self.root);
        if let Some(toml::Value::Table(targets)) = manifest.get_mut("target") {
            for (_, target) in targets.iter_mut() {
                if let toml::Value::Table(target) = target {
                    absolutize_auxiliary(target, &self.root);
                }
            }
        }
        // Cargo reads `[profile.*]` and the resolver from the workspace
        // root and ignores a member's own; a standalone package's own
        // settings stay. The `[workspace]` table is emptied (no members, no
        // root above) apart from that resolver.
        let settings = self
            .workspace
            .as_ref()
            .map_or(&self.manifest, |(_, table)| table);
        match settings.get("profile").cloned() {
            Some(profile) => manifest.insert("profile".to_string(), profile),
            None => manifest.remove("profile"),
        };
        let resolver = settings
            .get("workspace")
            .and_then(|workspace| workspace.get("resolver"))
            .cloned();
        let mut workspace = toml::Table::new();
        if let Some(resolver) = resolver {
            workspace.insert("resolver".to_string(), resolver);
        }
        manifest.insert("workspace".to_string(), toml::Value::Table(workspace));
        manifest
    }

    /// `<root>/target/varyk/<name>`: the hidden crate (spec 2.4).
    pub fn hidden_crate_dir(&self) -> PathBuf {
        self.root.join("target/varyk").join(&self.name)
    }

    /// `<root>/target/varyk/cache`: the shared cargo target directory.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("target/varyk/cache")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::codes;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory removed when it goes out of scope.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "varyk_package_test_{label}_{}_{n}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }

        fn write(&self, path: &str, text: &str) -> PathBuf {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const MANIFEST: &str = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

    /// A package in a fresh directory with `manifest` and the given root.
    fn package(label: &str, manifest: &str, root: &str) -> TempDir {
        let dir = TempDir::new(label);
        dir.write("Cargo.toml", manifest);
        dir.write(root, "fn main() {}\n");
        dir
    }

    fn load_codes(manifest: &str) -> Vec<&'static str> {
        let dir = package("codes", manifest, "src/main.vr");
        let mut sources = Vec::new();
        match load(&dir.0.join("Cargo.toml"), &mut sources) {
            Ok(_) => Vec::new(),
            Err(diagnostics) => diagnostics.iter().map(|d| d.code).collect(),
        }
    }

    #[test]
    fn find_walks_up_and_stops_at_the_first_manifest() {
        let dir = TempDir::new("find");
        dir.write("Cargo.toml", MANIFEST);
        let inner = dir.write("a/Cargo.toml", MANIFEST);
        dir.write("a/b/c/file.vr", "");

        assert_eq!(find(&dir.0.join("a/b/c/file.vr")), Some(inner));
    }

    #[test]
    fn load_reads_the_name_version_and_kind() {
        let dir = package("load", MANIFEST, "src/lib.vr");
        let mut sources = Vec::new();

        let package = load(&dir.0.join("Cargo.toml"), &mut sources).expect("loads");

        assert_eq!(package.name, "shop");
        assert_eq!(package.version, "0.1.0");
        assert_eq!(package.kind, Kind::Library);
        assert_eq!(package.entry, dir.0.join("src/lib.vr"));
        assert_eq!(package.lock, None);
        assert_eq!(sources.len(), 1, "the manifest is a source file");
    }

    /// V0404 messages for a package whose `[dependencies]` is `deps`, a
    /// compiler at `compiler`, and the given lock text.
    fn std_check(
        deps: &str,
        lock: Option<&str>,
        uses_std: bool,
        compiler: &str,
    ) -> Vec<Diagnostic> {
        let dir = package("std", &format!("{MANIFEST}{deps}"), "src/main.vr");
        if let Some(lock) = lock {
            dir.write("Cargo.lock", lock);
        }
        let mut sources = Vec::new();
        let package = load(&dir.0.join("Cargo.toml"), &mut sources).expect("loads");
        check_std_dependency(&package, uses_std, compiler)
    }

    fn accepted(deps: &str) -> bool {
        std_check(deps, None, true, "0.2.5").is_empty()
    }

    const STD_LOCK: &str =
        "version = 4\n\n[[package]]\nname = \"varyk-std\"\nversion = \"0.2.0\"\n";

    #[test]
    fn a_missing_std_dependency_is_v0404_with_the_line_to_write() {
        let found = std_check("[dependencies]\n", None, true, "0.2.5");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code, codes::V0404);
        assert!(found[0].notes[0].contains("varyk-std = \"0.2.5\""));
        assert!(found[0].fix_it.is_none());
        assert_eq!(
            std_check("", None, true, "0.2.5").len(),
            1,
            "no table at all"
        );
    }

    #[test]
    fn std_requirements_on_the_compilers_minor_are_accepted() {
        for req in [
            "\"0.2\"",
            "\"0.2.0\"",
            "\"^0.2\"",
            "\"0.2.9\"",
            "\"^0.2.1\"",
            "{ version = \"0.2\" }",
            "{ version = \"0.2.0\", features = [\"x\"] }",
            "{ version = \"0.2\", optional = false }",
        ] {
            assert!(
                accepted(&format!("[dependencies]\nvaryk-std = {req}\n")),
                "{req}"
            );
        }
    }

    #[test]
    fn other_std_requirements_are_v0404() {
        for req in [
            "\"0.1\"",
            "\"0.3\"",
            "\"~0.2\"",
            "\"=0.2.0\"",
            "\">=0.2\"",
            "\"*\"",
            "\"0\"",
            "\"1.2\"",
            "\"0.2.x\"",
            "\"0.2, 0.3\"",
            "3",
            "{ path = \"../std\" }",
            "{ version = \"0.2\", path = \"../std\" }",
            "{ git = \"https://x/y\" }",
            "{ version = \"0.2\", optional = true }",
            "{ version = \"0.2\", package = \"x\" }",
            "{ features = [\"x\"] }",
            "{ version = \"~0.2\" }",
        ] {
            let found = std_check(
                &format!("[dependencies]\nvaryk-std = {req}\n"),
                None,
                true,
                "0.2.5",
            );
            assert_eq!(found.len(), 1, "{req}");
            assert_eq!(found[0].code, codes::V0404, "{req}");
        }
    }

    #[test]
    fn the_std_requirement_follows_the_compilers_version_argument() {
        let deps = "[dependencies]\nvaryk-std = \"0.3\"\n";
        assert!(std_check(deps, None, true, "0.3.1").is_empty());
        assert_eq!(std_check(deps, None, true, "0.2.0").len(), 1);
    }

    #[test]
    fn a_lock_older_than_the_compiler_is_v0404_with_cargo_update() {
        let deps = "[dependencies]\nvaryk-std = \"0.2\"\n";
        let found = std_check(deps, Some(STD_LOCK), true, "0.2.1");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code, codes::V0404);
        assert_eq!(found[0].notes, ["run `cargo update -p varyk-std`"]);
        assert!(
            std_check(deps, Some(STD_LOCK), true, "0.2.0").is_empty(),
            "equal is fine"
        );
        assert!(
            std_check(deps, None, true, "0.2.1").is_empty(),
            "no lock is fine"
        );
        let other = "version = 4\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n";
        assert!(std_check(deps, Some(other), true, "0.2.1").is_empty());
    }

    #[test]
    fn a_package_that_does_not_use_std_is_never_checked() {
        assert!(
            std_check(
                "[dependencies]\nvaryk-std = \"0.1\"\n",
                Some(STD_LOCK),
                false,
                "0.2.1"
            )
            .is_empty()
        );
        assert!(std_check("", None, false, "0.2.1").is_empty());
    }

    #[test]
    fn load_finds_the_lock_beside_the_manifest() {
        let dir = package("lock", MANIFEST, "src/main.vr");
        let lock = dir.write("Cargo.lock", "version = 4\n");
        let mut sources = Vec::new();

        let package = load(&dir.0.join("Cargo.toml"), &mut sources).expect("loads");

        assert_eq!(package.kind, Kind::Binary);
        assert_eq!(package.lock, Some(lock));
    }

    #[test]
    fn edition_2021_is_v0400() {
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
        assert_eq!(load_codes(manifest), vec![codes::V0400]);
    }

    #[test]
    fn a_missing_edition_is_v0400() {
        assert_eq!(
            load_codes("[package]\nname = \"shop\"\nversion = \"0.1.0\"\n"),
            vec![codes::V0400]
        );
    }

    #[test]
    fn workspace_true_on_a_dependency_is_v0401() {
        let manifest = format!("{MANIFEST}\n[dependencies]\nregex = {{ workspace = true }}\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);
    }

    #[test]
    fn workspace_true_on_the_package_version_is_v0401() {
        let manifest = "[package]\nname = \"shop\"\nversion.workspace = true\nedition = \"2024\"\n";
        assert_eq!(load_codes(manifest), vec![codes::V0401]);
    }

    #[test]
    fn target_specific_dependencies_are_v0401() {
        let manifest = format!("{MANIFEST}\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);
    }

    #[test]
    fn a_version_cargo_rejects_is_v0403_and_a_missing_one_is_fine() {
        for bad in [
            "garbage",
            "1.2",
            "01.2.3",
            "1.2.3.4",
            "1.2.3-",
            "1.2.3+a..b",
            "1.2.3-01",
            "1.2.3-rc.01",
            "18446744073709551616.0.0",
        ] {
            let manifest =
                format!("[package]\nname = \"shop\"\nversion = \"{bad}\"\nedition = \"2024\"\n");
            assert_eq!(load_codes(&manifest), vec![codes::V0403], "{bad}");
        }
        assert_eq!(
            load_codes("[package]\nname = \"shop\"\nversion = 1\nedition = \"2024\"\n"),
            vec![codes::V0403]
        );
        for good in [
            "0.1.0",
            "1.2.3-alpha.1+build-5",
            "10.0.0-rc-1",
            "1.2.3-0.a01+007",
            "1.2.3-18446744073709551616",
        ] {
            let manifest =
                format!("[package]\nname = \"shop\"\nversion = \"{good}\"\nedition = \"2024\"\n");
            assert_eq!(load_codes(&manifest), Vec::<&str>::new(), "{good}");
        }
        assert_eq!(
            load_codes("[package]\nname = \"shop\"\nedition = \"2024\"\n"),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn workspace_true_on_a_dev_dependency_is_v0401() {
        let manifest = format!("{MANIFEST}\n[dev-dependencies]\nx = {{ workspace = true }}\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);
        let manifest = format!("{MANIFEST}\n[build-dependencies]\nx = {{ workspace = true }}\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);
    }

    #[test]
    fn a_patch_table_in_the_package_or_its_workspace_is_v0401() {
        let manifest =
            format!("{MANIFEST}\n[patch.crates-io]\nhelper = {{ path = \"../helper\" }}\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);
        let manifest =
            format!("{MANIFEST}\n[replace]\n\"helper:1.0.0\" = {{ path = \"../helper\" }}\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);

        let dir = TempDir::new("workspace_patch");
        dir.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\"]\n\n[patch.crates-io]\nhelper = { path = \"vendor/helper\" }\n",
        );
        dir.write("app/Cargo.toml", MANIFEST);
        dir.write("app/src/main.vr", "fn main() {}\n");
        let diagnostics = load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![codes::V0401]
        );
        assert!(
            diagnostics[0].message.contains("enclosing workspace"),
            "{}",
            diagnostics[0].message
        );

        // A package naming its root with `workspace = "path"` belongs to that
        // root, a sibling here, and its patch counts.
        dir.write(
            "ws/Cargo.toml",
            "[workspace]\nmembers = [\"../app\"]\n\n[patch.crates-io]\nhelper = { path = \"vendor/helper\" }\n",
        );
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"app\"]\n");
        dir.write(
            "app/Cargo.toml",
            &MANIFEST.replace("[package]\n", "[package]\nworkspace = \"../ws\"\n"),
        );
        let diagnostics = load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![codes::V0401]
        );
        dir.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\"]\n\n[patch.crates-io]\nhelper = { path = \"vendor/helper\" }\n",
        );

        // A named root whose `workspace` is not a table is no workspace either.
        dir.write("ws/Cargo.toml", "workspace = \"bad\"\n");
        dir.write(
            "app/Cargo.toml",
            &MANIFEST.replace("[package]\n", "[package]\nworkspace = \"../ws\"\n"),
        );
        let diagnostics = load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![codes::V0403]
        );

        // An ancestor whose `workspace` is not a table is a manifest cargo
        // refuses on the way up; so does `check`.
        dir.write("Cargo.toml", "workspace = \"bad\"\n");
        dir.write("app/Cargo.toml", MANIFEST);
        let diagnostics = load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![codes::V0403]
        );
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"app\"]\n");

        // A named root that is not a workspace is refused, as cargo refuses it.
        dir.write(
            "app/Cargo.toml",
            &MANIFEST.replace("[package]\n", "[package]\nworkspace = \"../nowhere\"\n"),
        );
        let diagnostics = load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![codes::V0403]
        );
        dir.write("app/Cargo.toml", MANIFEST);

        // A package that is its own workspace root looks no further up.
        dir.write("app/Cargo.toml", &format!("{MANIFEST}\n[workspace]\n"));
        assert!(load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).is_ok());
        dir.write("app/Cargo.toml", MANIFEST);

        // A workspace that excludes the package leaves it standalone, patch or not.
        dir.write(
            "Cargo.toml",
            "[workspace]\nexclude = [\"app\"]\n\n[patch.crates-io]\nhelper = { path = \"vendor/helper\" }\n",
        );
        assert!(load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).is_ok());

        // A workspace without a patch is fine.
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"app\"]\n");
        assert!(load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).is_ok());
    }

    #[test]
    fn workspace_true_under_a_target_table_is_v0401() {
        for kind in ["dev-dependencies", "build-dependencies"] {
            let manifest =
                format!("{MANIFEST}\n[target.'cfg(unix)'.{kind}]\nx = {{ workspace = true }}\n");
            assert_eq!(load_codes(&manifest), vec![codes::V0401], "{kind}");
        }
    }

    #[test]
    fn a_lints_table_or_a_custom_build_script_is_v0401() {
        let manifest = format!("{MANIFEST}\n[lints]\nworkspace = true\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);
        let manifest = format!("{MANIFEST}\n[lints.rust]\nunsafe_code = \"forbid\"\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                        build = \"custom.rs\"\n";
        assert_eq!(load_codes(manifest), vec![codes::V0401]);
        // The script `init` writes may be named explicitly; turning the
        // script off would break plain `cargo build` of that package.
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                        build = \"build.rs\"\n";
        assert_eq!(load_codes(manifest), Vec::<&str>::new());
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                        build = false\n";
        assert_eq!(load_codes(manifest), vec![codes::V0401]);
    }

    #[test]
    fn the_isolated_manifest_keeps_cargo_settings_and_drops_the_layout() {
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                        rust-version = \"1.85\"\ninclude = [\"src/**\"]\n\
                        exclude = [\"notes/\"]\nworkspace = \"..\"\n\n\
                        [workspace]\nresolver = \"2\"\n\n\
                        [dependencies]\nhelper = { path = \"../helper\" }\n\n\
                        [build-dependencies]\ngen = { path = \"../gen\" }\n\n\
                        [target.'cfg(unix)'.dev-dependencies]\nprobe = { path = \"../probe\" }\n\n\
                        [profile.release]\nlto = true\n";
        // The package names its root, which must be a workspace; the root's
        // profiles and resolver are the ones cargo, and so Varyk, use.
        let dir = TempDir::new("isolated");
        dir.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"pkg\"]\nresolver = \"2\"\n\n[profile.release]\nlto = true\n",
        );
        dir.write("pkg/Cargo.toml", manifest);
        dir.write("pkg/src/main.vr", "fn main() {}\n");
        let package = load(&dir.0.join("pkg/Cargo.toml"), &mut Vec::new()).expect("loads");

        let isolated = package.isolated_manifest();

        assert_eq!(isolated["package"]["rust-version"].as_str(), Some("1.85"));
        assert_eq!(isolated["package"]["build"].as_bool(), Some(false));
        for key in ["include", "exclude", "workspace"] {
            assert!(isolated["package"].get(key).is_none(), "{key} was kept");
        }
        assert_eq!(isolated["workspace"]["resolver"].as_str(), Some("2"));
        assert_eq!(isolated["workspace"].as_table().unwrap().len(), 1);
        assert_eq!(isolated["profile"]["release"]["lto"].as_bool(), Some(true));
        for path in [
            isolated["dependencies"]["helper"]["path"].as_str().unwrap(),
            isolated["build-dependencies"]["gen"]["path"]
                .as_str()
                .unwrap(),
            isolated["target"]["cfg(unix)"]["dev-dependencies"]["probe"]["path"]
                .as_str()
                .unwrap(),
        ] {
            assert!(Path::new(path).is_absolute(), "{path}");
        }
    }

    #[test]
    fn a_workspace_member_takes_the_lock_profile_and_resolver_from_the_root() {
        let dir = TempDir::new("member");
        dir.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\"]\nresolver = \"2\"\n\n[profile.release]\nlto = true\n",
        );
        dir.write("Cargo.lock", "# root lock\n");
        dir.write(
            "app/Cargo.toml",
            &format!("{MANIFEST}\n[profile.release]\nlto = false\n"),
        );
        dir.write("app/Cargo.lock", "# stray member lock\n");
        dir.write("app/src/main.vr", "fn main() {}\n");

        let package = load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).expect("loads");

        // The root is found through canonical paths.
        assert_eq!(
            package
                .lock
                .as_deref()
                .map(|lock| fs::canonicalize(lock).unwrap()),
            Some(fs::canonicalize(dir.0.join("Cargo.lock")).unwrap())
        );
        let isolated = package.isolated_manifest();
        assert_eq!(isolated["profile"]["release"]["lto"].as_bool(), Some(true));
        assert_eq!(isolated["workspace"]["resolver"].as_str(), Some("2"));

        // Excluded, the package stands alone and its own settings hold.
        dir.write(
            "Cargo.toml",
            "[workspace]\nexclude = [\"app\"]\nresolver = \"2\"\n\n[profile.release]\nlto = true\n",
        );
        let package = load(&dir.0.join("app/Cargo.toml"), &mut Vec::new()).expect("loads");
        assert_eq!(
            package.lock.as_deref(),
            Some(dir.0.join("app/Cargo.lock").as_path())
        );
        let isolated = package.isolated_manifest();
        assert_eq!(isolated["profile"]["release"]["lto"].as_bool(), Some(false));
        assert!(isolated["workspace"].as_table().unwrap().is_empty());
    }

    #[test]
    fn an_unknown_manifest_key_is_v0401_and_the_common_ones_pass() {
        let manifest = format!("{MANIFEST}\n[cargo-features]\nx = 1\n");
        assert_eq!(load_codes(&manifest), vec![codes::V0401]);
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                        im-a-typo = true\n";
        assert_eq!(load_codes(manifest), vec![codes::V0401]);
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                        rust-version = \"1.85\"\nauthors = [\"a\"]\ndescription = \"d\"\n\
                        documentation = \"https://d\"\nreadme = \"README.md\"\nhomepage = \"https://h\"\n\
                        repository = \"https://r\"\nlicense = \"MIT\"\nkeywords = [\"k\"]\n\
                        categories = [\"c\"]\npublish = false\nresolver = \"2\"\n\n\
                        [package.metadata.varyk]\n\n[dependencies]\n\n[dev-dependencies]\n\n\
                        [build-dependencies]\n\n[features]\n\n[profile.release]\nlto = true\n\n\
                        [badges]\n";
        assert_eq!(load_codes(manifest), Vec::<&str>::new());
    }

    #[test]
    fn a_rust_version_cargo_rejects_is_v0403() {
        for bad in ["banana", "1.85.0-beta", "01.2", "1.2.3.4", ""] {
            let manifest = format!(
                "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                 rust-version = \"{bad}\"\n"
            );
            assert_eq!(load_codes(&manifest), vec![codes::V0403], "{bad}");
        }
        for good in ["1", "1.85", "1.85.0"] {
            let manifest = format!(
                "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                 rust-version = \"{good}\"\n"
            );
            assert_eq!(load_codes(&manifest), Vec::<&str>::new(), "{good}");
        }
    }

    #[test]
    fn a_package_key_of_the_wrong_shape_is_v0403() {
        for bad in [
            "license = 1",
            "keywords = \"k\"",
            "publish = \"no\"",
            "metadata = 1",
        ] {
            let manifest = format!(
                "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{bad}\n"
            );
            assert_eq!(load_codes(&manifest), vec![codes::V0403], "{bad}");
        }
        for good in [
            "readme = false",
            "publish = [\"crates-io\"]",
            "build = \"build.rs\"",
        ] {
            let manifest = format!(
                "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{good}\n"
            );
            assert_eq!(load_codes(&manifest), Vec::<&str>::new(), "{good}");
        }
    }

    #[test]
    fn a_target_cargo_would_discover_by_convention_is_v0401() {
        for path in [
            "src/bin/tool.rs",
            "examples/demo.rs",
            "tests/it.rs",
            "benches/perf.rs",
        ] {
            let dir = package("conventional", MANIFEST, "src/main.vr");
            dir.write(path, "fn main() {}\n");
            let diagnostics = load(&dir.0.join("Cargo.toml"), &mut Vec::new()).unwrap_err();
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                vec![codes::V0401],
                "{path}"
            );
        }
        // A plain file by one of those names is not a target directory.
        let dir = package("file_named_tests", MANIFEST, "src/main.vr");
        dir.write("tests", "notes\n");
        assert!(load(&dir.0.join("Cargo.toml"), &mut Vec::new()).is_ok());
        // The other kind's root file is a second target; the stub of the
        // package's own kind is expected.
        let dir = package("second_root", MANIFEST, "src/main.vr");
        dir.write("src/main.rs", "// stub\n");
        assert!(load(&dir.0.join("Cargo.toml"), &mut Vec::new()).is_ok());
        dir.write("src/lib.rs", "// a library too\n");
        let diagnostics = load(&dir.0.join("Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![codes::V0401]
        );
    }

    #[test]
    fn a_target_table_or_a_layout_key_is_not_supported() {
        for table in [
            "[[bin]]\nname = \"shop\"",
            "[[test]]\nname = \"it\"",
            "[[example]]\nname = \"x\"",
        ] {
            let manifest = format!("{MANIFEST}\n{table}\n");
            assert_eq!(load_codes(&manifest), vec![codes::V0402], "{table}");
        }
        let manifest = format!("{MANIFEST}\n[lib]\nname = \"shop_lib\"\n");
        let dir = package("lib_table", &manifest, "src/lib.vr");
        let diagnostics = load(&dir.0.join("Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![codes::V0402]
        );
        for key in [
            "autobins = false",
            "autotests = true",
            "default-run = \"shop\"",
        ] {
            let manifest = format!(
                "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{key}\n"
            );
            assert_eq!(load_codes(&manifest), vec![codes::V0401], "{key}");
        }
    }

    #[test]
    fn a_top_level_key_that_is_not_a_table_is_v0403() {
        for key in ["workspace", "dependencies", "features", "profile"] {
            let manifest = format!("{key} = \"not-a-table\"\n{MANIFEST}");
            let codes = load_codes(&manifest);
            assert!(codes.contains(&codes::V0403), "{key}: {codes:?}");
        }
    }

    #[test]
    fn a_links_key_is_v0401() {
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                        links = \"foo\"\n";
        assert_eq!(load_codes(manifest), vec![codes::V0401]);
    }

    #[test]
    fn a_package_with_both_roots_is_v0403_at_the_name() {
        let dir = package("both", MANIFEST, "src/main.vr");
        dir.write("src/lib.vr", "pub fn f() {}\n");

        let diagnostics = load(&dir.0.join("Cargo.toml"), &mut Vec::new()).unwrap_err();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, codes::V0403);
        let span = diagnostics[0].span;
        assert_eq!(
            &MANIFEST[span.start as usize..span.end as usize],
            "\"shop\""
        );
    }

    fn named(name: &str) -> String {
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n")
    }

    #[test]
    fn a_name_that_climbs_out_of_the_package_is_v0403() {
        assert_eq!(load_codes(&named("../..")), vec![codes::V0403]);
    }

    #[test]
    fn a_name_starting_with_a_digit_is_v0403() {
        assert_eq!(load_codes(&named("2fast")), vec![codes::V0403]);
        assert_eq!(load_codes(&named("-foo")), vec![codes::V0403]);
        assert_eq!(load_codes(&named("_foo")), Vec::<&str>::new());
    }

    #[test]
    fn the_name_cache_is_v0403() {
        assert_eq!(load_codes(&named("cache")), vec![codes::V0403]);
        // The build directories share a file system that may ignore case.
        assert_eq!(load_codes(&named("Cache")), vec![codes::V0403]);
    }

    #[test]
    fn the_name_package_is_v0403() {
        assert_eq!(load_codes(&named("package")), vec![codes::V0403]);
    }

    #[test]
    fn a_program_named_after_a_cargo_build_folder_is_v0403() {
        for name in ["deps", "examples", "build", "incremental", "Deps"] {
            assert_eq!(load_codes(&named(name)), vec![codes::V0403], "{name}");
        }
    }

    #[test]
    fn a_library_may_be_named_after_a_cargo_build_folder() {
        for name in ["deps", "examples", "build", "incremental"] {
            let dir = package("lib_name", &named(name), "src/lib.vr");
            let mut sources = Vec::new();

            let package = load(&dir.0.join("Cargo.toml"), &mut sources).expect("loads");

            assert_eq!(package.name, name);
        }
    }

    #[test]
    fn a_program_named_after_a_cargo_build_folder_says_so_at_the_name() {
        let dir = package("bin_name", &named("deps"), "src/main.vr");
        let mut sources = Vec::new();

        let diagnostics = load(&dir.0.join("Cargo.toml"), &mut sources).unwrap_err();

        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message.contains("cannot be called `deps`"),
            "{}",
            diagnostics[0].message
        );
    }

    #[test]
    fn an_invalid_name_diagnostic_points_at_the_value() {
        let manifest = named("../..");
        let dir = package("name_span", &manifest, "src/main.vr");
        let diagnostics = load(&dir.0.join("Cargo.toml"), &mut Vec::new()).unwrap_err();
        let span = diagnostics[0].span;
        assert_eq!(
            &manifest[span.start as usize..span.end as usize],
            "\"../..\""
        );
    }

    #[test]
    fn a_name_with_letters_digits_hyphens_and_underscores_is_accepted() {
        assert_eq!(load_codes(&named("my-shop_2")), Vec::<&str>::new());
    }

    #[test]
    fn a_missing_name_is_v0403() {
        assert_eq!(
            load_codes("[package]\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
            vec![codes::V0403]
        );
    }

    #[test]
    fn a_manifest_without_a_package_table_is_v0403() {
        assert_eq!(load_codes("[dependencies]\n"), vec![codes::V0403]);
    }

    #[test]
    fn invalid_toml_is_v0403_with_the_reason() {
        let dir = package("bad_toml", "[package\nname = \"x\"\n", "src/main.vr");
        let diagnostics = load(&dir.0.join("Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, codes::V0403);
        assert!(
            !diagnostics[0].message.contains("line"),
            "{:?}",
            diagnostics[0].message
        );
        assert!(
            diagnostics[0].message.contains(']'),
            "{:?}",
            diagnostics[0].message
        );
    }

    #[test]
    fn an_unreadable_manifest_is_v0403() {
        let dir = TempDir::new("unreadable");
        let manifest = dir.0.join("Cargo.toml");
        fs::write(&manifest, [0xff, 0xfe]).unwrap();
        dir.write("src/main.vr", "fn main() {}\n");
        let diagnostics = load(&manifest, &mut Vec::new()).unwrap_err();
        assert_eq!(diagnostics[0].code, codes::V0403);
    }

    #[test]
    fn a_package_with_neither_root_is_v0403() {
        let dir = TempDir::new("no_root");
        dir.write("Cargo.toml", MANIFEST);
        let diagnostics = load(&dir.0.join("Cargo.toml"), &mut Vec::new()).unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, codes::V0403);
    }

    #[test]
    fn an_edition_table_without_workspace_is_v0400() {
        assert_eq!(
            load_codes("[package]\nname = \"shop\"\nedition = {}\n"),
            vec![codes::V0400]
        );
    }

    #[test]
    fn ignored_tables_are_accepted() {
        let manifest = format!(
            "{MANIFEST}\n[package.metadata.varyk]\n\n[features]\nfast = []\n\n\
             [dev-dependencies]\nx = \"1\"\n\n[profile.release]\nlto = true\n"
        );
        assert_eq!(load_codes(&manifest), Vec::<&str>::new());
    }

    #[test]
    fn a_manifest_diagnostic_points_into_the_manifest() {
        let manifest = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
        let dir = package("span", manifest, "src/main.vr");
        let mut sources = Vec::new();

        let diagnostics = load(&dir.0.join("Cargo.toml"), &mut sources).unwrap_err();

        let span = diagnostics[0].span;
        assert_eq!(sources[span.file.0 as usize].path, dir.0.join("Cargo.toml"));
        assert_eq!(
            &manifest[span.start as usize..span.end as usize],
            "\"2021\""
        );
    }

    #[test]
    fn a_relative_path_dependency_becomes_absolute() {
        let manifest = format!(
            "{MANIFEST}\n[dependencies]\nhelper = {{ path = \"../helper\", features = [\"x\"] }}\n\
             regex = \"1\"\n"
        );
        let dir = package("deps", &manifest, "src/main.vr");
        let mut sources = Vec::new();

        let package = load(&dir.0.join("Cargo.toml"), &mut sources).expect("loads");

        let helper = package.dependencies["helper"].as_table().unwrap();
        let path = PathBuf::from(helper["path"].as_str().unwrap());
        assert!(path.is_absolute(), "{path:?}");
        assert_eq!(path, std::path::absolute(dir.0.join("../helper")).unwrap());
        assert_eq!(helper["features"].as_array().unwrap().len(), 1);
        assert_eq!(package.dependencies["regex"].as_str(), Some("1"));
    }

    #[test]
    fn for_file_is_the_package_for_its_root_file() {
        let dir = package("for_file", MANIFEST, "src/main.vr");
        let mut sources = Vec::new();

        let package = Package::for_file(&dir.0.join("src/main.vr"), &mut sources)
            .expect("package mode")
            .expect("loads");

        assert_eq!(package.name, "shop");
    }

    #[test]
    fn for_file_is_none_for_another_file_in_src() {
        let dir = package("other", MANIFEST, "src/main.vr");
        let other = dir.write("src/other.vr", "fn main() {}\n");

        assert!(Package::for_file(&other, &mut Vec::new()).is_none());
    }

    #[test]
    fn for_file_is_none_for_a_file_outside_src() {
        let dir = package("outside", MANIFEST, "src/main.vr");
        let outside = dir.write("examples/hello.vr", "fn main() {}\n");

        assert!(Package::for_file(&outside, &mut Vec::new()).is_none());
    }

    #[test]
    fn for_file_is_none_when_the_manifest_has_neither_root() {
        let dir = TempDir::new("neither");
        dir.write("Cargo.toml", MANIFEST);
        let file = dir.write("src/main.rs.vr", "fn main() {}\n");

        assert!(Package::for_file(&file, &mut Vec::new()).is_none());
        assert_eq!(root_of(&dir.0), None);
    }

    #[test]
    fn hidden_crate_and_cache_dirs_are_under_the_package_root() {
        let dir = package("dirs", MANIFEST, "src/main.vr");
        let package = load(&dir.0.join("Cargo.toml"), &mut Vec::new()).expect("loads");

        assert_eq!(package.hidden_crate_dir(), dir.0.join("target/varyk/shop"));
        assert_eq!(package.cache_dir(), dir.0.join("target/varyk/cache"));
    }
}
