//! Command-line interface: argument parsing (`clap`) and dispatch.
//!
//! Shape (spec section 6.7):
//!
//! ```text
//! varyk check <file.vr>                          parse and analyze; never runs cargo
//! varyk build <file.vr> [--release] [--emit-rust] generate and build; prints the executable path
//! varyk run   <file.vr> [--release] [-- args...]  build, then execute, forwarding the exit code
//! ```
//!
//! Every command accepts `--message-format=json` to emit structured
//! diagnostics instead of the human renderer. Human diagnostics go to
//! stderr; JSON diagnostics go to stdout, one object per line (spec
//! section 6.6's output-streams paragraph).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use varyk_syntax::{FileId, SourceFile};

use crate::backend::{Backend, RustBackend};
use crate::check_file;
use crate::diagnostics::{Diagnostic, render_human, render_json};
use crate::driver::{self, DriverError};
use crate::hir::HirProgram;

/// The `varyk` command-line interface.
#[derive(Debug, Parser)]
#[command(name = "varyk", version, about = "The Varyk compiler")]
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
}

/// The `check`, `build`, and `run` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Parse and analyze; never runs cargo.
    Check {
        /// The entry `.vr` file.
        file: PathBuf,
    },
    /// Generate and build; prints the executable path.
    Build {
        /// The entry `.vr` file.
        file: PathBuf,
        /// Build the generated crate in release mode.
        #[arg(long)]
        release: bool,
        /// Also print the generated Rust to stdout.
        #[arg(long)]
        emit_rust: bool,
    },
    /// Build, then execute, forwarding the exit code.
    Run {
        /// The entry `.vr` file.
        file: PathBuf,
        /// Build the generated crate in release mode.
        #[arg(long)]
        release: bool,
        /// Arguments forwarded to the built program, after `--`.
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
        Command::Check { file } => run_check(&file, message_format),
        Command::Build {
            file,
            release,
            emit_rust,
        } => run_build(&file, release, emit_rust, message_format),
        Command::Run {
            file,
            release,
            args,
        } => run_run(&file, release, &args, message_format),
    }
}

/// Runs `check`: reads and parses `path`, then renders any diagnostics to
/// the stream `message_format` selects.
///
/// Reading `path` happens in `load`, not in `check_file`: an unreadable
/// file is a plain CLI-level error, not a diagnostic, so it's reported and
/// exits before `check_file` (which needs an already-read `SourceFile`)
/// is ever called.
fn run_check(path: &Path, message_format: MessageFormat) -> ExitCode {
    match load(path, message_format) {
        Ok(_) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// Reads `path` and checks it into HIR; reports an unreadable file or any
/// diagnostics itself and returns the exit code to use on failure.
fn load(path: &Path, message_format: MessageFormat) -> Result<HirProgram, ExitCode> {
    let mut sources = Vec::new();

    if path.extension().and_then(|ext| ext.to_str()) != Some("vr") {
        eprintln!("error: expected a `.vr` file, got `{}`", path.display());
        return Err(ExitCode::FAILURE);
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read `{}`: {err}", path.display());
            return Err(ExitCode::FAILURE);
        }
    };
    let entry = SourceFile::new(FileId(sources.len() as u32), path.to_path_buf(), text);

    match check_file(entry, &mut sources) {
        Ok(program) => Ok(program),
        Err(diagnostics) => {
            emit_diagnostics(&diagnostics, &sources, message_format);
            Err(ExitCode::FAILURE)
        }
    }
}

/// Runs `build`: checks `path`, generates the Rust crate (printing it
/// first with `--emit-rust`), builds it, and prints the executable path.
fn run_build(
    path: &Path,
    release: bool,
    emit_rust: bool,
    message_format: MessageFormat,
) -> ExitCode {
    match generate_and_build(path, release, emit_rust, message_format) {
        Ok(exe) => {
            println!("{}", exe.display());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Runs `run`: builds like `build` without printing anything of its own,
/// then executes the program with `args`, handing it both output streams
/// and exiting with its status code (1 if it has none, e.g. on a signal).
fn run_run(path: &Path, release: bool, args: &[String], message_format: MessageFormat) -> ExitCode {
    let exe = match generate_and_build(path, release, false, message_format) {
        Ok(exe) => exe,
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
/// print the generated Rust, and build with cargo. Reports every failure
/// itself and returns the exit code to use.
fn generate_and_build(
    path: &Path,
    release: bool,
    emit_rust: bool,
    message_format: MessageFormat,
) -> Result<PathBuf, ExitCode> {
    let program = load(path, message_format)?;

    let generated = RustBackend.generate(&program, &driver::package_name_for(path));

    if emit_rust {
        print!("{}", driver::emit_rust(&generated));
    }

    match driver::build(&generated, &driver::build_dir_for(path), release) {
        Ok(exe) => Ok(exe),
        Err(DriverError::Cargo { stderr }) => {
            eprintln!(
                "error: the generated Rust did not compile; run with --emit-rust to inspect it"
            );
            eprint!("{stderr}");
            Err(ExitCode::FAILURE)
        }
        Err(DriverError::Io(err)) => {
            eprintln!("error: cannot build `{}`: {err}", path.display());
            Err(ExitCode::FAILURE)
        }
    }
}

/// Renders `diagnostics` to the stream `message_format` selects: human
/// output to stderr, JSON to stdout.
fn emit_diagnostics(
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    message_format: MessageFormat,
) {
    match message_format {
        MessageFormat::Human => eprintln!("{}", render_human(diagnostics, sources)),
        MessageFormat::Json => println!("{}", render_json(diagnostics, sources)),
    }
}
