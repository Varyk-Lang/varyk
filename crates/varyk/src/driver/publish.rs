//! `varyk publish` (spec 2.6): assembles a plain Rust crate at
//! `<package root>/target/varyk/package/<name>/` that a consumer builds
//! or installs with no `varyk` on the path, then hands it to `cargo
//! publish`. This module owns only the assembly (`assemble`); `cli.rs`
//! runs the full check, generates the crate, calls it, and spawns `cargo
//! publish` in the directory it returns.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::backend::GeneratedCrate;
use crate::package::Package;

use super::generate::{has_entries, owned, write_tree};

/// Assembles a plain Rust crate for `package` at `<package root>/target/
/// varyk/package/<name>/` (spec 2.6) and returns that directory. The
/// directory is cleared first (like the hidden crate's `src/`, spec 2.3),
/// then filled with:
///
/// - the tree of spec 2.3 (`write_tree`), so `src/main.rs`/`src/lib.rs`
///   is the generated root, not the `init` stub, and every copied `.rs`
///   module sits at its place;
/// - every `.vr` source under `package`'s `src/`, alongside the
///   generated or copied file it produced, for readers;
/// - the user's `Cargo.toml`, with `[package] build = false`,
///   the layout keys dropped, paths made absolute, and an empty
///   `[workspace]` table, exactly as `Package::isolated_manifest` makes
///   it for the hidden crate (spec 2.4), so `cargo package`/`cargo
///   publish` never climbs up into the package root's own, unbuildable
///   manifest or a real enclosing workspace, and what `varyk build`
///   accepted is what gets published;
/// - the files the manifest's `readme` and `license-file` name, and any
///   `README*`/`LICENSE*` at the package root.
///
/// No `build.rs` is ever written, so the assembled crate needs no
/// `varyk`: `cargo build`, `cargo publish`, and `cargo install` all work
/// against it like any plain Rust crate.
pub fn assemble(package: &Package, crate_: &GeneratedCrate) -> io::Result<PathBuf> {
    let dest = package
        .root
        .join("target/varyk/package")
        .join(&package.name);
    // `dest` is cleared, so it is held to `write_tree`'s rule: not a link
    // itself (a linked `target/` further up is fine, as it is for
    // `varyk build`), and either empty or carrying the marker `write_tree`
    // left the last time; anything else is somebody's files.
    if fs::symlink_metadata(&dest).is_ok_and(|meta| meta.is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "`{}` is a symbolic link; `varyk publish` does not follow links there",
                dest.display()
            ),
        ));
    }
    if dest.exists() {
        // The same rule `write_tree` applies: ours (the marker, checked
        // without following links) or empty, else somebody's files.
        if !owned(&dest) && has_entries(&dest) {
            return Err(io::Error::other(format!(
                "`{}` holds files Varyk did not write; move it away",
                dest.display()
            )));
        }
        fs::remove_dir_all(&dest)?;
    }
    fs::create_dir_all(&dest)?;

    write_tree(crate_, &dest)?;

    write_manifest(package, &dest)?;
    // The lock pins what `varyk build` built against; without it `cargo
    // publish` would resolve afresh and could verify against newer code.
    if let Some(lock) = &package.lock {
        fs::copy(lock, dest.join("Cargo.lock"))?;
    }
    copy_vr_sources(&package.root.join("src"), &dest.join("src"))?;
    copy_readme_and_license(&package.manifest, package, &dest)?;

    Ok(dest)
}

/// Writes `dest/Cargo.toml`: the package's isolated manifest
/// (`Package::isolated_manifest`), the very one the hidden crate is built
/// with, so what `varyk build` accepted is what gets published.
fn write_manifest(package: &Package, dest: &Path) -> io::Result<()> {
    fs::write(
        dest.join("Cargo.toml"),
        package.isolated_manifest().to_string(),
    )
}

/// Copies every `.vr` file under `src_dir` to the same relative path
/// under `dest_src_dir` (spec 2.6: "the `.vr` sources at their places in
/// `src/`, for readers"), beside the `.rs` file `write_tree` already
/// wrote or copied there.
fn copy_vr_sources(src_dir: &Path, dest_src_dir: &Path) -> io::Result<()> {
    // The directory itself may be a link too (`src -> elsewhere`); the
    // same rule as for the entries below.
    if fs::symlink_metadata(src_dir).is_ok_and(|meta| meta.is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "`{}` is a symbolic link; `varyk publish` does not follow links, a published \
                 crate carries only files under its own root",
                src_dir.display()
            ),
        ));
    }
    if !src_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(src_dir)? {
        let entry = entry?;
        let path = entry.path();
        // `file_type` does not follow links. A link under `src/` could
        // reach files outside the package, or itself, so it is refused
        // rather than followed, like `emit` refuses a linked directory.
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "`{}` is a symbolic link; `varyk publish` does not follow links, a published \
                     crate carries only files under its own root",
                    path.display()
                ),
            ));
        }
        if file_type.is_dir() {
            copy_vr_sources(&path, &dest_src_dir.join(entry.file_name()))?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("vr") {
            fs::create_dir_all(dest_src_dir)?;
            fs::copy(&path, dest_src_dir.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Copies the manifest's `readme` and `license-file` targets, and any
/// `README*`/`LICENSE*` file at the package root, to the same
/// root-relative path under `dest` (spec 2.6). The two are not the same
/// kind of copy: a manifest field names a specific file the manifest
/// promises exists, so a missing one is an error naming the field and
/// the path; the root glob just picks up whatever is there, so it is
/// silently best-effort.
fn copy_readme_and_license(
    manifest: &toml::Table,
    package: &Package,
    dest: &Path,
) -> io::Result<()> {
    if let Some(toml::Value::Table(package_table)) = manifest.get("package") {
        let mut copied: Vec<&str> = Vec::new();
        // Both fields may name one file, `NOTICE` and `./NOTICE` alike; it
        // is copied once.
        let same = |a: &str, b: &str| {
            let parts = |name: &str| {
                Path::new(name)
                    .components()
                    .filter(|c| *c != Component::CurDir)
                    .map(|c| c.as_os_str().to_owned())
                    .collect::<Vec<_>>()
            };
            parts(a) == parts(b)
        };
        for key in ["readme", "license-file"] {
            if let Some(toml::Value::String(name)) = package_table.get(key) {
                if copied.iter().any(|done| same(done, name)) {
                    continue;
                }
                copy_named_file(package, dest, key, name)?;
                copied.push(name);
            }
        }
    }
    copy_root_glob(package, dest)
}

/// Copies the file at `name` (relative to `package`'s root, possibly
/// nested, e.g. `docs/README.md`) to the same relative path under
/// `dest`, creating parent directories as needed. `key` is the manifest
/// field that named it, for the error when it is not actually there.
fn copy_named_file(package: &Package, dest: &Path, key: &str, name: &str) -> io::Result<()> {
    // The file lands at the same relative path under `dest`, so a name
    // that climbs out of the package root (`../README.md`, an absolute
    // path) would climb out of `dest` as well and overwrite whatever is
    // there; cargo accepts such names, the assembled crate cannot carry
    // them.
    let escapes = Path::new(name)
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
    if escapes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "`{key}` names `{name}`, which lies outside the package; a published crate can \
                 only carry files under its own root"
            ),
        ));
    }
    let source = package.root.join(name);
    if !source.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("`{key}` names `{name}`, which does not exist"),
        ));
    }
    // A link (the file itself, or a directory on its path) can lead
    // outside the root too; the resolved file must still be under the
    // resolved root.
    let root = if package.root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        package.root.as_path()
    };
    if !fs::canonicalize(&source)?.starts_with(fs::canonicalize(root)?) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "`{key}` names `{name}`, a link to a file outside the package; a published \
                 crate can only carry files under its own root"
            ),
        ));
    }
    let target = dest.join(name);
    // The generated tree is already there; a name that lands on one of its
    // files (`src/lib.rs`, `Cargo.toml`) would replace generated code.
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "`{key}` names `{name}`, which is a file of the assembled crate; name a file \
                 the crate does not generate"
            ),
        ));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&source, &target)?;
    Ok(())
}

/// Copies any `README*`/`LICENSE*` file at `package`'s root to the same
/// name under `dest`; best-effort, since these are not named by the
/// manifest and so cannot be missing.
fn copy_root_glob(package: &Package, dest: &Path) -> io::Result<()> {
    // Found from the package root itself, the root is the empty path,
    // which `read_dir` does not accept as the current directory.
    let root = if package.root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        package.root.as_path()
    };
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let wanted = name
            .to_str()
            .is_some_and(|name| name.starts_with("README") || name.starts_with("LICENSE"));
        if wanted {
            fs::copy(entry.path(), dest.join(&name))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::generate::MARKER;
    use super::*;
    use crate::backend::{GeneratedFile, Writer};
    use crate::package::Kind;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory removed when it goes out of scope, even on a
    /// test failure.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "varyk_publish_test_{label}_{}_{n}",
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

    fn generated_file(path: &str, line: &str) -> GeneratedFile {
        let mut writer = Writer::new(path);
        writer.line(0, line, None);
        writer.finish()
    }

    fn package_at(root: &Path, name: &str, dependencies: toml::Table) -> Package {
        Package {
            root: root.to_path_buf(),
            name: name.to_string(),
            version: "0.1.0".to_string(),
            kind: Kind::Library,
            entry: root.join("src/lib.vr"),
            dependencies,
            dev_dependencies: Vec::new(),
            manifest: fs::read_to_string(root.join("Cargo.toml"))
                .expect("the test writes Cargo.toml first")
                .parse()
                .expect("a valid manifest"),
            workspace: None,
            lock: None,
            std_spans: crate::package::StdSpans {
                anchor: varyk_syntax::Span::new(varyk_syntax::FileId(0), 0, 0),
                entry: None,
            },
        }
    }

    #[test]
    fn assemble_writes_build_false_drops_the_layout_keys_and_keeps_the_other_fields() {
        let dir = TempDir::new("manifest");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             license = \"MIT\"\n\
             include = [\"src/**/*.vr\"]\nexclude = [\"notes/\"]\nworkspace = \"..\"\n\
             \n\
             [dependencies]\nregex = \"1\"\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        let mut dependencies = toml::Table::new();
        dependencies.insert("regex".to_string(), toml::Value::String("1".to_string()));
        let package = package_at(&dir.0, "shapes", dependencies);
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let dest = assemble(&package, &crate_).expect("assembles");

        assert_eq!(dest, dir.0.join("target/varyk/package/shapes"));
        let manifest: toml::Table = fs::read_to_string(dest.join("Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(manifest["package"]["build"].as_bool(), Some(false));
        assert_eq!(manifest["package"]["license"].as_str(), Some("MIT"));
        for key in ["include", "exclude", "workspace"] {
            assert!(manifest["package"].get(key).is_none(), "{key} was kept");
        }
        assert_eq!(manifest["dependencies"]["regex"].as_str(), Some("1"));
        assert!(manifest["workspace"].as_table().unwrap().is_empty());
        assert!(!dest.join("build.rs").exists());
    }

    #[test]
    fn assemble_makes_auxiliary_dependency_paths_absolute_under_target_tables_too() {
        let dir = TempDir::new("aux_paths");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [build-dependencies]\ngen = { path = \"../gen\" }\n\n\
             [target.'cfg(unix)'.dev-dependencies]\nprobe = { path = \"../probe\" }\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let dest = assemble(&package, &crate_).expect("assembles");

        let manifest: toml::Table = fs::read_to_string(dest.join("Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();
        for path in [
            manifest["build-dependencies"]["gen"]["path"]
                .as_str()
                .unwrap(),
            manifest["target"]["cfg(unix)"]["dev-dependencies"]["probe"]["path"]
                .as_str()
                .unwrap(),
        ] {
            assert!(Path::new(path).is_absolute(), "{path}");
        }
    }

    #[test]
    fn assemble_writes_the_generated_tree_and_copies_the_rs_and_vr_sources() {
        let dir = TempDir::new("tree");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        dir.write("src/lib.vr", "mod util;\n\npub fn area() {}\n");
        dir.write("src/shape.vr", "pub struct Shape;\n");
        let util_rs = dir.write("src/util.rs", "pub fn scale(x: i64) -> i64 {\n    x\n}\n");
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![
                generated_file("src/lib.rs", "pub fn area() {}"),
                generated_file("src/shape.rs", "pub struct Shape;"),
            ],
            vec![("src/util.rs".to_string(), util_rs)],
        );

        let dest = assemble(&package, &crate_).expect("assembles");

        assert!(dest.join("src/lib.rs").is_file(), "generated root");
        assert!(dest.join("src/shape.rs").is_file(), "generated module");
        assert_eq!(
            fs::read_to_string(dest.join("src/util.rs")).unwrap(),
            "pub fn scale(x: i64) -> i64 {\n    x\n}\n",
            "copied .rs, byte-for-byte"
        );
        assert_eq!(
            fs::read_to_string(dest.join("src/lib.vr")).unwrap(),
            "mod util;\n\npub fn area() {}\n"
        );
        assert_eq!(
            fs::read_to_string(dest.join("src/shape.vr")).unwrap(),
            "pub struct Shape;\n"
        );
    }

    #[test]
    fn assemble_copies_the_readme_and_license_named_by_the_manifest_and_at_the_root() {
        let dir = TempDir::new("readme");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             readme = \"README.md\"\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        dir.write("README.md", "# shapes\n");
        dir.write("LICENSE-MIT", "MIT license text\n");
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let dest = assemble(&package, &crate_).expect("assembles");

        assert_eq!(
            fs::read_to_string(dest.join("README.md")).unwrap(),
            "# shapes\n"
        );
        assert_eq!(
            fs::read_to_string(dest.join("LICENSE-MIT")).unwrap(),
            "MIT license text\n"
        );
    }

    #[test]
    fn assemble_copies_a_nested_readme_path() {
        let dir = TempDir::new("nested_readme");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             readme = \"docs/README.md\"\nlicense-file = \"licenses/MIT.txt\"\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        dir.write("docs/README.md", "# shapes\n");
        dir.write("licenses/MIT.txt", "MIT license text\n");
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let dest = assemble(&package, &crate_).expect("assembles");

        assert_eq!(
            fs::read_to_string(dest.join("docs/README.md")).unwrap(),
            "# shapes\n"
        );
        assert_eq!(
            fs::read_to_string(dest.join("licenses/MIT.txt")).unwrap(),
            "MIT license text\n"
        );
    }

    #[test]
    fn assemble_errors_when_the_manifest_names_a_readme_that_does_not_exist() {
        let dir = TempDir::new("missing_readme");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             readme = \"README.md\"\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        // No README.md is written, even though the manifest names one.
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let err = assemble(&package, &crate_).expect_err("a missing named file is an error");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let message = err.to_string();
        assert!(message.contains("readme"), "{message}");
        assert!(message.contains("README.md"), "{message}");
    }

    #[test]
    fn assemble_refuses_a_readme_outside_the_package_and_writes_nothing_there() {
        let dir = TempDir::new("escaping_readme");
        dir.write("outside/Cargo.toml", "outside");
        dir.write(
            "pkg/Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             readme = \"../outside/Cargo.toml\"\n",
        );
        dir.write("pkg/src/lib.vr", "pub fn area() {}\n");
        let package = package_at(&dir.0.join("pkg"), "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let err = assemble(&package, &crate_).expect_err("an escaping name is an error");

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("outside the package"), "{err}");
        assert_eq!(
            fs::read_to_string(dir.0.join("outside/Cargo.toml")).unwrap(),
            "outside"
        );
        assert!(!dir.0.join("pkg/target/varyk/package/outside").exists());
    }

    #[cfg(unix)]
    #[test]
    fn assemble_refuses_a_readme_that_links_outside_the_package() {
        let dir = TempDir::new("linked_readme");
        dir.write("outside/README.md", "outside");
        dir.write(
            "pkg/Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             readme = \"README.md\"\n",
        );
        dir.write("pkg/src/lib.vr", "pub fn area() {}\n");
        std::os::unix::fs::symlink(dir.0.join("outside/README.md"), dir.0.join("pkg/README.md"))
            .unwrap();
        let package = package_at(&dir.0.join("pkg"), "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let err = assemble(&package, &crate_).expect_err("a link outside the package is an error");

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("outside the package"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn assemble_refuses_a_linked_src_directory() {
        let dir = TempDir::new("linked_src_root");
        dir.write("outside/lib.vr", "pub fn area() {}\n");
        dir.write("outside/secret.vr", "fn secret() {}\n");
        dir.write(
            "pkg/Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        std::os::unix::fs::symlink(dir.0.join("outside"), dir.0.join("pkg/src")).unwrap();
        let package = package_at(&dir.0.join("pkg"), "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let err = assemble(&package, &crate_).expect_err("a linked src is an error");

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            !dir.0
                .join("pkg/target/varyk/package/shapes/src/secret.vr")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn assemble_refuses_a_symlink_under_src_and_copies_nothing_through_it() {
        let dir = TempDir::new("linked_src");
        dir.write("outside/secret.vr", "fn secret() {}\n");
        dir.write(
            "pkg/Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        dir.write("pkg/src/lib.vr", "pub fn area() {}\n");
        std::os::unix::fs::symlink(dir.0.join("outside"), dir.0.join("pkg/src/linked")).unwrap();
        let package = package_at(&dir.0.join("pkg"), "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let err = assemble(&package, &crate_).expect_err("a link under src is an error");

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("symbolic link"), "{err}");
        assert!(
            !dir.0
                .join("pkg/target/varyk/package/shapes/src/linked")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn assemble_refuses_a_linked_package_directory_and_clears_nothing_through_it() {
        let dir = TempDir::new("linked_assembly");
        dir.write("outside/shapes/keep.txt", "keep");
        dir.write(
            "pkg/Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        dir.write("pkg/src/lib.vr", "pub fn area() {}\n");
        fs::create_dir_all(dir.0.join("pkg/target/varyk")).unwrap();
        std::os::unix::fs::symlink(
            dir.0.join("outside"),
            dir.0.join("pkg/target/varyk/package"),
        )
        .unwrap();
        let package = package_at(&dir.0.join("pkg"), "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let err =
            assemble(&package, &crate_).expect_err("foreign files behind the link are an error");

        assert!(err.to_string().contains("did not write"), "{err}");
        assert!(
            dir.0.join("outside/shapes/keep.txt").exists(),
            "cleared through the link"
        );
    }

    #[test]
    fn assemble_copies_a_file_named_by_both_readme_and_license_file_once() {
        let dir = TempDir::new("shared_notice");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             readme = \"NOTICE\"\nlicense-file = \"NOTICE\"\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        dir.write("NOTICE", "notice");
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let dest = assemble(&package, &crate_).expect("assembles");

        assert_eq!(fs::read_to_string(dest.join("NOTICE")).unwrap(), "notice");
    }

    #[test]
    fn assemble_copies_the_lock_file_when_there_is_one() {
        let dir = TempDir::new("lock");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        dir.write("Cargo.lock", "# lock\n");
        dir.write("src/lib.vr", "pub fn area() {}\n");
        let mut package = package_at(&dir.0, "shapes", toml::Table::new());
        package.lock = Some(dir.0.join("Cargo.lock"));
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let dest = assemble(&package, &crate_).expect("assembles");

        assert_eq!(
            fs::read_to_string(dest.join("Cargo.lock")).unwrap(),
            "# lock\n"
        );
    }

    #[test]
    fn assemble_treats_dot_slash_and_bare_names_as_one_file() {
        let dir = TempDir::new("dot_slash_notice");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             readme = \"./NOTICE\"\nlicense-file = \"NOTICE\"\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        dir.write("NOTICE", "notice");
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let dest = assemble(&package, &crate_).expect("assembles");

        assert_eq!(fs::read_to_string(dest.join("NOTICE")).unwrap(), "notice");
    }

    #[test]
    fn assemble_refuses_a_readme_that_lands_on_a_generated_file() {
        let dir = TempDir::new("readme_collision");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             readme = \"src/lib.rs\"\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        dir.write("src/lib.rs", "// the include stub\n");
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        let err = assemble(&package, &crate_).expect_err("a colliding name is an error");

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains("assembled crate"), "{err}");
    }

    #[test]
    fn assemble_clears_the_destination_before_writing() {
        let dir = TempDir::new("stale");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        dir.write("src/lib.vr", "pub fn area() {}\n");
        let stale = dir
            .0
            .join("target/varyk/package/shapes")
            .join("stale-leftover.txt");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        fs::write(&stale, "old").unwrap();
        // A previous assembly left its marker, so the directory is ours.
        fs::write(stale.parent().unwrap().join(MARKER), "").unwrap();
        let package = package_at(&dir.0, "shapes", toml::Table::new());
        let crate_ = crate_with(
            vec![generated_file("src/lib.rs", "pub fn area() {}")],
            vec![],
        );

        assemble(&package, &crate_).expect("assembles");

        assert!(!stale.exists(), "stale file remains");
    }

    fn crate_with(files: Vec<GeneratedFile>, copied: Vec<(String, PathBuf)>) -> GeneratedCrate {
        GeneratedCrate {
            package_name: "shapes".to_string(),
            cargo_toml: String::new(),
            files,
            copied,
        }
    }
}
