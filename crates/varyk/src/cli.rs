//! Command-line interface: argument parsing (`clap`) and dispatch.
//!
//! Shape (spec section 6.7):
//!
//! ```text
//! varyk check [file.vr]                          parse and analyze; runs cargo only to
//!                                                  read the package graph (M5b2 spec 4.1)
//! varyk build [file.vr] [--release] [--emit-rust] generate and build; prints the executable path
//! varyk run   [file.vr] [--release] [-- args...]  build, then execute, forwarding the exit code
//! varyk test  [file.vr]                          build the tests and run them, exiting with
//!                                                  the first failing run's code (M5a spec 2.7)
//! varyk init  [dir] [--lib]                       write a new package (M3 spec 2.5)
//! varyk add   [cargo add args]                   run cargo add in the package
//! varyk publish [-- cargo args]                   check, assemble a plain Rust crate, and
//!                                                  run cargo publish there, forwarding its
//!                                                  exit code and output (M3 spec 2.6)
//! ```
//!
//! With no file, each works on the package found upward from the current
//! directory (M3 spec 2.1).
//!
//! Every command accepts `--message-format=json` to emit structured
//! diagnostics instead of the human renderer. Human diagnostics go to
//! stderr; JSON diagnostics go to stdout, one object per line (spec
//! section 6.6's output-streams paragraph), except under `run`, where
//! stdout is the program's own, so they go to stderr, and under `test`,
//! whose stdout is the test runner's.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, ExitCode};

use clap::{Parser, Subcommand, ValueEnum};
use varyk_syntax::{FileId, SourceFile};

use crate::backend::{
    Backend, CrateInfo, GeneratedCrate, RustBackend, StdDependency, with_local_std,
};
use crate::diagnostics::{Diagnostic, render_human, render_json};
use crate::driver::{self, DriverError, PackageCrate};
use crate::hir::HirProgram;
use crate::package::{self, Kind, Package};
use crate::packages::Graph;
use crate::{CheckedPackage, check_file};

/// The `varyk` command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "varyk",
    version,
    about = "The Varyk compiler. Varyk is a small language for APIs, workers, and microservices that compiles to Rust."
)]
pub struct Cli {
    /// Diagnostic output format; applies to every subcommand.
    #[arg(long, value_enum, global = true, default_value = "human")]
    pub message_format: MessageFormat,

    #[command(subcommand)]
    pub command: Command,
}

/// How diagnostics are rendered: for humans at a terminal, or as
/// structured JSON for editors and other tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum MessageFormat {
    Human,
    Json,
    /// JSON on stderr: `run`'s `Json`, whose stdout is the program's.
    #[value(skip)]
    JsonToStderr,
}

/// Prints one JSON line to the stream `message_format` selects.
fn json_line(message_format: MessageFormat, line: &str) {
    if message_format == MessageFormat::JsonToStderr {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}

/// The `check`, `build`, `run`, `test`, `init`, `add`, and
/// `publish` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check the program for errors.
    ///
    /// Runs cargo only to learn which packages the build uses, when the
    /// package lists a dependency besides `varyk-std`.
    Check {
        /// The entry `.vr` file; without one, the package here.
        file: Option<PathBuf>,
    },
    /// Generate and build the Rust; prints the executable path.
    Build {
        /// The entry `.vr` file; without one, the package here.
        file: Option<PathBuf>,
        /// Build the generated crate in release mode.
        #[arg(long)]
        release: bool,
        /// Also print the generated Rust to stdout.
        #[arg(long)]
        emit_rust: bool,
    },
    /// Build, then run the program, forwarding its exit code.
    Run {
        /// The entry `.vr` file; without one, the package here.
        file: Option<PathBuf>,
        /// Build the generated crate in release mode.
        #[arg(long)]
        release: bool,
        /// Arguments forwarded to the built program, after `--`.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Build the program's `#[test]` functions and run them.
    ///
    /// The output and exit code are the test runner's; a Rust error is
    /// reported as under `build`.
    Test {
        /// The entry `.vr` file; without one, the package here.
        file: Option<PathBuf>,
    },
    /// Write a new package.
    ///
    /// Writes `Cargo.toml`, `.gitignore`, and the root `.vr` file. Refuses
    /// if any file it would write already exists.
    Init {
        /// Where to write the package; without one, the current directory.
        dir: Option<PathBuf>,
        /// Write a library (`src/lib.vr`) instead of a program.
        #[arg(long)]
        lib: bool,
    },
    /// Add a dependency to the package's `Cargo.toml`.
    ///
    /// Runs `cargo add` with the arguments given, in the package found
    /// upward from the current directory, and forwards its output and exit
    /// code; Varyk interprets none of the arguments except the shorthand
    /// `sql`, which adds the official package `varyk-sql` as `sql`.
    Add {
        /// Arguments for `cargo add`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Publish the package to crates.io as a plain Rust crate.
    ///
    /// Checks the package, assembles a plain Rust crate, and runs `cargo
    /// publish` there, forwarding its exit code and output. Always works on
    /// the package found upward from the current directory; there is no
    /// single-file form.
    Publish {
        /// Stop after assembling the crate and print its directory; runs
        /// no cargo, so nothing is published.
        #[arg(long, conflicts_with = "args")]
        assemble_only: bool,
        /// Arguments forwarded to `cargo publish`, after `--`; Varyk
        /// interprets none of them (`--dry-run`, `--allow-dirty`,
        /// `--token`, and the rest are cargo's).
        #[arg(last = true)]
        args: Vec<String>,
    },
}

/// Parses the process arguments and dispatches to the requested
/// subcommand, returning the process exit code.
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let message_format = cli.message_format;
    match cli.command {
        Command::Check { file } => run_check(file.as_deref(), message_format),
        Command::Build {
            file,
            release,
            emit_rust,
        } => run_build(file.as_deref(), release, emit_rust, message_format),
        Command::Run {
            file,
            release,
            args,
        } => run_run(file.as_deref(), release, &args, message_format),
        Command::Test { file } => run_test(file.as_deref(), message_format),
        Command::Init { dir, lib } => run_init(dir.as_deref(), lib),
        Command::Add { args } => run_add(&args),
        Command::Publish {
            assemble_only,
            args,
        } => run_publish(assemble_only, &args, message_format),
    }
}

/// What a command works on: a single file, or a package (M3 spec 2.1).
struct Target {
    /// The entry `.vr` file.
    entry: PathBuf,
    package: Option<Package>,
    /// Every file read so far (the manifest, in package mode).
    sources: Vec<SourceFile>,
}

/// Decides what a command works on (M3 spec 2.1): with no `file`, the
/// package found upward from the current directory; with one, the
/// package whose root `file` is, else `file` alone. Reports a failure
/// (no package, a bad manifest, a file that is not `.vr`) itself and
/// returns the exit code to use.
fn locate(file: Option<&Path>, message_format: MessageFormat) -> Result<Target, ExitCode> {
    locate_for(file, message_format, false)
}

/// [`locate`], for `varyk publish` when `publish` is set: it takes no
/// file, so its advice never suggests one.
fn locate_for(
    file: Option<&Path>,
    message_format: MessageFormat,
    publish: bool,
) -> Result<Target, ExitCode> {
    let mut sources = Vec::new();
    let located = match file {
        None => {
            let Some(manifest) = manifest_here() else {
                if publish {
                    eprintln!(
                        "error: no Varyk package here; run `varyk publish` inside a package \
                         (make one with `varyk init`)"
                    );
                } else {
                    eprintln!(
                        "error: no Varyk package here; name a `.vr` file (as in `varyk run \
                         main.vr`) or run inside a package; `varyk init` makes one"
                    );
                }
                return Err(ExitCode::FAILURE);
            };
            if manifest.parent().and_then(package::root_of).is_none() {
                let fix = if publish {
                    "add one of them"
                } else {
                    "name a `.vr` file or add one of them"
                };
                eprintln!(
                    "error: no Varyk package here: `{}` was found, but the package has no \
                     `src/main.vr` or `src/lib.vr`; {fix}",
                    manifest.display()
                );
                return Err(ExitCode::FAILURE);
            }
            Some(package::load(&manifest, &mut sources))
        }
        Some(path) => {
            if path.extension().and_then(|ext| ext.to_str()) != Some("vr") {
                eprintln!("error: expected a `.vr` file, got `{}`", path.display());
                return Err(ExitCode::FAILURE);
            }
            Package::for_file(path, &mut sources)
        }
    };
    match located {
        None => Ok(Target {
            entry: file.expect("no file always finds a package").to_path_buf(),
            package: None,
            sources,
        }),
        Some(Ok(package)) => Ok(Target {
            entry: package.entry.clone(),
            package: Some(package),
            sources,
        }),
        Some(Err(diagnostics)) => {
            emit_diagnostics(&diagnostics, &sources, message_format);
            Err(ExitCode::FAILURE)
        }
    }
}

/// The nearest `Cargo.toml` upward from the current directory, spelled
/// relative to it when it is at or below it.
fn manifest_here() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let manifest = package::find(&cwd)?;
    let manifest = manifest
        .strip_prefix(&cwd)
        .map(Path::to_path_buf)
        .unwrap_or(manifest);
    Some(manifest)
}

/// Runs `check`: reads and parses the target, then renders any
/// diagnostics to the stream `message_format` selects. Runs cargo only to
/// read the package graph (see [`load`]).
///
/// Reading the entry happens in `load`, not in `check_file`: an
/// unreadable file is a plain CLI-level error, not a diagnostic, so it's
/// reported and exits before `check_file` (which needs an already-read
/// `SourceFile`) is ever called.
fn run_check(file: Option<&Path>, message_format: MessageFormat) -> ExitCode {
    match locate(file, message_format).and_then(|target| load(target, message_format)) {
        Ok(_) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// A checked target: its program, the Varyk packages of its build, and
/// the package graph they were found in, when one was read.
struct Loaded {
    program: HirProgram,
    packages: Vec<CheckedPackage>,
    graph: Option<Graph>,
}

/// Reads `target`'s entry and checks it into HIR, after reading the
/// package graph when the package lists a dependency besides `varyk-std`
/// (M5b2 spec 4.1); reports an unreadable file or any diagnostics itself
/// and returns the exit code to use on failure.
fn load(mut target: Target, message_format: MessageFormat) -> Result<(Loaded, Target), ExitCode> {
    let path = &target.entry;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read `{}`: {err}", path.display());
            return Err(ExitCode::FAILURE);
        }
    };
    let graph = match &target.package {
        Some(package) => match Graph::read(package) {
            Ok(graph) => graph,
            Err(diagnostics) => {
                emit_diagnostics(&diagnostics, &target.sources, message_format);
                return Err(ExitCode::FAILURE);
            }
        },
        None => None,
    };
    let sources = &mut target.sources;
    let entry = SourceFile::new(FileId(sources.len() as u32), path.to_path_buf(), text);

    match check_file(entry, target.package.as_ref(), graph.as_ref(), sources) {
        Ok(checked) => {
            let build_uses_std = checked.program.uses_std
                || checked
                    .packages
                    .iter()
                    .any(|package| package.program.uses_std);
            let std_problems = target.package.as_ref().map_or_else(Vec::new, |package| {
                package::check_build_std(
                    package,
                    checked.program.uses_std,
                    build_uses_std,
                    graph.as_ref().and_then(Graph::std_version),
                    env!("CARGO_PKG_VERSION"),
                )
            });
            // The program needs `varyk-std` only because a package logs, so
            // whatever is wrong with its `varyk-std` line, say why it needs it.
            let std_problems: Vec<_> = std_problems
                .into_iter()
                .map(|problem| match &checked.logging_package {
                    Some(logger) => problem.with_note(format!(
                        "the package `{logger}` writes log lines, which need it"
                    )),
                    None => problem,
                })
                .collect();
            if std_problems.is_empty() {
                let loaded = Loaded {
                    program: checked.program,
                    packages: checked.packages,
                    graph,
                };
                Ok((loaded, target))
            } else {
                emit_diagnostics(&std_problems, &target.sources, message_format);
                Err(ExitCode::FAILURE)
            }
        }
        Err(diagnostics) => {
            emit_diagnostics(&diagnostics, sources, message_format);
            Err(ExitCode::FAILURE)
        }
    }
}

/// Runs `build`: checks the target, generates the Rust crate (printing it
/// first with `--emit-rust`), builds it, and prints the executable path
/// (nothing for a library, which has none).
fn run_build(
    file: Option<&Path>,
    release: bool,
    emit_rust: bool,
    message_format: MessageFormat,
) -> ExitCode {
    let target = match locate(file, message_format) {
        Ok(target) => target,
        Err(code) => return code,
    };
    match generate_and_build(target, release, emit_rust, message_format) {
        Ok(Some(exe)) => {
            println!("{}", exe.display());
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// Runs `run`: builds like `build` without printing anything of its own,
/// then executes the program with `args`, handing it both output streams
/// and exiting with its status code (1 if it has none, e.g. on a signal).
/// A library is an error before anything is checked.
fn run_run(
    file: Option<&Path>,
    release: bool,
    args: &[String],
    message_format: MessageFormat,
) -> ExitCode {
    // The program's own output owns stdout.
    let message_format = match message_format {
        MessageFormat::Json => MessageFormat::JsonToStderr,
        other => other,
    };
    let target = match locate(file, message_format) {
        Ok(target) => target,
        Err(code) => return code,
    };
    if target
        .package
        .as_ref()
        .is_some_and(|package| package.kind == Kind::Library)
    {
        eprintln!("error: this package is a library; it has nothing to run");
        return ExitCode::FAILURE;
    }
    let exe = match generate_and_build(target, release, false, message_format) {
        Ok(Some(exe)) => exe,
        Ok(None) => unreachable!("a binary always has an executable"),
        Err(code) => return code,
    };
    match driver::run(&exe, args) {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        },
        Err(err) => {
            eprintln!("error: cannot run `{}`: {err}", exe.display());
            ExitCode::FAILURE
        }
    }
}

/// The shared part of `build` and `run`: check, generate, optionally
/// print the generated Rust, and build with cargo, in the package's
/// hidden crate or the single file's build directory (M3 spec 2.4).
/// Reports every failure itself and returns the exit code to use.
fn generate_and_build(
    target: Target,
    release: bool,
    emit_rust: bool,
    message_format: MessageFormat,
) -> Result<Option<PathBuf>, ExitCode> {
    let (loaded, target) = load(target, message_format)?;
    let (generated, packages) = generate(&target, &loaded)?;

    if emit_rust {
        print!("{}", driver::emit_rust(&generated));
    }

    let (build_dir, cache_dir, lock) = build_dirs(&target, loaded.graph.as_ref());
    let built = driver::build(
        &generated,
        &packages,
        &build_dir,
        &cache_dir,
        lock.as_deref(),
        release,
    );
    reported(built, &target, message_format)
}

/// What a build of `target` hands cargo (M5b2 spec 4.3): the program's
/// crate and the crate of every Varyk package of its build, whose
/// dependencies on Varyk packages are those crates. Reports a directory
/// it cannot name itself and returns the exit code to use.
fn generate(
    target: &Target,
    loaded: &Loaded,
) -> Result<(GeneratedCrate, Vec<PackageCrate>), ExitCode> {
    let (dirs, packages) = match (&target.package, &loaded.graph) {
        (Some(package), Some(graph)) => {
            let dirs = driver::package_dirs(&package.root, graph).map_err(|err| {
                eprintln!("error: cannot build `{}`: {err}", target.entry.display());
                ExitCode::FAILURE
            })?;
            let packages = driver::package_crates(&loaded.packages, graph, &dirs);
            (dirs, packages)
        }
        _ => (Vec::new(), Vec::new()),
    };
    let info = crate_info(target, &loaded.program, loaded.graph.as_ref(), &dirs);
    Ok((RustBackend.generate(&loaded.program, &info), packages))
}

/// Where `target` is built (M3 spec 2.4): the package's hidden crate, its
/// cache directory, and its lock, the one the package graph was read with
/// when there is a graph (M5b2 spec 4.1), or the single file's build
/// directory and the shared cache, with no lock.
fn build_dirs(target: &Target, graph: Option<&Graph>) -> (PathBuf, PathBuf, Option<PathBuf>) {
    match &target.package {
        Some(package) => (
            package.hidden_crate_dir(),
            package.cache_dir(),
            // The graph's lock is copied into the hidden crate as it is. For
            // a Varyk package cargo resolved from a registry or git, the copy
            // still names that source, and cargo rewrites it to the `path`
            // crate the driver wrote; so the copy is refreshed on every
            // build, which is harmless: cargo decides what to recompile from
            // its fingerprints, not from the lock.
            graph.map_or_else(|| package.lock.clone(), |graph| Some(graph.lock_path())),
        ),
        None => (
            driver::build_dir_for(&target.entry),
            driver::cache_dir(),
            None,
        ),
    }
}

/// What a cargo run for `target` made, after printing its compiler
/// messages; on failure, reports why and returns the exit code to use.
fn reported<T>(
    result: Result<(T, Vec<driver::Message>), DriverError>,
    target: &Target,
    message_format: MessageFormat,
) -> Result<T, ExitCode> {
    match result {
        Ok((made, messages)) => {
            print_messages(messages, &target.sources, message_format, false);
            Ok(made)
        }
        Err(DriverError::Cargo { stderr, messages }) => {
            let printed_error = print_messages(messages, &target.sources, message_format, true);
            if !printed_error {
                // No compiler error at all, maybe some warnings: cargo
                // itself failed (a missing dependency, a failing build
                // script, ...), and its own words say why.
                eprintln!("error: cargo could not build the package:");
                eprint!("{stderr}");
            }
            Err(ExitCode::FAILURE)
        }
        Err(DriverError::Io(err)) => {
            eprintln!("error: cannot build `{}`: {err}", target.entry.display());
            Err(ExitCode::FAILURE)
        }
        Err(DriverError::Spawn(err)) => {
            eprintln!("{}", cannot_run_cargo(&err));
            Err(ExitCode::FAILURE)
        }
    }
}

/// Runs `test` (M5a spec 2.7): checks and generates as `build` does,
/// builds the tests with `cargo test --no-run`, then runs each test
/// executable with this process's streams, exiting with the first
/// non-zero code (1 for a run without one, e.g. on a signal), or 0 when
/// every run passes.
fn run_test(file: Option<&Path>, message_format: MessageFormat) -> ExitCode {
    // The test runner's output owns stdout.
    let message_format = match message_format {
        MessageFormat::Json => MessageFormat::JsonToStderr,
        other => other,
    };
    let loaded = locate(file, message_format).and_then(|target| load(target, message_format));
    let (loaded, target) = match loaded {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let (generated, packages) = match generate(&target, &loaded) {
        Ok(generated) => generated,
        Err(code) => return code,
    };
    let (build_dir, cache_dir, lock) = build_dirs(&target, loaded.graph.as_ref());
    let built = driver::test(
        &generated,
        &packages,
        &build_dir,
        &cache_dir,
        lock.as_deref(),
    );
    let executables = match reported(built, &target, message_format) {
        Ok(executables) => executables,
        Err(code) => return code,
    };
    let mut result = ExitCode::SUCCESS;
    let mut failed = false;
    for exe in executables {
        let code = match driver::run(&exe, &[]) {
            Ok(status) => status.code().unwrap_or(1),
            Err(err) => {
                eprintln!("error: cannot run `{}`: {err}", exe.display());
                1
            }
        };
        if code != 0 && !failed {
            failed = true;
            result = ExitCode::from(u8::try_from(code).unwrap_or(1));
        }
    }
    result
}

/// The crate `target` generates into (M3 spec 2.4): the package's name
/// and isolated manifest, or a generated name with a minimal manifest for
/// a single file, depending on `varyk-std` when `program` uses it (M5a
/// spec 5.2). Under `VARYK_STD_PATH`, `varyk-std` is that directory (M5a
/// spec 5.3); `varyk publish` writes the package's own manifest instead.
/// With a package `graph`, each dependency on a Varyk package is the
/// crate written for it in `dirs`, by graph index (M5b2 spec 4.3).
fn crate_info(
    target: &Target,
    program: &HirProgram,
    graph: Option<&Graph>,
    dirs: &[PathBuf],
) -> CrateInfo {
    match &target.package {
        Some(package) => {
            let deps = graph.map_or(&[][..], Graph::root_deps);
            let manifest = driver::with_package_paths(package.generated_manifest(), deps, dirs);
            CrateInfo {
                name: package.name.clone(),
                manifest: with_local_std(manifest),
                std_dependency: None,
            }
        }
        None => CrateInfo::single_file(
            driver::package_name_for(&target.entry),
            StdDependency::for_program(program.uses_std),
        ),
    }
}

/// `word` as a shell reads it back: as is when every character is one no
/// shell treats specially (`A-Z`, `a-z`, `0-9`, `.`, `_`, `/`, `-`), else
/// in single quotes, a `'` in it written `'\''`.
fn shell_quote(word: &str) -> String {
    let plain = word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// Whether `dir` holds a Varyk package: a `Cargo.toml` and the root
/// `.vr` file `root` (`src/main.vr`).
fn is_varyk_package(dir: &Path, root: &str) -> bool {
    dir.join("Cargo.toml").is_file() && dir.join(root).is_file()
}

/// The directory `init` was given, for a message: "this directory" for
/// the current one.
fn shown_dir(given: Option<&Path>) -> String {
    match given.map(|dir| dir.display().to_string()) {
        Some(dir) if dir != "." && !dir.is_empty() => format!("`{dir}`"),
        _ => "this directory".to_string(),
    }
}

/// Runs `init`: writes the package of spec 2.5 under `dir` (the current
/// directory when there is none), reporting a refusal (any file already
/// there) or an I/O failure itself.
fn run_init(given: Option<&Path>, lib: bool) -> ExitCode {
    let dir = given.unwrap_or_else(|| Path::new("."));
    match driver::init::write(dir, lib) {
        Ok(_written) => {
            // A library has nothing to run.
            let run = if lib { "build" } else { "run" };
            let cd = match given.map(|dir| dir.display().to_string()) {
                Some(dir) if dir != "." && !dir.is_empty() => {
                    format!("`cd {}`, then ", shell_quote(&dir))
                }
                _ => String::new(),
            };
            println!(
                "created the package `{}` in `{}`; run {cd}`varyk {run}`",
                driver::init::package_name(dir),
                dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(driver::init::InitError::Exists(existing)) => {
            let names: Vec<String> = existing
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            let (init, stem) = if lib {
                ("varyk init --lib", "src/lib")
            } else {
                ("varyk init", "src/main")
            };
            if is_varyk_package(dir, &format!("{stem}.vr")) {
                eprintln!(
                    "error: {} is already a Varyk package (it has `Cargo.toml` and \
                     `{stem}.vr`), so `varyk init` wrote nothing",
                    shown_dir(given)
                );
                return ExitCode::FAILURE;
            }
            eprintln!(
                "error: these files already exist: {}; to start a package in an existing \
                 project, run `{init}` in an empty directory and move its `Cargo.toml` \
                 target table and `{stem}.vr` into your project; the package needs \
                 `edition = \"2024\"`",
                names.join(", ")
            );
            ExitCode::FAILURE
        }
        Err(driver::init::InitError::OtherRoot(other)) => {
            let already = if is_varyk_package(dir, &other.display().to_string()) {
                format!("{} is already a Varyk package: ", shown_dir(given))
            } else {
                String::new()
            };
            eprintln!(
                "error: {already}`{}` already exists, and a package has `src/main.vr` or \
                 `src/lib.vr`, not both; run `varyk init{}` in another directory",
                other.display(),
                if lib { " --lib" } else { "" }
            );
            ExitCode::FAILURE
        }
        Err(driver::init::InitError::Io(err)) => {
            eprintln!(
                "error: cannot write a package at `{}`: {err}",
                dir.display()
            );
            ExitCode::FAILURE
        }
    }
}

/// Runs `publish`: checks the package found upward from the current
/// directory (there is no single-file form, so `file` is always `None`),
/// generates its crate, assembles a plain Rust crate with
/// `driver::publish::assemble` (M3 spec 2.6), and runs `cargo publish
/// <args>` there, forwarding its exit code and output (inherited, since
/// `Command::status` does not capture them); with `assemble_only`, prints
/// the assembled directory instead of running cargo.
fn run_publish(assemble_only: bool, args: &[String], message_format: MessageFormat) -> ExitCode {
    let target = match locate_for(None, message_format, true) {
        Ok(target) => target,
        Err(code) => return code,
    };
    let package = target
        .package
        .clone()
        .expect("`locate(None, ..)` always resolves a package or returns an error");
    let (loaded, target) = match load(target, message_format) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    // The assembly writes the package's own manifest, with its `version`s
    // (M5b2 spec 4.5); only the generated tree is taken from here.
    let info = crate_info(&target, &loaded.program, None, &[]);
    let generated = RustBackend.generate(&loaded.program, &info);
    let dest = match driver::assemble(&package, &generated) {
        Ok(dest) => dest,
        Err(err) => {
            eprintln!(
                "error: cannot assemble `{}` to publish: {err}",
                package.name
            );
            return ExitCode::FAILURE;
        }
    };
    if assemble_only {
        println!("{}", dest.display());
        return ExitCode::SUCCESS;
    }
    match StdCommand::new("cargo")
        .arg("publish")
        .args(args)
        .current_dir(&dest)
        .status()
    {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        },
        Err(err) => {
            eprintln!("{}", cannot_run_cargo(&err));
            ExitCode::FAILURE
        }
    }
}

/// The official packages `varyk add` has a shorthand for: the name typed,
/// and the crate it adds (renamed to the shorthand so code can name it).
const SHORTHANDS: &[(&str, &str)] = &[("sql", "varyk-sql")];

/// The arguments after `cargo add` for a `varyk add` call: a leading
/// shorthand becomes its crate and `--rename`, the rest is passed as
/// written, except a second shorthand, which is refused. Any other call
/// passes through unchanged.
fn add_args(args: &[String]) -> Result<Vec<String>, String> {
    let is_shorthand = |arg: &String| SHORTHANDS.iter().find(|(name, _)| name == arg);
    let Some(first) = args.first().and_then(is_shorthand) else {
        return Ok(args.to_vec());
    };
    let rest = &args[1..];
    if rest.iter().any(|arg| is_shorthand(arg).is_some()) {
        return Err("add one official package per `varyk add` call".to_string());
    }
    let mut out = vec![
        first.1.to_string(),
        "--rename".to_string(),
        first.0.to_string(),
    ];
    out.extend(rest.iter().cloned());
    Ok(out)
}

/// Runs `add`: `cargo add` with `args` in the package found upward from
/// the current directory, its output and exit code passed through.
fn run_add(args: &[String]) -> ExitCode {
    let args = match add_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let Some(manifest) = manifest_here() else {
        eprintln!("error: no Varyk package here; run `varyk add` inside a package");
        return ExitCode::FAILURE;
    };
    let dir = match manifest.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    match StdCommand::new("cargo")
        .arg("add")
        .args(args)
        .current_dir(dir)
        .status()
    {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        },
        Err(err) => {
            eprintln!("{}", cannot_run_cargo(&err));
            ExitCode::FAILURE
        }
    }
}

/// The one line for a `cargo` that could not be started: where to get it
/// when it is not installed, else the system's own reason.
fn cannot_run_cargo(err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::NotFound {
        "error: cannot run `cargo`: not found; install Rust from https://rustup.rs".to_string()
    } else {
        format!("error: cannot run `cargo`: {err}")
    }
}

/// Prints every rustc message from a build (spec 5): a `Generated` V0900
/// through the normal diagnostic renderer (so a `--message-format=json`
/// caller gets a structured diagnostic), a `User` message verbatim to
/// stderr since it is the user's own Rust rendered as rustc worded it,
/// and an `Other` message as cargo's text. Under `--message-format=json`,
/// a `User` message is instead one JSON object on stdout (`level`, the
/// user's `file`, `line`, `column`, `message`, `rustc_code`), and an
/// `Other` one an object with `level` and `message`. M1's one-line "did not
/// compile" note is prepended once, only when the build failed with an
/// error in the generated Rust: a `Generated` one, or an `Other` one
/// (rustc's, against no Varyk or `.rs` line). A warning never earns it,
/// and neither does an error in the user's own `.rs` file.
///
/// Returns whether an error was printed, so the caller can fall back to
/// cargo's raw stderr when cargo failed without reporting a single
/// compiler error (e.g. a missing dependency, a failing build script, or
/// a linker error), even when warnings were printed.
fn print_messages(
    messages: Vec<driver::Message>,
    sources: &[SourceFile],
    message_format: MessageFormat,
    failed: bool,
) -> bool {
    use driver::Message;

    let json = message_format != MessageFormat::Human;
    let mut diagnostics = Vec::new();
    let mut generated_error = false;
    let mut user_error = false;
    let mut others = Vec::new();
    for message in messages {
        match message {
            Message::Generated(diagnostic) => {
                diagnostics.push(diagnostic);
                generated_error = true;
            }
            // Shown with the program's, but the program's generated Rust
            // is not what failed.
            Message::Package(diagnostic) => diagnostics.push(diagnostic),
            Message::User {
                rendered,
                level,
                file,
                line,
                column,
                message,
                code,
                notes,
            } => {
                if json {
                    let value = serde_json::json!({
                        "level": level,
                        "file": file.to_string_lossy(),
                        "line": line,
                        "column": column,
                        "message": message,
                        "rustc_code": code,
                        "notes": notes,
                    });
                    json_line(message_format, &value.to_string());
                } else {
                    eprint!("{rendered}");
                }
                user_error |= level == "error";
            }
            Message::Other {
                rendered,
                level,
                message,
            } => others.push((rendered, level, message)),
        }
    }
    let any_diagnostic = !diagnostics.is_empty();
    if any_diagnostic {
        emit_diagnostics(&diagnostics, sources, message_format);
    }
    let other_error = others.iter().any(|(_, level, _)| level == "error");
    if failed && (generated_error || other_error) {
        eprintln!(
            "error: the generated Rust did not compile; run `varyk build --emit-rust` to inspect it"
        );
    }
    for (rendered, level, message) in &others {
        if json {
            let value = serde_json::json!({ "level": level, "message": message });
            json_line(message_format, &value.to_string());
        } else {
            eprint!("{rendered}");
        }
    }
    any_diagnostic || user_error || other_error
}

/// Renders `diagnostics` to the stream `message_format` selects: human
/// output to stderr, JSON to stdout (to stderr under `run`).
fn emit_diagnostics(
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    message_format: MessageFormat,
) {
    match message_format {
        MessageFormat::Human => eprintln!("{}", render_human(diagnostics, sources)),
        MessageFormat::Json | MessageFormat::JsonToStderr => {
            json_line(message_format, &render_json(diagnostics, sources));
        }
    }
}

#[cfg(test)]
mod add_tests {
    use super::add_args;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_shorthand_becomes_its_row() {
        assert_eq!(
            add_args(&args(&["sql"])),
            Ok(args(&["varyk-sql", "--rename", "sql"]))
        );
    }

    #[test]
    fn later_arguments_follow_the_row() {
        assert_eq!(
            add_args(&args(&["sql", "--features", "postgres"])),
            Ok(args(&[
                "varyk-sql",
                "--rename",
                "sql",
                "--features",
                "postgres"
            ]))
        );
    }

    #[test]
    fn a_second_shorthand_is_refused() {
        assert_eq!(
            add_args(&args(&["sql", "sql"])),
            Err("add one official package per `varyk add` call".to_string())
        );
    }

    #[test]
    fn a_shorthand_leaves_other_crates_as_written() {
        assert_eq!(
            add_args(&args(&["sql", "serde"])),
            Ok(args(&["varyk-sql", "--rename", "sql", "serde"]))
        );
    }

    #[test]
    fn a_call_not_starting_with_a_shorthand_passes_through() {
        for call in [&["serde"][..], &["varyk-sql", "--rename", "sql"]] {
            assert_eq!(add_args(&args(call)), Ok(args(call)));
        }
    }
}
