// Written by `varyk init`. Runs `varyk emit` to turn `src/`
// into a Rust crate under `OUT_DIR`, so plain `cargo build` compiles it
// through `include!` in the stub `src/main.rs` (or `src/lib.rs`).
use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-env-changed=VARYK");

    let varyk = env::var("VARYK").unwrap_or_else(|_| "varyk".to_string());
    let out_dir = env::var("OUT_DIR").expect("cargo sets OUT_DIR for a build script");
    let dest = PathBuf::from(out_dir).join("varyk");

    let status = Command::new(&varyk)
        .arg("emit")
        .arg("--out-dir")
        .arg(&dest)
        .status();

    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        // `varyk emit` already reported its own diagnostics on stderr,
        // which cargo shows when the build script fails.
        Ok(_) => {
            eprintln!("run `varyk build` to see this without cargo's wrapping");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!(
                "varyk was not found (tried `{varyk}`); install it with `cargo install varyk` \
                 or set VARYK to its path"
            );
            ExitCode::FAILURE
        }
    }
}
