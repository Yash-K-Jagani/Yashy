//! The manifest: `yashy.toml`'s rules, on top of the TOML subset.
//!
//! The reader in [`crate::toml`] knows syntax. This module knows the manifest:
//! which tables and keys exist, that a version is an exact pin, and that a git
//! dependency names a commit. Every rejection carries [`ParseError`], so a
//! malformed manifest is `E1800` at a line and column, never a panic and never
//! a silently ignored typo.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::toml::{self, Document, Location, ParseError, Value};

/// A parsed `yashy.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub package: Package,
    pub dependencies: BTreeMap<String, Dependency>,
    pub dev_dependencies: BTreeMap<String, Dependency>,
    /// The `[workspace]` table, when this manifest is a workspace root.
    pub workspace: Option<Workspace>,
    /// Feature name -> enabled dependency names or other feature names.
    pub features: BTreeMap<String, Vec<String>>,
    /// Dependencies declared `optional = true`; they are only resolved when a
    /// feature enables them.
    pub optional_dependencies: BTreeSet<String>,
}

/// The `[workspace]` table. Members are paths relative to the root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    pub members: Vec<String>,
    pub default_members: Vec<String>,
}

/// The `[package]` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub edition: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
}

/// One entry in `[dependencies]` or `[dev-dependencies]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dependency {
    Path { path: String },
    Git { url: String, rev: String },
    Registry { version: String },
}

impl Dependency {
    /// The exact version this dependency pins, when it pins one by version.
    pub fn version(&self) -> Option<&str> {
        match self {
            Dependency::Registry { version } => Some(version),
            _ => None,
        }
    }

    /// One line naming the source, for tables and diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Dependency::Path { path } => format!("path {path}"),
            Dependency::Git { url, rev } => format!("git {url}@{rev}"),
            Dependency::Registry { version } => format!("registry {version}"),
        }
    }
}

impl Manifest {
    /// Parse manifest text.
    pub fn parse(text: &str) -> Result<Manifest, ParseError> {
        let document = toml::parse_document(text)?;
        manifest_from(&document)
    }

    /// Load and parse a `yashy.toml`.
    pub fn load(path: &Path) -> Result<Manifest, ParseError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Manifest::parse(&text),
            Err(error) => Err(ParseError {
                line: 1,
                column: 1,
                message: format!("cannot read {}: {error}", path.display()),
            }),
        }
    }
}

fn manifest_from(document: &Document) -> Result<Manifest, ParseError> {
    let root = &document.root;
    let error_at = |path: &str, message: String| -> ParseError {
        let Location { line, column } = document.location(path);
        ParseError {
            line,
            column,
            message,
        }
    };
    let error = |message: String| error_at("package", message);

    for key in root.keys() {
        if !matches!(
            key.as_str(),
            "package" | "dependencies" | "dev-dependencies" | "workspace" | "features"
        ) {
            return Err(error_at(
                key,
                format!(
                    "`{key}` is not a manifest table; expected `package`, `dependencies` \
                     or `dev-dependencies`"
                ),
            ));
        }
    }

    let package_table = match root.get("package") {
        Some(Value::Table(table)) => table,
        Some(_) => return Err(error("`package` must be a table".to_string())),
        None if root.get("workspace").is_some() => return parse_workspace_only(document),
        None => return Err(error("a manifest needs a `[package]` table".to_string())),
    };
    for key in package_table.keys() {
        if !matches!(
            key.as_str(),
            "name" | "version" | "edition" | "description" | "license" | "authors"
        ) {
            return Err(error_at(
                &format!("package.{key}"),
                format!(
                    "`{key}` is not a package field; expected name, version, edition, \
                     description, license or authors"
                ),
            ));
        }
    }

    let name = required_string(package_table, "name", document)?;
    if name.is_empty() {
        return Err(error_at(
            "package.name",
            "the package name cannot be empty".to_string(),
        ));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(error_at(
            "package.name",
            format!("`{name}` is not a package name; use letters, digits, `_` and `-`"),
        ));
    }
    let version = required_string(package_table, "version", document)?;
    check_exact_version(&version, document.location("package.version"))?;

    let dependencies = dependencies_table(root, document, "dependencies")?;
    let dev_dependencies = dependencies_table(root, document, "dev-dependencies")?;

    Ok(Manifest {
        package: Package {
            name,
            version,
            edition: optional_string(package_table, "edition", document)?,
            description: optional_string(package_table, "description", document)?,
            license: optional_string(package_table, "license", document)?,
        },
        dependencies,
        dev_dependencies,
        workspace: parse_workspace(root, document)?,
        features: parse_features(root, document)?,
        optional_dependencies: optional_dependencies(root),
    })
}

/// A workspace-root manifest has no package of its own; the synthetic empty
/// package keeps every existing caller total while `Workspace` carries the
/// real facts.
fn parse_workspace_only(document: &Document) -> Result<Manifest, ParseError> {
    let root = &document.root;
    let workspace = parse_workspace(root, document)?.ok_or_else(|| ParseError {
        line: 1,
        column: 1,
        message: "a workspace-root manifest needs a `[workspace]` table".to_string(),
    })?;
    Ok(Manifest {
        package: Package {
            name: String::new(),
            version: "0.0.0".to_string(),
            edition: None,
            description: None,
            license: None,
        },
        dependencies: BTreeMap::new(),
        dev_dependencies: BTreeMap::new(),
        workspace: Some(workspace),
        features: BTreeMap::new(),
        optional_dependencies: BTreeSet::new(),
    })
}

/// Parse `[features]`: each feature names dependencies or other features.
fn parse_features(
    root: &toml::Table,
    document: &Document,
) -> Result<BTreeMap<String, Vec<String>>, ParseError> {
    let Some(Value::Table(table)) = root.get("features") else {
        return Ok(BTreeMap::new());
    };
    let mut features = BTreeMap::new();
    for (name, value) in table {
        let Value::Array(items) = value else {
            let location = document.location(&format!("features.{name}"));
            return Err(ParseError {
                line: location.line,
                column: location.column,
                message: format!("feature `{name}` must be an array of strings"),
            });
        };
        let mut entries = Vec::new();
        for item in items {
            match item {
                Value::String(text) => entries.push(text.clone()),
                _ => {
                    let location = document.location(&format!("features.{name}"));
                    return Err(ParseError {
                        line: location.line,
                        column: location.column,
                        message: format!("feature `{name}` must be an array of strings"),
                    });
                }
            }
        }
        features.insert(name.clone(), entries);
    }
    Ok(features)
}

/// Dependencies declared `optional = true` in either dependency table.
fn optional_dependencies(root: &toml::Table) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for table_name in ["dependencies", "dev-dependencies"] {
        if let Some(Value::Table(table)) = root.get(table_name) {
            for (name, value) in table {
                if let Value::Table(fields) = value {
                    if let Some(Value::Bool(true)) = fields.get("optional") {
                        out.insert(name.clone());
                    }
                }
            }
        }
    }
    out
}
fn parse_workspace(
    root: &toml::Table,
    document: &Document,
) -> Result<Option<Workspace>, ParseError> {
    let Some(Value::Table(table)) = root.get("workspace") else {
        return Ok(None);
    };
    let mut workspace = Workspace::default();
    for key in table.keys() {
        if key != "members" && key != "default-members" {
            let location = document.location(&format!("workspace.{key}"));
            return Err(ParseError {
                line: location.line,
                column: location.column,
                message: format!(
                    "`{key}` is not a workspace field; expected `members` or `default-members`"
                ),
            });
        }
    }
    workspace.members = workspace_list(table, "members", document)?;
    workspace.default_members = workspace_list(table, "default-members", document)?;
    if workspace.members.is_empty() {
        let location = document.location("workspace.members");
        return Err(ParseError {
            line: location.line,
            column: location.column,
            message: "a workspace needs at least one member".to_string(),
        });
    }
    if workspace.default_members.is_empty() {
        workspace.default_members = workspace.members.clone();
    }
    for member in workspace
        .default_members
        .iter()
        .chain(workspace.members.iter())
    {
        if !workspace.members.contains(member) {
            return Err(ParseError {
                line: 1,
                column: 1,
                message: format!("default member `{member}` is not in `workspace.members`"),
            });
        }
    }
    Ok(Some(workspace))
}

fn workspace_list(
    table: &toml::Table,
    key: &str,
    document: &Document,
) -> Result<Vec<String>, ParseError> {
    let Some(value) = table.get(key) else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        let location = document.location(&format!("workspace.{key}"));
        return Err(ParseError {
            line: location.line,
            column: location.column,
            message: format!("`workspace.{key}` must be an array of strings"),
        });
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => Ok(text.clone()),
            _ => {
                let location = document.location(&format!("workspace.{key}"));
                Err(ParseError {
                    line: location.line,
                    column: location.column,
                    message: format!("`workspace.{key}` must be an array of strings"),
                })
            }
        })
        .collect()
}
fn required_string(
    table: &toml::Table,
    key: &str,
    document: &Document,
) -> Result<String, ParseError> {
    match table.get(key) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => {
            let Location { line, column } = document.location(&format!("package.{key}"));
            Err(ParseError {
                line,
                column,
                message: format!("`{key}` must be a string"),
            })
        }
        None => {
            let Location { line, column } = document.location("package");
            Err(ParseError {
                line,
                column,
                message: format!("the manifest is missing `{key}`"),
            })
        }
    }
}

fn optional_string(
    table: &toml::Table,
    key: &str,
    document: &Document,
) -> Result<Option<String>, ParseError> {
    match table.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => {
            let Location { line, column } = document.location(&format!("package.{key}"));
            Err(ParseError {
                line,
                column,
                message: format!("`{key}` must be a string"),
            })
        }
    }
}

/// An exact version is `major.minor.patch`, each component digits.
/// A dependency version may be an exact pin or a SemVer range.
pub fn check_version_req(version: &str, location: Location) -> Result<(), ParseError> {
    if crate::VersionReq::parse(version).is_some() {
        return Ok(());
    }
    Err(ParseError {
        line: location.line,
        column: location.column,
        message: format!(
            "`{version}` is not a version requirement; write `1.2.3`, `^1.2` or `~1.2.0`"
        ),
    })
}

pub fn check_exact_version(version: &str, location: Location) -> Result<(), ParseError> {
    if version.contains(['^', '~', '*', '<', '>', '=']) || version.is_empty() {
        return Err(ParseError {
            line: location.line,
            column: location.column,
            message: format!(
                "`{version}` is not an exact version; write `1.2.3` (a pin has no `^`, `~` \
                 or range)"
            ),
        });
    }
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()))
    {
        return Err(ParseError {
            line: location.line,
            column: location.column,
            message: format!("`{version}` is not an exact version; write `major.minor.patch`"),
        });
    }
    Ok(())
}

fn dependencies_table(
    root: &toml::Table,
    document: &Document,
    table_name: &str,
) -> Result<BTreeMap<String, Dependency>, ParseError> {
    let Some(entry) = root.get(table_name) else {
        return Ok(BTreeMap::new());
    };
    let Value::Table(table) = entry else {
        let Location { line, column } = document.location(table_name);
        return Err(ParseError {
            line,
            column,
            message: format!("`{table_name}` must be a table"),
        });
    };
    let mut dependencies = BTreeMap::new();
    for (name, value) in table {
        let path = format!("{table_name}.{name}");
        let dependency = match value {
            // `name = "1.2.3"` is the registry shorthand.
            Value::String(version) => {
                check_exact_version(version, document.location(&path))?;
                Dependency::Registry {
                    version: version.clone(),
                }
            }
            Value::Table(fields) => dependency_from_table(name, &path, fields, document)?,
            _ => {
                let Location { line, column } = document.location(&path);
                return Err(ParseError {
                    line,
                    column,
                    message: format!(
                        "dependency `{name}` must be a version string or a table with \
                         `path`, `git`+`rev`, or `version`"
                    ),
                });
            }
        };
        dependencies.insert(name.clone(), dependency);
    }
    Ok(dependencies)
}

fn dependency_from_table(
    name: &str,
    path: &str,
    fields: &toml::Table,
    document: &Document,
) -> Result<Dependency, ParseError> {
    for key in fields.keys() {
        if !matches!(
            key.as_str(),
            "path" | "git" | "rev" | "version" | "optional"
        ) {
            let Location { line, column } = document.location(&format!("{path}.{key}"));
            return Err(ParseError {
                line,
                column,
                message: format!(
                    "`{key}` is not a dependency field; expected path, git, rev, version or optional"
                ),
            });
        }
    }
    let at = |field: &str| document.location(&format!("{path}.{field}"));
    let text = |field: &str| -> Option<&str> { fields.get(field).and_then(Value::as_str) };
    let sources = ["path", "git", "version"]
        .iter()
        .filter(|field| fields.contains_key(**field))
        .count();
    if let Some(rev) = fields.get("rev") {
        if fields.get("git").is_none() {
            let Location { line, column } = at("rev");
            return Err(ParseError {
                line,
                column,
                message: "`rev` only means something on a `git` dependency".to_string(),
            });
        }
        if !matches!(rev, Value::String(_)) {
            let Location { line, column } = at("rev");
            return Err(ParseError {
                line,
                column,
                message: "`rev` must be a commit string".to_string(),
            });
        }
    }
    if sources != 1 {
        let Location { line, column } = document.location(path);
        return Err(ParseError {
            line,
            column,
            message: format!(
                "dependency `{name}` must declare exactly one source: `path`, `git`+`rev`, \
                 or `version`"
            ),
        });
    }
    if let Some(rev) = fields.get("rev") {
        if fields.get("git").is_none() {
            let Location { line, column } = at("rev");
            return Err(ParseError {
                line,
                column,
                message: "`rev` only means something on a `git` dependency".to_string(),
            });
        }
        if !matches!(rev, Value::String(_)) {
            let Location { line, column } = at("rev");
            return Err(ParseError {
                line,
                column,
                message: "`rev` must be a commit string".to_string(),
            });
        }
    }
    if fields.contains_key("git") {
        let Some(url) = text("git") else {
            let Location { line, column } = at("git");
            return Err(ParseError {
                line,
                column,
                message: "`git` must be a URL string".to_string(),
            });
        };
        if url.is_empty() {
            let Location { line, column } = at("git");
            return Err(ParseError {
                line,
                column,
                message: "a git URL cannot be empty".to_string(),
            });
        }
        let Some(rev) = text("rev") else {
            let Location { line, column } = document.location(path);
            return Err(ParseError {
                line,
                column,
                message: format!(
                    "a git dependency is pinned to a commit: `{name}` needs `rev = \"<sha>\"`"
                ),
            });
        };
        if rev.is_empty() {
            let Location { line, column } = at("rev");
            return Err(ParseError {
                line,
                column,
                message: "a git `rev` cannot be empty".to_string(),
            });
        }
        return Ok(Dependency::Git {
            url: url.to_string(),
            rev: rev.to_string(),
        });
    }
    if let Some(version) = text("version") {
        check_version_req(version, at("version"))?;
        return Ok(Dependency::Registry {
            version: version.to_string(),
        });
    }
    let Some(path_field) = text("path") else {
        let Location { line, column } = at("path");
        return Err(ParseError {
            line,
            column,
            message: "`path` must be a string".to_string(),
        });
    };
    if path_field.is_empty() {
        let Location { line, column } = at("path");
        return Err(ParseError {
            line,
            column,
            message: "a dependency path cannot be empty".to_string(),
        });
    }
    Ok(Dependency::Path {
        path: path_field.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[package]
name = "demo"
version = "1.2.3"
license = "MIT"

[dependencies]
http = "0.3.1"

[dependencies.net]
path = "../net"

[dependencies.extra]
git = "https://example.invalid/extra"
rev = "0123456789abcdef"

[dev-dependencies]
assert = { }
"#;

    #[test]
    fn the_sample_maps_to_the_struct() {
        // The inline table in the sample is outside the subset on purpose:
        // the test proves the rejection, then the rest of the sample parses.
        let error = Manifest::parse(SAMPLE).expect_err("inline tables are refused");
        assert!(error.message.contains("not a value in the subset"));

        let text = SAMPLE.replace("assert = { }", "assert = \"1.0.0\"");
        let manifest = Manifest::parse(&text).expect("the sample parses");
        assert_eq!(manifest.package.name, "demo");
        assert_eq!(manifest.package.version, "1.2.3");
        assert_eq!(manifest.package.license.as_deref(), Some("MIT"));
        assert_eq!(
            manifest.dependencies["http"],
            Dependency::Registry {
                version: "0.3.1".to_string()
            }
        );
        assert_eq!(
            manifest.dependencies["net"],
            Dependency::Path {
                path: "../net".to_string()
            }
        );
        assert!(matches!(
            manifest.dependencies["extra"],
            Dependency::Git { .. }
        ));
        assert_eq!(
            manifest.dev_dependencies["assert"],
            Dependency::Registry {
                version: "1.0.0".to_string()
            }
        );
    }

    #[test]
    fn a_range_is_refused_with_the_exact_spelling() {
        let text =
            "[package]\nname = \"x\"\nversion = \"1.2.3\"\n[dependencies]\nhttp = \"^1.0.0\"\n";
        let error = Manifest::parse(text).expect_err("a range");
        assert!(error.message.contains("exact version"), "{}", error.message);
        assert_eq!(error.line, 5);
    }

    #[test]
    fn a_git_dependency_without_a_rev_says_it_is_pinned_to_a_commit() {
        let text = "[package]\nname = \"x\"\nversion = \"1.2.3\"\n[dependencies.a]\ngit = \"https://example.invalid/a\"\n";
        let error = Manifest::parse(text).expect_err("no rev");
        assert!(error.message.contains("pinned to a commit"), "{error}");
        assert!(error.message.contains('a'));
    }

    #[test]
    fn a_missing_name_names_the_key() {
        let error = Manifest::parse("[package]\nversion = \"1.2.3\"\n").expect_err("no name");
        assert!(error.message.contains("`name`"), "{error}");
    }

    #[test]
    fn a_misspelled_table_is_not_silently_ignored() {
        let error =
            Manifest::parse("[package]\nname = \"x\"\nversion = \"1.0.0\"\n[dependecies]\n")
                .unwrap_err();
        assert!(error.message.contains("dependecies"), "{error}");
        assert_eq!(error.line, 4);
    }
}
