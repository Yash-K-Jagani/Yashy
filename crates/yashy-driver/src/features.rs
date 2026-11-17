//! Feature selection and unification.
//!
//! A feature is a name in `[features]`; enabling it activates the dependencies
//! and other features it names. The selected set is sorted so it can be used
//! as a cache key, recorded in the lockfile and printed by `--print-cfg`.

use std::collections::BTreeSet;

use yashy_manifest::Manifest;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Features {
    pub enabled: BTreeSet<String>,
}

/// Select features for one manifest.
///
/// `requested` comes from `--features`; a name outside `[features]` is a
/// diagnostic rather than a silently false condition.
pub fn select(
    manifest: &Manifest,
    requested: &[String],
    no_default_features: bool,
) -> Result<Features, String> {
    let mut enabled = BTreeSet::new();
    if !no_default_features {
        if let Some(default) = manifest.features.get("default") {
            expand(manifest, default, &mut enabled)?;
        }
    }
    for name in requested {
        if !manifest.features.contains_key(name) {
            return Err(format!(
                "unknown feature `{name}`; expected one of: {}",
                manifest
                    .features
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        expand(manifest, &[name.clone()], &mut enabled)?;
    }
    Ok(Features { enabled })
}

fn expand(
    manifest: &Manifest,
    names: &[String],
    enabled: &mut BTreeSet<String>,
) -> Result<(), String> {
    for name in names {
        if !enabled.insert(name.clone()) {
            continue;
        }
        if let Some(next) = manifest.features.get(name) {
            expand(manifest, next, enabled)?;
        }
    }
    Ok(())
}

/// Members requesting different features unify to their union.
pub fn unify(sets: &[Features]) -> Features {
    let mut enabled = BTreeSet::new();
    for set in sets {
        enabled.extend(set.enabled.iter().cloned());
    }
    Features { enabled }
}

/// The dependencies that should be resolved for a selected feature set.
pub fn active_dependencies<'a>(
    manifest: &'a Manifest,
    features: &Features,
) -> Vec<(&'a String, &'a yashy_manifest::Dependency)> {
    manifest
        .dependencies
        .iter()
        .filter(|(name, _)| {
            !manifest.optional_dependencies.contains(*name) || features.enabled.contains(*name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::parse(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
             [features]\ndefault = [\"std\"]\nstd = []\ntls = [\"net\"]\n\n\
             [dependencies.net]\nversion = \"1.0.0\"\noptional = true\n",
        )
        .expect("feature manifest")
    }

    #[test]
    fn default_features_are_enabled_and_can_be_disabled() {
        let manifest = manifest();
        let default = select(&manifest, &[], false).unwrap();
        assert!(default.enabled.contains("std"));
        let none = select(&manifest, &[], true).unwrap();
        assert!(none.enabled.is_empty());
    }

    #[test]
    fn an_optional_dependency_is_active_only_with_its_feature() {
        let manifest = manifest();
        let off = select(&manifest, &[], true).unwrap();
        assert!(active_dependencies(&manifest, &off).is_empty());
        let on = select(&manifest, &["tls".to_string()], true).unwrap();
        let names: Vec<&str> = active_dependencies(&manifest, &on)
            .into_iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, vec!["net"]);
    }

    #[test]
    fn unknown_features_are_diagnostics() {
        let manifest = manifest();
        assert!(select(&manifest, &["nope".to_string()], false).is_err());
    }

    #[test]
    fn two_members_unify_to_the_union() {
        let left = Features {
            enabled: BTreeSet::from(["a".to_string()]),
        };
        let right = Features {
            enabled: BTreeSet::from(["b".to_string()]),
        };
        assert_eq!(unify(&[left, right]).enabled.len(), 2);
    }
}
