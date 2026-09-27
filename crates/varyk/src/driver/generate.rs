//! Writing a [`GeneratedCrate`] to disk (spec 2.3: `write_tree` produces
//! the complete `src/` tree, byte-identical wherever it is written), and
//! rendering it as text for `--emit-rust`.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::backend::GeneratedCrate;

/// Writes `generated`'s `Cargo.toml` and its `src/` tree under
/// `build_dir`.
pub(super) fn write_files(build_dir: &Path, generated: &GeneratedCrate) -> io::Result<()> {
    write_tree(generated, build_dir)?;
    write_if_changed(&build_dir.join("Cargo.toml"), &generated.cargo_toml)
}

/// The file that marks a directory as one Varyk writes into, so a later
/// `write_tree` may clear its `src/`.
pub const MARKER: &str = ".varyk-generated";

/// Writes every generated file and copies every `.rs` module of
/// `generated` under `dir` (spec 2.3): `dir/src` is cleared of anything
/// not in `generated` first, so a deleted module leaves no stale file.
/// Files whose content is already on disk are left untouched, so cargo's
/// freshness check keeps working and concurrent builds of one program do
/// not rewrite each other's files.
///
/// Clearing only ever happens in a directory Varyk owns (see [`owned`]):
/// a directory that already holds anything and that Varyk did not write
/// is refused with an error, and nothing is written. So is a `dir`
/// or `dir/src` that is itself a symbolic link, wherever it points, since
/// writing through it would land in a directory Varyk cannot vouch for;
/// a link further up the path (macOS's `/var`, say) is fine. A directory
/// that is written gets the [`MARKER`] file.
pub fn write_tree(generated: &GeneratedCrate, dir: &Path) -> io::Result<()> {
    let src_dir = dir.join("src");
    for path in [dir, src_dir.as_path()] {
        if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(io::Error::other(format!(
                "`{}` is a symbolic link, and may lead to files Varyk did not write; \
                 give an empty or new directory",
                path.display()
            )));
        }
    }
    if !owned(dir) && has_entries(dir) {
        return Err(io::Error::other(format!(
            "`{}` holds files Varyk did not write; give an empty or new directory",
            dir.display()
        )));
    }
    fs::create_dir_all(&src_dir)?;
    // `symlink_metadata` does not follow links: a marker that is a link
    // (dangling or not) is left alone rather than written through.
    let marker = dir.join(MARKER);
    if fs::symlink_metadata(&marker).is_err() {
        fs::write(&marker, "")?;
    }

    let mut wanted: HashSet<PathBuf> = HashSet::new();
    for file in &generated.files {
        wanted.insert(dir.join(&file.path));
    }
    for (path, _) in &generated.copied {
        wanted.insert(dir.join(path));
    }
    remove_stale(&src_dir, &wanted)?;

    for file in &generated.files {
        write_if_changed(&dir.join(&file.path), &file.text)?;
    }
    for (path, source) in &generated.copied {
        copy_if_changed(source, &dir.join(path))?;
    }
    Ok(())
}

/// Whether Varyk may clear `dir/src`: `dir` carries the [`MARKER`], or is
/// a direct child of a `target/varyk/` directory (where every build
/// directory lives, including ones written before the marker existed).
/// Nothing else: the directory `varyk emit` gets from a build script is
/// empty on its first run and carries the marker from then on.
pub(super) fn owned(dir: &Path) -> bool {
    // `symlink_metadata` does not follow links: a marker that is a link
    // to some real file elsewhere does not make the directory ours.
    if fs::symlink_metadata(dir.join(MARKER)).is_ok_and(|meta| meta.is_file()) {
        return true;
    }
    let Some(real) = resolved(dir) else {
        return false;
    };
    let mut parents = real.ancestors().skip(1).map(Path::file_name);
    parents.next() == Some(Some("varyk".as_ref()))
        && parents.next() == Some(Some("target".as_ref()))
}

/// `path` with every symbolic link and `..` resolved: the deepest
/// ancestor that exists, canonicalized, with the rest appended. `None`
/// when that fails, or when a `..` remains in the part that does not
/// exist yet.
fn resolved(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    let mut existing = absolute.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = fs::canonicalize(existing) {
            let mut real = real;
            for name in rest.iter().rev() {
                real.push(name);
            }
            return Some(real);
        }
        let name = existing.file_name()?;
        rest.push(name.to_owned());
        existing = existing.parent()?;
    }
}

/// Whether `dir` is a directory with anything in it.
pub(super) fn has_entries(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// The first of `inputs` (files and directories a build reads) that lies
/// inside `dir/src`, which `write_tree` would clear: writing there would
/// destroy the program being built. `None` when `dir/src` does not exist
/// yet or holds none of them. Symbolic links are resolved on both sides.
pub fn input_inside<'a>(
    dir: &Path,
    inputs: impl IntoIterator<Item = &'a Path>,
) -> Option<&'a Path> {
    let src = fs::canonicalize(dir.join("src")).ok()?;
    inputs.into_iter().find(|input| {
        let Ok(input) = fs::canonicalize(input) else {
            return false;
        };
        input.starts_with(&src)
    })
}

/// Removes every file under `current` (recursively) that is not in
/// `wanted`, then removes any directory this leaves empty. A symbolic
/// link is never followed: an unwanted one is removed as a link, and
/// whatever it points to is left alone.
fn remove_stale(current: &Path, wanted: &HashSet<PathBuf>) -> io::Result<()> {
    if !fs::symlink_metadata(current).is_ok_and(|meta| meta.is_dir()) {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            remove_stale(&path, wanted)?;
            if fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(&path)?;
            }
        } else if !wanted.contains(&path) {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Makes the hidden crate's lock at `dest` mirror the package's: a copy
/// of `lock` when the package has one, no file when it has none. Without
/// the removal, a package whose `Cargo.lock` was deleted would keep
/// building against the resolution the last copy pinned, where cargo run
/// on the package itself would resolve afresh.
pub(super) fn sync_lock(lock: Option<&Path>, dest: &Path) -> io::Result<()> {
    match lock {
        Some(lock) => copy_if_changed(lock, dest),
        None => match fs::remove_file(dest) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// Copies `source` to `dest` byte-for-byte, unless `dest` already holds
/// exactly `source`'s bytes.
pub(super) fn copy_if_changed(source: &Path, dest: &Path) -> io::Result<()> {
    let content = fs::read(source)?;
    if fs::read(dest).is_ok_and(|existing| existing == content) {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temporary = dest.as_os_str().to_owned();
    temporary.push(format!(".tmp{}", std::process::id()));
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, &content)?;
    #[cfg(windows)]
    {
        let _ = fs::remove_file(dest);
    }
    fs::rename(&temporary, dest)
}

/// Writes `content` to `path`, creating parent directories, unless the
/// file already holds exactly `content`. The write goes to a temporary
/// file that is then renamed into place, so a concurrent reader never
/// sees a half-written file.
fn write_if_changed(path: &Path, content: &str) -> io::Result<()> {
    if fs::read(path).is_ok_and(|existing| existing == content.as_bytes()) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".tmp{}", std::process::id()));
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, content)?;
    // On Windows, `rename` fails if `path` already exists; clear it first
    // (ignoring "not found") so the rename below can succeed there too.
    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
    }
    fs::rename(&temporary, path)
}

/// The generated crate as text for `--emit-rust`: `Cargo.toml`, then every
/// generated file, then every copied `.rs` module (read back from disk,
/// since the backend does not keep its text), each after a header line
/// `// ==== <path> ====`. `.rs` content passes through
/// `rustfmt --edition 2024` when it is on the path; if it is absent or
/// fails, the content is used as is.
pub fn emit_rust(generated: &GeneratedCrate) -> String {
    let mut out = String::new();
    push_emit(&mut out, "Cargo.toml", &generated.cargo_toml);
    for file in &generated.files {
        push_emit(&mut out, &file.path, &file.text);
    }
    for (path, source) in &generated.copied {
        let content = fs::read_to_string(source).unwrap_or_default();
        push_emit(&mut out, path, &content);
    }
    out
}

/// Appends one file's `// ==== <path> ====` header and content to `out`.
fn push_emit(out: &mut String, path: &str, content: &str) {
    let content = if path.ends_with(".rs") {
        rustfmt(content).unwrap_or_else(|| content.to_string())
    } else {
        content.to_string()
    };
    out.push_str(&format!("// ==== {path} ====\n"));
    out.push_str(&content);
    if !content.ends_with('\n') {
        out.push('\n');
    }
}

/// Formats `source` with `rustfmt --edition 2024` over stdin and stdout;
/// `None` if rustfmt cannot be spawned or reports an error.
fn rustfmt(source: &str) -> Option<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Write from another thread while this one collects the output, so
    // neither side can block on a full pipe.
    let mut stdin = child.stdin.take()?;
    let source = source.to_string();
    let writer = std::thread::spawn(move || stdin.write_all(source.as_bytes()));
    let output = child.wait_with_output().ok()?;
    let written = writer.join().ok()?;
    if written.is_err() || !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{GeneratedFile, Writer};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory removed when it goes out of scope, even on a
    /// test failure.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "varyk_generate_test_{label}_{}_{n}",
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

    fn generated_file(path: &str, line: &str) -> GeneratedFile {
        let mut writer = Writer::new(path);
        writer.line(0, line, None);
        writer.finish()
    }

    fn crate_with(files: Vec<GeneratedFile>, copied: Vec<(String, PathBuf)>) -> GeneratedCrate {
        GeneratedCrate {
            package_name: "p".to_string(),
            cargo_toml: String::new(),
            files,
            copied,
        }
    }

    #[test]
    fn sync_lock_copies_the_lock_and_removes_the_copy_when_the_lock_is_gone() {
        let dir = TempDir::new("lock");
        let source = dir.0.join("Cargo.lock");
        let dest = dir.0.join("hidden/Cargo.lock");
        fs::write(&source, "# lock\n").unwrap();

        sync_lock(Some(&source), &dest).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "# lock\n");

        sync_lock(None, &dest).unwrap();
        assert!(!dest.exists());
        sync_lock(None, &dest).unwrap();
    }

    #[test]
    fn write_tree_clears_a_stale_file() {
        let dir = TempDir::new("stale");
        fs::create_dir_all(dir.0.join("src")).unwrap();
        fs::write(dir.0.join("src/old.rs"), "stale").unwrap();
        fs::write(dir.0.join(MARKER), "").unwrap();

        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        write_tree(&generated, &dir.0).unwrap();

        assert!(!dir.0.join("src/old.rs").exists(), "stale file remains");
        assert!(dir.0.join("src/main.rs").exists());
    }

    #[test]
    fn write_tree_writes_a_nested_file() {
        let dir = TempDir::new("nested");

        let generated = crate_with(
            vec![generated_file("src/shop/cart.rs", "fn cart() {}")],
            Vec::new(),
        );
        write_tree(&generated, &dir.0).unwrap();

        let content = fs::read_to_string(dir.0.join("src/shop/cart.rs")).unwrap();
        assert_eq!(content, "fn cart() {}\n");
    }

    #[test]
    fn write_tree_copies_a_rs_file_byte_for_byte() {
        let dir = TempDir::new("copy_dest");
        let source_dir = TempDir::new("copy_source");
        let source_path = source_dir.0.join("ext.rs");
        fs::write(&source_path, b"pub fn f() {}\n// trailing, no newline").unwrap();

        let generated = crate_with(
            Vec::new(),
            vec![("src/ext.rs".to_string(), source_path.clone())],
        );
        write_tree(&generated, &dir.0).unwrap();

        let copied = fs::read(dir.0.join("src/ext.rs")).unwrap();
        let original = fs::read(&source_path).unwrap();
        assert_eq!(copied, original);
    }

    #[cfg(unix)]
    #[test]
    fn write_tree_refuses_a_directory_whose_marker_is_a_symlink() {
        let dir = TempDir::new("linked_marker");
        fs::create_dir_all(dir.0.join("src")).unwrap();
        fs::write(dir.0.join("src/main.vr"), "fn main() {}").unwrap();
        fs::write(dir.0.join("elsewhere"), "").unwrap();
        std::os::unix::fs::symlink(dir.0.join("elsewhere"), dir.0.join(MARKER)).unwrap();

        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        let err = write_tree(&generated, &dir.0).unwrap_err();

        assert!(err.to_string().contains("did not write"), "{err}");
        assert!(dir.0.join("src/main.vr").exists());
        assert!(!dir.0.join("src/main.rs").exists());
    }

    #[test]
    fn only_a_direct_child_of_target_varyk_is_owned_without_a_marker() {
        let dir = TempDir::new("target_varyk");
        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        // `target/varyk/<name>`: an old build directory, cleared.
        let build = dir.0.join("target/varyk/app");
        fs::create_dir_all(build.join("src")).unwrap();
        fs::write(build.join("src/stale.rs"), "").unwrap();
        write_tree(&generated, &build).expect("a build directory is Varyk's");
        assert!(!build.join("src/stale.rs").exists());
        // Deeper down it is somebody else's.
        let deeper = dir.0.join("target/varyk/app/nested");
        fs::create_dir_all(deeper.join("src")).unwrap();
        fs::write(deeper.join("src/mine.rs"), "").unwrap();
        let err = write_tree(&generated, &deeper).unwrap_err();
        assert!(err.to_string().contains("did not write"), "{err}");
        assert!(deeper.join("src/mine.rs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn write_tree_does_not_write_the_marker_through_a_link() {
        let dir = TempDir::new("dangling_marker");
        let build = dir.0.join("target/varyk/app");
        fs::create_dir_all(&build).unwrap();
        std::os::unix::fs::symlink(dir.0.join("elsewhere"), build.join(MARKER)).unwrap();
        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );

        write_tree(&generated, &build).expect("a build directory is Varyk's");

        assert!(!dir.0.join("elsewhere").exists(), "wrote through the link");
        assert!(build.join("src/main.rs").exists());
    }

    #[test]
    fn write_tree_refuses_a_directory_it_did_not_write_and_touches_nothing() {
        let dir = TempDir::new("foreign");
        fs::create_dir_all(dir.0.join("src")).unwrap();
        fs::write(dir.0.join("src/main.vr"), "fn main() {}").unwrap();

        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        let err = write_tree(&generated, &dir.0).unwrap_err();

        assert!(err.to_string().contains("did not write"), "{err}");
        assert!(dir.0.join("src/main.vr").exists());
        assert!(!dir.0.join("src/main.rs").exists());
        assert!(!dir.0.join(MARKER).exists());
    }

    #[test]
    fn write_tree_marks_a_fresh_directory_and_clears_it_on_the_second_run() {
        let dir = TempDir::new("fresh");
        let first = crate_with(
            vec![
                generated_file("src/main.rs", "fn main() {}"),
                generated_file("src/old.rs", "fn old() {}"),
            ],
            Vec::new(),
        );
        write_tree(&first, &dir.0).unwrap();
        assert!(dir.0.join(MARKER).is_file());

        let second = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        write_tree(&second, &dir.0).unwrap();

        assert!(dir.0.join("src/main.rs").exists());
        assert!(!dir.0.join("src/old.rs").exists(), "stale file remains");
    }

    #[cfg(unix)]
    #[test]
    fn write_tree_never_follows_a_symlinked_directory() {
        let dir = TempDir::new("symlink");
        let outside = TempDir::new("symlink_target");
        fs::write(outside.0.join("keep.txt"), "keep").unwrap();
        fs::create_dir_all(dir.0.join("src")).unwrap();
        fs::write(dir.0.join(MARKER), "").unwrap();
        std::os::unix::fs::symlink(&outside.0, dir.0.join("src/link")).unwrap();

        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        write_tree(&generated, &dir.0).unwrap();

        assert!(outside.0.join("keep.txt").is_file(), "followed the link");
    }

    fn assert_refuses_symlink(generated: &GeneratedCrate, dir: &Path) {
        let err = write_tree(generated, dir).unwrap_err().to_string();
        assert!(err.contains("is a symbolic link"), "{err}");
        assert!(err.contains("did not write"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn write_tree_refuses_a_src_that_is_a_symlink_and_leaves_its_target_alone() {
        let dir = TempDir::new("src_link");
        let user = TempDir::new("src_link_user");
        fs::create_dir_all(user.0.join("src")).unwrap();
        fs::write(user.0.join("src/main.rs"), "// mine").unwrap();
        // Owned, as a directory under `target/varyk` is, so only the link
        // stands between the write and the user's `main.rs`.
        fs::write(dir.0.join(MARKER), "").unwrap();
        std::os::unix::fs::symlink(user.0.join("src"), dir.0.join("src")).unwrap();

        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        assert_refuses_symlink(&generated, &dir.0);
        assert_eq!(
            fs::read_to_string(user.0.join("src/main.rs")).unwrap(),
            "// mine"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_tree_refuses_a_dir_that_is_a_symlink_even_to_its_own_output() {
        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        let earlier = TempDir::new("dir_link_earlier");
        write_tree(&generated, &earlier.0).unwrap();
        let empty = TempDir::new("dir_link_empty");
        let links = TempDir::new("dir_link");
        for (name, target) in [("to_earlier", &earlier.0), ("to_empty", &empty.0)] {
            let link = links.0.join(name);
            std::os::unix::fs::symlink(target, &link).unwrap();
            assert_refuses_symlink(&generated, &link);
        }
        assert!(!empty.0.join(MARKER).exists());
        assert!(!empty.0.join("src").exists());
    }

    #[cfg(unix)]
    #[test]
    fn write_tree_refuses_a_dangling_symlink() {
        let links = TempDir::new("dangling");
        let link = links.0.join("out");
        std::os::unix::fs::symlink(links.0.join("missing"), &link).unwrap();
        let generated = crate_with(
            vec![generated_file("src/main.rs", "fn main() {}")],
            Vec::new(),
        );
        assert_refuses_symlink(&generated, &link);
        assert!(!links.0.join("missing").exists());
    }
}
