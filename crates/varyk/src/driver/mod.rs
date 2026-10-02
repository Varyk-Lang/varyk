//! The driver (spec 6.4): writes a [`GeneratedCrate`] to its build
//! directory, runs cargo on it, and executes the result.
//!
//! For a single file, everything lives under `target/varyk/` relative to
//! the current directory: one build directory per entry file, and one
//! shared cargo target directory, `target/varyk/cache/`, so dependencies
//! compile once across programs. A package's hidden crate and cache live
//! under its own root instead (M3 spec 2.4, `Package::hidden_crate_dir`).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use varyk_syntax::FileId;

use crate::CheckedPackage;
use crate::backend::{Backend, CrateInfo, GeneratedCrate, RustBackend, with_local_std};
use crate::packages::{DepTarget, Graph};

mod cargo;
pub mod generate;
pub mod init;
pub mod messages;
pub mod publish;

pub use cargo::DriverError;
pub use generate::emit_rust;
pub use messages::Message;
pub use publish::assemble;

/// Root of everything the driver writes for a single file, relative to
/// the current directory.
const VARYK_DIR: &str = "target/varyk";

/// The shared cargo target directory for a single file, relative to the
/// current directory.
pub fn cache_dir() -> PathBuf {
    Path::new(VARYK_DIR).join("cache")
}

/// FNV-1a 64-bit hash of `bytes`, truncated to its low 32 bits. Used
/// instead of [`std::hash::DefaultHasher`], which is not stable across
/// toolchains and would change the build directory and package name for
/// the same entry file on a compiler upgrade.
fn fnv1a_32(bytes: &[u8]) -> u32 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET_BASIS;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash as u32
}

/// `<stem>-<8 hex chars of a hash of the absolute entry path>`: the
/// unsanitized name shared by the build directory and the package name.
fn name_for(entry: &Path) -> String {
    let absolute = std::path::absolute(entry).unwrap_or_else(|_| entry.to_path_buf());
    let hash = fnv1a_32(absolute.to_string_lossy().as_bytes());
    let stem = entry
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{stem}-{hash:08x}")
}

/// The build directory for `entry`: `target/varyk/<stem>-<hash>`, relative
/// to the current directory. Two entries with the same stem in different
/// directories get different build directories.
pub fn build_dir_for(entry: &Path) -> PathBuf {
    Path::new(VARYK_DIR).join(name_for(entry))
}

/// The cargo package (and binary) name for `entry`: the build directory's
/// name, sanitized into a crate identifier.
pub fn package_name_for(entry: &Path) -> String {
    sanitize_name(&name_for(entry))
}

/// Lowercases `name` and replaces every character that is not an ASCII
/// letter or digit with `_`; prefixes `v_` if the result would otherwise
/// start with a digit, be empty, or collide with `cache` (the shared
/// build cache directory, spec 2.4), `package` (where `varyk publish`
/// assembles its crate), `packages` (where the packages a build uses are
/// written), or a folder of Cargo's own that a program cannot be named
/// after (`deps`, `examples`, `build`, `incremental`) — the ways a
/// sanitized name could fail `package::name_problem` or
/// `package::program_name_problem`. Shared by `package_name_for` (a build
/// directory's name) and `init` (a directory's own name).
pub fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty()
        || sanitized.starts_with(|c: char| c.is_ascii_digit())
        || sanitized == "cache"
        || sanitized == "package"
        || sanitized == "packages"
        || crate::package::program_name_problem(&sanitized).is_some()
    {
        format!("v_{sanitized}")
    } else {
        sanitized
    }
}

/// Where the crates of a build's Varyk packages are written, under the
/// program's package (M5b2 spec 4.3).
const PACKAGES_DIR: &str = "target/varyk/packages";

/// The crate the driver writes for a Varyk package of the build (M5b2
/// spec 4.3): its manifest is the package's own, built from its generated
/// root, and its `src/` holds its generated tree and its `.rs` modules,
/// nothing else of the package's directory.
#[derive(Debug, Clone)]
pub struct PackageCrate {
    pub name: String,
    pub version: String,
    /// Its `Cargo.toml` among the sources, where a rustc error in the
    /// crate is shown.
    pub manifest_file: FileId,
    /// `<root>/target/varyk/packages/<name>-<version>`, absolute.
    pub dir: PathBuf,
    pub generated: GeneratedCrate,
}

/// The directory of the crate written for each Varyk package of `graph`,
/// by graph index: `target/varyk/packages/<name>-<version>` under `root`,
/// the program's package, made absolute.
pub fn package_dirs(root: &Path, graph: &Graph) -> io::Result<Vec<PathBuf>> {
    graph
        .varyk_packages()
        .iter()
        .map(|package| {
            let name = format!("{}-{}", package.name, package.version);
            std::path::absolute(root.join(PACKAGES_DIR).join(name))
        })
        .collect()
}

/// The crate of every package of `checked`, the Varyk packages of
/// `graph`'s build, to be written in `dirs` (see [`package_dirs`]): each
/// generated from its own HIR, whose paths to another package's items use
/// its own keys (M5b2 spec 5), with its isolated manifest given the
/// generated root as its library target, named after the package with
/// `-` as `_` (cargo refuses a `-` there), its dependencies on Varyk
/// packages turned into those packages' crates, and, under
/// `VARYK_STD_PATH`, its `varyk-std` that directory (spec 4.7).
pub fn package_crates(
    checked: &[CheckedPackage],
    graph: &Graph,
    dirs: &[PathBuf],
) -> Vec<PackageCrate> {
    checked
        .iter()
        .filter_map(|package| {
            let deps = &graph.varyk_packages().get(package.graph_index)?.deps;
            let dir = dirs.get(package.graph_index)?;
            let manifest = with_lib_target(package.manifest.clone(), &package.name);
            let info = CrateInfo {
                name: package.name.clone(),
                manifest: with_local_std(with_package_paths(manifest, deps, dirs)),
                std_dependency: None,
            };
            Some(PackageCrate {
                name: package.name.clone(),
                version: package.version.clone(),
                manifest_file: package.manifest_file,
                dir: dir.clone(),
                generated: RustBackend.generate(&package.program, &info),
            })
        })
        .collect()
}

/// `manifest` with its `[lib]` target the generated root, `src/lib.rs`,
/// named `name` with `-` as `_`.
fn with_lib_target(mut manifest: toml::Table, name: &str) -> toml::Table {
    let lib = manifest
        .entry("lib")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let toml::Value::Table(lib) = lib {
        lib.insert(
            "name".to_string(),
            toml::Value::String(name.replace('-', "_")),
        );
        lib.insert(
            "path".to_string(),
            toml::Value::String("src/lib.rs".to_string()),
        );
    }
    manifest
}

/// The keys of a dependency on a Varyk package that a build drops, since
/// it is the crate the driver writes for the package (M5b2 spec 4.3).
const SOURCE_KEYS: [&str; 7] = ["version", "git", "branch", "tag", "rev", "registry", "path"];

/// `manifest` with each `[dependencies]` entry that `deps` (keys with `-`
/// as `_`) resolves to a Varyk package made a `path` to the crate written
/// for it in `dirs`, by graph index: the key stays, and so do `package`,
/// `features`, and `default-features`; where the package comes from
/// (`version`, `git`, `branch`, `tag`, `rev`, `registry`) goes (M5b2 spec
/// 4.3). For every manifest the driver writes for a build; never for the
/// one `varyk publish` assembles.
pub fn with_package_paths(
    mut manifest: toml::Table,
    deps: &[(String, DepTarget)],
    dirs: &[PathBuf],
) -> toml::Table {
    let Some(toml::Value::Table(dependencies)) = manifest.get_mut("dependencies") else {
        return manifest;
    };
    for (key, value) in dependencies.iter_mut() {
        let name = key.replace('-', "_");
        let target = deps.iter().find(|(dep, _)| *dep == name).map(|(_, to)| *to);
        let Some(dir) = target.and_then(|target| match target {
            DepTarget::Varyk(index) => dirs.get(index),
            DepTarget::Rust => None,
        }) else {
            continue;
        };
        let mut table = match value {
            toml::Value::Table(table) => std::mem::take(table),
            _ => toml::Table::new(),
        };
        for key in SOURCE_KEYS {
            table.remove(key);
        }
        table.insert(
            "path".to_string(),
            toml::Value::String(dir.to_string_lossy().into_owned()),
        );
        *value = toml::Value::Table(table);
    }
    manifest
}

/// Writes every crate of `packages` in its directory, each file only when
/// its content changed and files no longer generated removed (M5b2 spec
/// 4.3).
fn write_packages(packages: &[PackageCrate]) -> io::Result<()> {
    for package in packages {
        generate::write_files(&package.dir, &package.generated)?;
    }
    Ok(())
}

/// Writes the crate of every Varyk package of the build in `packages`,
/// then `generated` under `build_dir`, copies `lock` (the lock the
/// package graph was read with, else the package's `Cargo.lock`, when it
/// has one) in beside it (M3 spec 2.2, M5b2 spec 4.1; never copied
/// back), builds it with cargo into `target_dir`, and returns the
/// executable's path, read from cargo's `compiler-artifact` message, or
/// `None` for a library, which has none. This is the only reliable way to
/// find it: under a configured target (`CARGO_BUILD_TARGET` or
/// `[build].target`), cargo nests the profile directory under an extra
/// `<triple>/` path component that `build` cannot predict on its own.
///
/// Files whose content is already on disk are left untouched, so cargo's
/// freshness check keeps working and concurrent builds of one program do
/// not rewrite each other's files. Cargo's output is captured; on failure
/// its stderr and every classified compiler message are returned in
/// [`DriverError::Cargo`].
///
/// Every compiler message from the run is classified (spec 5) and
/// returned alongside the executable, on success as well as failure, so a
/// build that succeeds with a warning in a copied `.rs` module still
/// reports it (only cargo's own exit status distinguishes failure; a
/// warning never fails the build).
pub fn build(
    generated: &GeneratedCrate,
    packages: &[PackageCrate],
    build_dir: &Path,
    target_dir: &Path,
    lock: Option<&Path>,
    release: bool,
) -> Result<(Option<PathBuf>, Vec<Message>), DriverError> {
    write_packages(packages)?;
    generate::write_files(build_dir, generated)?;
    generate::sync_lock(lock, &build_dir.join("Cargo.lock"))?;
    let binary = generated
        .files
        .iter()
        .any(|file| file.path == "src/main.rs");
    let map = generated.source_map();
    let goal = cargo::Goal::Build { release };
    let (stdout, stderr, messages) = cargo::run_cargo(
        build_dir,
        target_dir,
        goal,
        &map,
        &generated.copied,
        packages,
    )?;
    if !binary {
        return Ok((None, messages));
    }
    match cargo::executable_from(stdout.lines(), &generated.package_name) {
        Some(exe) => Ok((Some(PathBuf::from(exe)), messages)),
        // Cargo exited 0 but named no binary artifact for this package: an
        // anomaly, not a compile error, so `messages` (from the same
        // successful run) are the only ones worth carrying.
        None => Err(DriverError::Cargo {
            stderr: format!("cargo produced no executable\n{stderr}"),
            messages,
        }),
    }
}

/// Writes `packages` and `generated` as [`build`] does, builds its
/// tests with `cargo test --no-run` into `target_dir` (M5a spec 2.7), and
/// returns the test executables, read from cargo's `compiler-artifact`
/// messages, alongside every classified compiler message; rustc's errors
/// are mapped as for a build (M3 spec 5).
pub fn test(
    generated: &GeneratedCrate,
    packages: &[PackageCrate],
    build_dir: &Path,
    target_dir: &Path,
    lock: Option<&Path>,
) -> Result<(Vec<PathBuf>, Vec<Message>), DriverError> {
    write_packages(packages)?;
    generate::write_files(build_dir, generated)?;
    generate::sync_lock(lock, &build_dir.join("Cargo.lock"))?;
    let map = generated.source_map();
    let (stdout, _, messages) = cargo::run_cargo(
        build_dir,
        target_dir,
        cargo::Goal::Test,
        &map,
        &generated.copied,
        packages,
    )?;
    Ok((cargo::test_executables_from(stdout.lines()), messages))
}

/// Runs the built program with `args`, handing it this process's stdin,
/// stdout, and stderr, and returns its exit status.
pub fn run(exe: &Path, args: &[String]) -> io::Result<ExitStatus> {
    Command::new(exe).args(args).status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_stem_in_different_directories_gets_different_names() {
        let interop = Path::new("examples/interop/main.vr");
        let modules = Path::new("examples/modules/main.vr");

        assert_ne!(build_dir_for(interop), build_dir_for(modules));
        assert_ne!(package_name_for(interop), package_name_for(modules));
    }

    /// Pinned so a future change to [`fnv1a_32`] (or a swap for a
    /// different hash) is caught: unlike `DefaultHasher`, this hash must
    /// give the same build directory and package name across toolchains.
    #[test]
    fn fnv1a_32_of_a_fixed_string_is_pinned() {
        assert_eq!(fnv1a_32(b"hello"), 0x80aabd0b);
    }

    #[test]
    fn build_dir_is_the_stem_and_eight_hex_chars_under_target_varyk() {
        let dir = build_dir_for(Path::new("examples/hello.vr"));

        assert_eq!(dir.parent(), Some(Path::new("target/varyk")));
        let name = dir.file_name().unwrap().to_str().unwrap();
        let hash = name.strip_prefix("hello-").expect("name starts with stem");
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn package_name_is_the_build_dir_name_sanitized() {
        let entry = Path::new("examples/hello.vr");
        let dir_name = build_dir_for(entry)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        assert_eq!(package_name_for(entry), dir_name.replace('-', "_"));
    }

    #[test]
    fn a_hyphen_in_the_stem_becomes_an_underscore() {
        let name = package_name_for(Path::new("my-Program.vr"));

        assert!(name.starts_with("my_program_"), "{name}");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{name}"
        );
    }

    #[test]
    fn a_leading_digit_gets_a_v_prefix() {
        let name = package_name_for(Path::new("2fast.vr"));

        assert!(name.starts_with("v_2fast_"), "{name}");
    }

    /// A directory name sanitized for `init` must always pass the
    /// manifest's own name rule (`package::name_problem`), including its
    /// edge cases that a plain build-directory name never hits: a
    /// directory literally called `cache` or `package`, one named after a
    /// folder of Cargo's own (`init` makes a program by default), and one
    /// whose name has no ASCII letters or digits at all.
    #[test]
    fn a_sanitized_directory_name_always_passes_the_manifest_name_rule() {
        for name in [
            "My Project",
            "2fast",
            "cache",
            "package",
            "packages",
            "deps",
            "Examples",
            "build",
            "incremental",
            "..",
            "café",
            "___",
        ] {
            let sanitized = sanitize_name(name);
            assert_eq!(
                crate::package::name_problem(&sanitized),
                None,
                "{name:?} -> {sanitized:?}"
            );
            assert_eq!(
                crate::package::program_name_problem(&sanitized),
                None,
                "{name:?} -> {sanitized:?}"
            );
        }
    }

    fn table(text: &str) -> toml::Table {
        text.parse().expect("valid TOML")
    }

    /// Each dependency on a Varyk package becomes a `path` to its crate,
    /// keeping its key, `package`, `features`, and `default-features`; a
    /// Rust crate's stays as written (M5b2 spec 4.3).
    #[test]
    fn a_dependency_on_a_varyk_package_becomes_a_path_to_its_crate() {
        let manifest = table(
            "[dependencies]\n\
             units = \"0.1.0\"\n\
             u = { version = \"0.2\", package = \"units\", registry = \"r\", \
             features = [\"f\"], default-features = false }\n\
             route-planner = { git = \"https://x\", branch = \"b\", tag = \"t\", rev = \"r\", \
             path = \"../route\" }\n\
             regex-lite = \"0.1\"\n",
        );
        let deps = [
            ("units".to_string(), DepTarget::Varyk(0)),
            ("u".to_string(), DepTarget::Varyk(1)),
            ("route_planner".to_string(), DepTarget::Varyk(2)),
            ("regex_lite".to_string(), DepTarget::Rust),
        ];
        let dirs = [
            PathBuf::from("/p/units-0.1.0"),
            PathBuf::from("/p/units-0.2.0"),
            PathBuf::from("/p/route-planner-0.1.0"),
        ];

        let rewritten = with_package_paths(manifest, &deps, &dirs);

        let expected = table(
            "[dependencies]\n\
             units = { path = \"/p/units-0.1.0\" }\n\
             u = { path = \"/p/units-0.2.0\", package = \"units\", features = [\"f\"], \
             default-features = false }\n\
             route-planner = { path = \"/p/route-planner-0.1.0\" }\n\
             regex-lite = \"0.1\"\n",
        );
        assert_eq!(rewritten, expected);
    }

    /// A package crate's target is the generated root, named after the
    /// package with `-` as `_`, since cargo refuses a `-` there.
    #[test]
    fn a_package_crate_s_library_is_the_generated_root_named_without_a_hyphen() {
        let manifest = table("[lib]\npath = \"src/lib.vr\"\n");
        assert_eq!(
            with_lib_target(manifest, "route-planner"),
            table("[lib]\nname = \"route_planner\"\npath = \"src/lib.rs\"\n")
        );
        assert_eq!(
            with_lib_target(toml::Table::new(), "units"),
            table("[lib]\nname = \"units\"\npath = \"src/lib.rs\"\n")
        );
    }
}
