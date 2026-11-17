//! Version requirements: caret, tilde, exact, inequalities and wildcards.
//!
//! The grammar is the one a manifest author expects from Cargo; it is parsed
//! without a regex crate and refuses anything it cannot explain. A requirement
//! never includes a pre-release unless the requirement itself names one.

use std::fmt;

use crate::version::Version;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionReq {
    Caret(Version),
    Tilde(Version),
    Exact(Version),
    Gte(Version),
    Lt(Version),
    Wildcard {
        major: Option<u64>,
        minor: Option<u64>,
    },
    All(Vec<VersionReq>),
}

fn parse_partial(text: &str) -> Option<(u64, Option<u64>, Option<u64>, Vec<String>)> {
    let (core, pre) = match text.split_once('-') {
        Some((left, right)) => (left, right.split('.').map(str::to_string).collect()),
        None => (text, Vec::new()),
    };
    let mut parts = core.split('.');
    let major = parts.next()?;
    let minor = parts.next();
    let patch = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let number = |text: &str| -> Option<u64> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        if text.len() > 1 && text.starts_with('0') {
            return None;
        }
        text.parse().ok()
    };
    let minor_parsed = match minor {
        Some(text) => Some(number(text)?),
        None => None,
    };
    let patch_parsed = match patch {
        Some(text) => Some(number(text)?),
        None => None,
    };
    Some((number(major)?, minor_parsed, patch_parsed, pre))
}

fn complete(text: &str) -> Option<Version> {
    let (major, minor, patch, pre) = parse_partial(text)?;
    if minor.is_none() || patch.is_none() && !pre.is_empty() {
        return None;
    }
    let joined = if pre.is_empty() {
        format!("{major}.{}.{}", minor.unwrap_or(0), patch.unwrap_or(0))
    } else {
        format!(
            "{major}.{}.{}-{}",
            minor.unwrap_or(0),
            patch.unwrap_or(0),
            pre.join(".")
        )
    };
    Version::parse(&joined)
}

impl VersionReq {
    /// Parse one requirement, or several separated by commas.
    pub fn parse(text: &str) -> Option<VersionReq> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let parts: Vec<&str> = text.split(',').map(str::trim).collect();
        if parts.len() > 1 {
            let mut all = Vec::new();
            for part in parts {
                all.push(VersionReq::parse_one(part)?);
            }
            return Some(VersionReq::All(all));
        }
        VersionReq::parse_one(text)
    }

    fn parse_one(text: &str) -> Option<VersionReq> {
        if text == "*" {
            return Some(VersionReq::Wildcard {
                major: None,
                minor: None,
            });
        }
        let (op, rest) = if let Some(rest) = text.strip_prefix("^=") {
            ("^", rest)
        } else if let Some(rest) = text.strip_prefix("^") {
            ("^", rest)
        } else if let Some(rest) = text.strip_prefix("~") {
            ("~", rest)
        } else if let Some(rest) = text.strip_prefix("==") {
            ("=", rest)
        } else if let Some(rest) = text.strip_prefix("=") {
            ("=", rest)
        } else if let Some(rest) = text.strip_prefix(">=") {
            (">=", rest)
        } else if let Some(rest) = text.strip_prefix("<=") {
            ("<=", rest)
        } else if let Some(rest) = text.strip_prefix(">") {
            (">", rest)
        } else if let Some(rest) = text.strip_prefix("<") {
            ("<", rest)
        } else {
            ("^", text)
        };
        let rest = rest.trim();
        if rest.ends_with(".*") || rest.ends_with('.') {
            let bare = rest.trim_end_matches('.').trim_end_matches(".*");
            let (major, minor, patch, pre) = parse_partial(bare)?;
            if patch.is_some() || !pre.is_empty() {
                return None;
            }
            return Some(VersionReq::Wildcard {
                major: Some(major),
                minor,
            });
        }
        let version = match op {
            ">=" => complete(rest)?,
            "<" => complete(rest)?,
            ">" | "<=" => return None, // not part of the supported grammar
            _ => complete(rest)?,
        };
        Some(match op {
            "^" => VersionReq::Caret(version),
            "~" => VersionReq::Tilde(version),
            "=" => VersionReq::Exact(version),
            ">=" => VersionReq::Gte(version),
            "<" => VersionReq::Lt(version),
            _ => return None,
        })
    }

    /// Whether `version` satisfies this requirement.
    pub fn matches(&self, version: &Version) -> bool {
        if version.is_prerelease() && !self.mentions_prerelease() {
            return false;
        }
        match self {
            VersionReq::Exact(wanted) => version.cmp(wanted).is_eq(),
            VersionReq::Gte(minimum) => version >= minimum,
            VersionReq::Lt(maximum) => version < maximum,
            VersionReq::Caret(base) => {
                version >= base
                    && match (base.major, base.minor, base.patch) {
                        (0, 0, patch) => {
                            version.major == 0 && version.minor == 0 && version.patch == patch
                        }
                        (0, minor, _) => version.major == 0 && version.minor == minor,
                        (major, _, _) => version.major == major,
                    }
            }
            VersionReq::Tilde(base) => {
                version >= base && version.major == base.major && version.minor == base.minor
            }
            VersionReq::Wildcard { major, minor } => match (major, minor) {
                (None, _) => true,
                (Some(major), None) => version.major == *major,
                (Some(major), Some(minor)) => version.major == *major && version.minor == *minor,
            },
            VersionReq::All(parts) => parts.iter().all(|part| part.matches(version)),
        }
    }

    fn mentions_prerelease(&self) -> bool {
        match self {
            VersionReq::Exact(version)
            | VersionReq::Caret(version)
            | VersionReq::Tilde(version)
            | VersionReq::Gte(version)
            | VersionReq::Lt(version) => version.is_prerelease(),
            VersionReq::Wildcard { .. } => false,
            VersionReq::All(parts) => parts.iter().any(VersionReq::mentions_prerelease),
        }
    }

    /// The canonical spelling written back to a manifest.
    pub fn canonical(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionReq::Caret(version) => write!(f, "^{version}"),
            VersionReq::Tilde(version) => write!(f, "~{version}"),
            VersionReq::Exact(version) => write!(f, "={version}"),
            VersionReq::Gte(version) => write!(f, ">={version}"),
            VersionReq::Lt(version) => write!(f, "<{version}"),
            VersionReq::Wildcard { major, minor } => match (major, minor) {
                (None, _) => f.write_str("*"),
                (Some(major), None) => write!(f, "{major}.*"),
                (Some(major), Some(minor)) => write!(f, "{major}.{minor}.*"),
            },
            VersionReq::All(parts) => {
                for (index, part) in parts.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{part}")?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn req(text: &str) -> VersionReq {
        VersionReq::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn caret_selects_the_compatible_major() {
        assert!(req("^1.2").matches(&v("1.9.0")));
        assert!(!req("^1.2").matches(&v("2.0.0")));
        assert!(!req("^1.2").matches(&v("1.1.9")));
        assert!(req("^0.2.3").matches(&v("0.2.9")));
        assert!(!req("^0.2.3").matches(&v("0.3.0")));
        assert!(req("^0.0.3").matches(&v("0.0.3")));
        assert!(!req("^0.0.3").matches(&v("0.0.4")));
    }

    #[test]
    fn tilde_selects_the_minor_series() {
        assert!(req("~1.2").matches(&v("1.2.9")));
        assert!(!req("~1.2").matches(&v("1.3.0")));
    }

    #[test]
    fn wildcards_and_intersections_work() {
        assert!(req("1.2.*").matches(&v("1.2.99")));
        assert!(!req("1.*").matches(&v("2.0.0")));
        assert!(req(">=1.0, <2.0").matches(&v("1.9.0")));
        assert!(!req(">=1.0, <2.0").matches(&v("2.0.0")));
    }

    #[test]
    fn prereleases_need_an_explicit_mention() {
        assert!(!req("^1.0").matches(&v("1.1.0-alpha")));
        assert!(req("^1.1.0-alpha").matches(&v("1.1.0-alpha.1")));
    }

    #[test]
    fn bad_requirements_are_refused() {
        for text in ["", "^", "^1.2.3.4", ">=1", "1.2.3, nonsense"] {
            assert!(VersionReq::parse(text).is_none(), "{text} must be refused");
        }
    }
}
