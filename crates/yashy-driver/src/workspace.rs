//! Workspace member discovery.
//!
//! One manifest is the workspace root: it has `[workspace]` and no package of
//! its own. Members are directories with a package manifest. The directory is
//! the identity; a member's package name must be unique so `-p` is unambigous.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use yashy_manifest::Manifest;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub root: PathBuf,
    pub members: Vec<String>,
    pub default_members: Vec<String>,
}

/// Load the workspace rooted at `root`, when there is one.
pub fn load(root: &Path) -> Result<Option<Workspace>, String> {
    let path = root.join("yashy.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let manifest = Manifest::load(&path).map_err(|error| {
        format!(
            "{}:{}:{}: {}",
            path.display(),
            error.line,
            error.column,
            error.message
        )
    })?;
    let Some(table) = manifest.workspace else {
        return Ok(None);
    };
    Ok(Some(Workspace {
        root: root.to_path_buf(),
        members: table.members,
        default_members: table.default_members,
    }))
}

impl Workspace {
    /// Resolve `-p <name>`, `--workspace` or the default selection to member
    /// directories in declaration order.
    pub fn select(
        &self,
        package: Option<&str>,
        all: bool,
        allowed_external: &[String],
    ) -> Result<Vec<PathBuf>, String> {
        self.validate(allowed_external)?;
        let wanted: Vec<String> = if all {
            self.members.clone()
        } else if let Some(name) = package {
            let found = self
                .members
                .iter()
                .find(|member| self.package_name(member).as_deref() == Some(name));
            match found {
                Some(member) => vec![member.clone()],
                None => return Err(format!("no workspace member named `{name}`")),
            }
        } else {
            self.default_members.clone()
        };
        wanted
            .iter()
            .map(|member| self.root.join(member))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|path| {
                if path.join("yashy.toml").is_file() {
                    Ok(path)
                } else {
                    Err(format!(
                        "workspace member {} has no yashy.toml",
                        path.display()
                    ))
                }
            })
            .collect()
    }

    fn package_name(&self, member: &str) -> Option<String> {
        let path = self.root.join(member).join("yashy.toml");
        Manifest::load(&path)
            .ok()
            .map(|manifest| manifest.package.name)
    }

    /// Unique member names, and no outside path dependency unless allowed.
    fn validate(&self, allowed_external: &[String]) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for member in &self.members {
            let path = self.root.join(member);
            let manifest_path = path.join("yashy.toml");
            let manifest = Manifest::load(&manifest_path).map_err(|error| {
                format!(
                    "{}:{}:{}: {}",
                    manifest_path.display(),
                    error.line,
                    error.column,
                    error.message
                )
            })?;
            if manifest.package.name.is_empty() {
                return Err(format!("workspace member `{member}` has no package name"));
            }
            if !names.insert(manifest.package.name.clone()) {
                return Err(format!(
                    "duplicate package name `{}` in the workspace",
                    manifest.package.name
                ));
            }
            for dependency in manifest
                .dependencies
                .values()
                .chain(manifest.dev_dependencies.values())
            {
                if let yashy_manifest::Dependency::Path { path: dependency } = dependency {
                    let resolved = crate::resolve::lexical_absolute(&path.join(dependency));
                    if !resolved.starts_with(&self.root)
                        && !allowed_external.iter().any(|allowed| allowed == dependency)
                    {
                        return Err(format!(
                            "member `{member}` depends on path `{dependency}` outside the workspace; allow it explicitly"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_without_a_manifest_is_none() {
        let dir = std::env::temp_dir().join(format!("yashy-workspace-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load(&dir).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
