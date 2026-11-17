//! Capability declarations and the names the whole toolchain shares.
//!
//! A capability is a permission a package declares in its manifest. A missing
//! declaration means no permission, never full access. The same names are used
//! by the checker's call-site rule, `audit`, the sandbox and the trace
//! redactor, so a capability cannot mean one thing in one layer and another in
//! the next.

use std::collections::BTreeMap;

/// The closed set of capabilities this compiler understands.
pub const NAMES: &[&str] = &[
    "fs.read", "fs.write", "net", "env", "clock", "proc", "rng", "ffi",
];

/// A package's declared permissions. Each value is a list of patterns; an
/// empty list means the capability is not granted at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    grants: BTreeMap<String, Vec<String>>,
}

impl CapabilitySet {
    /// Build a set from `name -> patterns`, refusing unknown names.
    pub fn from_map(values: &BTreeMap<String, Vec<String>>) -> Result<CapabilitySet, String> {
        let mut grants = BTreeMap::new();
        for (name, patterns) in values {
            if !NAMES.contains(&name.as_str()) {
                return Err(format!(
                    "unknown capability `{name}`; expected one of: {}",
                    NAMES.join(", ")
                ));
            }
            if !patterns.is_empty() {
                grants.insert(name.clone(), patterns.clone());
            }
        }
        Ok(CapabilitySet { grants })
    }

    /// Whether `name` is granted at all.
    pub fn has(&self, name: &str) -> bool {
        self.grants.contains_key(name)
    }

    /// Whether `path` matches one of the granted patterns for `name`.
    /// Patterns use `*` as a single-segment wildcard; `..` never matches a
    /// wildcard, so `data/*` cannot grant `data/../secret`.
    pub fn allows_path(&self, name: &str, path: &str) -> bool {
        let Some(patterns) = self.grants.get(name) else {
            return false;
        };
        if path.contains("..") {
            return false;
        }
        patterns.iter().any(|pattern| glob_match(pattern, path))
    }

    /// The declared grants, sorted, for `--print-capabilities`.
    pub fn entries(&self) -> Vec<(&str, &[String])> {
        self.grants
            .iter()
            .map(|(name, patterns)| (name.as_str(), patterns.as_slice()))
            .collect()
    }

    /// The union of several sets; a name keeps the union of its patterns.
    pub fn union(sets: &[CapabilitySet]) -> CapabilitySet {
        let mut grants = BTreeMap::new();
        for set in sets {
            for (name, patterns) in &set.grants {
                let entry: &mut Vec<String> = grants.entry(name.clone()).or_default();
                for pattern in patterns {
                    if !entry.contains(pattern) {
                        entry.push(pattern.clone());
                    }
                }
            }
        }
        CapabilitySet { grants }
    }

    /// Intersect with a policy ceiling: only names the policy allows survive.
    pub fn restrict(&self, allowed: &[String]) -> CapabilitySet {
        let mut grants = BTreeMap::new();
        for (name, patterns) in &self.grants {
            if allowed.is_empty() || allowed.contains(name) {
                grants.insert(name.clone(), patterns.clone());
            }
        }
        CapabilitySet { grants }
    }
}

fn glob_match(pattern: &str, path: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == path;
    }
    let (prefix, suffix) = pattern.split_once('*').unwrap();
    if !path.starts_with(prefix) || !path.ends_with(suffix) {
        return false;
    }
    let middle = &path[prefix.len()..path.len() - suffix.len()];
    !middle.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(entries: &[(&str, &[&str])]) -> CapabilitySet {
        let mut map = BTreeMap::new();
        for (name, patterns) in entries {
            map.insert(
                (*name).to_string(),
                patterns.iter().map(|text| (*text).to_string()).collect(),
            );
        }
        CapabilitySet::from_map(&map).unwrap()
    }

    #[test]
    fn a_missing_declaration_is_no_capability() {
        let set = CapabilitySet::default();
        assert!(!set.has("fs.read"));
        assert!(!set.allows_path("fs.read", "data/x"));
    }

    #[test]
    fn a_glob_cannot_escape_with_parent_segments() {
        let set = set(&[("fs.read", &["data/*"])]);
        assert!(set.allows_path("fs.read", "data/x"));
        assert!(!set.allows_path("fs.read", "data/../secret"));
        assert!(!set.allows_path("fs.read", "data/a/b"));
    }

    #[test]
    fn union_and_restriction_are_deterministic() {
        let left = set(&[("net", &[]), ("fs.read", &["a/*"])]);
        let right = set(&[("fs.read", &["b/*"])]);
        let union = CapabilitySet::union(&[left, right]);
        assert_eq!(union.entries().len(), 2);
        let restricted = union.restrict(&["net".to_string()]);
        assert!(!restricted.has("fs.read"));
        assert!(restricted.has("net"));
    }
}
