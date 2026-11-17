//! A minimal in-toto/SLSA v1 provenance statement.
//!
//! The statement is JSON, not a bespoke schema: a future registry can consume
//! it unchanged. A claim alone is not a gate — `verify-attestation` checks the
//! subject and material digests, and a rebuild is the stronger check.

use std::collections::BTreeMap;

use yashy_common::Json;

use crate::lock::sha256_hex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub build_type: String,
    pub builder: String,
    pub target: String,
    pub materials: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub subject: Vec<(String, String)>,
    pub predicate: Provenance,
}

impl Statement {
    pub fn to_json(&self) -> Json {
        let mut object = Json::object();
        object.set("_type", Json::string("https://in-toto.io/Statement/v1"));
        object.set(
            "subject",
            Json::array(
                self.subject
                    .iter()
                    .map(|(name, digest)| {
                        let mut item = Json::object();
                        item.set("name", Json::string(name));
                        let mut set = Json::object();
                        set.set("sha256", Json::string(digest));
                        item.set("digest", set);
                        item
                    })
                    .collect(),
            ),
        );
        object.set(
            "predicateType",
            Json::string("https://slsa.dev/provenance/v1"),
        );
        let mut predicate = Json::object();
        predicate.set("buildType", Json::string(&self.predicate.build_type));
        let mut builder = Json::object();
        builder.set("id", Json::string(&self.predicate.builder));
        predicate.set("builder", builder);
        predicate.set("target", Json::string(&self.predicate.target));
        predicate.set(
            "materials",
            Json::array(
                self.predicate
                    .materials
                    .iter()
                    .map(|(name, digest)| {
                        let mut item = Json::object();
                        item.set("uri", Json::string(name));
                        let mut set = Json::object();
                        set.set("sha256", Json::string(digest));
                        item.set("digest", set);
                        item
                    })
                    .collect(),
            ),
        );
        object.set("predicate", predicate);
        object
    }

    /// Parse a statement written by [`Statement::bytes`].
    pub fn from_json_text(text: &[u8]) -> Result<Statement, String> {
        use yashy_common::json_rpc::{self, Value};
        let text = std::str::from_utf8(text).map_err(|error| error.to_string())?;
        let value = json_rpc::parse(text).map_err(|error| format!("invalid statement JSON: {error}"))?;
        let field = |object: &Value, name: &str| -> Result<String, String> {
            match object.get(name) {
                Some(Value::Str(text)) => Ok(text.clone()),
                _ => Err(format!("statement has no `{name}` string")),
            }
        };
        if field(&value, "_type")? != "https://in-toto.io/Statement/v1" {
            return Err("statement does not use in-toto Statement v1".to_string());
        }
        if field(&value, "predicateType")? != "https://slsa.dev/provenance/v1" {
            return Err("predicate does not use SLSA v1".to_string());
        }
        let mut subject = Vec::new();
        if let Some(Value::Arr(items)) = value.get("subject") {
            for item in items {
                let name = field(item, "name")?;
                let digest = match item.get_path(&["digest", "sha256"]) {
                    Some(Value::Str(text)) => text.clone(),
                    _ => return Err("subject has no sha256 digest".to_string()),
                };
                subject.push((name, digest));
            }
        }
        let predicate = value
            .get("predicate")
            .ok_or_else(|| "statement has no predicate".to_string())?;
        let mut materials = BTreeMap::new();
        if let Some(Value::Arr(items)) = predicate.get("materials") {
            for item in items {
                let uri = field(item, "uri")?;
                let digest = match item.get_path(&["digest", "sha256"]) {
                    Some(Value::Str(text)) => text.clone(),
                    _ => return Err("material has no sha256 digest".to_string()),
                };
                materials.insert(uri, digest);
            }
        }
        Ok(Statement {
            subject,
            predicate: Provenance {
                build_type: field(predicate, "buildType")?,
                builder: match predicate.get_path(&["builder", "id"]) {
                    Some(Value::Str(text)) => text.clone(),
                    _ => return Err("predicate has no builder id".to_string()),
                },
                target: field(predicate, "target")?,
                materials,
            },
        })
    }

    /// The statement bytes that are signed.
    pub fn bytes(&self) -> Vec<u8> {
        format!("{}\n", self.to_json().render()).into_bytes()
    }
}

/// Build a statement for one artifact and its named inputs.
pub fn for_artifact(
    name: &str,
    bytes: &[u8],
    target: &str,
    builder: &str,
    materials: BTreeMap<String, String>,
) -> Statement {
    Statement {
        subject: vec![(name.to_string(), sha256_hex(bytes))],
        predicate: Provenance {
            build_type: "https://yashy.dev/builds/local-package/v1".to_string(),
            builder: builder.to_string(),
            target: target.to_string(),
            materials,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_uses_the_standard_types() {
        let statement = for_artifact(
            "demo.tar.gz",
            b"artifact",
            "x86_64-pc-windows-msvc",
            "yashyc 1.0.0",
            BTreeMap::from([("yashy.toml".to_string(), sha256_hex(b"manifest"))]),
        );
        let text = statement.to_json().render();
        assert!(text.contains("https://in-toto.io/Statement/v1"), "{text}");
        assert!(text.contains("https://slsa.dev/provenance/v1"), "{text}");
        assert!(text.contains(&sha256_hex(b"artifact")), "{text}");
        assert!(text.contains("yashy.toml"), "{text}");
    }
}
