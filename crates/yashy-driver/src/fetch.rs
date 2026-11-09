//! Fetching package sources into a local, content-keyed store.
//!
//! The store is deliberately dumb: one directory per `(url, rev)` pair, the
//! checkout inside it, and no registry service. `--offline` is checked before
//! any command runs, so an offline build either finds the directory or fails
//! with `E1803`; it never silently reaches for the network.
//!
//! The store lives under `YASHY_PACKAGE_STORE` when set, otherwise under the
//! compiler's cache directory (`YASHY_CACHE_DIR` or the platform convention).
//! A test points both at a private directory, which is why the root is a
//! constructor argument rather than a global.

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use yashy_manifest::sha256_hex;

use crate::registry::{extract_tarball, read_tarball, RegistryEntry};

/// A failure to obtain a package source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchError {
    pub message: String,
}

impl FetchError {
    fn new(message: impl Into<String>) -> FetchError {
        FetchError {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The package store, rooted at one directory.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open the default store, creating it if it does not exist.
    pub fn open() -> io::Result<Store> {
        let store = Store {
            root: default_store_root(),
        };
        std::fs::create_dir_all(store.root.join("git"))?;
        Ok(store)
    }

    /// A store rooted at an explicit directory, for tests.
    pub fn at(root: PathBuf) -> Store {
        Store { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The checkout for a registry entry, verifying its tarball hash.
    pub fn registry_checkout(
        &self,
        entry: &RegistryEntry,
        registry_root: &Path,
        offline: bool,
    ) -> Result<PathBuf, FetchError> {
        let hash = entry.hash.trim_start_matches("sha256:");
        let directory = self.root.join("registry").join(hash);
        if directory.is_dir() {
            return Ok(directory);
        }
        if offline {
            return Err(FetchError::new(format!(
                "`{}@{}` is not in the local store; run `yashy fetch` without `--offline` (or `yashy update`) first",
                entry.name, entry.version
            )));
        }
        let tarball = registry_root
            .join("packages")
            .join(format!("{hash}.tar"));
        let bytes = std::fs::read(&tarball).map_err(|error| {
            FetchError::new(format!(
                "cannot read registry tarball {}: {error}",
                tarball.display()
            ))
        })?;
        let _entries = read_tarball(&bytes).map_err(|error| {
            FetchError::new(format!("cannot read registry tarball: {error}"))
        })?;
        let staging = self.root.join(format!(
            ".staging-registry-{}-{:x}",
            std::process::id(),
            unique_suffix()
        ));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|error| {
            FetchError::new(format!(
                "cannot create the registry staging directory: {error}"
            ))
        })?;
        extract_tarball(&staging, &bytes).map_err(|error| {
            let _ = std::fs::remove_dir_all(&staging);
            FetchError::new(format!("cannot extract registry tarball: {error}"))
        })?;
        let extracted = yashy_manifest::hash_tree(&staging).map_err(|error| {
            let _ = std::fs::remove_dir_all(&staging);
            FetchError::new(format!(
                "cannot hash extracted registry package: {error}"
            ))
        })?;
        if extracted != entry.hash {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(FetchError::new(format!(
                "registry tarball for `{}@{}` does not match its tree hash ({extracted} != {})",
                entry.name, entry.version, entry.hash
            )));
        }
        make_read_only(&staging);
        std::fs::create_dir_all(directory.parent().expect("the store has a parent")).map_err(|error| {
            let _ = std::fs::remove_dir_all(&staging);
            FetchError::new(format!(
                "cannot create the registry store directory: {error}"
            ))
        })?;
        match std::fs::rename(&staging, &directory) {
            Ok(()) => Ok(directory),
            Err(_) if directory.is_dir() => {
                let _ = std::fs::remove_dir_all(&staging);
                Ok(directory)
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging);
                Err(FetchError::new(format!(
                    "cannot move the registry package into the store: {error}"
                )))
            }
        }
    }

    /// The checkout for `url` at `rev`, fetching it when allowed.
    ///
    /// A directory that exists is trusted: it was written only after a
    /// successful checkout, and its name contains the rev, so the tree cannot
    /// silently be a different commit.
    pub fn git_checkout(&self, url: &str, rev: &str, offline: bool) -> Result<PathBuf, FetchError> {
        let slug = url_slug(url);
        let directory = self.root.join("git").join(slug).join(safe_component(rev));
        if directory.is_dir() {
            return Ok(directory);
        }
        if offline {
            return Err(FetchError::new(format!(
                "`{url}` at `{rev}` is not in the local store; \
                 run `yashy fetch` without `--offline` (or `yashy update`) first"
            )));
        }
        if !git_available() {
            return Err(FetchError::new(format!(
                "cannot fetch `{url}`: `git` is not on PATH; install git or use a \
                 path dependency"
            )));
        }
        std::fs::create_dir_all(directory.parent().expect("the store has a parent")).map_err(
            |error| {
                FetchError::new(format!(
                    "cannot create the package store for `{url}`: {error}"
                ))
            },
        )?;
        // Stage the clone at a shallow path. Git writes deep paths inside
        // `.git/objects/pack/`, and putting the rev and a long unique suffix in
        // the staging name pushes them past the Windows limit before the clone
        // can finish. The final rename lands in the keyed directory, which is
        // only ever read back.
        let staging = self.root.join("git").join(format!(
            ".staging-{}-{:x}",
            std::process::id(),
            unique_suffix()
        ));
        let _ = std::fs::remove_dir_all(&staging);

        run_git(
            &[
                "clone",
                "--quiet",
                "--no-checkout",
                "--",
                url,
                &path_text(&staging),
            ],
            None,
            url,
            rev,
        )?;
        run_git(
            &[
                "-C",
                &path_text(&staging),
                "checkout",
                "--quiet",
                "--detach",
                rev,
            ],
            None,
            url,
            rev,
        )?;
        // The checkout is read-only once it is in the store: a build may read
        // it, nothing may edit it in place.
        make_read_only(&staging);
        match std::fs::rename(&staging, &directory) {
            Ok(()) => Ok(directory),
            // A concurrent fetcher won the race; its tree is the one keyed by
            // the same rev, so use it.
            Err(_) if directory.is_dir() => {
                let _ = std::fs::remove_dir_all(&staging);
                Ok(directory)
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging);
                Err(FetchError::new(format!(
                    "cannot move the checkout of `{url}` into the store: {error}"
                )))
            }
        }
    }
}

pub fn default_registry_root_for(store: &Path) -> PathBuf {
    if let Some(value) = env::var_os("YASHY_REGISTRY") {
        let text = value.to_string_lossy();
        if let Ok(path) = crate::registry::registry_path(&text) {
            return path;
        }
    }
    store.join("registry")
}

/// `%LOCALAPPDATA%/yashy/packages` and its siblings, with the two explicit
/// overrides first.
pub fn default_store_root() -> PathBuf {
    if let Some(dir) = env::var_os("YASHY_PACKAGE_STORE") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(dir) = env::var_os("YASHY_CACHE_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("packages");
        }
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("yashy").join("packages");
    }
    if let Some(xdg) = env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("yashy").join("packages");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("yashy")
            .join("packages");
    }
    env::temp_dir().join("yashy-packages")
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_git(args: &[&str], _cwd: Option<&Path>, url: &str, rev: &str) -> Result<(), FetchError> {
    let output = Command::new("git").args(args).output().map_err(|error| {
        FetchError::new(format!(
            "cannot run `git` to fetch `{url}` at `{rev}`: {error}"
        ))
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    // A rev that does not exist is the common mistake; name it rather than
    // dumping git's whole advice block.
    if detail.contains("did not match any file(s) known to git")
        || detail.contains("unknown revision")
        || detail.contains("not a valid object name")
    {
        return Err(FetchError::new(format!(
            "`{rev}` does not name a commit in `{url}`; fix the manifest's `rev`"
        )));
    }
    Err(FetchError::new(format!(
        "cannot fetch `{url}` at `{rev}`: {}",
        if detail.is_empty() {
            "git failed without a message"
        } else {
            detail
        }
    )))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

/// A filesystem-safe, collision-resistant directory for a URL.
fn url_slug(url: &str) -> String {
    let readable: String = url
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let digest = sha256_hex(url.as_bytes());
    format!("{readable}-{}", &digest[..16])
}

fn safe_component(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "rev".to_string()
    } else {
        cleaned
    }
}

fn make_read_only(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            make_read_only(&path);
        } else if file_type.is_file() {
            let mut permissions = match std::fs::metadata(&path) {
                Ok(metadata) => metadata.permissions(),
                Err(_) => continue,
            };
            permissions.set_readonly(true);
            let _ = std::fs::set_permissions(&path, permissions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_slug_is_readable_and_unique() {
        let a = url_slug("https://example.invalid/a");
        let b = url_slug("https://example.invalid/b");
        assert!(a.starts_with("https___example.invalid_a"), "{a}");
        assert_ne!(a, b);
        assert!(!a.contains('/'), "{a}");
    }

    #[test]
    fn a_rev_cannot_escape_the_store() {
        assert_eq!(safe_component("../../etc/passwd"), ".._.._etc_passwd");
    }

    #[test]
    fn an_offline_miss_names_the_fetch_command() {
        let store =
            Store::at(env::temp_dir().join(format!("yashy-fetch-test-{}", std::process::id())));
        let error = store
            .git_checkout("https://example.invalid/x", "abc", true)
            .expect_err("a cold store cannot satisfy an offline fetch");
        assert!(error.message.contains("yashy fetch"), "{error}");
        assert!(error.message.contains("--offline"), "{error}");
    }
}
