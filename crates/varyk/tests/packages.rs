//! Packages end to end (M3 spec 2.1, 2.2, 2.4): each test copies a
//! fixture from `tests/fixtures/packages/` out of the source tree and runs
//! the real `varyk` binary inside it. No test needs the network: the one
//! dependency is a `path` crate beside the package.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{empty_dir, package_dir, std_config, varyk_in, varyk_run_with};

use varyk::backend::{Backend, CrateInfo, RustBackend};
use varyk::driver::publish;
use varyk::package::{self, Package};
use varyk_syntax::{FileId, SourceFile};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        stderr(output)
    );
}

#[test]
fn basic_runs_from_the_package_with_no_argument() {
    let dir = package_dir("basic");

    let output = varyk_in(&dir, &["run"]);

    assert_success(&output);
    assert_eq!(stdout(&output), "hello from a package\n");
    assert!(dir.join("target/varyk/basic/Cargo.toml").is_file());
    assert!(dir.join("target/varyk/cache").is_dir());
}

#[test]
fn basic_runs_through_its_root_file_in_package_mode() {
    let dir = package_dir("basic");

    let output = varyk_in(&dir, &["run", "src/main.vr"]);

    assert_success(&output);
    assert_eq!(stdout(&output), "hello from a package\n");
    assert!(
        dir.join("target/varyk/basic/Cargo.toml").is_file(),
        "the root file builds the package's hidden crate, not a single file's"
    );
}

#[test]
fn basic_builds_a_binary_named_after_the_package() {
    let dir = package_dir("basic");

    let output = varyk_in(&dir, &["build"]);

    assert_success(&output);
    let exe = stdout(&output);
    let exe = Path::new(exe.trim());
    assert_eq!(
        exe.file_stem().and_then(|stem| stem.to_str()),
        Some("basic")
    );
    assert!(exe.is_file(), "{exe:?}");
}

#[test]
fn with_dep_builds_and_runs_through_its_rust_facade() {
    let dir = package_dir("with_dep");

    let output = varyk_in(&dir, &["run"]);

    assert_success(&output);
    assert_eq!(stdout(&output), "the answer is 42\n");

    // `cd <pkg> && varyk run src/main.vr`: the root file named from inside
    // the package is package mode, so the `path` dependency is there.
    let output = varyk_in(&dir, &["run", "src/main.vr"]);

    assert_success(&output);
    assert_eq!(stdout(&output), "the answer is 42\n");
}

#[test]
fn a_facade_may_use_and_extern_crate_its_dependencies() {
    // The importer rejects a `.rs` module naming a crate the generated
    // crate cannot have; in a package, `[dependencies]` are crates it has.
    let dir = package_dir("with_dep");
    fs::write(
        dir.join("src/util.rs"),
        "extern crate dep;\n\nuse dep::answer as dep_answer;\n\npub fn answer() -> i64 {\n    dep_answer()\n}\n",
    )
    .unwrap();

    let output = varyk_in(&dir, &["run"]);

    assert_success(&output);
    assert_eq!(stdout(&output), "the answer is 42\n");
}

/// Cargo reads `.cargo/config.toml` from the current directory upward, so
/// `varyk` must run it from inside the package even when invoked elsewhere.
#[test]
fn the_packages_cargo_config_applies_when_varyk_runs_from_elsewhere() {
    let dir = empty_dir("cwd_config");
    let output = common::varyk(&["init", dir.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));
    fs::create_dir_all(dir.join(".cargo")).unwrap();
    fs::write(
        dir.join(".cargo/config.toml"),
        "[env]\nVARYK_TEST_FLAG = \"on\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/util.rs"),
        "pub fn answer() -> i64 {\n    if env!(\"VARYK_TEST_FLAG\") == \"on\" { 42 } else { 0 }\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/main.vr"),
        "mod util;\n\nfn main() {\n    println!(\"{}\", util::answer());\n}\n",
    )
    .unwrap();
    let name = dir.file_name().unwrap().to_str().unwrap().to_string();

    let output = varyk_in(
        dir.parent().unwrap(),
        &["build", &format!("{name}/src/main.vr")],
    );

    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn an_extern_crate_alias_is_a_local_name() {
    let dir = package_dir("with_dep");
    fs::write(
        dir.join("src/util.rs"),
        "extern crate std as s;\nuse s::mem;\n\npub fn answer() -> i64 {\n    mem::size_of::<i64>() as i64\n}\n",
    )
    .unwrap();

    let output = varyk_in(&dir, &["check"]);

    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn an_include_inside_a_macro_definition_is_v0104_too() {
    // Invoked or not, the included file is not in the generated crate; the
    // rule does not look at whether the macro runs.
    let dir = package_dir("with_dep");
    fs::write(
        dir.join("src/util.rs"),
        "macro_rules! load {\n    () => {\n        include!(\"extra.rs\")\n    };\n}\n\n\
         pub fn answer() -> i64 {\n    42\n}\n",
    )
    .unwrap();

    let output = varyk_in(&dir, &["check"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("error[V0104]"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_name_declared_only_inside_an_inline_module_is_not_local_at_the_root() {
    let dir = package_dir("with_dep");
    fs::write(
        dir.join("src/util.rs"),
        "mod inner {\n    pub mod ghost {\n        pub struct Thing;\n    }\n}\n\n\
         use ghost::Thing;\n\npub fn answer() -> i64 {\n    42\n}\n",
    )
    .unwrap();

    let output = varyk_in(&dir, &["check"]);

    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(stderr.contains("error[V0104]"), "{stderr}");
    assert!(stderr.contains("uses the crate `ghost`"), "{stderr}");
}

#[test]
fn a_facade_naming_an_undeclared_crate_is_v0104_naming_the_dependencies() {
    let dir = package_dir("with_dep");
    fs::write(
        dir.join("src/util.rs"),
        "use other::x;\n\npub fn answer() -> i64 {\n    42\n}\n",
    )
    .unwrap();

    let output = varyk_in(&dir, &["check"]);

    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(stderr.contains("error[V0104]"), "{stderr}");
    assert!(
        stderr.contains("uses the crate `other`, which is not in `[dependencies]`"),
        "{stderr}"
    );
}

#[test]
fn the_hidden_crate_manifest_carries_the_dependency_with_an_absolute_path() {
    let dir = package_dir("with_dep");

    assert_success(&varyk_in(&dir, &["build"]));

    let manifest = fs::read_to_string(dir.join("target/varyk/with_dep/Cargo.toml")).unwrap();
    let manifest: toml::Table = manifest.parse().unwrap();
    let dep = manifest["dependencies"]["dep"]["path"].as_str().unwrap();
    assert!(Path::new(dep).is_absolute(), "{dep}");
    assert_eq!(
        Path::new(dep).canonicalize().unwrap(),
        dir.join("dep").canonicalize().unwrap()
    );
    assert_eq!(manifest["package"]["name"].as_str(), Some("with_dep"));
    assert_eq!(manifest["package"]["version"].as_str(), Some("0.1.0"));
    assert!(manifest["workspace"].as_table().unwrap().is_empty());
}

#[test]
fn an_optional_dependency_turned_on_by_a_default_feature_builds_and_runs() {
    // `[features]` is copied into the hidden crate's manifest; without it
    // cargo leaves the optional `dep` out and the facade cannot find it.
    let dir = package_dir("with_dep");
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"with_dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [dependencies]\ndep = { path = \"dep\", optional = true }\n\n\
         [features]\ndefault = [\"dep\"]\n",
    )
    .unwrap();

    let output = varyk_in(&dir, &["run"]);

    assert_success(&output);
    assert_eq!(stdout(&output), "the answer is 42\n");
    let manifest = fs::read_to_string(dir.join("target/varyk/with_dep/Cargo.toml")).unwrap();
    let manifest: toml::Table = manifest.parse().unwrap();
    assert_eq!(
        manifest["features"]["default"].as_array().unwrap()[0].as_str(),
        Some("dep")
    );
}

#[test]
fn a_lock_beside_the_manifest_is_copied_into_the_hidden_crate() {
    let dir = package_dir("basic");
    // Version 3: cargo keeps an existing lock's format, while one it
    // writes itself is version 4, so the version shows the copy.
    let lock = "# This file is automatically @generated by Cargo.\n\
                # It is not intended for manual editing.\n\
                version = 3\n\n[[package]]\nname = \"basic\"\nversion = \"0.1.0\"\n";
    fs::write(dir.join("Cargo.lock"), lock).unwrap();

    assert_success(&varyk_in(&dir, &["build"]));

    let copied = fs::read_to_string(dir.join("target/varyk/basic/Cargo.lock")).unwrap();
    assert!(copied.contains("version = 3"), "{copied}");
    assert_eq!(fs::read_to_string(dir.join("Cargo.lock")).unwrap(), lock);
}

#[test]
fn a_library_checks_without_main() {
    let dir = package_dir("lib");

    let output = varyk_in(&dir, &["check"]);

    assert_success(&output);
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
}

#[test]
fn a_library_builds_into_a_lib_rs_and_prints_nothing() {
    let dir = package_dir("lib");

    let output = varyk_in(&dir, &["build"]);

    assert_success(&output);
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    assert!(dir.join("target/varyk/shapes/src/lib.rs").is_file());
    assert!(!dir.join("target/varyk/shapes/src/main.rs").exists());
}

#[test]
fn run_on_a_library_is_an_error() {
    let dir = package_dir("lib");

    let output = varyk_in(&dir, &["run"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "error: this package is a library; it has nothing to run\n"
    );
}

#[test]
fn check_in_a_package_never_runs_cargo() {
    let dir = package_dir("with_dep");
    let empty = dir.join("no-tools");
    fs::create_dir_all(&empty).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_varyk"))
        .arg("check")
        .current_dir(&dir)
        .env("PATH", &empty)
        .output()
        .expect("spawn varyk");

    assert_success(&output);
    assert!(!dir.join("target").exists(), "check wrote a build tree");
}

#[test]
fn no_argument_outside_a_package_is_an_error() {
    // The copy's own `Cargo.toml` is the nearest manifest, and with
    // `src/main.vr` gone its package has neither root file.
    let dir = package_dir("basic");
    let src = dir.join("src");
    fs::remove_file(src.join("main.vr")).unwrap();

    let output = varyk_in(&dir, &["check"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "error: no Varyk package here: `Cargo.toml` was found, but the package has no \
         `src/main.vr` or `src/lib.vr`; name a `.vr` file or add one of them\n"
    );
    let output = varyk_in(&dir, &["publish"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "error: no Varyk package here: `Cargo.toml` was found, but the package has no \
         `src/main.vr` or `src/lib.vr`; add one of them\n"
    );
}

/// With no package upward, the advice fits the command: `publish` takes
/// no file, so it is told to run inside a package.
#[test]
fn with_no_package_the_advice_fits_the_command() {
    // Outside the repository, so no `Cargo.toml` is found upward.
    let dir = std::env::temp_dir().join(format!("varyk-no-package-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    if dir
        .ancestors()
        .any(|folder| folder.join("Cargo.toml").is_file())
    {
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    let check = varyk_in(&dir, &["check"]);
    let publish = varyk_in(&dir, &["publish"]);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(
        stderr(&check),
        "error: no Varyk package here; name a `.vr` file (as in `varyk run main.vr`) or run \
         inside a package; `varyk init` makes one\n"
    );
    assert_eq!(publish.status.code(), Some(1));
    assert_eq!(
        stderr(&publish),
        "error: no Varyk package here; run `varyk publish` inside a package (make one with \
         `varyk init`)\n"
    );
}

#[test]
fn emit_out_dir_writes_the_tree_and_copies_rs_files_without_running_cargo() {
    let dir = package_dir("with_dep");
    let out = dir.join("emitted");
    let empty = dir.join("no-tools");
    fs::create_dir_all(&empty).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_varyk"))
        .args(["emit", "--out-dir"])
        .arg(&out)
        .current_dir(&dir)
        .env("PATH", &empty)
        .output()
        .expect("spawn varyk");

    assert_success(&output);
    assert!(!dir.join("target").exists(), "emit ran cargo");
    assert!(out.join("src/main.rs").is_file());
    assert!(
        !out.join("Cargo.toml").exists(),
        "emit writes only the tree"
    );

    let copied = fs::read(out.join("src/util.rs")).unwrap();
    let original = fs::read(dir.join("src/util.rs")).unwrap();
    assert_eq!(copied, original, "a copied .rs module is byte-for-byte");
}

#[test]
fn init_in_an_empty_dir_writes_the_five_files_named_after_the_directory() {
    let parent = empty_dir("init_basic");
    let dir = parent.join("greeting");
    fs::create_dir_all(&dir).unwrap();

    let output = varyk_in(&dir, &["init"]);

    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "created the package `greeting` in `.`; run `varyk run` or `cargo run`\n"
    );
    for path in [
        "Cargo.toml",
        ".gitignore",
        "build.rs",
        "src/main.rs",
        "src/main.vr",
    ] {
        assert!(dir.join(path).is_file(), "missing {path}");
    }
    assert!(!dir.join("src/lib.rs").exists());
    assert!(!dir.join("src/lib.vr").exists());

    let manifest = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("name = \"greeting\""), "{manifest}");
    assert!(manifest.contains("edition = \"2024\""), "{manifest}");
    assert!(!manifest.contains("[workspace]"), "{manifest}");
    assert!(
        manifest.ends_with(&format!(
            "[dependencies]\nvaryk-std = \"{}\"\n",
            env!("CARGO_PKG_VERSION")
        )),
        "{manifest}"
    );

    let stub = fs::read_to_string(dir.join("src/main.rs")).unwrap();
    assert_eq!(
        stub,
        "::std::include!(::std::concat!(::std::env!(\"OUT_DIR\"), \"/varyk/src/main.rs\"));\n"
    );
    let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert_eq!(gitignore, "/target\n.env\n");
}

#[test]
fn init_lib_writes_lib_rs_and_lib_vr_instead_of_main() {
    let parent = empty_dir("init_lib");
    let dir = parent.join("shapes");
    fs::create_dir_all(&dir).unwrap();

    let output = varyk_in(&parent, &["init", "--lib", "shapes"]);

    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "created the package `shapes` in `shapes`; run `cd shapes`, then `varyk build` or `cargo build`\n"
    );
    assert!(dir.join("src/lib.rs").is_file());
    assert!(dir.join("src/lib.vr").is_file());
    assert!(!dir.join("src/main.rs").exists());
    assert!(!dir.join("src/main.vr").exists());

    let stub = fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert_eq!(
        stub,
        "::std::include!(::std::concat!(::std::env!(\"OUT_DIR\"), \"/varyk/src/lib.rs\"));\n"
    );
}

/// `init` inside a Varyk package says so first, for either kind.
#[test]
fn init_in_a_varyk_package_says_it_is_one() {
    let dir = empty_dir("init_twice");
    assert_success(&varyk_in(&dir, &["init"]));

    let again = varyk_in(&dir, &["init"]);
    assert_eq!(again.status.code(), Some(1));
    assert_eq!(
        stderr(&again),
        "error: this directory is already a Varyk package (it has `Cargo.toml` and \
         `src/main.vr`), so `varyk init` wrote nothing\n"
    );
    let lib = varyk_in(&dir, &["init", "--lib"]);
    assert_eq!(lib.status.code(), Some(1));
    assert!(
        stderr(&lib).starts_with("error: this directory is already a Varyk package: `src/main.vr`"),
        "{}",
        stderr(&lib)
    );
}

#[test]
fn init_refuses_when_cargo_toml_already_exists_and_writes_nothing() {
    let dir = empty_dir("init_conflict");
    fs::write(dir.join("Cargo.toml"), "leave me alone").unwrap();

    let output = varyk_in(&dir, &["init"]);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert_eq!(
        message,
        "error: these files already exist: Cargo.toml; to add Varyk to an existing project, \
         run `varyk init` in an empty directory, move its `build.rs` (merging by hand if you \
         already have one) and `src/main.vr` into your project, and replace your \
         `src/main.rs` with its `src/main.rs` after moving your code into a `.rs` module; the \
         package needs `edition = \"2024\"`, and a Rust project with nested modules cannot \
         adopt Varyk yet\n"
    );
    assert!(
        !dir.join("build.rs").exists(),
        "init wrote past the conflict"
    );
    assert!(!dir.join("src").exists(), "init wrote past the conflict");
    assert_eq!(
        fs::read_to_string(dir.join("Cargo.toml")).unwrap(),
        "leave me alone"
    );
}

/// A fresh `init`ed package's binary, built and run with plain `cargo`,
/// `VARYK` pointing at the `varyk` under test.
fn cargo_build_and_run(dir: &Path) -> Output {
    let build = Command::new("cargo")
        .arg("build")
        .args(["--config", &std_config()])
        .current_dir(dir)
        .env("VARYK", env!("CARGO_BIN_EXE_varyk"))
        .output()
        .expect("spawn cargo build");
    assert_success(&build);

    let manifest: toml::Table = fs::read_to_string(dir.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let name = manifest["package"]["name"].as_str().unwrap().to_string();
    // Cargo builds under `CARGO_TARGET_DIR` when it is set (relative to
    // where cargo ran), else under the package's own `target`.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| dir.join("target"), |target| dir.join(target));
    let exe = target.join("debug").join(&name);
    assert!(exe.is_file(), "{exe:?}");

    Command::new(&exe).output().expect("spawn the built binary")
}

#[test]
fn cargo_build_of_an_inited_package_compiles_and_runs_with_varyk_on_the_path() {
    let dir = empty_dir("cargo_build");
    assert_success(&varyk_in(&dir, &["init"]));

    let output = cargo_build_and_run(&dir);
    assert_success(&output);
    assert_eq!(stdout(&output), "Hello, world!\n");

    // Editing `main.vr` and rebuilding picks up the change: `build.rs`
    // reruns because it declares `rerun-if-changed=src`.
    fs::write(
        dir.join("src/main.vr"),
        "fn main() {\n    println!(\"updated\");\n}\n",
    )
    .unwrap();

    let output = cargo_build_and_run(&dir);
    assert_success(&output);
    assert_eq!(stdout(&output), "updated\n");
}

#[test]
fn an_inited_package_with_an_in_tree_path_dependency_builds_with_varyk_and_cargo() {
    // The task-10 carry-forward: a `path` dependency living inside the
    // package used to fail `varyk build` unless the dependency had its
    // own `[workspace]` table, because cargo parsed the enclosing
    // package's target-less manifest while walking up from it. Once
    // `init` has written the stub `src/main.rs`, the enclosing manifest
    // has a target, and the root workspace's `exclude` (this task) keeps
    // cargo from treating either package as a member of the *compiler's*
    // workspace; neither manifest below declares `[workspace]`.
    let dir = empty_dir("with_in_tree_dep");
    assert_success(&varyk_in(&dir, &["init"]));

    fs::create_dir_all(dir.join("dep/src")).unwrap();
    fs::write(
        dir.join("dep/Cargo.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("dep/src/lib.rs"),
        "pub fn answer() -> i64 {\n    42\n}\n",
    )
    .unwrap();

    let mut manifest = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    manifest.push_str("dep = { version = \"0.1.0\", path = \"dep\" }\n");
    fs::write(dir.join("Cargo.toml"), manifest).unwrap();

    fs::write(
        dir.join("src/util.rs"),
        "pub fn answer() -> i64 {\n    dep::answer()\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/main.vr"),
        "mod util;\n\nfn main() {\n    println!(\"the answer is {}\", util::answer());\n}\n",
    )
    .unwrap();

    assert_success(&varyk_in(&dir, &["build"]));

    let output = cargo_build_and_run(&dir);
    assert_success(&output);
    assert_eq!(stdout(&output), "the answer is 42\n");
}

/// A package named `name` whose `dep` dependency exports `give_drop!`
/// and `println!`, which both write an `impl Drop`, and whose `hook.rs`
/// is `hook`; `main.vr` takes a payload out of a temporary of `ext::G2`.
fn package_with_dependency_drop_macro(name: &str, hook: &str) -> PathBuf {
    let dir = empty_dir(name);
    assert_success(&varyk_in(&dir, &["init"]));
    fs::create_dir_all(dir.join("dep/src")).unwrap();
    fs::write(
        dir.join("dep/Cargo.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("dep/src/lib.rs"),
        "#[macro_export]\nmacro_rules! give_drop {\n    ($t:ty) => {\n        impl Drop for $t {\n            fn drop(&mut self) {}\n        }\n    };\n}\n\n#[macro_export]\nmacro_rules! println {\n    ($t:ty) => {\n        impl Drop for $t {\n            fn drop(&mut self) {}\n        }\n    };\n}\n",
    )
    .unwrap();
    let mut manifest = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    manifest.push_str("dep = { version = \"0.1.0\", path = \"dep\" }\n");
    fs::write(dir.join("Cargo.toml"), manifest).unwrap();
    fs::write(
        dir.join("src/ext.rs"),
        "pub enum G2 {\n    Held(String),\n    Empty,\n}\n\npub fn make() -> G2 {\n    G2::Held(String::from(\"hi\"))\n}\n\npub fn keep(s: String) {\n    println!(\"{s}\");\n}\n",
    )
    .unwrap();
    fs::write(dir.join("src/hook.rs"), hook).unwrap();
    fs::write(
        dir.join("src/main.vr"),
        "mod ext;\nmod hook;\n\nfn main() {\n    match ext::make() {\n        ext::G2::Held(s) => ext::keep(s),\n        ext::G2::Empty => {}\n    }\n}\n",
    )
    .unwrap();
    dir
}

/// The package of [`package_with_dependency_drop_macro`]; returns the
/// stderr of a `check` that must fail with V0304: Varyk cannot see the
/// `impl Drop`, so it treats every enum as possibly running code when
/// thrown away, and rejects the `match` at `check` (rustc's E0509) rather
/// than at build.
fn check_with_dependency_drop_macro(name: &str, hook: &str) -> String {
    let dir = package_with_dependency_drop_macro(name, hook);
    let output = varyk_in(&dir, &["check"]);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(!output.status.success(), "check passes:\n{stderr}");
    assert!(stderr.contains("error[V0304]"), "{stderr}");
    stderr
}

/// An `impl Drop` a dependency's macro writes, reached through a local
/// macro that calls it.
#[test]
fn a_local_macro_relaying_to_a_dependency_macro_marks_every_enum() {
    let stderr = check_with_dependency_drop_macro(
        "relay_drop",
        "macro_rules! relay {\n    () => {\n        dep::give_drop!(crate::ext::G2);\n    };\n}\n\nrelay!();\n",
    );
    assert!(
        stderr.contains("`hook.rs` calls `relay!`, which calls a macro defined elsewhere"),
        "{stderr}"
    );
}

/// The same `impl Drop` reached through a local macro that pastes its
/// tokens as they are, or written by the dependency's macro called in a
/// `const _` block or a function body: every enum is marked all the same.
#[test]
fn a_dependency_macro_passed_through_or_called_in_a_body_marks_every_enum() {
    for (name, hook, why) in [
        (
            "pass_drop",
            "macro_rules! fwd {\n    ($($t:tt)*) => {\n        $($t)*\n    };\n}\n\nfwd! { dep::give_drop!(crate::ext::G2); }\n",
            "`hook.rs` calls `fwd!`, which calls a macro defined elsewhere",
        ),
        (
            "const_drop",
            "const _: () = {\n    dep::give_drop!(crate::ext::G2);\n};\n",
            "`hook.rs` calls `give_drop!`, a macro defined elsewhere",
        ),
        (
            "body_drop",
            "pub fn hook() {\n    dep::give_drop!(crate::ext::G2);\n}\n",
            "`hook.rs` calls `give_drop!`, a macro defined elsewhere",
        ),
    ] {
        let stderr = check_with_dependency_drop_macro(name, hook);
        assert!(stderr.contains(why), "{name}: {stderr}");
    }
}

/// A `std::name!` path is the standard library's only when the file
/// names nothing `std` or `core`: here `std` is the dependency, whose
/// `println!` writes an `impl Drop`.
#[test]
fn a_std_path_is_not_trusted_when_the_file_names_something_std() {
    for (name, hook) in [
        (
            "use_as_std",
            "use dep as std;\n\npub fn hook() {\n    std::println!(crate::ext::G2);\n}\n",
        ),
        (
            "mod_std",
            "mod std {\n    pub use dep::println;\n}\n\npub fn hook() {\n    std::println!(crate::ext::G2);\n}\n",
        ),
    ] {
        let stderr = check_with_dependency_drop_macro(name, hook);
        assert!(stderr.contains("println!"), "{name}: {stderr}");
    }
}

/// A leading `::std::` always names the standard library, so a facade
/// calling `::std::println!` keeps payload moves even beside
/// `use dep as std;`.
#[test]
fn a_leading_colon_std_path_is_trusted_beside_a_use_named_std() {
    let dir = package_with_dependency_drop_macro(
        "colon_std",
        "use dep as std;\n\npub fn hook() {\n    ::std::println!(\"hook\");\n}\n",
    );

    let output = varyk_in(&dir, &["run"]);

    assert_success(&output);
    assert_eq!(stdout(&output), "hi\n");
}

#[test]
fn another_file_in_src_is_a_single_file() {
    let dir = package_dir("basic");
    fs::write(
        dir.join("src/other.vr"),
        "fn main() {\n    println!(\"single\");\n}\n",
    )
    .unwrap();

    let output = varyk_in(&dir, &["run", "src/other.vr"]);

    assert_success(&output);
    assert_eq!(stdout(&output), "single\n");
    assert!(!dir.join("target/varyk/basic").exists());
}

// --- `varyk publish` (M3 spec 2.6) -----------------------------------------
//
// `assemble` is exercised through the library API (`varyk::driver::
// publish::assemble`), not through `cargo publish`, so these tests never
// touch the network. The `lib` fixture carries a nested `.vr` module
// (`shape`), a copied `.rs` module (`util`), a `README.md`, a
// `LICENSE-MIT`, and `license`/`description`/`readme` manifest fields, so
// every part of `assemble` is exercised and `cargo package` is happy.

/// Loads the package at `dir`, checks it, generates its crate, and
/// assembles it with the library API directly, returning the loaded
/// package and the assembled directory.
fn assemble_package(dir: &Path) -> (Package, PathBuf) {
    let mut sources = Vec::new();
    let package = package::load(&dir.join("Cargo.toml"), &mut sources).expect("loads");
    let text = fs::read_to_string(&package.entry).expect("read the entry file");
    let entry = SourceFile::new(FileId(sources.len() as u32), package.entry.clone(), text);
    let crates: Vec<String> = package.dependencies.keys().cloned().collect();
    let program = varyk::check_file(
        entry,
        package.kind,
        Some(varyk::Dependencies {
            crates: &crates,
            dev: &package.dev_dependencies,
        }),
        &mut sources,
    )
    .unwrap_or_else(|diagnostics| panic!("expected the program to check, got {diagnostics:#?}"));
    let info = CrateInfo {
        name: package.name.clone(),
        manifest: package.isolated_manifest(),
        std_dependency: None,
    };
    let generated = RustBackend.generate(&program, &info);
    let dest = publish::assemble(&package, &generated).expect("assembles");
    (package, dest)
}

#[test]
fn publish_assemble_writes_the_manifest_with_build_false_and_the_dependencies() {
    let dir = package_dir("lib");

    let (_package, dest) = assemble_package(&dir);

    assert_eq!(dest, dir.join("target/varyk/package/shapes"));
    let manifest: toml::Table = fs::read_to_string(dest.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(manifest["package"]["build"].as_bool(), Some(false));
    assert_eq!(manifest["package"]["name"].as_str(), Some("shapes"));
    assert_eq!(manifest["package"]["license"].as_str(), Some("MIT"));
    assert!(manifest["package"]["description"].as_str().is_some());
    assert_eq!(manifest["package"]["readme"].as_str(), Some("README.md"));
    assert!(
        manifest["dependencies"].as_table().unwrap().is_empty(),
        "the original (empty) [dependencies]"
    );
    assert!(!dest.join("build.rs").exists());
}

#[test]
fn publish_assemble_writes_the_generated_tree_module_files_copied_rs_and_vr_sources() {
    let dir = package_dir("lib");

    let (_package, dest) = assemble_package(&dir);

    assert!(
        dest.join("src/lib.rs").is_file(),
        "the generated root, not the init stub"
    );
    let lib_rs = fs::read_to_string(dest.join("src/lib.rs")).unwrap();
    assert!(!lib_rs.contains("include!"), "{lib_rs}");
    assert!(
        dest.join("src/shape.rs").is_file(),
        "a generated module file"
    );
    assert_eq!(
        fs::read_to_string(dest.join("src/util.rs")).unwrap(),
        fs::read_to_string(dir.join("src/util.rs")).unwrap(),
        "a copied .rs module, byte-for-byte"
    );
    for name in ["lib.vr", "shape.vr"] {
        assert_eq!(
            fs::read_to_string(dest.join("src").join(name)).unwrap(),
            fs::read_to_string(dir.join("src").join(name)).unwrap(),
            "{name} at its src/ place"
        );
    }
}

/// Every file under `dir`, recursively, by its path relative to `dir`,
/// leaving out `.vr` sources and the generated-tree marker.
fn tree_files(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("read the tree") {
            let path = entry.expect("read the tree").path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|ext| ext == "vr")
                || path.file_name() == Some(varyk::driver::generate::MARKER.as_ref())
            {
                continue;
            } else {
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Spec 2.3: one routine writes the tree for all three destinations, so
/// the hidden crate's `src/`, `emit --out-dir`'s `src/`, and the publish
/// assembly's `src/` are byte for byte the same.
#[test]
fn the_hidden_crate_emit_and_publish_write_the_same_src_tree() {
    let dir = package_dir("lib");

    assert_success(&varyk_in(&dir, &["build"]));
    assert_success(&varyk_in(&dir, &["emit", "--out-dir", "emitted"]));
    let (_package, assembled) = assemble_package(&dir);

    let hidden = tree_files(&dir.join("target/varyk/shapes/src"));
    assert!(
        hidden.contains_key(Path::new("lib.rs")),
        "{:?}",
        hidden.keys()
    );
    assert!(
        hidden.contains_key(Path::new("util.rs")),
        "{:?}",
        hidden.keys()
    );
    assert_eq!(
        hidden,
        tree_files(&dir.join("emitted/src")),
        "emit --out-dir"
    );
    assert_eq!(
        hidden,
        tree_files(&assembled.join("src")),
        "publish assembly"
    );
}

#[test]
fn publish_assemble_copies_the_readme_and_license_from_the_package_root() {
    let dir = package_dir("lib");

    let (_package, dest) = assemble_package(&dir);

    assert_eq!(
        fs::read_to_string(dest.join("README.md")).unwrap(),
        fs::read_to_string(dir.join("README.md")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(dest.join("LICENSE-MIT")).unwrap(),
        fs::read_to_string(dir.join("LICENSE-MIT")).unwrap()
    );
}

#[test]
fn cargo_package_succeeds_in_the_assembled_directory() {
    let dir = package_dir("lib");
    let (_package, dest) = assemble_package(&dir);

    let output = Command::new("cargo")
        .args(["package", "--no-verify"])
        .current_dir(&dest)
        .output()
        .expect("spawn cargo package");

    assert_success(&output);
}

#[test]
fn publish_assemble_of_a_binary_package_has_the_generated_root_and_cargo_packages() {
    let dir = package_dir("basic");

    let (_package, dest) = assemble_package(&dir);

    assert!(
        dest.join("src/main.rs").is_file(),
        "the generated root, not the init stub"
    );
    let main_rs = fs::read_to_string(dest.join("src/main.rs")).unwrap();
    assert!(!main_rs.contains("include!"), "{main_rs}");

    let output = Command::new("cargo")
        .args(["package", "--no-verify"])
        .current_dir(&dest)
        .output()
        .expect("spawn cargo package");

    assert_success(&output);
}

#[test]
fn a_consumer_with_no_varyk_builds_and_calls_the_assembled_library() {
    let dir = package_dir("lib");
    let (_package, dest) = assemble_package(&dir);
    // `assemble` never writes a `build.rs`, and the manifest it writes
    // sets `[package] build = false`, so nothing in the build below could
    // run `varyk` even if it were reachable.
    assert!(!dest.join("build.rs").exists());

    let consumer = empty_dir("publish_consumer");
    fs::write(
        consumer.join("Cargo.toml"),
        format!(
            "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nshapes = {{ path = {dest:?} }}\n"
        ),
    )
    .unwrap();
    fs::create_dir_all(consumer.join("src")).unwrap();
    fs::write(
        consumer.join("src/main.rs"),
        "fn main() {\n    println!(\"{}\", shapes::area(3, 4));\n}\n",
    )
    .unwrap();

    // `PATH` holds only cargo's own toolchain directory (which also has
    // `rustc`) plus the system linker directories; none of them can hold
    // a `varyk` binary, so this also proves cargo itself never looked for
    // one.
    let cargo = PathBuf::from(env!("CARGO"));
    let cargo_dir = cargo.parent().expect("cargo has a parent directory");
    let path = format!("{}:/usr/bin:/bin", cargo_dir.display());

    let output = Command::new(&cargo)
        .arg("run")
        .current_dir(&consumer)
        .env("PATH", &path)
        .output()
        .expect("spawn cargo run");

    assert_success(&output);
    assert_eq!(stdout(&output), "12\n");
}

#[test]
fn publish_from_the_package_root_assembles_before_handing_over_to_cargo() {
    // Run from the package root, the manifest is found as the relative
    // `Cargo.toml`, so the package root is the empty path; assembly must
    // still read that directory for its README and license. An argument
    // cargo rejects stops `cargo publish` before it could reach the
    // network, after the assembly is done.
    let dir = package_dir("lib");

    let output = varyk_in(&dir, &["publish", "--", "--no-such-flag"]);

    assert!(!output.status.success());
    assert!(
        !stderr(&output).contains("cannot assemble"),
        "{}",
        stderr(&output)
    );
    let dest = dir.join("target/varyk/package/shapes");
    assert!(dest.join("README.md").is_file());
    assert!(dest.join("LICENSE-MIT").is_file());
}

/// `--assemble-only` stops after the assembly and prints its directory:
/// a real `cargo publish` of this package, with no token, would fail.
#[test]
fn publish_assemble_only_assembles_prints_the_directory_and_runs_no_cargo() {
    let dir = package_dir("lib");

    let output = varyk_in(&dir, &["publish", "--assemble-only"]);

    assert_success(&output);
    let dest = dir.join("target/varyk/package/shapes");
    assert_eq!(
        dir.join(stdout(&output).trim_end()),
        dest,
        "{}",
        stdout(&output)
    );
    assert!(dest.join("Cargo.toml").is_file());
    assert!(dest.join("src/lib.rs").is_file());
    assert!(!dest.join("target").exists(), "cargo ran in the assembly");
}

#[test]
fn emit_into_the_package_itself_is_refused_and_every_file_is_left_intact() {
    let dir = package_dir("with_dep");
    let before: Vec<(PathBuf, Vec<u8>)> = fs::read_dir(dir.join("src"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();

    let output = varyk_in(&dir, &["emit", "--out-dir", "."]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let message = stderr(&output);
    assert!(message.contains("a file of this program"), "{message}");
    for (path, bytes) in &before {
        assert_eq!(
            &fs::read(path).unwrap(),
            bytes,
            "{} changed",
            path.display()
        );
    }
    assert_eq!(
        fs::read_dir(dir.join("src")).unwrap().count(),
        before.len(),
        "a file was added to src/"
    );
    assert!(!dir.join(".varyk-generated").exists());
}

#[test]
fn emit_twice_into_a_fresh_dir_clears_a_stale_file_the_second_time() {
    let dir = package_dir("with_dep");
    let out = empty_dir("emit_twice").join("out");

    assert_success(&varyk_in(
        &dir,
        &["emit", "--out-dir", out.to_str().unwrap()],
    ));
    fs::write(out.join("src/stale.rs"), "// stale").unwrap();
    assert_success(&varyk_in(
        &dir,
        &["emit", "--out-dir", out.to_str().unwrap()],
    ));

    assert!(out.join("src/main.rs").is_file());
    assert!(!out.join("src/stale.rs").exists(), "stale file remains");
}

#[test]
fn emit_into_a_directory_varyk_did_not_write_is_refused() {
    let dir = package_dir("with_dep");
    let out = empty_dir("emit_foreign");
    fs::create_dir_all(out.join("src")).unwrap();
    fs::write(out.join("src/notes.txt"), "mine").unwrap();

    let output = varyk_in(&dir, &["emit", "--out-dir", out.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("did not write"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(out.join("src/notes.txt")).unwrap(),
        "mine"
    );
}

#[test]
fn emit_into_a_non_empty_directory_with_no_src_is_refused() {
    let dir = package_dir("with_dep");
    let out = empty_dir("emit_notes");
    fs::write(out.join("notes.txt"), "mine").unwrap();

    let output = varyk_in(&dir, &["emit", "--out-dir", out.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("did not write"),
        "{}",
        stderr(&output)
    );
    let names: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["notes.txt"], "something was written");
    assert_eq!(fs::read_to_string(out.join("notes.txt")).unwrap(), "mine");
}

#[test]
fn emit_into_an_existing_empty_directory_then_again_into_it_works() {
    let dir = package_dir("with_dep");
    let out = empty_dir("emit_empty");

    assert_success(&varyk_in(
        &dir,
        &["emit", "--out-dir", out.to_str().unwrap()],
    ));
    assert!(out.join(".varyk-generated").is_file());
    assert_success(&varyk_in(
        &dir,
        &["emit", "--out-dir", out.to_str().unwrap()],
    ));
    assert!(out.join("src/main.rs").is_file());
}

/// A directory outside the package whose `src/keep.rs` is the user's.
fn victim(label: &str) -> PathBuf {
    let victim = empty_dir(label);
    fs::create_dir_all(victim.join("src")).unwrap();
    fs::write(victim.join("src/keep.rs"), "// mine").unwrap();
    victim
}

#[test]
fn emit_out_dir_going_up_out_of_target_varyk_is_refused() {
    let dir = package_dir("with_dep");
    let victim = victim("emit_dotdot_victim");
    fs::create_dir_all(dir.join("target/varyk")).unwrap();
    let name = victim.file_name().unwrap().to_str().unwrap();
    // `dir` and `victim` are siblings under `packages/`.
    let out = format!("target/varyk/../../../{name}");

    let output = varyk_in(&dir, &["emit", "--out-dir", &out]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("`..`"), "{}", stderr(&output));
    assert_eq!(
        fs::read_to_string(victim.join("src/keep.rs")).unwrap(),
        "// mine"
    );
}

#[cfg(unix)]
#[test]
fn emit_out_dir_through_a_link_in_target_varyk_is_refused() {
    let dir = package_dir("with_dep");
    let victim = victim("emit_link_victim");
    fs::create_dir_all(dir.join("target/varyk")).unwrap();
    std::os::unix::fs::symlink(&victim, dir.join("target/varyk/link")).unwrap();

    let output = varyk_in(&dir, &["emit", "--out-dir", "target/varyk/link"]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("did not write"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(victim.join("src/keep.rs")).unwrap(),
        "// mine"
    );
}

#[cfg(unix)]
#[test]
fn emit_out_dir_that_is_a_dangling_link_is_refused_as_a_link() {
    let dir = package_dir("with_dep");
    let links = empty_dir("emit_dangling");
    let out = links.join("out");
    std::os::unix::fs::symlink(links.join("missing"), &out).unwrap();

    let output = varyk_in(&dir, &["emit", "--out-dir", out.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("is a symbolic link"),
        "{}",
        stderr(&output)
    );
    assert!(!links.join("missing").exists());
}

/// `write_tree` checks only `DIR` and `DIR/src` for a link: cargo's
/// `OUT_DIR` under the system temporary directory (`/var/folders/...` on
/// macOS, where `/var` links to `/private/var`) keeps working.
#[test]
fn cargo_build_of_an_inited_package_in_the_system_temp_dir_works() {
    let dir = std::env::temp_dir().join(format!("varyk-temp-build-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    if dir
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .any(|folder| folder.join("Cargo.toml").is_file())
    {
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    assert_success(&varyk_in(&dir, &["init"]));

    let output = cargo_build_and_run(&dir);
    let _ = fs::remove_dir_all(&dir);
    assert_success(&output);
    assert_eq!(stdout(&output), "Hello, world!\n");
}

#[test]
fn a_cargo_failure_with_no_compiler_message_prints_cargo_s_reason_once() {
    let dir = package_dir("basic");
    let manifest = dir.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        format!("{text}\n[dependencies]\nouterr = {{ path = \"../outerr\" }}\n"),
    )
    .unwrap();

    let output = varyk_in(&dir, &["build"]);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert_eq!(
        message.matches("cargo could not build the package").count(),
        1,
        "{message}"
    );
    assert!(!message.contains("did not compile"), "{message}");
    let first = message
        .find("outerr")
        .expect("cargo's own reason names the dependency");
    let cargo_line = message[first..].lines().next().unwrap();
    assert_eq!(message.matches(cargo_line).count(), 1, "{message}");
}

#[test]
fn publish_assemble_makes_a_relative_dev_dependency_path_absolute() {
    let dir = package_dir("lib");
    let name = format!(
        "{}-helper",
        dir.file_name().unwrap().to_str().expect("utf-8 name")
    );
    let helper = dir.with_file_name(&name);
    fs::create_dir_all(helper.join("src")).unwrap();
    fs::write(
        helper.join("Cargo.toml"),
        "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(helper.join("src/lib.rs"), "").unwrap();
    let manifest = dir.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        format!("{text}\n[dev-dependencies]\nhelper = {{ path = \"../{name}\" }}\n"),
    )
    .unwrap();

    let (_package, dest) = assemble_package(&dir);

    let written: toml::Table = fs::read_to_string(dest.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let path = written["dev-dependencies"]["helper"]["path"]
        .as_str()
        .unwrap();
    assert!(Path::new(path).is_absolute(), "{path}");
    let output = Command::new("cargo")
        .args(["package", "--no-verify", "--allow-dirty"])
        .current_dir(&dest)
        .output()
        .expect("spawn cargo package");
    assert_success(&output);
}

#[test]
fn cargo_build_of_an_inited_package_explains_an_emit_failure_and_a_missing_varyk() {
    let dir = empty_dir("cargo_build_fails");
    assert_success(&varyk_in(&dir, &["init"]));
    fs::write(dir.join("src/main.vr"), "fn main() {\n    let x = ;\n}\n").unwrap();

    let broken = Command::new("cargo")
        .arg("build")
        .args(["--config", &std_config()])
        .current_dir(&dir)
        .env("VARYK", env!("CARGO_BIN_EXE_varyk"))
        .output()
        .expect("spawn cargo build");
    assert!(!broken.status.success());
    assert!(
        stderr(&broken).contains("run `varyk build` to see this without cargo's wrapping"),
        "{}",
        stderr(&broken)
    );

    let missing = Command::new("cargo")
        .arg("build")
        .args(["--config", &std_config()])
        .current_dir(&dir)
        .env("VARYK", "/no/such/varyk")
        .output()
        .expect("spawn cargo build");
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("varyk was not found (tried `/no/such/varyk`)"),
        "{}",
        stderr(&missing)
    );
}

/// A dependency's build script fails after another dependency printed a
/// warning: the warning is no error, so cargo's own reason is still
/// shown, and nothing claims the generated Rust did not compile.
#[test]
fn a_failing_build_script_after_a_warning_still_prints_cargo_s_reason() {
    let dir = package_dir("warning_and_failing_build");

    let output = varyk_in(&dir, &["build"]);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("unused variable"), "{message}");
    assert!(
        message.contains("cargo could not build the package")
            && message.contains("this build script fails on purpose"),
        "{message}"
    );
    assert!(!message.contains("did not compile"), "{message}");
}

/// Whether a crate is a dev-dependency is read from the package's own
/// `Cargo.toml`, never from one in a subdirectory next to a `.rs` module
/// (a vendored copy of some crate, say).
#[test]
fn a_cargo_toml_beside_a_rs_module_is_not_consulted_for_dev_dependencies() {
    let dir = package_dir("basic");
    fs::write(
        dir.join("src/main.vr"),
        "mod shop;\n\nfn main() {\n    shop::run();\n}\n",
    )
    .unwrap();
    fs::write(dir.join("src/shop.vr"), "mod util;\n\npub fn run() {}\n").unwrap();
    fs::create_dir_all(dir.join("src/shop")).unwrap();
    fs::write(dir.join("src/shop/util.rs"), "use rand::Rng;\n").unwrap();
    fs::write(
        dir.join("src/shop/Cargo.toml"),
        "[package]\nname = \"vendored\"\nversion = \"0.1.0\"\n\n[dev-dependencies]\nrand = \"0.8\"\n",
    )
    .unwrap();

    let output = varyk_in(&dir, &["check"]);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("error[V0104]"), "{message}");
    assert!(
        message.contains("which is not in `[dependencies]`"),
        "{message}"
    );
    assert!(!message.contains("dev-dependencies"), "{message}");
}

#[test]
fn init_of_the_current_directory_says_no_cd_and_quotes_one_with_a_space() {
    let dir = empty_dir("init_dot").join("dot");
    fs::create_dir_all(&dir).unwrap();
    let output = varyk_in(&dir, &["init", "."]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "created the package `dot` in `.`; run `varyk run` or `cargo run`\n"
    );

    let parent = empty_dir("init_space");
    let output = varyk_in(&parent, &["init", "my shop"]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "created the package `my_shop` in `my shop`; run `cd 'my shop'`, then `varyk run` or `cargo run`\n"
    );

    // Any character a shell treats specially is quoted, a `'` escaped.
    let parent = empty_dir("init_quote");
    let output = varyk_in(&parent, &["init", "it's"]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "created the package `it_s` in `it's`; run `cd 'it'\\''s'`, then `varyk run` or `cargo run`\n"
    );
}

#[test]
fn init_lib_refuses_where_src_main_vr_exists() {
    let dir = empty_dir("init_both_roots");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.vr"), "fn main() {}\n").unwrap();

    let output = varyk_in(&dir, &["init", "--lib"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("`src/main.vr` already exists"),
        "{}",
        stderr(&output)
    );
    assert!(!dir.join("src/lib.vr").exists());
    assert!(!dir.join("Cargo.toml").exists());
}

/// A package with no `varyk-std` line and a sibling crate `dep` to add.
fn package_with_a_dep(label: &str) -> PathBuf {
    let dir = empty_dir(label);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n",
    )
    .unwrap();
    fs::write(dir.join("src/main.vr"), "fn main() {}\n").unwrap();
    // The stub `varyk init` writes; cargo add wants a target.
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::create_dir_all(dir.join("dep/src")).unwrap();
    fs::write(
        dir.join("dep/Cargo.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(dir.join("dep/src/lib.rs"), "").unwrap();
    dir
}

#[test]
fn add_runs_cargo_add_in_the_package_directory_from_a_subdirectory() {
    let dir = package_with_a_dep("add");

    // Cargo runs in the package directory, so a relative `--path` is
    // relative to it, wherever `varyk` was started. Colour is off so the
    // text check below holds where CI sets `CARGO_TERM_COLOR=always`.
    let output = varyk_run_with(
        &["add", "dep", "--path", "dep", "--offline"],
        &dir.join("src"),
        &[("CARGO_TERM_COLOR", "never")],
        &[],
    );

    assert_success(&output);
    let manifest = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains("dep = { version = \"0.1.0\", path = \"dep\" }"),
        "{manifest}"
    );
    assert!(
        stderr(&output).contains("Adding dep"),
        "cargo's output is passed through: {}",
        stderr(&output)
    );
}

#[test]
fn add_passes_cargos_failure_through_as_an_exit_code() {
    let dir = package_with_a_dep("add_fails");

    let output = varyk_in(&dir, &["add", "dep", "--path", "no-such-dir", "--offline"]);

    assert_eq!(output.status.code(), Some(101), "{}", stderr(&output));
    assert!(!stderr(&output).is_empty());
}

#[test]
fn add_outside_a_package_says_so() {
    let dir = std::env::temp_dir().join(format!("varyk-add-nowhere-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // Only meaningful where no `Cargo.toml` sits above the temporary directory.
    if dir
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .any(|folder| folder.join("Cargo.toml").is_file())
    {
        let _ = fs::remove_dir_all(&dir);
        return;
    }

    let output = varyk_in(&dir, &["add", "serde"]);
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("no Varyk package here"),
        "{}",
        stderr(&output)
    );
}

// --- varyk test (M5a spec 2.7) ------------------------------------------------

/// `varyk test` in a package with a passing and a failing test runs both,
/// exits non-zero, and names the failing test and its Varyk line.
#[test]
fn varyk_test_reports_a_failing_test_with_its_varyk_line() {
    let dir = package_dir("failing_test");

    let output = varyk_in(&dir, &["test"]);

    let (out, err) = (stdout(&output), stderr(&output));
    assert!(
        !output.status.success() && output.status.code().is_some(),
        "{:?}\n{out}\n{err}",
        output.status
    );
    assert!(out.contains("test store::adds_two_numbers ... ok"), "{out}");
    assert!(out.contains("test store::adds_wrongly ... FAILED"), "{out}");
    let all = format!("{out}{err}");
    assert!(
        all.contains("assertion failed at src/store.vr:12: left is 4, right is 5"),
        "{all}"
    );
}

/// The location in an assertion's message is relative to the package
/// root wherever `varyk test` runs, so the generated Rust never carries
/// the directory it was run from (M5a spec 2.7).
#[test]
fn varyk_test_from_a_subdirectory_names_the_line_from_the_package_root() {
    let dir = package_dir("failing_test");

    let output = varyk_in(&dir.join("src"), &["test"]);

    let all = format!("{}{}", stdout(&output), stderr(&output));
    assert!(!output.status.success(), "{all}");
    assert!(
        all.contains("assertion failed at src/store.vr:12: left is 4, right is 5"),
        "{all}"
    );
    let generated = fs::read_to_string(dir.join("target/varyk/failing_test/src/store.rs"))
        .expect("the generated module");
    assert!(
        generated.contains("\"assertion failed at src/store.vr:7"),
        "{generated}"
    );
    assert!(
        !generated.contains(dir.to_string_lossy().as_ref()),
        "{generated}"
    );
}

/// `varyk test` on the `users` example passes, in a copy, so nothing is
/// written into the source tree.
#[test]
fn varyk_test_passes_on_the_users_example() {
    let dir = common::example_dir("users");

    let output = varyk_in(&dir, &["test"]);

    let (out, err) = (stdout(&output), stderr(&output));
    assert!(output.status.success(), "{out}\n{err}");
    assert!(
        out.contains("test store::rejects_a_user_with_no_name ... ok"),
        "{out}"
    );
    assert!(out.contains("3 passed; 0 failed"), "{out}");
}
