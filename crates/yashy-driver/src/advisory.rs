//! Signed advisory feeds.
//!
//! An advisory names a package, a version range, a severity and a fixed
//! version. The feed is a small, versioned JSON document signed with the same
//! Ed25519 key format as packages, so a client can refuse a forged or stale
//! feed without trusting a web page. A project inside an advisory range is
//! refused with the advisory id, the range and the fix.

use std::path::Path;

use yashy_common::json_rpc::{self, Value};
use yashy_common::Json;
use yashy_manifest::signature::{self, Keyring, Signature, VerifyError};
use yashy_manifest::{Version, VersionReq};

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
