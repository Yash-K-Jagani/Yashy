//! An SPDX-2.3 SBOM for a package or the compiler itself.
//!
//! The document is a sorted, deterministic JSON value so a reviewer can diff
//! two builds. It records what a package is, what it depends on and the hashes
//! that let a verifier rebuild it.

use yashy_common::Json;

use crate::lock::{hash_tree, sha256_hex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SbomPackage {
    pub name: String,
    pub version: String,
    pub hash: String,
    pub license: String,
    pub supplier: String,
    pub dependencies: Vec<String>,
}

fn spdx_package(package: &SbomPackage) -> Json {
    let mut value = Json::object();
    value.set("SPDXID", Json::string(format!("SPDXRef-{}", package.name)));
    value.set("name", Json::string(&package.name));
    value.set("versionInfo", Json::string(&package.version));
    value.set("downloadLocation", Json::string("NOASSERTION"));
    value.set("filesAnalyzed", Json::bool(false));
    value.set("licenseConcluded", Json::string(&package.license));
    value.set("licenseDeclared", Json::string(&package.license));
    value.set("supplier", Json::string(&package.supplier));
    value.set(
        "checksums",
        Json::array(vec![{
            let mut checksum = Json::object();
            checksum.set("algorithm", Json::string("SHA256"));
            checksum.set(
                "checksumValue",
                Json::string(package.hash.trim_start_matches("sha256:")),
            );
            checksum
        }]),
    );
    value
}

/// Render SPDX JSON for the given packages, with optional build inputs.
pub fn spdx_json(packages: &[SbomPackage]) -> Json {
    spdx_json_with_metadata(packages, &std::collections::BTreeMap::new())
}

/// The same document with named build inputs, such as the compiler version and
/// target triple. A verifier can recompute these from the release.
pub fn spdx_json_with_metadata(
    packages: &[SbomPackage],
    metadata: &std::collections::BTreeMap<String, String>,
) -> Json {
    let mut sorted = packages.to_vec();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    let mut document = Json::object();
    document.set("spdxVersion", Json::string("SPDX-2.3"));
    document.set("dataLicense", Json::string("CC0-1.0"));
    document.set("SPDXID", Json::string("SPDXRef-DOCUMENT"));
    document.set("name", Json::string("yashy-package"));
    document.set(
        "documentNamespace",
        Json::string("https://yashy.dev/spdx/package"),
    );
    if !metadata.is_empty() {
        let mut annotations = Vec::new();
        for (name, value) in metadata {
            let mut annotation = Json::object();
            annotation.set("annotationDate", Json::string("1970-01-01T00:00:00Z"));
            annotation.set("annotator", Json::string("Tool: yashyc"));
            annotation.set("comment", Json::string(format!("{name}={value}")));
            annotations.push(annotation);
        }
        document.set("annotations", Json::array(annotations));
    }
    document.set(
        "packages",
        Json::array(sorted.iter().map(spdx_package).collect()),
    );
    let mut relationships = Vec::new();
    for package in &sorted {
        for dependency in &package.dependencies {
            let mut relationship = Json::object();
            relationship.set(
                "spdxElementId",
                Json::string(format!("SPDXRef-{}", package.name)),
            );
            relationship.set("relationshipType", Json::string("DEPENDS_ON"));
            relationship.set(
                "relatedSpdxElement",
                Json::string(format!("SPDXRef-{dependency}")),
            );
            relationships.push(relationship);
        }
    }
    document.set("relationships", Json::array(relationships));
    document
}

/// Hash a package directory into an SBOM entry, with metadata from its manifest.
pub fn from_directory(path: &std::path::Path) -> Result<SbomPackage, String> {
    let manifest_path = path.join("yashy.toml");
    let manifest = crate::Manifest::load(&manifest_path).map_err(|error| {
        format!(
            "{}:{}: {}",
            manifest_path.display(),
            error.line,
            error.message
        )
    })?;
    let hash =
        hash_tree(path).map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
    let dependencies = manifest
        .dependencies
        .keys()
        .chain(manifest.dev_dependencies.keys())
        .cloned()
        .collect::<Vec<_>>();
    Ok(SbomPackage {
        name: manifest.package.name,
        version: manifest.package.version,
        hash,
        license: manifest
            .package
            .license
            .unwrap_or_else(|| "NOASSERTION".to_string()),
        supplier: "NOASSERTION".to_string(),
        dependencies,
    })
}

/// The compiler itself is one SBOM package, keyed by its binary bytes.
pub fn compiler(exe: &[u8], version: &str) -> SbomPackage {
    SbomPackage {
        name: "yashyc".to_string(),
        version: version.to_string(),
        hash: sha256_hex(exe),
        license: "MIT".to_string(),
        supplier: "Yashy".to_string(),
        dependencies: Vec::new(),
    }
}

/// Vendored third-party data, keyed by its exact bytes.
///
/// The Unicode Character Database files are pinned and checked in so builds
/// are offline. Their hash and version travel in the SBOM, making the pin
/// auditable rather than a comment in a generated header.
pub fn vendored_data(name: &str, version: &str, bytes: &[u8], license: &str) -> SbomPackage {
    SbomPackage {
        name: name.to_string(),
        version: version.to_string(),
        hash: sha256_hex(bytes),
        license: license.to_string(),
        supplier: "Unicode, Inc.".to_string(),
        dependencies: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spdx_is_sorted_and_names_every_package() {
        let packages = vec![
            SbomPackage {
                name: "z".to_string(),
                version: "1.0.0".to_string(),
                hash: "sha256:abc".to_string(),
                license: "MIT".to_string(),
                supplier: "Yashy".to_string(),
                dependencies: vec!["a".to_string()],
            },
            SbomPackage {
                name: "a".to_string(),
                version: "2.0.0".to_string(),
                hash: "sha256:def".to_string(),
                license: "MIT".to_string(),
                supplier: "Yashy".to_string(),
                dependencies: Vec::new(),
            },
        ];
        let json = spdx_json(&packages).render();
        assert!(json.contains("SPDX-2.3"), "{json}");
        let first_a = json.find("SPDXRef-a").unwrap();
        let first_z = json.find("SPDXRef-z").unwrap();
        assert!(first_a < first_z, "{json}");
        assert!(json.contains("DEPENDS_ON"), "{json}");
    }
}
