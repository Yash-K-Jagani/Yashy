//! A read-only package audit.
//!
//! The audit is a thin composition of the checks that already exist: the
//! manifest parses, the lock agrees with it, and the resolved packages carry
//! a license and no build script. It never runs code from a dependency.

use std::path::{Path, PathBuf};

use yashy_common::codes;
use yashy_common::Json;
use yashy_manifest::{Lockfile, Manifest};

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
        let mut object = Json::object();
        object.set("schema_version", Json::number(1));
        object.set("ok", Json::bool(self.ok));
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

fn has_build_script(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "build.rs") {
            return true;
        }
        if path.is_dir() && path.file_name().is_some_and(|name| name != ".git") && has_build_script(&path) {
            return true;
        }
    }
    false
}

pub fn audit_project(directory: &Path, store: &Store) -> Result<AuditReport, AuditError> {
    let manifest = match Manifest::load(&directory.join("yashy.toml")) {
        Ok(manifest) => manifest,
        Err(error) => {
            return Ok(AuditReport {
                ok: false,
                packages: 0,
                source_bytes: 0,
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
    let mut findings = Vec::new();
    let mut report = AuditReport {
        ok: true,
        packages: 0,
        source_bytes: 0,
        findings: Vec::new(),
    };

    let lock = match Lockfile::load(&directory.join("yashy.lock")) {
        Ok(lock) => Some(lock),
        Err(error) if directory.join("yashy.lock").is_file() => {
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
    let mut packages: Vec<(String, PathBuf)> = vec![(
        manifest.package.name.clone(),
        directory.to_path_buf(),
    )];
    for package in &graph.packages {
        packages.push((package.name.clone(), package.root.clone()));
    }
    packages.sort();
    for (name, root) in packages {
        report.packages += 1;
        report.source_bytes = report
            .source_bytes
            .saturating_add(source_bytes(&root));
        match Manifest::load(&root.join("yashy.toml")) {
            Ok(package_manifest) => {
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
            }
            Err(error) => {
                findings.push(finding(
                    codes::MANIFEST_SYNTAX,
                    &name,
                    format!("yashy.toml:{}:{}: {}", error.line, error.column, error.message),
                ));
            }
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
