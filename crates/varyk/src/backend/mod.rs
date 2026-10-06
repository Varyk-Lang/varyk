//! Backends (spec 6.4): turn a checked [`HirProgram`] into an artifact.

use std::path::PathBuf;

use varyk_syntax::Span;

use crate::hir::HirProgram;

mod rust;
mod rust_expr;
mod writer;

pub use rust::RustBackend;
pub use writer::{GeneratedFile, Mark, Writer};

/// A generated Cargo project, not yet written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCrate {
    pub package_name: String,
    pub cargo_toml: String,
    /// Every generated `.rs` file, each with its own line-level source
    /// map (spec 5).
    pub files: Vec<GeneratedFile>,
    /// Every copied `.rs` module: its tree path (e.g. `src/greet.rs`) and
    /// the user's file to copy it from byte-for-byte (spec 2.3).
    pub copied: Vec<(String, PathBuf)>,
}

impl GeneratedCrate {
    /// A read-only view over every generated file's line-level source map
    /// (spec 5), for mapping a rustc message back to Varyk source.
    pub fn source_map(&self) -> SourceMap<'_> {
        SourceMap { files: &self.files }
    }
}

/// The crate the generated tree is built as (M3 spec 2.4): its name,
/// and the whole `Cargo.toml` it is built with, a package's isolated
/// manifest (`Package::isolated_manifest`, the same one `varyk publish`
/// writes, so the two builds cannot disagree) or a single file's minimal
/// one.
#[derive(Debug, Clone, PartialEq)]
pub struct CrateInfo {
    /// The crate's name; the same as `manifest`'s `package.name`, kept
    /// here because the generated tree is named after it before any
    /// manifest is written.
    pub name: String,
    pub manifest: toml::Table,
    /// The `varyk-std` dependency a single file's manifest was given (M5a
    /// spec 5.2); `None` for a file that does not use `varyk-std` and for
    /// a package, whose own manifest names it.
    pub std_dependency: Option<StdDependency>,
}

/// Where a single file's crate gets `varyk-std` from (M5a spec 5.2, 5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdDependency {
    /// Exactly this version, from crates.io: the compiler's own.
    Version(String),
    /// The crate in this directory, from `VARYK_STD_PATH`, which the test
    /// suite sets to build against the workspace's `crates/varyk-std`.
    Path(PathBuf),
}

/// The environment variable naming a local `varyk-std` to build against
/// (M5a spec 5.3); not documented for users.
pub const STD_PATH_VAR: &str = "VARYK_STD_PATH";

impl StdDependency {
    /// The dependency of a program that uses `varyk-std` (`uses_std`), or
    /// `None`: the directory `VARYK_STD_PATH` names when it is set, else
    /// the compiler's version.
    pub fn for_program(uses_std: bool) -> Option<StdDependency> {
        if !uses_std {
            return None;
        }
        Some(match local_std() {
            Some(path) => StdDependency::Path(path),
            None => StdDependency::Version(env!("CARGO_PKG_VERSION").to_string()),
        })
    }

    /// The dependency's `Cargo.toml` value: `"=0.2.0"`, or `{ path = ".." }`.
    fn value(&self) -> toml::Value {
        match self {
            StdDependency::Version(version) => toml::Value::String(format!("={version}")),
            StdDependency::Path(path) => path_value(path),
        }
    }
}

/// The directory `VARYK_STD_PATH` names, when it is set and not empty.
fn local_std() -> Option<PathBuf> {
    std::env::var_os(STD_PATH_VAR)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// `{ path = ".." }` for `path`.
fn path_value(path: &std::path::Path) -> toml::Value {
    let mut table = toml::Table::new();
    table.insert(
        "path".to_string(),
        toml::Value::String(path.to_string_lossy().into_owned()),
    );
    toml::Value::Table(table)
}

/// A package's isolated manifest with its `varyk-std` dependency, if it
/// has one, replaced by a `path` dependency on the directory
/// `VARYK_STD_PATH` names, when that is set (M5a spec 5.3). `varyk
/// publish` never calls this.
pub fn with_local_std(mut manifest: toml::Table) -> toml::Table {
    let Some(path) = local_std() else {
        return manifest;
    };
    if let Some(toml::Value::Table(dependencies)) = manifest.get_mut("dependencies") {
        if dependencies.contains_key("varyk-std") {
            dependencies.insert("varyk-std".to_string(), path_value(&path));
        }
    }
    manifest
}

/// `manifest` with `[patch.crates-io] varyk-std = { path = .. }` on the
/// directory `VARYK_STD_PATH` names, when that is set: for the manifest
/// the package graph is read from, which reaches every package's own
/// `varyk-std` line (M5b2 spec 4.7).
pub fn with_std_patch(mut manifest: toml::Table) -> toml::Table {
    let Some(path) = local_std() else {
        return manifest;
    };
    let mut crates_io = toml::Table::new();
    crates_io.insert("varyk-std".to_string(), path_value(&path));
    let mut patch = toml::Table::new();
    patch.insert("crates-io".to_string(), toml::Value::Table(crates_io));
    manifest.insert("patch".to_string(), toml::Value::Table(patch));
    manifest
}

impl CrateInfo {
    /// A single file's crate: `name`, version `0.0.0`, edition 2024, an
    /// empty `[workspace]` table so an enclosing workspace does not claim
    /// it, and no dependencies but `varyk-std` when `std_dependency` is
    /// set (M5a spec 5.2).
    pub fn single_file(name: String, std_dependency: Option<StdDependency>) -> Self {
        let mut package = toml::Table::new();
        package.insert("name".to_string(), toml::Value::String(name.clone()));
        package.insert(
            "version".to_string(),
            toml::Value::String("0.0.0".to_string()),
        );
        package.insert(
            "edition".to_string(),
            toml::Value::String("2024".to_string()),
        );
        let mut manifest = toml::Table::new();
        manifest.insert("package".to_string(), toml::Value::Table(package));
        manifest.insert(
            "workspace".to_string(),
            toml::Value::Table(toml::Table::new()),
        );
        let mut dependencies = toml::Table::new();
        if let Some(dependency) = &std_dependency {
            dependencies.insert("varyk-std".to_string(), dependency.value());
        }
        manifest.insert("dependencies".to_string(), toml::Value::Table(dependencies));
        CrateInfo {
            name,
            manifest,
            std_dependency,
        }
    }
}

/// Takes a checked program and produces a generated crate.
pub trait Backend {
    fn generate(&self, program: &HirProgram, info: &CrateInfo) -> GeneratedCrate;
}

/// A read-only view over a [`GeneratedCrate`]'s generated files, for
/// mapping a rustc diagnostic's file and line back to the Varyk span that
/// produced it.
pub struct SourceMap<'a> {
    files: &'a [GeneratedFile],
}

impl SourceMap<'_> {
    /// The Varyk span that produced `path`'s line `line` (1-based, as
    /// rustc's JSON messages report it). `None` when `path` is not one of
    /// this crate's generated files, `line` is out of range, or the line
    /// is structural and carries no span.
    pub fn lookup(&self, path: &str, line: u32) -> Option<Span> {
        let file = self.files.iter().find(|file| file.path == path)?;
        let index = line.checked_sub(1)?;
        file.lines.get(index as usize).copied().flatten()
    }

    /// What `path`'s line `line` (1-based) belongs to: a route or an
    /// `App::new` call (milestone 5b4 spec 7.5); `None` for any other
    /// line, or one [`SourceMap::lookup`] does not find.
    pub fn mark(&self, path: &str, line: u32) -> Option<Mark> {
        let file = self.files.iter().find(|file| file.path == path)?;
        let index = line.checked_sub(1)?;
        file.marks.get(index as usize).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use varyk_syntax::FileId;

    fn span(start: u32, end: u32) -> Span {
        Span::new(FileId(0), start, end)
    }

    fn crate_with(files: Vec<GeneratedFile>) -> GeneratedCrate {
        GeneratedCrate {
            package_name: "p".to_string(),
            cargo_toml: String::new(),
            files,
            copied: Vec::new(),
        }
    }

    #[test]
    fn lookup_finds_the_span_at_a_1_based_line_in_the_named_file() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() {", Some(span(0, 10)));
        writer.line(1, "let x = 1;", Some(span(15, 25)));
        writer.line(0, "}", None);
        let generated = crate_with(vec![writer.finish()]);

        let map = generated.source_map();

        assert_eq!(map.lookup("src/main.rs", 1), Some(span(0, 10)));
        assert_eq!(map.lookup("src/main.rs", 2), Some(span(15, 25)));
    }

    #[test]
    fn lookup_is_none_for_a_structural_line_an_unknown_file_or_an_out_of_range_line() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() {", Some(span(0, 10)));
        writer.line(0, "}", None);
        let generated = crate_with(vec![writer.finish()]);

        let map = generated.source_map();

        assert_eq!(map.lookup("src/main.rs", 2), None, "a structural line");
        assert_eq!(map.lookup("src/other.rs", 1), None, "an unknown file");
        assert_eq!(map.lookup("src/main.rs", 99), None, "an out-of-range line");
        assert_eq!(map.lookup("src/main.rs", 0), None, "line 0 is not 1-based");
    }

    /// Every line of a route's adapter and its registration call carries
    /// the route call's span and [`Mark::Route`], a registration inside
    /// an `if` included, and the line of `App::new` [`Mark::State`], so a
    /// thread-safety error there gets their words (milestone 5b4 spec
    /// 7.5); adapters are numbered in source order.
    #[test]
    fn route_lines_and_the_app_new_line_are_marked_in_the_source_map() {
        use crate::test_packages::{Dep, varyk_http};
        let text = "struct State {\n    name: string,\n}\nasync fn home(state: Shared<State>) -> string {\n    state.name.clone()\n}\nfn main() {\n    let mut app = http::App::new(Shared::new(State { name: \"a\" }));\n    app.get(\"/\", home);\n    if true {\n        app.get(\"/b\", home);\n    }\n}\n";
        let program = varyk_http()
            .check(text, &[("http", Dep::Package("varyk-http"))])
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        let generated =
            RustBackend.generate(&program, &CrateInfo::single_file("p".to_string(), None));
        let map = generated.source_map();
        let main = &generated.files[0];
        assert_eq!(main.path, "src/main.rs");
        let lines: Vec<&str> = main.text.lines().collect();
        let first = lines
            .iter()
            .position(|line| line.starts_with("async fn varyk_route_0("))
            .expect("the first adapter")
            - 1;
        let second = lines
            .iter()
            .position(|line| line.starts_with("async fn varyk_route_1("))
            .expect("the second adapter")
            - 1;
        let source = |span: Span| &text[span.start as usize..span.end as usize];
        // The `if` is one statement, whose lines all carry its span, and
        // so the mark of the route inside.
        let in_if = lines
            .iter()
            .position(|line| *line == "    if true {")
            .expect("the if");
        for (index, line) in lines.iter().enumerate() {
            let number = index as u32 + 1;
            let mark = map.mark("src/main.rs", number);
            let span = map.lookup("src/main.rs", number);
            if line.contains("App::new(") {
                assert_eq!(mark, Some(Mark::State), "{line}");
            } else if line.contains("route(varyk_route_") || (in_if..in_if + 3).contains(&index) {
                assert_eq!(mark, Some(Mark::Route), "{line}");
            } else if (first..second - 1).contains(&index) {
                assert_eq!(mark, Some(Mark::Route), "{line}");
                assert_eq!(span.map(source), Some("app.get(\"/\", home)"), "{line}");
            } else if index >= second {
                assert_eq!(mark, Some(Mark::Route), "{line}");
                assert_eq!(span.map(source), Some("app.get(\"/b\", home)"), "{line}");
            } else {
                assert_eq!(mark, None, "{line}");
            }
        }
        assert!(
            main.text
                .contains("    app.get(\"/\", ::http::route(varyk_route_0));\n"),
            "{}",
            main.text
        );
        assert!(
            main.text
                .contains("        app.get(\"/b\", ::http::route(varyk_route_1));\n"),
            "{}",
            main.text
        );
    }

    /// A hook's adapter shares the module's count with the routes, lends
    /// the hook the request, an `after` hook the response as its
    /// parameter's mode says, and the state when it takes it; every line
    /// of it carries the hook call's span and [`Mark::Route`] (milestone
    /// 5b4 spec 4).
    #[test]
    fn hook_adapters_share_the_route_count_and_map_to_the_hook_call() {
        use crate::test_packages::{Dep, varyk_http};
        let text = "struct State {\n    name: string,\n}\nasync fn home() {}\nasync fn admin(req: http::Request, state: Shared<State>) -> Result<bool, Error> {\n    Ok(true)\n}\nasync fn look(req: http::Request, res: http::Response) {}\nfn main() {\n    let mut app = http::App::new(Shared::new(State { name: \"a\" }));\n    app.after(look);\n    app.get(\"/\", home);\n    app.before_on(\"/admin\", admin);\n}\n";
        let program = varyk_http()
            .check(text, &[("http", Dep::Package("varyk-http"))])
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        let generated =
            RustBackend.generate(&program, &CrateInfo::single_file("p".to_string(), None));
        let map = generated.source_map();
        let main = &generated.files[0];
        for line in [
            "    app.after(::http::after_hook(varyk_route_0));",
            "    app.get(\"/\", ::http::route(varyk_route_1));",
            "    app.before_on(\"/admin\", ::http::before_hook(varyk_route_2));",
            "async fn varyk_route_0(varyk_req: ::http::Request, varyk_res: ::http::Response) -> ::http::Response {",
            "    let mut varyk_res = varyk_res;",
            "    look(&varyk_req, &varyk_res).await;",
            "    varyk_res",
            "async fn varyk_route_2(varyk_req: ::http::Request) -> Option<::http::Response> {",
            "    let varyk_1 = match varyk_req.state::<State>() {",
            "        Err(r) => return Some(r),",
            "    ::http::respond_before(admin(&varyk_req, &varyk_1).await)",
        ] {
            assert!(
                main.text.lines().any(|have| have == line),
                "{line}\n{}",
                main.text
            );
        }
        let lines: Vec<&str> = main.text.lines().collect();
        let start = lines
            .iter()
            .position(|line| line.starts_with("async fn varyk_route_2("))
            .expect("the before_on adapter");
        let source = |span: Span| &text[span.start as usize..span.end as usize];
        for number in start..lines.len() {
            let number = number as u32 + 1;
            assert_eq!(map.mark("src/main.rs", number), Some(Mark::Route));
            assert_eq!(
                map.lookup("src/main.rs", number).map(source),
                Some("app.before_on(\"/admin\", admin)")
            );
        }
    }

    /// A route's adapter binds the path, query, body, and state parameters
    /// before the package's own types, so a 400 comes before a live
    /// handler's early response, and the handler is still called in its
    /// parameter order (milestone 5b4 spec 4).
    #[test]
    fn a_route_adapter_binds_package_types_after_the_other_parameters() {
        use crate::test_packages::{Dep, varyk_http};
        let text = "struct State {\n    n: i64,\n}\nasync fn show(req: http::Request, id: i64) {}\nfn main() {\n    let mut app = http::App::new(Shared::new(State { n: 1 }));\n    app.get(\"/{id}\", show);\n}\n";
        let program = varyk_http()
            .check(text, &[("http", Dep::Package("varyk-http"))])
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        let generated =
            RustBackend.generate(&program, &CrateInfo::single_file("p".to_string(), None));
        let lines: Vec<&str> = generated.files[0].text.lines().collect();
        let position = |needle: &str| {
            lines
                .iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle}\n{}", generated.files[0].text))
        };
        let param = position("let varyk_1 = match varyk_req.param::<i64>(\"id\")");
        let bind =
            position("let varyk_0 = match varyk_req.bind::<::http::server::Request>().await");
        assert!(param < bind, "{}", generated.files[0].text);
        position("    show(&varyk_0, varyk_1).await;");
    }

    /// In the package named `varyk-http` itself, whose `App` is declared
    /// in `server` and named at the root by `pub use`, an adapter and its
    /// registration write the package root, `crate`, as the path to the
    /// contract items (milestone 5b4 spec 2.1, 7.2).
    #[test]
    fn the_package_s_own_routes_write_crate_as_the_package() {
        use crate::test_packages::varyk_http;
        let build = varyk_http();
        let program = &build.checked[0].program;
        let generated =
            RustBackend.generate(program, &CrateInfo::single_file("p".to_string(), None));
        let tests = generated
            .files
            .iter()
            .find(|file| file.path == "src/tests.rs")
            .expect("the tests module");
        for line in [
            "        app.get(\"/greet/{name}\", crate::route(varyk_route_0));",
            "async fn varyk_route_0(varyk_req: crate::Request) -> crate::Response {",
        ] {
            assert!(
                tests.text.lines().any(|have| have == line),
                "{line}\n{}",
                tests.text
            );
        }
    }

    #[test]
    fn a_single_file_manifest_has_no_dependencies() {
        let text = CrateInfo::single_file("hello".to_string(), StdDependency::for_program(false))
            .manifest
            .to_string();
        let manifest: toml::Table = text.parse().expect("valid TOML");

        assert_eq!(
            manifest["package"]["name"].as_str(),
            Some("hello"),
            "{text}"
        );
        assert_eq!(manifest["package"]["version"].as_str(), Some("0.0.0"));
        assert_eq!(manifest["package"]["edition"].as_str(), Some("2024"));
        assert!(manifest["workspace"].as_table().unwrap().is_empty());
        assert!(manifest["dependencies"].as_table().unwrap().is_empty());
    }

    #[test]
    fn a_single_file_using_varyk_std_depends_on_it_exactly_or_by_path() {
        let version = StdDependency::Version("0.3.1".to_string());
        let info = CrateInfo::single_file("p".to_string(), Some(version.clone()));
        assert_eq!(info.std_dependency, Some(version));
        let text = info.manifest.to_string();
        let manifest: toml::Table = text.parse().expect("valid TOML");
        assert_eq!(
            manifest["dependencies"]["varyk-std"].as_str(),
            Some("=0.3.1"),
            "{text}"
        );

        let local = StdDependency::Path(PathBuf::from("/w/crates/varyk-std"));
        let text = CrateInfo::single_file("p".to_string(), Some(local))
            .manifest
            .to_string();
        let manifest: toml::Table = text.parse().expect("valid TOML");
        assert_eq!(
            manifest["dependencies"]["varyk-std"]["path"].as_str(),
            Some("/w/crates/varyk-std"),
            "{text}"
        );
    }
}
