//! Capability policy: the manifest grants, the policy is the ceiling.
//!
//! A package that declares nothing gets nothing. A workspace policy narrows
//! the grant further; it can never add a capability a package did not ask for.

use yashy_manifest::{CapabilitySet, Manifest};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedCapabilities {
    pub granted: CapabilitySet,
    pub denied: Vec<String>,
}

/// Resolve the capabilities of one package against an optional allowlist.
pub fn resolve(manifest: &Manifest, allow: &[String]) -> ResolvedCapabilities {
    let declared = manifest.capabilities.clone().unwrap_or_default();
    let granted = declared.restrict(allow);
    let mut denied = Vec::new();
    for (name, _) in declared.entries() {
        if !granted.has(name) {
            denied.push(name.to_string());
        }
    }
    denied.sort();
    ResolvedCapabilities { granted, denied }
}

/// The transitive union across a root and its resolved packages.
pub fn transitive(sets: &[CapabilitySet]) -> CapabilitySet {
    CapabilitySet::union(sets)
}

/// Refuse a capability that the root policy does not name.
pub fn check_request(requested: &str, allow: &[String]) -> Result<(), String> {
    if !yashy_manifest::capability::NAMES.contains(&requested) {
        return Err(format!("unknown capability `{requested}`"));
    }
    if !allow.is_empty() && !allow.iter().any(|name| name == requested) {
        return Err(format!("policy denies capability `{requested}`"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_declaration_grants_nothing() {
        let manifest = Manifest::parse("[package]\nname = \"app\"\nversion = \"0.1.0\"\n").unwrap();
        let resolved = resolve(&manifest, &[]);
        assert!(!resolved.granted.has("net"));
        assert!(resolved.denied.is_empty());
    }

    #[test]
    fn a_policy_can_narrow_a_grant() {
        let manifest = Manifest::parse(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
             [capabilities]\nnet = []\n\"fs.read\" = [\"data/*\"]\n",
        )
        .unwrap();
        let resolved = resolve(&manifest, &["net".to_string()]);
        assert!(resolved.granted.has("net"));
        assert!(!resolved.granted.has("fs.read"));
        assert_eq!(resolved.denied, vec!["fs.read".to_string()]);
    }

    #[test]
    fn a_request_outside_the_policy_is_denied() {
        assert!(check_request("net", &["net".to_string()]).is_ok());
        assert!(check_request("net", &[]).is_ok());
        assert!(check_request("net", &["fs.read".to_string()]).is_err());
        assert!(check_request("telepathy", &[]).is_err());
    }
}
