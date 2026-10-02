//! `varyk init` (spec 2.5): writes a package only `varyk` builds: its
//! `Cargo.toml` names the `.vr` root, and there is no `build.rs` or stub.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::sanitize_name;

const GITIGNORE: &str = include_str!("templates/gitignore.txt");
const MAIN_VR: &str = include_str!("templates/main.vr.txt");
const LIB_VR: &str = include_str!("templates/lib.vr.txt");

/// Why [`write`] did not write anything.
#[derive(Debug)]
pub enum InitError {
    /// Every file `write` would otherwise create that is already there,
    /// relative to the target directory; nothing was written.
    Exists(Vec<PathBuf>),
    /// The root file of the other kind of package (`src/main.vr` for a
    /// library, `src/lib.vr` for a program) is already there, relative to
    /// the target directory; nothing was written.
    OtherRoot(PathBuf),
    /// Creating a directory or writing a file failed.
    Io(io::Error),
}

/// The three files `init` writes (spec 2.5), as tree-relative paths and
/// their content: `name` is the package's (from the target directory) and
/// `lib` selects `src/lib.vr` over `src/main.vr`. Pure, so a
/// fixture example can be checked against a fresh `init` for equality.
pub fn files(name: &str, lib: bool) -> Vec<(PathBuf, String)> {
    let mut files = vec![
        (PathBuf::from("Cargo.toml"), cargo_toml(name, lib)),
        (PathBuf::from(".gitignore"), GITIGNORE.to_string()),
    ];
    if lib {
        files.push((PathBuf::from("src/lib.vr"), LIB_VR.to_string()));
    } else {
        files.push((PathBuf::from("src/main.vr"), MAIN_VR.to_string()));
    }
    files
}

/// `Cargo.toml`: `name`, version `0.1.0`, edition 2024, the target table
/// that names the `.vr` root (before `[dependencies]`, so a line appended
/// to the file lands in it), and `[dependencies]` with `varyk-std` at the
/// compiler's full version (M5a spec 5.2). No `[workspace]` table (spec
/// 2.5), so the package is never its own workspace root.
fn cargo_toml(name: &str, lib: bool) -> String {
    let target = if lib {
        "[lib]\npath = \"src/lib.vr\"\n".to_string()
    } else {
        format!("[[bin]]\nname = \"{name}\"\npath = \"src/main.vr\"\n")
    };
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n\
         \n\
         {target}\
         \n\
         [dependencies]\n\
         varyk-std = \"{}\"\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// The package name `init` derives from `dir`: its own (last) path
/// component, sanitized the same way a generated single-file package name
/// is (`driver::sanitize_name`), so it always passes the manifest's own
/// name rule (`package::name_problem`).
pub fn package_name(dir: &Path) -> String {
    let absolute = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let name = absolute
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    sanitize_name(&name)
}

/// Writes the three files of spec 2.5 under `dir` (created if it does not
/// exist), refusing if anything is already at any of their paths, a
/// file, a directory, or a link: nothing is written, and every path that
/// is taken is reported (spec 2.5's "there is no merge"). It refuses too when the root file of the other kind of
/// package is there, since a package cannot have both. Returns every file written, in the order of [`files`].
pub fn write(dir: &Path, lib: bool) -> Result<Vec<PathBuf>, InitError> {
    let name = package_name(dir);
    let entries = files(&name, lib);

    // The other kind of package first: its files overlap this one's, and
    // that it is already a package is what matters.
    let other = PathBuf::from(if lib { "src/main.vr" } else { "src/lib.vr" });
    if dir.join(&other).is_file() {
        return Err(InitError::OtherRoot(other));
    }
    let existing: Vec<PathBuf> = entries
        .iter()
        .map(|(path, _)| path.clone())
        // `symlink_metadata` sees a dangling link too, which `fs::write`
        // would follow and fail on. A directory on the way (`src`) that is
        // not a plain directory, a link or a file, counts as taken as
        // well: writing through a link would put the files somewhere
        // else, and a file there would fail after the root files were
        // written.
        .flat_map(|path| {
            let parents = path
                .ancestors()
                .skip(1)
                .filter(|parent| !parent.as_os_str().is_empty())
                .filter(|parent| {
                    fs::symlink_metadata(dir.join(parent)).is_ok_and(|meta| !meta.is_dir())
                })
                .map(Path::to_path_buf)
                .collect::<Vec<_>>();
            let taken = dir.join(&path).symlink_metadata().is_ok();
            parents.into_iter().chain(taken.then_some(path))
        })
        .collect();
    if !existing.is_empty() {
        return Err(InitError::Exists(existing));
    }

    let mut written = Vec::with_capacity(entries.len());
    for (path, content) in &entries {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(InitError::Io)?;
        }
        fs::write(&full, content).map_err(InitError::Io)?;
        written.push(full);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory removed when it goes out of scope.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "varyk_init_test_{label}_{}_{n}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn files_of_a_binary_are_the_three_of_spec_2_5() {
        let entries = files("greeting", false);

        let paths: Vec<&Path> = entries.iter().map(|(path, _)| path.as_path()).collect();
        assert_eq!(
            paths,
            vec![
                Path::new("Cargo.toml"),
                Path::new(".gitignore"),
                Path::new("src/main.vr"),
            ]
        );
        assert!(
            entries[0]
                .1
                .contains("[[bin]]\nname = \"greeting\"\npath = \"src/main.vr\"")
        );
    }

    #[test]
    fn files_of_a_library_swap_main_for_lib() {
        let entries = files("greeting", true);

        let paths: Vec<&Path> = entries.iter().map(|(path, _)| path.as_path()).collect();
        assert_eq!(
            paths,
            vec![
                Path::new("Cargo.toml"),
                Path::new(".gitignore"),
                Path::new("src/lib.vr"),
            ]
        );
        assert!(entries[0].1.contains("[lib]\npath = \"src/lib.vr\""));
    }

    #[test]
    fn write_creates_a_fresh_directory_with_the_three_files() {
        let dir = TempDir::new("fresh");
        let target = dir.0.join("greeting");

        let written = write(&target, false).expect("writes");

        assert_eq!(written.len(), 3);
        for path in &written {
            assert!(path.is_file(), "{path:?}");
        }
        let manifest = fs::read_to_string(target.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("name = \"greeting\""), "{manifest}");
        assert!(!manifest.contains("[workspace]"), "{manifest}");
    }

    #[test]
    fn write_refuses_when_a_file_already_exists_and_writes_nothing() {
        let dir = TempDir::new("conflict");
        fs::write(dir.0.join("Cargo.toml"), "leave me alone").unwrap();

        let err = write(&dir.0, false).unwrap_err();

        match err {
            InitError::Exists(existing) => {
                assert_eq!(existing, vec![PathBuf::from("Cargo.toml")]);
            }
            other => panic!("expected Exists, got {other:?}"),
        }
        assert!(
            !dir.0.join(".gitignore").exists(),
            "init wrote past the conflict"
        );
        assert_eq!(
            fs::read_to_string(dir.0.join("Cargo.toml")).unwrap(),
            "leave me alone"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_a_linked_src_directory_and_writes_nothing() {
        let dir = TempDir::new("linked_src");
        fs::create_dir_all(dir.0.join("outside")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("outside"), dir.0.join("src")).unwrap();

        let err = write(&dir.0, false).expect_err("a linked src is a conflict");

        match err {
            InitError::Exists(paths) => assert!(paths.contains(&PathBuf::from("src")), "{paths:?}"),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!dir.0.join("Cargo.toml").exists(), "Cargo.toml was written");
        assert!(
            fs::read_dir(dir.0.join("outside"))
                .unwrap()
                .next()
                .is_none(),
            "wrote through the link"
        );
    }

    #[test]
    fn write_refuses_a_file_named_src_and_writes_nothing() {
        let dir = TempDir::new("file_src");
        fs::write(dir.0.join("src"), "x").unwrap();

        let err = write(&dir.0, false).expect_err("a file at src is a conflict");

        match err {
            InitError::Exists(paths) => assert!(paths.contains(&PathBuf::from("src")), "{paths:?}"),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!dir.0.join("Cargo.toml").exists(), "Cargo.toml was written");
    }

    #[test]
    fn write_refuses_when_a_directory_sits_at_an_output_path_and_writes_nothing() {
        let dir = TempDir::new("dir_in_the_way");
        fs::create_dir_all(dir.0.join(".gitignore")).unwrap();

        let err = write(&dir.0, false).expect_err("a directory at .gitignore is a conflict");

        match err {
            InitError::Exists(paths) => assert_eq!(paths, vec![PathBuf::from(".gitignore")]),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!dir.0.join("Cargo.toml").exists(), "Cargo.toml was written");
        assert!(!dir.0.join("src").exists(), "src was written");
    }

    #[test]
    fn a_directory_named_cache_or_package_gets_a_prefixed_name() {
        assert_eq!(package_name(Path::new("/tmp/cache")), "v_cache");
        assert_eq!(package_name(Path::new("/tmp/package")), "v_package");
    }

    #[test]
    fn write_says_a_whole_program_package_is_there_before_listing_its_files() {
        let dir = TempDir::new("other_package");
        write(&dir.0, false).unwrap();

        match write(&dir.0, true).unwrap_err() {
            InitError::OtherRoot(other) => assert_eq!(other, PathBuf::from("src/main.vr")),
            other => panic!("expected OtherRoot, got {other:?}"),
        }
        assert!(!dir.0.join("src/lib.vr").exists());
    }

    #[test]
    fn write_refuses_a_library_where_a_program_root_exists_and_writes_nothing() {
        let dir = TempDir::new("other_root");
        fs::create_dir_all(dir.0.join("src")).unwrap();
        fs::write(dir.0.join("src/main.vr"), "fn main() {}\n").unwrap();

        let err = write(&dir.0, true).unwrap_err();

        match err {
            InitError::OtherRoot(other) => assert_eq!(other, PathBuf::from("src/main.vr")),
            other => panic!("expected OtherRoot, got {other:?}"),
        }
        assert!(!dir.0.join("Cargo.toml").exists());
        assert!(!dir.0.join("src/lib.vr").exists());
    }
}
