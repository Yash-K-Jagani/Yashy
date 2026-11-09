//! Resolving a manifest's dependency graph to source directories.
//!
//! Resolution is exact-pin only and deliberately small: there is no version
//! solver, no backtracking and no registry protocol. Every package is either a
//! path, a git URL at a commit, or (in a later task) a registry entry with an
//! exact version. The graph is built depth-first with a visiting stack, so a
//! cycle is a diagnostic naming the path that closed it, never a stack
//! overflow.
//!
//! A path package is read in place. A git package is read from the store, so
//! `--offline` means "the checkout is already there or the build stops". The
//! graph also verifies `yashy.lock`: a package whose version, source or content
//! hash differs is `E1802`, and `yashy update` is the only thing that rewrites
//! the lock.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use yashy_manifest::{hash_tree, Dependency, LockedPackage, Lockfile, Manifest};

use crate::fetch::{FetchError, Store};

/// One resolved package, with the directory its modules are loaded from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    /// The lockfile source spelling: `path+...` or `git+<url>?rev=<sha>`.
    pub source: String,
    /// The package root (the directory containing `yashy.toml`).
    pub root: PathBuf,
    /// Direct dependencies as `"name version"`, sorted.
    pub dependencies: Vec<String>,
}

/// The whole dependency graph, dependencies included.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedGraph {
    /// Sorted by name, so the lockfile and diagnostics are deterministic.
    pub packages: Vec<ResolvedPackage>,
}

impl ResolvedGraph {
    /// The directories an import may be resolved against: each package's
    /// `src/` when it has one, otherwise its root.
    pub fn module_roots(&self) -> Vec<PathBuf> {
        let mut roots = BTreeSet::new();
        for package in &self.packages {
            let src = package.root.join("src");
            roots.insert(if src.is_dir() {
                src
            } else {
                package.root.clone()
            });
        }
        roots.into_iter().collect()
    }

    /// Turn the graph into the lockfile `yashy update` writes.
    pub fn to_lock(&self) -> Result<Lockfile, ResolveError> {
        let mut packages = Vec::with_capacity(self.packages.len());
        for package in &self.packages {
            let hash = hash_tree(&package.root).map_err(|error| {
                ResolveError::new(
                    yashy_common::codes::LOCKFILE_MISMATCH,
                    format!(
                        "cannot hash `{}` at {}: {error}",
                        package.name,
                        package.root.display()
                    ),
                )
            })?;
            packages.push(LockedPackage {
                name: package.name.clone(),
                version: package.version.clone(),
                source: package.source.clone(),
                hash,
                dependencies: package.dependencies.clone(),
            });
        }
        Ok(Lockfile::new(packages))
    }
}

/// A resolution failure, with the code its diagnostic prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveError {
    pub code: &'static str,
    pub message: String,
}

impl ResolveError {
    pub fn new(code: &'static str, message: impl Into<String>) -> ResolveError {
        ResolveError {
            code,
            message: message.into(),
        }
    }

    fn cycle(chain: Vec<String>) -> ResolveError {
        ResolveError::new(
            yashy_common::codes::DEPENDENCY_CYCLE,
            format!("dependency cycle: {}", chain.join(" -> ")),
        )
    }

    fn offline(message: impl Into<String>) -> ResolveError {
        ResolveError::new(yashy_common::codes::OFFLINE_MISSING, message)
    }

    fn lock(message: impl Into<String>) -> ResolveError {
        ResolveError::new(yashy_common::codes::LOCKFILE_MISMATCH, message)
    }

    fn manifest(message: impl Into<String>) -> ResolveError {
        ResolveError::new(yashy_common::codes::MANIFEST_SYNTAX, message)
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Resolve `manifest` at `base` into a graph.
///
/// `lock` is checked when present: every resolved package must appear with the
/// same version, source and content hash, and no lock entry may be unused. A
/// caller generating a new lock (`yashy update`) passes `None`.
pub fn resolve(
    manifest: &Manifest,
    lock: Option<&Lockfile>,
    base: &Path,
    offline: bool,
    store: &Store,
) -> Result<ResolvedGraph, ResolveError> {
    let mut resolver = Resolver {
        lock,
        offline,
        store,
        packages: BTreeMap::new(),
        visiting: Vec::new(),
    };
    resolver.visit_manifest(manifest, base)?;
    let graph = ResolvedGraph {
        packages: resolver.packages.into_values().collect(),
    };
    if let Some(lock) = lock {
        graph.check_lock(lock)?;
    }
    Ok(graph)
}

struct Resolver<'a> {
    lock: Option<&'a Lockfile>,
    offline: bool,
    store: &'a Store,
    packages: BTreeMap<String, ResolvedPackage>,
    /// Package names currently on the DFS stack, outermost first.
    visiting: Vec<String>,
}

impl Resolver<'_> {
    fn visit_manifest(&mut self, manifest: &Manifest, base: &Path) -> Result<(), ResolveError> {
        let dependencies: Vec<(&String, &Dependency)> = manifest
            .dependencies
            .iter()
            .chain(manifest.dev_dependencies.iter())
            .collect();
        for (name, dependency) in dependencies {
            self.visit_dependency(name, dependency, base)?;
        }
        Ok(())
    }

    fn visit_dependency(
        &mut self,
        name: &str,
        dependency: &Dependency,
        base: &Path,
    ) -> Result<(), ResolveError> {
        let (root, source, declared_version) = self.locate(name, dependency, base)?;
        let child = load_package(name, &root, &declared_version)?;

        if let Some(position) = self.visiting.iter().position(|seen| seen == name) {
            let mut chain = self.visiting[position..].to_vec();
            chain.push(name.to_string());
            return Err(ResolveError::cycle(chain));
        }

        if let Some(existing) = self.packages.get(name) {
            if existing.version != child.version || existing.root != root {
                return Err(ResolveError::lock(format!(
                    "`{name}` is required at two different versions or sources: \
                     {} ({}) and {} ({}); exact pins must agree",
                    existing.version,
                    existing.root.display(),
                    child.version,
                    root.display()
                )));
            }
            return Ok(());
        }

        self.visiting.push(name.to_string());
        let mut child_dependencies = Vec::new();
        if let Some(child_manifest) = &child.manifest {
            self.visit_manifest(child_manifest, &root)?;
            let mut names: Vec<String> = child_manifest
                .dependencies
                .keys()
                .chain(child_manifest.dev_dependencies.keys())
                .cloned()
                .collect();
            names.sort();
            names.dedup();
            for dependency_name in names {
                if let Some(resolved) = self.packages.get(&dependency_name) {
                    child_dependencies.push(format!("{dependency_name} {}", resolved.version));
                }
            }
        }
        self.visiting.pop();

        self.packages.insert(
            name.to_string(),
            ResolvedPackage {
                name: name.to_string(),
                version: child.version,
                source,
                root,
                dependencies: child_dependencies,
            },
        );
        Ok(())
    }

    fn locate(
        &self,
        name: &str,
        dependency: &Dependency,
        base: &Path,
    ) -> Result<(PathBuf, String, Option<String>), ResolveError> {
        match dependency {
            Dependency::Path { path } => {
                let root = lexical_absolute(&base.join(path));
                if !root.is_dir() {
                    return Err(ResolveError::offline(format!(
                        "`{name}` is a path dependency at `{}`, which is not a directory",
                        root.display()
                    )));
                }
                Ok((
                    root,
                    LockedPackage::path_source(path),
                    self.locked_version(name),
                ))
            }
            Dependency::Git { url, rev } => {
                let root = self
                    .store
                    .git_checkout(url, rev, self.offline)
                    .map_err(fetch_error)?;
                Ok((
                    root,
                    LockedPackage::git_source(url, rev),
                    self.locked_version(name),
                ))
            }
            Dependency::Registry { version } => {
                let registry_root = crate::fetch::default_registry_root_for(self.store.root());
                let entry = crate::registry::lookup(&registry_root, name, version)
                    .map_err(|error| ResolveError::offline(error.message))?
                    .ok_or_else(|| {
                        ResolveError::offline(format!(
                            "`{name}` {version} is a registry dependency, but no registry is \
                             configured in this build; use a `path` or `git`+`rev` dependency"
                        ))
                    })?;
                let root = self
                    .store
                    .registry_checkout(&entry, &registry_root, self.offline)
                    .map_err(fetch_error)?;
                Ok((
                    root,
                    format!("registry+file://{}", registry_root.display()),
                    self.locked_version(name),
                ))
            }
        }
    }

    fn locked_version(&self, name: &str) -> Option<String> {
        self.lock
            .and_then(|lock| lock.packages.iter().find(|package| package.name == name))
            .map(|package| package.version.clone())
    }
}

struct LoadedPackage {
    version: String,
    manifest: Option<Manifest>,
}

fn load_package(
    name: &str,
    root: &Path,
    locked_version: &Option<String>,
) -> Result<LoadedPackage, ResolveError> {
    let manifest_path = root.join("yashy.toml");
    if manifest_path.is_file() {
        let manifest = Manifest::load(&manifest_path).map_err(|error| {
            ResolveError::manifest(format!(
                "`{name}` at {}: {}:{}: {}",
                root.display(),
                error.line,
                error.column,
                error.message
            ))
        })?;
        if manifest.package.name != name {
            return Err(ResolveError::manifest(format!(
                "dependency `{name}` points at a package named `{}` in {}",
                manifest.package.name,
                manifest_path.display()
            )));
        }
        return Ok(LoadedPackage {
            version: manifest.package.version.clone(),
            manifest: Some(manifest),
        });
    }
    // A path package without a manifest is still source; its version comes
    // from the lock when there is one, so the graph is stable.
    Ok(LoadedPackage {
        version: locked_version
            .clone()
            .unwrap_or_else(|| "0.0.0".to_string()),
        manifest: None,
    })
}

fn fetch_error(error: FetchError) -> ResolveError {
    ResolveError::offline(error.message)
}

/// Make `path` absolute and remove `.`/`..` lexically, without touching the
/// filesystem (the directory may be handed to diagnostics before it exists).
fn lexical_absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

impl ResolvedGraph {
    /// The `E1802` half: the lock must describe exactly this graph.
    fn check_lock(&self, lock: &Lockfile) -> Result<(), ResolveError> {
        for package in &self.packages {
            let Some(entry) = lock
                .packages
                .iter()
                .find(|entry| entry.name == package.name && entry.version == package.version)
            else {
                return Err(ResolveError::lock(format!(
                    "`{}` {} resolves to {}, but yashy.lock does not record it; \
                     run `yashy update`",
                    package.name,
                    package.version,
                    package.root.display()
                )));
            };
            if entry.source != package.source {
                return Err(ResolveError::lock(format!(
                    "`{}` {} comes from {}, but yashy.lock records {}; run `yashy update`",
                    package.name, package.version, package.source, entry.source
                )));
            }
            let hash = hash_tree(&package.root).map_err(|error| {
                ResolveError::lock(format!(
                    "cannot hash `{}` at {}: {error}",
                    package.name,
                    package.root.display()
                ))
            })?;
            if entry.hash != hash {
                return Err(ResolveError::lock(format!(
                    "the source of `{}` {} hashes to {hash}, but yashy.lock records {}; \
                     run `yashy update`",
                    package.name, package.version, entry.hash
                )));
            }
        }
        for entry in &lock.packages {
            if !self
                .packages
                .iter()
                .any(|package| package.name == entry.name && package.version == entry.version)
            {
                return Err(ResolveError::lock(format!(
                    "yashy.lock records `{}` {}, but nothing depends on it; \
                     run `yashy update`",
                    entry.name, entry.version
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(text: &str) -> Manifest {
        Manifest::parse(text).expect("the test manifest parses")
    }

    fn store() -> Store {
        Store::at(std::env::temp_dir().join(format!("yashy-resolve-test-{}", std::process::id())))
    }

    #[test]
    fn a_cold_offline_git_dependency_is_e1803() {
        let manifest = manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.x]\n\
             git = \"https://example.invalid/x\"\nrev = \"abc\"\n",
        );
        let error = resolve(&manifest, None, Path::new("."), true, &store())
            .expect_err("the store is cold");
        assert_eq!(error.code, yashy_common::codes::OFFLINE_MISSING);
        assert!(error.message.contains("yashy fetch"), "{error}");
    }

    #[test]
    fn a_registry_dependency_says_it_is_not_configured() {
        let manifest = manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhttp = \"0.3.1\"\n",
        );
        let error =
            resolve(&manifest, None, Path::new("."), false, &store()).expect_err("no registry");
        assert!(error.message.contains("registry"), "{error}");
    }

    #[test]
    fn a_lock_that_lacks_a_package_is_e1802() {
        let base = std::env::temp_dir().join(format!("yashy-resolve-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("net")).unwrap();
        std::fs::write(base.join("net/lib.ya"), "pub fn x() {}\n").unwrap();
        let manifest = manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.net]\npath = \"net\"\n",
        );
        let empty = Lockfile::new(Vec::new());
        let error = resolve(&manifest, Some(&empty), &base, false, &store())
            .expect_err("the lock is empty");
        assert_eq!(error.code, yashy_common::codes::LOCKFILE_MISMATCH);
        assert!(error.message.contains("yashy update"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_cycle_names_the_path() {
        let base = std::env::temp_dir().join(format!("yashy-resolve-cycle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a")).unwrap();
        std::fs::create_dir_all(base.join("b")).unwrap();
        std::fs::write(
            base.join("yashy.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.a]\npath = \"a\"\n",
        )
        .unwrap();
        std::fs::write(
            base.join("a/yashy.toml"),
            "[package]\nname = \"a\"\nversion = \"1.0.0\"\n\n[dependencies.b]\npath = \"../b\"\n",
        )
        .unwrap();
        std::fs::write(
            base.join("b/yashy.toml"),
            "[package]\nname = \"b\"\nversion = \"1.0.0\"\n\n[dependencies.a]\npath = \"../a\"\n",
        )
        .unwrap();
        let manifest = Manifest::load(&base.join("yashy.toml")).unwrap();
        let error = resolve(&manifest, None, &base, false, &store()).expect_err("a cycle");
        assert_eq!(error.code, yashy_common::codes::DEPENDENCY_CYCLE);
        assert!(error.message.contains("a -> b -> a"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
