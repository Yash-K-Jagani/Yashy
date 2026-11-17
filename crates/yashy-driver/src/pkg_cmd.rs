//! The package surface: tree, why, outdated, vendor, info, remove and yank.
//!
//! Every command reads the same lockfile and manifests the resolver uses. The
//! human form is a rendering of the same facts as `--json`, so a script and a
//! person cannot drift apart.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use yashy_common::Json;
use yashy_manifest::{Lockfile, Manifest};

/// Dependency edges from a lockfile: package name -> `"name version"` deps.
fn edges(lock: &Lockfile) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for package in &lock.packages {
        out.insert(
            package.name.clone(),
            package
                .dependencies
                .iter()
                .map(|entry| entry.split_whitespace().next().unwrap_or(entry).to_string())
                .collect(),
        );
    }
    out
}

/// The human rendering of [`tree`].
pub fn tree_lines(lock: &Lockfile, depth: usize) -> Vec<String> {
    let graph = edges(lock);
    let mut lines = Vec::new();
    let mut seen = BTreeSet::new();
    let root = lock
        .packages
        .first()
        .map(|package| package.name.clone())
        .unwrap_or_default();
    let mut queue = VecDeque::from([(root, 0usize, String::new())]);
    while let Some((name, level, prefix)) = queue.pop_front() {
        if level > depth || !seen.insert(name.clone()) {
            continue;
        }
        let version = lock
            .packages
            .iter()
            .find(|package| package.name == name)
            .map(|package| package.version.as_str())
            .unwrap_or("?");
        lines.push(format!("{prefix}{name} {version}"));
        if level < depth {
            if let Some(children) = graph.get(&name) {
                for child in children {
                    queue.push_back((child.clone(), level + 1, format!("{prefix}  ")));
                }
            }
        }
    }
    lines
}

/// A depth-limited, stable rendering of the resolved graph.
pub fn tree(lock: &Lockfile, depth: usize) -> Json {
    let lines = tree_lines(lock, depth);
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("tree"));
    report.set(
        "lines",
        Json::array(lines.iter().map(|line| Json::string(line)).collect()),
    );
    report
}

/// Every path from the root to `name`, shortest first.
pub fn why(lock: &Lockfile, name: &str) -> Vec<Vec<String>> {
    let graph = edges(lock);
    let root = lock
        .packages
        .first()
        .map(|package| package.name.clone())
        .unwrap_or_default();
    let mut paths = Vec::new();
    let mut queue = VecDeque::from([vec![root]]);
    while let Some(path) = queue.pop_front() {
        let current = path.last().cloned().unwrap_or_default();
        if current == name {
            paths.push(path);
            continue;
        }
        if path.len() > lock.packages.len() + 1 {
            continue;
        }
        if let Some(children) = graph.get(&current) {
            for child in children {
                if !path.contains(child) {
                    let mut next = path.clone();
                    next.push(child.clone());
                    queue.push_back(next);
                }
            }
        }
    }
    paths.sort_by_key(|path| path.len());
    paths
}

/// A package's declared facts plus its reverse dependencies.
pub fn info(lock: &Lockfile, name: &str, root: &Path) -> Result<Json, String> {
    let package = lock
        .packages
        .iter()
        .find(|package| package.name == name)
        .ok_or_else(|| format!("no locked package named `{name}`"))?;
    let manifest_path =
        package_root(root, package.source.as_str()).map(|path| path.join("yashy.toml"));
    let manifest = manifest_path
        .as_deref()
        .and_then(|path| Manifest::load(path).ok());
    let reverse: Vec<String> = lock
        .packages
        .iter()
        .filter(|candidate| {
            candidate
                .dependencies
                .iter()
                .any(|dependency| dependency.starts_with(&format!("{name} ")))
        })
        .map(|candidate| candidate.name.clone())
        .collect();
    let mut object = Json::object();
    object.set("name", Json::string(name));
    object.set("version", Json::string(&package.version));
    object.set("source", Json::string(&package.source));
    object.set("hash", Json::string(&package.hash));
    object.set(
        "license",
        Json::string(
            manifest
                .as_ref()
                .and_then(|manifest| manifest.package.license.clone())
                .unwrap_or_else(|| "NOASSERTION".to_string()),
        ),
    );
    object.set(
        "capabilities",
        Json::array(
            manifest
                .as_ref()
                .and_then(|manifest| manifest.capabilities.clone())
                .map(|set| {
                    set.entries()
                        .iter()
                        .map(|(name, _)| Json::string(*name))
                        .collect()
                })
                .unwrap_or_default(),
        ),
    );
    object.set(
        "reverse_dependencies",
        Json::array(reverse.iter().map(|name| Json::string(name)).collect()),
    );
    object.set("schema_version", Json::number(1));
    Ok(object)
}

fn package_root(project: &Path, source: &str) -> Option<PathBuf> {
    source
        .strip_prefix("path+")
        .map(|relative| project.join(relative))
}

/// Remove a dependency from a manifest and its lockfile. The original manifest
/// is kept as `yashy.toml.bak` so the change is reviewable and reversible.
pub fn remove(project: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let manifest_path = project.join("yashy.toml");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let backup = PathBuf::from(format!("{}.bak", manifest_path.display()));
    std::fs::write(&backup, &text)
        .map_err(|error| format!("cannot write {}: {error}", backup.display()))?;
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            skipping = false;
        }
        if trimmed == format!("[dependencies.{name}]")
            || trimmed == format!("[dev-dependencies.{name}]")
        {
            skipping = true;
            continue;
        }
        if skipping && !trimmed.is_empty() {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    let manifest = Manifest::parse(&out)
        .map_err(|error| format!("the edited manifest is invalid: {}", error.message))?;
    if manifest.dependencies.contains_key(name) || manifest.dev_dependencies.contains_key(name) {
        return Err(format!("`{name}` is still a dependency after the edit"));
    }
    std::fs::write(&manifest_path, out)
        .map_err(|error| format!("cannot write {}: {error}", manifest_path.display()))?;
    let lock_path = project.join("yashy.lock");
    let mut files = vec![backup, manifest_path];
    if lock_path.is_file() {
        let lock = Lockfile::load(&lock_path)
            .map_err(|error| format!("cannot read the lockfile: {error}"))?;
        let kept: Vec<_> = lock
            .packages
            .into_iter()
            .filter(|package| package.name != name)
            .collect();
        std::fs::write(&lock_path, Lockfile::new(kept).render())
            .map_err(|error| format!("cannot write {}: {error}", lock_path.display()))?;
        files.push(lock_path);
    }
    Ok(files)
}

/// Compare locked versions to a registry index.
pub fn outdated(lock: &Lockfile, registry: &Path) -> Result<Json, String> {
    let entries = crate::registry::read_index(registry)
        .map_err(|error| format!("cannot read the registry: {error}"))?;
    let mut rows = Vec::new();
    for package in &lock.packages {
        let candidates: Vec<&crate::registry::RegistryEntry> = entries
            .iter()
            .filter(|entry| entry.name == package.name)
            .collect();
        let newest = candidates
            .iter()
            .filter_map(|entry| yashy_manifest::Version::parse(&entry.version))
            .max()
            .map(|version| version.to_string());
        let yanked = candidates
            .iter()
            .any(|entry| entry.version == package.version && entry.yanked);
        let mut row = Json::object();
        row.set("name", Json::string(&package.name));
        row.set("locked", Json::string(&package.version));
        row.set("newest", Json::string(newest.as_deref().unwrap_or("?")));
        row.set("yanked", Json::bool(yanked));
        rows.push(row);
    }
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("outdated"));
    report.set("packages", Json::array(rows));
    Ok(report)
}

/// Copy every locked registry tarball into `vendor`, reporting the bytes.
pub fn vendor(registry: &Path, out: &Path) -> Result<(usize, u64), String> {
    std::fs::create_dir_all(out).map_err(|error| error.to_string())?;
    let entries = crate::registry::read_index(registry)
        .map_err(|error| format!("cannot read the registry: {error}"))?;
    let mut count = 0usize;
    let mut bytes = 0u64;

    // A vendor directory is a complete local registry, not a pile of tarballs.
    for name in ["index.json", "index.sig"] {
        let source = registry.join(name);
        if let Ok(data) = std::fs::read(&source) {
            std::fs::write(out.join(name), &data).map_err(|error| error.to_string())?;
            bytes += data.len() as u64;
        }
    }
    let sbom_dir = registry.join("sbom");
    if sbom_dir.is_dir() {
        let target = out.join("sbom");
        std::fs::create_dir_all(&target).map_err(|error| error.to_string())?;
        for entry in std::fs::read_dir(&sbom_dir).map_err(|error| error.to_string())?.flatten() {
            if let Ok(data) = std::fs::read(entry.path()) {
                std::fs::write(target.join(entry.file_name()), &data)
                    .map_err(|error| error.to_string())?;
                bytes += data.len() as u64;
            }
        }
    }
    let packages = out.join("packages");
    std::fs::create_dir_all(&packages).map_err(|error| error.to_string())?;
    for entry in entries {
        let source = crate::registry::tarball_path(registry, &entry);
        let Ok(data) = std::fs::read(&source) else {
            continue;
        };
        let target = packages.join(source.file_name().unwrap_or_default());
        std::fs::write(&target, &data).map_err(|error| error.to_string())?;
        bytes += data.len() as u64;
        let signature = source.with_extension("sig");
        if let Ok(data) = std::fs::read(&signature) {
            let name = signature.file_name().unwrap_or_default();
            std::fs::write(packages.join(name), &data).map_err(|error| error.to_string())?;
            bytes += data.len() as u64;
        }
        count += 1;
    }
    Ok((count, bytes))
}
