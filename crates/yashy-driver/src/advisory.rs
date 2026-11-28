//! Signed advisory feeds.
//!
//! An advisory names a package, a version range, a severity and a fixed
//! version. The feed is a small, versioned JSON document signed with the same
//! Ed25519 key format as packages, so a client can refuse a forged or stale
//! feed without trusting a web page. A project inside an advisory range is
//! refused with the advisory id, the range and the fix.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use yashy_common::json_rpc::{self, Value};
use yashy_common::Json;
use yashy_manifest::signature::{self, Keyring, Signature, VerifyError};
use yashy_manifest::{sha256_hex, Version, VersionReq};

/// One advisory record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advisory {
    pub id: String,
    pub package: String,
    pub range: String,
    pub severity: String,
    pub fixed: String,
    pub title: String,
}

impl Advisory {
    fn from_json(value: &Value) -> Result<Advisory, String> {
        let field = |name: &str| {
            value
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("an advisory needs a string `{name}`"))
        };
        let advisory = Advisory {
            id: field("id")?,
            package: field("package")?,
            range: field("range")?,
            severity: field("severity")?,
            fixed: field("fixed")?,
            title: field("title")?,
        };
        // Reject a range that cannot be parsed at load time, not at match time.
        advisory.requirement()?;
        Ok(advisory)
    }

    /// The parsed version requirement, or an error naming the bad range.
    pub fn requirement(&self) -> Result<VersionReq, String> {
        VersionReq::parse(&self.range)
            .ok_or_else(|| format!("advisory {}: bad range `{}`", self.id, self.range))
    }

    /// Whether `version` is inside this advisory's range.
    pub fn affects(&self, version: &str) -> Result<bool, String> {
        let parsed = Version::parse(version).ok_or_else(|| format!("bad version `{version}`"))?;
        Ok(self.requirement()?.matches(&parsed))
    }

    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("id", Json::string(&self.id));
        root.set("package", Json::string(&self.package));
        root.set("range", Json::string(&self.range));
        root.set("severity", Json::string(&self.severity));
        root.set("fixed", Json::string(&self.fixed));
        root.set("title", Json::string(&self.title));
        root
    }
}

/// A parsed feed with its signature kept aside for verification.
#[derive(Clone, Debug)]
pub struct Feed {
    pub advisories: Vec<Advisory>,
    body: String,
    signature: Signature,
}

impl Feed {
    /// Parse a feed document, validating the schema and every version range.
    pub fn parse(text: &str) -> Result<Feed, String> {
        let root = json_rpc::parse(text).map_err(|error| format!("feed is not JSON: {error}"))?;
        let version = root
            .get("schema_version")
            .and_then(Value::as_i64)
            .ok_or("feed has no schema_version")?;
        if version != 1 {
            return Err(format!("unknown advisory schema {version}"));
        }
        // The signed bytes travel with the feed so verification never depends
        // on re-rendering a parsed value into the original spelling.
        let body = root
            .get("body")
            .and_then(Value::as_str)
            .ok_or("feed has no signed body")?
            .to_string();
        let parsed_body =
            json_rpc::parse(&body).map_err(|error| format!("feed body is not JSON: {error}"))?;
        let items = parsed_body
            .get("advisories")
            .and_then(Value::as_array)
            .ok_or("feed body has no advisories array")?;
        let mut advisories = Vec::new();
        for item in items {
            advisories.push(Advisory::from_json(item)?);
        }
        let sig = root.get("signature").ok_or("feed has no signature")?;
        let field = |name: &str| {
            sig.get(name)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("signature needs a string `{name}`"))
        };
        let bytes_hex = field("bytes")?;
        let bytes =
            decode_hex(&bytes_hex).ok_or_else(|| "signature bytes must be hex".to_string())?;
        let signature = Signature {
            key_id: field("key_id")?,
            algorithm: {
                let name = field("algorithm")?;
                if name != "Ed25519" {
                    return Err(format!("unknown signature algorithm `{name}`"));
                }
                "Ed25519"
            },
            digest: field("digest")?,
            bytes,
        };
        Ok(Feed {
            advisories,
            body,
            signature,
        })
    }

    /// Verify the Ed25519 signature against an explicit keyring.
    pub fn verify(&self, keys: &Keyring) -> Result<(), VerifyError> {
        signature::verify(self.body.as_bytes(), &self.signature, keys)
    }

    /// The advisories that affect `package` at `version`.
    pub fn matches(&self, package: &str, version: &str) -> Result<Vec<&Advisory>, String> {
        let mut out = Vec::new();
        for advisory in &self.advisories {
            if advisory.package == package && advisory.affects(version)? {
                out.push(advisory);
            }
        }
        Ok(out)
    }

    /// The whole feed as JSON, with the signature and the parsed records.
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set(
            "advisories",
            Json::array(self.advisories.iter().map(Advisory::to_json).collect()),
        );
        root.set("key_id", Json::string(&self.signature.key_id));
        root
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

/// Read a keyring file: one `id hex-public-key` per line, `#` starts a comment.
pub fn load_keyring(path: &Path) -> Result<Keyring, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut keys = Keyring::default();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let id = parts
            .next()
            .ok_or_else(|| format!("{}:{}: no key id", path.display(), index + 1))?;
        let hex = parts
            .next()
            .ok_or_else(|| format!("{}:{}: no key bytes", path.display(), index + 1))?;
        let bytes = decode_hex(hex)
            .ok_or_else(|| format!("{}:{}: key is not hex", path.display(), index + 1))?;
        keys.insert(id, &bytes)
            .map_err(|error| format!("{}:{}: {error}", path.display(), index + 1))?;
    }
    Ok(keys)
}

/// Build a signed feed document from advisories and a raw 32-byte seed.
/// Used by fixtures and by a release publisher; a client never needs it.
pub fn sign_feed(advisories: Vec<Advisory>, key_id: &str, seed: &[u8]) -> Result<String, String> {
    let mut body = Json::object();
    body.set("schema_version", Json::number(1));
    body.set(
        "advisories",
        Json::array(advisories.iter().map(Advisory::to_json).collect()),
    );
    let bytes = body.render_compact();
    let signature = signature::sign(bytes.as_bytes(), key_id, seed)
        .map_err(|error| format!("cannot sign the feed: {error}"))?;
    let mut root = Json::object();
    root.set("schema_version", Json::number(1));
    root.set("body", Json::string(&bytes));
    let mut sig = Json::object();
    sig.set("key_id", Json::string(&signature.key_id));
    sig.set("algorithm", Json::string(signature.algorithm));
    sig.set("digest", Json::string(&signature.digest));
    sig.set("bytes", Json::string(encode_hex(&signature.bytes)));
    root.set("signature", sig);
    Ok(root.render())
}

/// The advisory cache root. The same cache the package store uses, under an
/// `advisories` directory, so a compiler-wide `YASHY_CACHE_DIR` clears both.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YASHY_CACHE_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("advisories");
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("yashy").join("advisories");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("yashy")
            .join("advisories");
    }
    std::env::temp_dir().join("yashy-advisories")
}

/// The cache file for one feed URL, given the cache root explicitly so a test
/// can point the CLI and itself at the same private directory.
pub fn cache_path_in(root: &Path, url: &str) -> PathBuf {
    let digest = sha256_hex(url.as_bytes());
    root.join(format!("{}.json", &digest[..16]))
}

/// Bytes from a local feed or a fetched URL, with the facts a client needs to
/// decide whether to trust them: the digest it can pin, where they came from
/// and how old the cache entry is.
pub struct FeedFile {
    pub bytes: Vec<u8>,
    pub origin: String,
    pub sha256: String,
    pub age_seconds: Option<u64>,
}

impl FeedFile {
    /// The JSON form shared by every `advisory --json` answer.
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("origin", Json::string(&self.origin));
        root.set("sha256", Json::string(&self.sha256));
        match self.age_seconds {
            Some(age) => {
                root.set("age_seconds", Json::number(age as i64));
            }
            None => {
                root.set("age_seconds", Json::null());
            }
        }
        root
    }
}

fn is_url(name: &str) -> bool {
    ["http://", "https://", "file://"]
        .iter()
        .any(|prefix| name.starts_with(*prefix))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Resolve a feed source under the caller's network and freshness policy.
///
/// Fetching is explicit: a URL is only fetched when `--fetch` is given, and
/// `--offline` refuses even then. There is no implicit network call behind a
/// check, and no silent fallback from a cache miss to a fetch. The pin is
/// checked over the exact bytes, before parsing or signature verification,
/// so a pinned client cannot be served a different document with a valid
/// signature from another key.
pub fn load_feed(
    name: &str,
    fetch: bool,
    offline: bool,
    pin: Option<&str>,
    max_age: Option<u64>,
) -> Result<FeedFile, String> {
    let root = cache_dir();
    let (bytes, origin, dated) = if is_url(name) {
        let cached = cache_path_in(&root, name);
        let stamp = cached.with_extension("fetched");
        if fetch {
            if offline {
                return Err(format!("`--offline` refuses to fetch {name}"));
            }
            let bytes = crate::registry::fetch_url(name)
                .map_err(|error| format!("cannot fetch {name}: {error}"))?;
            std::fs::create_dir_all(&root)
                .map_err(|error| format!("cannot create {}: {error}", root.display()))?;
            std::fs::write(&cached, &bytes)
                .map_err(|error| format!("cannot cache {}: {error}", cached.display()))?;
            std::fs::write(&stamp, now_seconds().to_string())
                .map_err(|error| format!("cannot date {}: {error}", stamp.display()))?;
            (bytes, format!("fetched {name}"), Some(stamp))
        } else if cached.is_file() {
            (
                std::fs::read(&cached).map_err(|error| format!("{}: {error}", cached.display()))?,
                format!("cached {name}"),
                Some(stamp),
            )
        } else if offline {
            return Err(format!(
                "no cached advisory feed for {name}; fetch it before going offline"
            ));
        } else {
            return Err(format!(
                "{name} is a URL: pass `--fetch` to fetch it, or `--offline` to use the cache"
            ));
        }
    } else {
        let path = Path::new(name);
        let bytes = std::fs::read(path).map_err(|error| format!("{name}: {error}"))?;
        // A local file can carry a fetch stamp beside it, which is how a
        // checked-out mirror records when it was refreshed. Without one, the
        // file's own mtime is the age.
        let stamp = path.with_extension("fetched");
        let dated = if stamp.is_file() { Some(stamp) } else { None };
        (bytes, format!("local {name}"), dated)
    };
    let sha256 = sha256_hex(&bytes);
    if let Some(pin) = pin {
        if pin != sha256 {
            return Err(format!(
                "advisory feed pin mismatch: expected `{pin}`, fetched `{sha256}`"
            ));
        }
    }
    let modified = if let Some(stamp) = &dated {
        std::fs::read_to_string(stamp)
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
    } else if !is_url(name) {
        std::fs::metadata(name)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs())
    } else {
        None
    };
    let age_seconds = modified.and_then(|then| now_seconds().checked_sub(then));
    if let Some(limit) = max_age {
        match age_seconds {
            Some(age) if age > limit => {
                return Err(format!(
                    "advisory feed is {age}s old (max {limit}s); fetch it again"
                ))
            }
            Some(_) => {}
            None => return Err("cannot determine the advisory feed age".to_string()),
        }
    }
    Ok(FeedFile {
        bytes,
        origin,
        sha256,
        age_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7; 32];

    fn advisory() -> Advisory {
        Advisory {
            id: "YASHY-2027-0001".to_string(),
            package: "compress".to_string(),
            range: "<2.0.1".to_string(),
            severity: "high".to_string(),
            fixed: "2.0.1".to_string(),
            title: "decoder overflow".to_string(),
        }
    }

    #[test]
    fn a_signed_feed_verifies_and_matches_a_range() {
        let text = sign_feed(vec![advisory()], "release", &SEED).expect("a signed feed");
        let feed = Feed::parse(&text).expect("the feed parses");
        let mut keys = Keyring::default();
        keys.insert("release", &signature::public_key(&SEED).unwrap())
            .unwrap();
        feed.verify(&keys).expect("the signature verifies");
        let found = feed.matches("compress", "2.0.0").expect("a version match");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "YASHY-2027-0001");
        assert!(feed.matches("compress", "2.0.1").unwrap().is_empty());
        assert!(feed.matches("other", "2.0.0").unwrap().is_empty());
    }

    #[test]
    fn a_tampered_feed_is_refused() {
        let text = sign_feed(vec![advisory()], "release", &SEED).expect("a signed feed");
        let tampered = text.replace("decoder overflow", "decoder is fine");
        let feed = Feed::parse(&tampered).expect("still parses");
        let mut keys = Keyring::default();
        keys.insert("release", &signature::public_key(&SEED).unwrap())
            .unwrap();
        assert!(feed.verify(&keys).is_err(), "the digest no longer matches");
    }

    #[test]
    fn an_unknown_schema_or_bad_range_is_refused() {
        assert!(Feed::parse("{\"schema_version\":2,\"advisories\":[]}").is_err());
        let bad = Advisory {
            range: "not a range".to_string(),
            ..advisory()
        };
        assert!(bad.requirement().is_err());
    }
}
