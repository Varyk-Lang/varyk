//! The package graph (M5b2 spec 4.1): which packages a build uses, as
//! cargo resolves them, and which of them are Varyk packages.
//!
//! The graph is read from `cargo metadata`, run on the package's isolated
//! manifest written to `target/varyk/packages/graph/`, whenever the
//! package lists a dependency besides `varyk-std`. A dependency whose
//! directory has `src/lib.vr` is a Varyk package; any other is a Rust
//! crate. The Varyk packages of the build are those reached through
//! `[dependencies]` from the program and, in turn, from the Varyk packages
//! so reached. A Varyk package reached any other way, and one that shares
//! its name and version with another package the build reaches by `path`
//! or builds as one, is refused (V0401); a cargo that fails is V0405.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use varyk_syntax::Span;

use crate::backend::with_std_patch;
use crate::diagnostics::{Diagnostic, codes};
use crate::driver::generate::{sync_lock, write_if_changed};
use crate::package::Package;

/// Where the graph manifest is written, under the package's directory.
pub const GRAPH_DIR: &str = "target/varyk/packages/graph";

/// The headline of V0405.
const CARGO_FAILED: &str = "cargo could not work out which packages this build uses";

/// The packages a build uses, as far as Varyk cares: the Varyk packages
/// and what each dependency key resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graph {
    packages: Vec<VarykPackage>,
    root_deps: Vec<(String, DepTarget)>,
    lock: PathBuf,
    /// The oldest `varyk-std` cargo resolved for the build, if any.
    std_version: Option<String>,
}

/// A Varyk package of the build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarykPackage {
    /// The directory holding its `Cargo.toml`, as cargo reports it.
    pub dir: PathBuf,
    pub name: String,
    pub version: String,
    /// Where cargo got it: `None` for one reached by `path`, else cargo's
    /// source id (`git+https://...`, `registry+https://...`).
    pub source: Option<String>,
    /// Its `[dependencies]`, each under its key with `-` read as `_`.
    pub deps: Vec<(String, DepTarget)>,
}

/// A Varyk package of the build as the messages of a package that uses
/// it name it (M5b2 spec 2.4): its name, its version, and the line that
/// adds it to that package's `[dependencies]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub name: String,
    pub version: String,
    pub line: String,
}

/// Dependencies, each under its key with `-` read as `_`.
type Deps = Vec<(String, DepTarget)>;

/// What a dependency key resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepTarget {
    /// The Varyk package at this index of [`Graph::varyk_packages`].
    Varyk(usize),
    /// A Rust crate.
    Rust,
}

impl Graph {
    /// The graph of `package`'s build, or `None` when it lists no
    /// dependency besides `varyk-std` (then no cargo runs and nothing is
    /// written). Writes the graph manifest, with the package's lock beside
    /// it, and asks `cargo metadata` for the host's graph.
    pub fn read(package: &Package) -> Result<Option<Graph>, Vec<Diagnostic>> {
        let listed = package
            .dependencies
            .keys()
            .chain(package.dev_dependencies.iter())
            .any(|key| key != "varyk-std");
        if !listed {
            return Ok(None);
        }
        let at = package.std_spans.anchor;
        let dir = package.root.join(GRAPH_DIR);
        let json = ask_cargo(package, &dir).map_err(|note| {
            vec![Diagnostic::new(codes::V0405, at, CARGO_FAILED).with_note(note)]
        })?;
        let is_varyk = |dir: &Path| dir.join("src/lib.vr").is_file();
        let (packages, root_deps) = classify(&json, &is_varyk)
            .map_err(|refusals| refusals.into_iter().map(|r| r.at(at)).collect::<Vec<_>>())?;
        Ok(Some(Graph {
            packages,
            root_deps,
            lock: dir.join("Cargo.lock"),
            std_version: resolved_std(&json),
        }))
    }

    /// A graph of `packages`, used through `root_deps`, without cargo.
    #[cfg(test)]
    pub(crate) fn for_tests(packages: Vec<VarykPackage>, root_deps: Deps) -> Graph {
        Graph {
            packages,
            root_deps,
            lock: PathBuf::new(),
            std_version: None,
        }
    }

    /// The oldest `varyk-std` cargo resolved for the build: the one the
    /// `varyk-std` lines of the program and its Varyk packages resolve to
    /// (M5b2 spec 4.2); `None` when nothing in the build uses it.
    pub fn std_version(&self) -> Option<&str> {
        self.std_version.as_deref()
    }

    /// Every Varyk package of the build, by index, as a package whose
    /// `Cargo.toml` is in `dir` would list it: by `path` from `dir` for one
    /// reached by `path`, else by version.
    pub fn listings(&self, dir: &Path) -> Vec<Listing> {
        let from = canonical(dir);
        self.packages
            .iter()
            .map(|package| {
                let name = &package.name;
                let line = match package.source.as_deref() {
                    None => {
                        let path = relative(&from, &canonical(&package.dir));
                        format!("{name} = {{ path = \"{path}\" }}")
                    }
                    Some(source) => registry_line(name, &package.version, source),
                };
                Listing {
                    name: name.clone(),
                    version: package.version.clone(),
                    line,
                }
            })
            .collect()
    }

    /// The Varyk packages of the build, each after the Varyk packages it
    /// depends on.
    pub fn varyk_packages(&self) -> &[VarykPackage] {
        &self.packages
    }

    /// The program's `[dependencies]`, each under its key with `-` read as
    /// `_`.
    pub fn root_deps(&self) -> &[(String, DepTarget)] {
        &self.root_deps
    }

    /// The lock cargo left beside the graph manifest: the one the build
    /// uses, so the graph checked is the graph built.
    pub fn lock_path(&self) -> PathBuf {
        self.lock.clone()
    }
}

/// The line that adds a package from `source`, a cargo source id, to a
/// `Cargo.toml`: a version for crates.io, else the `git` or `registry` it
/// came from, which a bare version would not find.
fn registry_line(name: &str, version: &str, source: &str) -> String {
    let crates_io = source.ends_with("github.com/rust-lang/crates.io-index")
        || source == "sparse+https://index.crates.io/";
    if crates_io {
        return format!("{name} = \"{version}\"");
    }
    if let Some(url) = source.strip_prefix("git+") {
        let (url, rev) = url.split_once('#').unwrap_or((url, ""));
        let (url, query) = url.split_once('?').unwrap_or((url, ""));
        // The pin cargo reports: a `tag` or `branch` as written, else the
        // commit it locked.
        let pin = query
            .split('&')
            .find(|pair| pair.starts_with("tag=") || pair.starts_with("branch="))
            .map(|pair| pair.replacen('=', " = \"", 1) + "\"")
            .or_else(|| (!rev.is_empty()).then(|| format!("rev = \"{rev}\"")));
        return match pin {
            Some(pin) => format!("{name} = {{ git = \"{url}\", {pin} }}"),
            None => format!("{name} = {{ git = \"{url}\" }}"),
        };
    }
    format!("{name} = {{ version = \"{version}\", registry = \"the registry's name\" }}")
}

/// `path` made absolute and with its links followed, as far as that can
/// be done; else `path` as it is.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    // The current directory is `""` to a package found there.
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };
    std::path::absolute(path)
        .and_then(|path| path.canonicalize())
        .unwrap_or_else(|_| path.to_path_buf())
}

/// `to` written from `from`, both absolute, with `/` between its parts:
/// `../units` from `/work/route` to `/work/units`, `.` for `from` itself.
fn relative(from: &Path, to: &Path) -> String {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let parts: Vec<String> = std::iter::repeat_n("..".to_string(), from.len() - shared)
        .chain(
            to.iter()
                .skip(shared)
                .map(|part| part.as_os_str().to_string_lossy().into_owned()),
        )
        .collect();
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

/// The oldest `varyk-std` in `cargo metadata`'s answer, if any.
fn resolved_std(json: &str) -> Option<String> {
    let (_, nodes) = parse(json)?;
    nodes
        .into_iter()
        .map(|(_, node)| node)
        .filter(|node| node.name == "varyk-std")
        .filter_map(|node| Some((crate::package::version_triple(&node.version)?, node.version)))
        .min()
        .map(|(_, version)| version)
}

/// Writes the graph manifest under `dir` (the package's isolated manifest,
/// its own `.vr` target kept as written, with `varyk-std` patched under
/// `VARYK_STD_PATH`), copies the package's lock beside it or removes a
/// stale copy, and returns `cargo metadata`'s JSON; on failure, the note
/// for V0405.
fn ask_cargo(package: &Package, dir: &Path) -> Result<String, String> {
    let manifest = dir.join("Cargo.toml");
    let text = with_std_patch(package.isolated_manifest()).to_string();
    write_if_changed(&manifest, &text)
        .and_then(|()| sync_lock(package.lock.as_deref(), &dir.join("Cargo.lock")))
        .map_err(|err| format!("cannot write `{}`: {err}", dir.display()))?;
    let dir = std::path::absolute(dir)
        .map_err(|err| format!("cannot find `{}`: {err}", dir.display()))?;
    let host = host(&dir)?;
    // Run from inside the package tree, as a build does, so the package's
    // `.cargo/config.toml` and toolchain file apply. `--color never`:
    // cargo's error text becomes part of V0405, and a configured
    // `CARGO_TERM_COLOR=always` would put terminal escapes into it.
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--color", "never"])
        .arg("--filter-platform")
        .arg(&host)
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| cannot_run("cargo", &err))?;
    if !output.status.success() {
        return Err(format!(
            "cargo said: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| "cargo's answer is not UTF-8 text".to_string())
}

/// The host triple `rustc -vV` reports, run in `dir`.
fn host(dir: &Path) -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| cannot_run("rustc", &err))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let host = stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_string());
    match host {
        Some(host) if output.status.success() => Ok(host),
        _ => Err(format!(
            "`rustc -vV` did not name the computer's platform: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

/// The note for a `tool` that could not be started.
fn cannot_run(tool: &str, err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::NotFound {
        format!("cannot run `{tool}`: not found; install Rust from https://rustup.rs")
    } else {
        format!("cannot run `{tool}`: {err}")
    }
}

/// A reason the graph cannot be used, before it has a place in the
/// manifest.
#[derive(Debug, PartialEq, Eq)]
struct Refusal {
    code: &'static str,
    message: String,
    note: String,
}

impl Refusal {
    fn at(self, span: Span) -> Diagnostic {
        Diagnostic::new(self.code, span, self.message).with_note(self.note)
    }

    /// V0401 for a Varyk package cargo would build on its own.
    fn reached(message: String) -> Refusal {
        Refusal {
            code: codes::V0401,
            message,
            note: "only `varyk` may build a Varyk package, and cargo would build this one on its \
                   own; a Varyk package can be used only from the `[dependencies]` of a Varyk \
                   program or package"
                .to_string(),
        }
    }
}

/// A package of the graph, from `resolve.nodes` joined with `packages`.
struct Node {
    name: String,
    version: String,
    /// `None` for a `path` package.
    source: Option<String>,
    dir: PathBuf,
    declared: Vec<Declared>,
    edges: Vec<Edge>,
}

/// A dependency as the package's manifest declares it.
struct Declared {
    /// The package's name.
    name: String,
    /// The key, when it is not the package's name.
    rename: Option<String>,
    optional: bool,
}

/// A dependency as cargo resolved it.
struct Edge {
    /// The key with `-` read as `_` (cargo's name for the crate).
    name: String,
    /// The package id it resolved to.
    pkg: String,
    /// Each `(kind, target)` it is listed under: `(None, None)` is plain
    /// `[dependencies]`.
    kinds: Vec<(Option<String>, Option<String>)>,
}

impl Edge {
    /// Whether it is listed under plain `[dependencies]`.
    fn plain(&self) -> bool {
        self.kinds
            .iter()
            .any(|(kind, target)| kind.is_none() && target.is_none())
    }
}

/// The text at `key` of `value`, `None` when it is `null` or absent.
fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

/// The root's id and every package of `cargo metadata`'s answer, in
/// `resolve.nodes`' order; `None` when the answer lacks what is read.
fn parse(json: &str) -> Option<(String, Vec<(String, Node)>)> {
    let value: Value = serde_json::from_str(json).ok()?;
    let mut packages = HashMap::new();
    for package in value.get("packages")?.as_array()? {
        let declared = package
            .get("dependencies")?
            .as_array()?
            .iter()
            .map(|dependency| {
                Some(Declared {
                    name: text(dependency, "name")?,
                    rename: text(dependency, "rename"),
                    optional: dependency.get("optional")?.as_bool()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let manifest = PathBuf::from(text(package, "manifest_path")?);
        packages.insert(
            text(package, "id")?,
            (
                text(package, "name")?,
                text(package, "version")?,
                text(package, "source"),
                manifest.parent()?.to_path_buf(),
                declared,
            ),
        );
    }
    let resolve = value.get("resolve")?;
    let root = text(resolve, "root")?;
    let mut nodes = Vec::new();
    for node in resolve.get("nodes")?.as_array()? {
        let id = text(node, "id")?;
        let edges = node
            .get("deps")?
            .as_array()?
            .iter()
            .map(|dep| {
                let kinds = dep
                    .get("dep_kinds")?
                    .as_array()?
                    .iter()
                    .map(|kind| (text(kind, "kind"), text(kind, "target")))
                    .collect();
                Some(Edge {
                    name: text(dep, "name")?,
                    pkg: text(dep, "pkg")?,
                    kinds,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let (name, version, source, dir, declared) = packages.remove(&id)?;
        nodes.push((
            id,
            Node {
                name,
                version,
                source,
                dir,
                declared,
                edges,
            },
        ));
    }
    Some((root, nodes))
}

/// The table a dependency of `kind` and `target` is listed in.
fn table(kind: Option<&str>, target: Option<&str>) -> String {
    let table = match kind {
        None => "dependencies",
        Some("dev") => "dev-dependencies",
        Some("build") => "build-dependencies",
        Some(other) => other,
    };
    match target {
        Some(target) => format!("[target.'{target}'.{table}]"),
        None => format!("[{table}]"),
    }
}

/// Where `node` comes from, for a message.
fn origin(node: &Node) -> String {
    match &node.source {
        Some(source) => format!("`{source}`"),
        None => format!("the folder `{}`", node.dir.display()),
    }
}

/// The Varyk packages of the build, dependency first, and the program's
/// dependencies, from `cargo metadata`'s JSON; `is_varyk` tells whether
/// a package's directory is a Varyk package's. Every way of reaching a
/// Varyk package spec 4.1 refuses is a refusal; an answer that cannot be
/// read is a V0405.
fn classify(
    json: &str,
    is_varyk: &dyn Fn(&Path) -> bool,
) -> Result<(Vec<VarykPackage>, Deps), Vec<Refusal>> {
    let unreadable = || {
        vec![Refusal {
            code: codes::V0405,
            message: CARGO_FAILED.to_string(),
            note: "cargo's answer (`cargo metadata`) could not be read".to_string(),
        }]
    };
    let Some((root, nodes)) = parse(json) else {
        return Err(unreadable());
    };
    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(at, (id, _))| (id.as_str(), at))
        .collect();
    let Some(&root_at) = index.get(root.as_str()) else {
        return Err(unreadable());
    };
    let nodes: Vec<&Node> = nodes.iter().map(|(_, node)| node).collect();
    let varyk: Vec<bool> = nodes
        .iter()
        .enumerate()
        .map(|(at, node)| at != root_at && is_varyk(&node.dir))
        .collect();
    let target_of = |edge: &Edge| index.get(edge.pkg.as_str()).copied();
    let is_varyk_at = |at: usize| varyk.get(at).copied().unwrap_or(false);

    // The Varyk packages of the build, each after those it depends on.
    let mut order = Vec::new();
    let mut seen = vec![false; nodes.len()];
    let mut stack = vec![(root_at, 0)];
    while let Some((at, next)) = stack.pop() {
        let Some(&node) = nodes.get(at) else {
            continue;
        };
        let found = node
            .edges
            .iter()
            .enumerate()
            .skip(next)
            .find_map(|(edge_at, edge)| {
                let to = target_of(edge)?;
                let fresh = seen.get(to).is_some_and(|seen| !seen);
                (edge.plain() && is_varyk_at(to) && fresh).then_some((edge_at, to))
            });
        match found {
            Some((edge, to)) => {
                if let Some(seen) = seen.get_mut(to) {
                    *seen = true;
                }
                stack.push((at, edge + 1));
                stack.push((to, 0));
            }
            None if at != root_at => order.push(at),
            None => {}
        }
    }
    let position: HashMap<usize, usize> = order
        .iter()
        .enumerate()
        .map(|(position, &at)| (at, position))
        .collect();

    let mut refusals = Vec::new();
    for user in std::iter::once(root_at).chain(order.iter().copied()) {
        let Some(&node) = nodes.get(user) else {
            continue;
        };
        let who = if user == root_at {
            "this package".to_string()
        } else {
            format!("the package `{}`", node.name)
        };
        for edge in &node.edges {
            let Some(to) = target_of(edge).filter(|&to| is_varyk_at(to)) else {
                continue;
            };
            let Some(used) = nodes.get(to).map(|node| &node.name) else {
                continue;
            };
            for (kind, target) in &edge.kinds {
                if kind.is_some() || target.is_some() {
                    refusals.push(Refusal::reached(format!(
                        "{who} lists the Varyk package `{used}` under `{}`, which is not \
                         supported yet",
                        table(kind.as_deref(), target.as_deref())
                    )));
                }
            }
            let optional = node.declared.iter().any(|declared| {
                declared.optional
                    && declared.name == *used
                    && declared
                        .rename
                        .as_deref()
                        .unwrap_or(&declared.name)
                        .replace('-', "_")
                        == edge.name
            });
            if optional {
                refusals.push(Refusal::reached(format!(
                    "{who} lists the Varyk package `{used}` as an optional dependency, which is \
                     not supported yet"
                )));
            }
        }
    }
    for (at, node) in nodes.iter().enumerate() {
        if at == root_at || is_varyk_at(at) {
            continue;
        }
        for edge in &node.edges {
            if let Some(used) = target_of(edge)
                .filter(|&to| is_varyk_at(to))
                .and_then(|to| nodes.get(to))
            {
                refusals.push(Refusal::reached(format!(
                    "the Rust crate `{}` uses the Varyk package `{}`, which is not supported yet",
                    node.name, used.name
                )));
            }
        }
    }
    for (place, &at) in order.iter().enumerate() {
        let Some(&node) = nodes.get(at) else {
            continue;
        };
        for (other_at, other) in nodes.iter().enumerate() {
            if other_at == at || other.name != node.name || other.version != node.version {
                continue;
            }
            let counts = match position.get(&other_at) {
                // Each pair of Varyk packages is reported once.
                Some(&other_place) => other_place > place,
                None => other.source.is_none(),
            };
            if counts {
                refusals.push(Refusal {
                    code: codes::V0401,
                    message: format!(
                        "the Varyk package `{}` {} is in this build twice, which is not \
                         supported yet",
                        node.name, node.version
                    ),
                    note: format!(
                        "one comes from {} and one from {}; Varyk builds each Varyk package \
                         from a folder of its own, and cargo cannot build two packages of one \
                         name and version from folders; keep one of them",
                        origin(node),
                        origin(other)
                    ),
                });
            }
        }
    }
    if !refusals.is_empty() {
        let mut unique: Vec<Refusal> = Vec::new();
        for refusal in refusals {
            if !unique.contains(&refusal) {
                unique.push(refusal);
            }
        }
        return Err(unique);
    }

    let deps_of = |at: usize| -> Deps {
        let edges = nodes.get(at).map_or(&[][..], |node| node.edges.as_slice());
        edges
            .iter()
            .filter(|edge| edge.plain())
            .filter_map(|edge| {
                let to = target_of(edge)?;
                let target = position
                    .get(&to)
                    .map_or(DepTarget::Rust, |&place| DepTarget::Varyk(place));
                Some((edge.name.clone(), target))
            })
            .collect()
    };
    let packages = order
        .iter()
        .filter_map(|&at| {
            let node = nodes.get(at)?;
            Some(VarykPackage {
                dir: node.dir.clone(),
                name: node.name.clone(),
                version: node.version.clone(),
                source: node.source.clone(),
                deps: deps_of(at),
            })
        })
        .collect();
    Ok((packages, deps_of(root_at)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_for_a_git_or_other_registry_package_names_where_it_came_from() {
        let crates_io = "registry+https://github.com/rust-lang/crates.io-index";
        assert_eq!(
            registry_line("units", "1.2.0", crates_io),
            "units = \"1.2.0\""
        );
        assert_eq!(
            registry_line(
                "units",
                "1.2.0",
                "git+https://example.com/units?tag=v1#abc123"
            ),
            "units = { git = \"https://example.com/units\", tag = \"v1\" }"
        );
        assert_eq!(
            registry_line("units", "1.2.0", "git+https://example.com/units#abc123"),
            "units = { git = \"https://example.com/units\", rev = \"abc123\" }"
        );
        assert_eq!(
            registry_line("units", "1.2.0", "git+https://example.com/units"),
            "units = { git = \"https://example.com/units\" }"
        );
        assert!(
            registry_line("units", "1.2.0", "registry+https://example.com/index")
                .contains("registry =")
        );
    }

    const BASIC: &str = include_str!("../tests/fixtures/graph/basic.json");
    const DEV: &str = include_str!("../tests/fixtures/graph/dev.json");
    const OPTIONAL: &str = include_str!("../tests/fixtures/graph/optional.json");
    const RUST: &str = include_str!("../tests/fixtures/graph/rust.json");
    const DUP: &str = include_str!("../tests/fixtures/graph/dup.json");
    const STD: &str = include_str!("../tests/fixtures/graph/std.json");

    /// Whether `dir` is one of `dirs`: the recorded paths do not exist, so
    /// the tests say which directories hold `src/lib.vr`.
    fn varyk_dirs(dirs: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |dir| dirs.iter().any(|varyk| dir == Path::new(varyk))
    }

    /// In `basic.json`, every dependency but `plain` is a Varyk package.
    const BASIC_VARYK: &[&str] = &["/work/units", "/work/units2", "/work/route", "/work/geokit"];

    fn basic() -> (Vec<VarykPackage>, Deps) {
        classify(BASIC, &varyk_dirs(BASIC_VARYK)).expect("the basic graph is accepted")
    }

    /// The index of the Varyk package at `dir`.
    fn at(packages: &[VarykPackage], dir: &str) -> usize {
        packages
            .iter()
            .position(|package| package.dir == Path::new(dir))
            .expect("a Varyk package of the build")
    }

    /// The messages of the refusals of `json` with `varyk` directories.
    fn refused(json: &str, varyk: &'static [&'static str]) -> Vec<(&'static str, String)> {
        classify(json, &varyk_dirs(varyk))
            .expect_err("the graph is refused")
            .into_iter()
            .map(|refusal| (refusal.code, refusal.message))
            .collect()
    }

    #[test]
    fn varyk_packages_are_told_from_rust_crates_by_their_directory() {
        let (packages, root_deps) = basic();
        let mut dirs: Vec<_> = packages.iter().map(|package| package.dir.clone()).collect();
        dirs.sort();
        let mut expected: Vec<PathBuf> = BASIC_VARYK.iter().map(PathBuf::from).collect();
        expected.sort();
        assert_eq!(dirs, expected);
        assert!(root_deps.contains(&("plain".to_string(), DepTarget::Rust)));

        // The same graph with nothing a Varyk package: all Rust crates.
        let (packages, root_deps) = classify(BASIC, &varyk_dirs(&[])).unwrap();
        assert!(packages.is_empty());
        assert!(
            root_deps
                .iter()
                .all(|(_, target)| *target == DepTarget::Rust)
        );
    }

    #[test]
    fn renamed_and_hyphenated_keys_are_named_with_underscores() {
        let (packages, root_deps) = basic();
        let key = |name: &str| {
            root_deps
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, target)| *target)
        };
        assert_eq!(
            key("route_planner"),
            Some(DepTarget::Varyk(at(&packages, "/work/route")))
        );
        assert_eq!(
            key("geo_kit"),
            Some(DepTarget::Varyk(at(&packages, "/work/geokit")))
        );
        assert_eq!(
            key("old_units"),
            Some(DepTarget::Varyk(at(&packages, "/work/units2")))
        );
        assert_eq!(key("route-planner"), None);
        assert_eq!(key("route"), None);
        assert_eq!(root_deps.len(), 5);
    }

    #[test]
    fn two_versions_of_one_package_are_two_packages() {
        let (packages, _) = basic();
        let mut units: Vec<_> = packages
            .iter()
            .filter(|package| package.name == "units")
            .map(|package| package.version.as_str())
            .collect();
        units.sort();
        assert_eq!(units, ["0.1.0", "0.2.0"]);
    }

    #[test]
    fn a_package_that_uses_a_package_points_at_it_and_comes_after_it() {
        let (packages, _) = basic();
        let route = at(&packages, "/work/route");
        let units = at(&packages, "/work/units");
        assert_eq!(
            packages[route].deps,
            [("units".to_string(), DepTarget::Varyk(units))]
        );
        assert!(units < route, "{packages:?}");
        assert_eq!(packages[route].name, "route");
        assert_eq!(packages[route].version, "0.1.0");
        // Every package comes after each Varyk package it depends on.
        for (place, package) in packages.iter().enumerate() {
            for (_, target) in &package.deps {
                if let DepTarget::Varyk(dep) = target {
                    assert!(*dep < place, "{packages:?}");
                }
            }
        }
    }

    #[test]
    fn a_varyk_package_under_dev_dependencies_is_refused() {
        assert_eq!(
            refused(DEV, &["/work/units"]),
            [(
                codes::V0401,
                "this package lists the Varyk package `units` under `[dev-dependencies]`, which \
                 is not supported yet"
                    .to_string()
            )]
        );
        // A Rust crate there is cargo's business.
        assert!(classify(DEV, &varyk_dirs(&[])).is_ok());
    }

    #[test]
    fn an_optional_varyk_package_a_feature_turned_on_is_refused() {
        assert_eq!(
            refused(OPTIONAL, &["/work/units"]),
            [(
                codes::V0401,
                "this package lists the Varyk package `units` as an optional dependency, which \
                 is not supported yet"
                    .to_string()
            )]
        );
        assert!(classify(OPTIONAL, &varyk_dirs(&[])).is_ok());
    }

    #[test]
    fn a_varyk_package_a_rust_crate_uses_is_refused_even_when_also_used_directly() {
        assert_eq!(
            refused(RUST, &["/work/units"]),
            [(
                codes::V0401,
                "the Rust crate `glue` uses the Varyk package `units`, which is not supported yet"
                    .to_string()
            )]
        );
        assert!(classify(RUST, &varyk_dirs(&[])).is_ok());
    }

    #[test]
    fn two_varyk_packages_of_one_name_and_version_are_refused_once() {
        let refusals = classify(
            DUP,
            &varyk_dirs(&["/work/units", "/cargo/git/checkouts/unitsgit/0000000"]),
        )
        .expect_err("refused");
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0].code, codes::V0401);
        assert_eq!(
            refusals[0].message,
            "the Varyk package `units` 0.1.0 is in this build twice, which is not supported yet"
        );
        assert!(
            refusals[0].note.contains("the folder `/work/units`"),
            "{refusals:?}"
        );
        assert!(
            refusals[0].note.contains("`git+file:///work/unitsgit#"),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_varyk_package_sharing_name_and_version_with_a_path_crate_is_refused() {
        // The git `units` is a Varyk package, the `path` one a Rust crate:
        // the driver would make the Varyk one a `path` package too.
        assert_eq!(
            refused(DUP, &["/cargo/git/checkouts/unitsgit/0000000"]),
            [(
                codes::V0401,
                "the Varyk package `units` 0.1.0 is in this build twice, which is not supported \
                 yet"
                .to_string()
            )]
        );
        // A Varyk package by `path` beside a Rust crate from git is fine.
        assert!(classify(DUP, &varyk_dirs(&["/work/units"])).is_ok());
    }

    #[test]
    fn an_answer_that_cannot_be_read_is_v0405() {
        for json in ["", "{}", "{\"packages\": [], \"resolve\": null}"] {
            let refusals = classify(json, &varyk_dirs(&[])).expect_err("unreadable");
            assert_eq!(refusals.len(), 1);
            assert_eq!(refusals[0].code, codes::V0405, "{json}");
        }
    }

    /// The `varyk-std` checks of a program `app` whose `[dependencies]`
    /// are `deps`, beside `lock` if any, in a build whose `varyk-std` is
    /// the one `STD` records, 0.4.0, with a compiler at 0.5.0.
    fn build_std(deps: &str, lock: Option<&str>, uses_std: bool) -> Vec<Diagnostic> {
        use crate::package::tests::{MANIFEST, package};
        let dir = package("graph_std", &format!("{MANIFEST}{deps}"), "src/main.vr");
        if let Some(lock) = lock {
            dir.write("Cargo.lock", lock);
        }
        let mut sources = Vec::new();
        let program = crate::package::load(&dir.0.join("Cargo.toml"), &mut sources)
            .unwrap_or_else(|diagnostics| panic!("loads: {diagnostics:#?}"));
        let resolved = resolved_std(STD);
        assert_eq!(resolved.as_deref(), Some("0.4.0"));
        crate::package::check_build_std(&program, uses_std, true, resolved.as_deref(), "0.5.0")
    }

    #[test]
    fn a_resolved_std_older_than_the_compiler_is_v0404_at_the_program_s_line() {
        let found = build_std(
            "[dependencies]\nroute = { path = \"../route\" }\nvaryk-std = \"0.5\"\n",
            None,
            true,
        );
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].code, codes::V0404);
        assert!(found[0].message.contains("`varyk-std` 0.4.0"), "{found:#?}");
        assert!(
            found[0].notes[0].contains("`cargo update -p varyk-std`"),
            "{found:#?}"
        );
        assert_eq!(
            found[0].span.end - found[0].span.start,
            "varyk-std".len() as u32
        );

        // A program that does not use it itself, without the line: at its
        // `[dependencies]`.
        let found = build_std(
            "[dependencies]\nroute = { path = \"../route\" }\n",
            None,
            false,
        );
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].code, codes::V0404);
        assert_eq!(
            found[0].span.end - found[0].span.start,
            "dependencies".len() as u32
        );
    }

    #[test]
    fn a_stale_lock_is_reported_once_beside_the_resolved_std() {
        let lock = "version = 4\n\n[[package]]\nname = \"varyk-std\"\nversion = \"0.4.0\"\n";
        let found = build_std(
            "[dependencies]\nroute = { path = \"../route\" }\nvaryk-std = \"0.5\"\n",
            Some(lock),
            true,
        );
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(found[0].message.contains("`Cargo.lock`"), "{found:#?}");
    }

    #[test]
    fn a_resolved_std_no_older_than_the_compiler_or_unused_is_accepted() {
        let resolved = resolved_std(STD);
        let dir = crate::package::tests::package(
            "graph_std_ok",
            &format!(
                "{}[dependencies]\nvaryk-std = \"0.4\"\n",
                crate::package::tests::MANIFEST
            ),
            "src/main.vr",
        );
        let mut sources = Vec::new();
        let program = crate::package::load(&dir.0.join("Cargo.toml"), &mut sources)
            .unwrap_or_else(|diagnostics| panic!("loads: {diagnostics:#?}"));
        let check = |build_uses_std, compiler| {
            crate::package::check_build_std(
                &program,
                true,
                build_uses_std,
                resolved.as_deref(),
                compiler,
            )
        };
        assert!(check(true, "0.4.0").is_empty());
        assert!(check(false, "0.4.1").is_empty());
        assert_eq!(check(true, "0.4.1").len(), 1);
        // No `varyk-std` in the build at all.
        assert_eq!(resolved_std(BASIC), None);
    }

    #[test]
    fn a_path_line_is_written_from_the_using_package() {
        let from = Path::new("/work/app");
        assert_eq!(relative(from, Path::new("/work/app/units")), "units");
        assert_eq!(relative(from, Path::new("/work/units")), "../units");
        assert_eq!(
            relative(from, Path::new("/other/units")),
            "../../other/units"
        );
        assert_eq!(relative(from, from), ".");
    }

    #[test]
    fn the_dependency_tables_are_named_as_written() {
        assert_eq!(table(Some("dev"), None), "[dev-dependencies]");
        assert_eq!(table(Some("build"), None), "[build-dependencies]");
        assert_eq!(
            table(None, Some("cfg(unix)")),
            "[target.'cfg(unix)'.dependencies]"
        );
    }
}
