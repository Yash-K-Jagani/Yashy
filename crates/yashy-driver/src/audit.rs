//! A read-only package audit.
//!
//! The audit is a thin composition of the checks that already exist: the
//! manifest parses, the lock agrees with it, and the resolved packages carry
//! a license and no build script. It never runs code from a dependency.

use std::path::{Path, PathBuf};

use yashy_common::codes;
use yashy_common::Json;
use yashy_manifest::{CapabilitySet, Lockfile, Manifest};

use crate::resolve::resolve;
use crate::Store;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditFinding {
    pub code: String,
    pub package: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditReport {
    pub ok: bool,
    pub packages: usize,
    pub source_bytes: u64,
    /// The sorted union of every package's `[capabilities]` names.
    ///
    /// The root is included, because the root's own grants are part of what
    /// runs. This is the set `audit --capabilities` prints: a dependency that
    /// asks for `net` is visible before anything executes, not only when the
    /// linker or a sandbox later refuses it.
    pub capabilities: Vec<String>,
    pub findings: Vec<AuditFinding>,
}

impl AuditReport {
    pub fn to_json(&self) -> String {
        let mut findings = Vec::new();
        for finding in &self.findings {
            let mut item = Json::object();
            item.set("code", Json::string(&finding.code));
            item.set("package", Json::string(&finding.package));
            item.set("detail", Json::string(&finding.detail));
            findings.push(item);
        }
        let capabilities = self
            .capabilities
            .iter()
            .map(|name| Json::string(name))
            .collect();
        let mut object = Json::object();
        object.set("schema_version", Json::number(1));
        object.set("ok", Json::bool(self.ok));
        object.set("capabilities", Json::array(capabilities));
        object.set("findings", Json::array(findings));
        object.render()
    }
}

#[derive(Debug)]
pub enum AuditError {
    Message(String),
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditError::Message(message) => write!(formatter, "{message}"),
        }
    }
}

fn finding(code: &str, package: &str, detail: impl Into<String>) -> AuditFinding {
    AuditFinding {
        code: code.to_string(),
        package: package.to_string(),
        detail: detail.into(),
    }
}

/// The shortest dependency path from the root to `name`, for audit findings.
fn dependency_path(root: &str, graph: &crate::resolve::ResolvedGraph, name: &str) -> String {
    use std::collections::VecDeque;
    let mut queue = VecDeque::from([vec![root.to_string()]]);
    while let Some(path) = queue.pop_front() {
        let current = path.last().cloned().unwrap_or_default();
        if current == name {
            return path.join(" -> ");
        }
        if let Some(package) = graph.packages.iter().find(|p| p.name == current) {
            for child in &package.dependencies {
                let child = child.split_whitespace().next().unwrap_or(child);
                if !path.iter().any(|entry| entry == child) {
                    let mut next = path.clone();
                    next.push(child.to_string());
                    queue.push_back(next);
                }
            }
        }
    }
    name.to_string()
}

fn source_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    source_bytes_into(root, &mut total);
    total
}

fn source_bytes_into(root: &Path, total: &mut u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if metadata.is_file() {
                *total = total.saturating_add(metadata.len());
            } else if metadata.is_dir() {
                source_bytes_into(&path, total);
            }
        }
    }
}

/// The sorted capability names every package declares, root included.
///
/// `yashy_manifest::CapabilitySet::union` keeps each name's patterns, so the
/// union answers "what may this program touch?", not "which package asked for
/// what" — the findings already carry the dependency path for that question.
fn capability_names(sets: &[CapabilitySet]) -> Vec<String> {
    let union = crate::policy::transitive(sets);
    let mut names: Vec<String> = union
        .entries()
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn has_build_script(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "build.rs") {
            return true;
        }
        if path.is_dir()
            && path.file_name().is_some_and(|name| name != ".git")
            && has_build_script(&path)
        {
            return true;
        }
    }
    false
}

pub fn audit_project(directory: &Path, store: &Store) -> Result<AuditReport, AuditError> {
    audit_project_with_feed(directory, store, None)
}

/// A read-only audit that also checks the resolved packages against a signed
/// advisory feed. The feed is matched against the version in each package's
/// own manifest, including the root, so an advisory cannot be dodged by a
/// lockfile that names a different range.
pub fn audit_project_with_feed(
    directory: &Path,
    store: &Store,
    feed: Option<&crate::advisory::Feed>,
) -> Result<AuditReport, AuditError> {
    let manifest = match Manifest::load(&directory.join("yashy.toml")) {
        Ok(manifest) => manifest,
        Err(error) => {
            return Ok(AuditReport {
                ok: false,
                packages: 0,
                source_bytes: 0,
                capabilities: Vec::new(),
                findings: vec![finding(
                    codes::MANIFEST_SYNTAX,
                    "",
                    format!(
                        "yashy.toml:{}:{}: {}",
                        error.line, error.column, error.message
                    ),
                )],
            });
        }
    };
    let audit_policy = manifest.audit.clone();
    let mut findings = Vec::new();
    let mut report = AuditReport {
        ok: true,
        packages: 0,
        source_bytes: 0,
        capabilities: Vec::new(),
        findings: Vec::new(),
    };

    let lock_path = crate::workspace::Workspace::shared_file(directory, "yashy.lock");
    let lock = match Lockfile::load(&lock_path) {
        Ok(lock) => Some(lock),
        Err(error) if lock_path.is_file() => {
            findings.push(finding(
                codes::LOCKFILE_MISMATCH,
                &manifest.package.name,
                format!(
                    "yashy.lock:{}:{}: {}",
                    error.line, error.column, error.message
                ),
            ));
            None
        }
        Err(_) => None,
    };
    if let Some(lock) = &lock {
        if let Err(error) = lock.check_manifest(&manifest, directory) {
            findings.push(finding(
                codes::LOCKFILE_MISMATCH,
                &manifest.package.name,
                error.message,
            ));
        }
    }

    let graph = match resolve(&manifest, lock.as_ref(), directory, true, store) {
        Ok(graph) => graph,
        Err(error) if error.code == codes::LOCKFILE_MISMATCH => {
            findings.push(finding(
                codes::LOCKFILE_MISMATCH,
                &manifest.package.name,
                error.message,
            ));
            resolve(&manifest, None, directory, true, store).map_err(|error| {
                AuditError::Message(format!("{}: {}", error.code, error.message))
            })?
        }
        Err(error) => {
            return Err(AuditError::Message(format!(
                "{}: {}",
                error.code, error.message
            )));
        }
    };
    let mut packages: Vec<(String, PathBuf)> =
        vec![(manifest.package.name.clone(), directory.to_path_buf())];
    for package in &graph.packages {
        packages.push((package.name.clone(), package.root.clone()));
    }
    let mut declared: Vec<CapabilitySet> = Vec::new();
    packages.sort();
    for (name, root) in packages {
        report.packages += 1;
        report.source_bytes = report.source_bytes.saturating_add(source_bytes(&root));
        match Manifest::load(&root.join("yashy.toml")) {
            Ok(package_manifest) => {
                declared.push(package_manifest.capabilities.clone().unwrap_or_default());
                if let Some(feed) = feed {
                    match feed.matches(&name, &package_manifest.package.version) {
                        Ok(matches) => {
                            for advisory in matches {
                                findings.push(finding(
                                    codes::AUDIT_FAILURE,
                                    &name,
                                    format!(
                                        "advisory {} affects {} ({}); fixed in {}",
                                        advisory.id, name, advisory.range, advisory.fixed
                                    ),
                                ));
                            }
                        }
                        Err(error) => findings.push(finding(
                            codes::AUDIT_FAILURE,
                            &name,
                            format!("advisory range: {error}"),
                        )),
                    }
                }
                if package_manifest.package.license.is_none() {
                    findings.push(finding(
                        codes::AUDIT_FAILURE,
                        &name,
                        "package has no license",
                    ));
                }
                if has_build_script(&root) {
                    findings.push(finding(
                        codes::AUDIT_FAILURE,
                        &name,
                        "package contains build.rs",
                    ));
                }
                if let Some(policy) = &audit_policy {
                    if !policy.allowed_licenses.is_empty() {
                        let license = package_manifest.package.license.as_deref().unwrap_or("");
                        if !policy
                            .allowed_licenses
                            .iter()
                            .any(|allowed| allowed == license)
                        {
                            findings.push(finding(
                                codes::POLICY_DENIED,
                                &name,
                                format!("license `{license}` is not in the audit allowlist"),
                            ));
                        }
                    }
                    if !policy.allowed_packages.is_empty()
                        && &name != &manifest.package.name
                        && !policy.allowed_packages.contains(&name)
                    {
                        findings.push(finding(
                            codes::POLICY_DENIED,
                            &name,
                            "package is not in the audit allowlist",
                        ));
                    }
                }
            }
            Err(error) => {
                findings.push(finding(
                    codes::MANIFEST_SYNTAX,
                    &name,
                    format!(
                        "yashy.toml:{}:{}: {}",
                        error.line, error.column, error.message
                    ),
                ));
            }
        }
    }

    report.capabilities = capability_names(&declared);

    if let Some(policy) = &audit_policy {
        for package in &graph.packages {
            let path = dependency_path(&manifest.package.name, &graph, &package.name);
            let child = package.root.join("yashy.toml");
            let child_manifest = Manifest::load(&child).ok();
            if let Some(child_manifest) = &child_manifest {
                if let Some(capabilities) = &child_manifest.capabilities {
                    for (name, _) in capabilities.entries() {
                        if policy.deny_capabilities.iter().any(|denied| denied == name)
                            || (!policy.allow_capabilities.is_empty()
                                && !policy
                                    .allow_capabilities
                                    .iter()
                                    .any(|allowed| allowed == name))
                        {
                            findings.push(finding(
                                codes::POLICY_DENIED,
                                &path,
                                format!("capability `{name}` is outside the policy"),
                            ));
                        }
                    }
                }
                if let Some(license) = &child_manifest.package.license {
                    if policy.deny_licenses.iter().any(|denied| denied == license) {
                        findings.push(finding(
                            codes::POLICY_DENIED,
                            &path,
                            format!("license `{license}` is denied by the policy"),
                        ));
                    }
                }
            }
            if let Some(registry) = package.source.strip_prefix("registry+") {
                if policy
                    .deny_registries
                    .iter()
                    .any(|denied| registry.contains(denied))
                {
                    findings.push(finding(
                        codes::POLICY_DENIED,
                        &path,
                        format!("registry `{registry}` is denied by the policy"),
                    ));
                }
            }
            if policy.require_provenance
                && !package.root.join("main.ya.attestation.json").is_file()
                && !directory.join("yashy.attestation.json").is_file()
            {
                findings.push(finding(
                    codes::SBOM_MISMATCH,
                    &path,
                    "the policy requires provenance, but no attestation was found",
                ));
            }
        }
        if policy.max_transitive > 0 && graph.packages.len() > policy.max_transitive {
            findings.push(finding(
                codes::POLICY_DENIED,
                &manifest.package.name,
                format!(
                    "{} dependencies exceed the policy cap of {}",
                    graph.packages.len(),
                    policy.max_transitive
                ),
            ));
        }
    }

    if audit_policy
        .as_ref()
        .is_some_and(|policy| policy.require_signatures)
    {
        let has_keys = crate::registry::load_keyring()
            .map(|keys| !keys.is_empty())
            .unwrap_or(false);
        if !has_keys {
            findings.push(finding(
                codes::SIGNATURE_INVALID,
                &manifest.package.name,
                "the audit policy requires signatures, but no trusted key is configured",
            ));
        }
    }
    findings.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.package.cmp(&right.package))
            .then_with(|| left.detail.cmp(&right.detail))
    });
    report.findings = findings;
    report.ok = report.findings.is_empty();
    Ok(report)
}
