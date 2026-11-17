//! SemVer 2.0 versions with the precedence rules the resolver needs.
//!
//! The repository builds offline, so this is a small, total parser rather than
//! a dependency. It accepts exactly the grammar in the spec: numeric core,
//! dot-separated pre-release and build identifiers, no leading zeroes in
//! numeric identifiers. Build metadata round-trips but never affects order.

use std::cmp::Ordering;
use std::fmt;

/// One dot-separated identifier in a pre-release or build field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identifier {
    Numeric(u64),
    Alphanumeric(String),
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Identifier::Numeric(value) => write!(f, "{value}"),
            Identifier::Alphanumeric(value) => f.write_str(value),
        }
    }
}

impl Identifier {
    fn parse(text: &str) -> Option<Identifier> {
        if text.is_empty()
            || !text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return None;
        }
        if text.bytes().all(|byte| byte.is_ascii_digit()) {
            if text.len() > 1 && text.starts_with('0') {
                return None;
            }
            return Some(Identifier::Numeric(text.parse().ok()?));
        }
        Some(Identifier::Alphanumeric(text.to_string()))
    }

    fn cmp_semver(&self, other: &Identifier) -> Ordering {
        match (self, other) {
            (Identifier::Numeric(left), Identifier::Numeric(right)) => left.cmp(right),
            (Identifier::Numeric(_), Identifier::Alphanumeric(_)) => Ordering::Less,
            (Identifier::Alphanumeric(_), Identifier::Numeric(_)) => Ordering::Greater,
            (Identifier::Alphanumeric(left), Identifier::Alphanumeric(right)) => left.cmp(right),
        }
    }
}

/// A `major.minor.patch[-pre][+build]` version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<Identifier>,
    pub build: Vec<Identifier>,
}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let (core_and_pre, build) = match text.split_once('+') {
            Some((left, right)) => {
                if right.contains('+') {
                    return None;
                }
                (left, parse_identifiers(right, true)?)
            }
            None => (text, Vec::new()),
        };
        let (core, pre) = match core_and_pre.split_once('-') {
            Some((left, right)) => {
                if right.contains('-') {
                    return None;
                }
                (left, parse_identifiers(right, true)?)
            }
            None => (core_and_pre, Vec::new()),
        };
        let mut parts = core.split('.');
        let major = parts.next()?;
        let minor = parts.next()?;
        let patch = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major: parse_number(major)?,
            minor: parse_number(minor)?,
            patch: parse_number(patch)?,
            pre,
            build,
        })
    }

    /// Whether this version carries a pre-release field.
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// Compare only the `major.minor.patch` core.
    pub fn cmp_core(&self, other: &Version) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }

    /// The core spelling, used in lockfile pins.
    pub fn core(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None;
    }
    text.parse().ok()
}

fn parse_identifiers(text: &str, require: bool) -> Option<Vec<Identifier>> {
    if text.is_empty() && !require {
        return Some(Vec::new());
    }
    text.split('.')
        .map(Identifier::parse)
        .collect::<Option<Vec<_>>>()
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            f.write_str("-")?;
            for (index, identifier) in self.pre.iter().enumerate() {
                if index > 0 {
                    f.write_str(".")?;
                }
                write!(f, "{identifier}")?;
            }
        }
        if !self.build.is_empty() {
            f.write_str("+")?;
            for (index, identifier) in self.build.iter().enumerate() {
                if index > 0 {
                    f.write_str(".")?;
                }
                write!(f, "{identifier}")?;
            }
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Version) -> Ordering {
        let core = self.cmp_core(other);
        if core != Ordering::Equal {
            return core;
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal, // build metadata is ignored
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (left, right) in self.pre.iter().zip(other.pre.iter()) {
                    let order = left.cmp_semver(right);
                    if order != Ordering::Equal {
                        return order;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn semver_precedence_matches_the_spec_example() {
        assert!(v("1.0.0-alpha") < v("1.0.0-alpha.1"));
        assert!(v("1.0.0-alpha.1") < v("1.0.0"));
        assert!(v("1.0.0-alpha.beta") < v("1.0.0-beta"));
        assert!(v("1.0.0-beta") < v("1.0.0-beta.2"));
        assert!(v("1.0.0-beta.2") < v("1.0.0-beta.11"));
        assert!(v("1.0.0-beta.11") < v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.1") < v("1.0.0"));
    }

    #[test]
    fn numeric_identifiers_sort_below_alphanumeric() {
        assert!(v("1.0.0-1") < v("1.0.0-alpha"));
        assert!(v("1.0.0-alpha") < v("1.0.0-beta"));
    }

    #[test]
    fn build_metadata_is_ignored_but_round_trips() {
        assert_eq!(v("1.0.0+a"), v("1.0.0+b"));
        assert_eq!(v("1.0.0-rc.1+build.7").to_string(), "1.0.0-rc.1+build.7");
    }

    #[test]
    fn malformed_versions_are_refused() {
        for text in [
            "1",
            "1.2",
            "01.2.3",
            "1.02.3",
            "1.0.0-",
            "1.0.0+",
            "1.0.0-a..b",
            "v1.0.0",
        ] {
            assert!(Version::parse(text).is_none(), "{text} must be refused");
        }
    }
}
