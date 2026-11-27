//! Short-lived approval tokens for destructive MCP calls.
//!
//! An approval is not a prompt and not a signature from the agent. It is a
//! file a human (or a trusted runner) writes under `.yashy/approvals/`, bound
//! to the tree hash, a canonical patch, the verb set and an expiry. The MCP
//! session consumes one token exactly once, before it runs the verb; a replay
//! has no file left to read.
//!
//! The token id is SHA-256 over the same fields, so a token minted for one
//! tree or one patch cannot be used for another.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use yashy_common::json_rpc::{self, Value};
use yashy_common::Json;
use yashy_manifest::sha256_hex;

/// One approval, as stored on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub tree_hash: String,
    pub patch_hash: String,
    pub verbs: Vec<String>,
    pub expires_unix_ms: u64,
    pub nonce: String,
}

impl Approval {
    pub fn to_json(&self, token: &str) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("token", Json::string(token));
        root.set("tree_hash", Json::string(&self.tree_hash));
        root.set("patch_hash", Json::string(&self.patch_hash));
        root.set(
            "verbs",
            Json::array(self.verbs.iter().map(|verb| Json::string(verb)).collect()),
        );
        root.set("expires_unix_ms", Json::number(self.expires_unix_ms as i64));
        root.set("nonce", Json::string(&self.nonce));
        root
    }

    pub fn from_json(value: &Value) -> Option<Approval> {
        let field = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_string);
        let expires = value.get("expires_unix_ms").and_then(Value::as_i64)?;
        if expires < 0 {
            return None;
        }
        let verbs = value.get("verbs").and_then(Value::as_array)?;
        let mut names = Vec::new();
        for verb in verbs {
            names.push(verb.as_str()?.to_string());
        }
        Some(Approval {
            tree_hash: field("tree_hash")?,
            patch_hash: field("patch_hash")?,
            verbs: names,
            expires_unix_ms: expires as u64,
            nonce: field("nonce")?,
        })
    }
}

/// Sort object keys recursively so the same patch written in a different key
/// order has the same fingerprint.
pub fn canonical(value: &Value) -> Value {
    match value {
        Value::Obj(fields) => {
            let mut fields: Vec<(String, Value)> = fields
                .iter()
                .map(|(key, value)| (key.clone(), canonical(value)))
                .collect();
            fields.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Obj(fields)
        }
        Value::Arr(items) => Value::Arr(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// The patch half of an approval's binding.
pub fn patch_hash(patch: &Value) -> String {
    sha256_hex(json_rpc::to_string(&canonical(patch)).as_bytes())
}

/// The token id: a pure function of everything the token grants.
pub fn token_id(approval: &Approval) -> String {
    sha256_hex(
        format!(
            "{}\n{}\n{}\n{}\n{}",
            approval.tree_hash,
            approval.patch_hash,
            approval.verbs.join(","),
            approval.expires_unix_ms,
            approval.nonce
        )
        .as_bytes(),
    )
}

fn directory(root: &Path) -> PathBuf {
    root.join(".yashy").join("approvals")
}

/// Validate the token spelling before using it as a file name: a token is 64
/// lower-case hex characters and nothing else.
fn token_file(root: &Path, token: &str) -> Result<PathBuf, String> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("approval token is not a 64-character hex id".to_string());
    }
    Ok(directory(root).join(format!("{token}.json")))
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Write a token. `create_new` means an id can never be silently replaced.
pub fn mint(
    root: &Path,
    tree_hash: &str,
    patch: &Value,
    verbs: Vec<String>,
    ttl_seconds: u64,
    nonce: &str,
) -> Result<(String, Approval), String> {
    std::fs::create_dir_all(directory(root))
        .map_err(|error| format!("{}: {error}", directory(root).display()))?;
    let approval = Approval {
        tree_hash: tree_hash.to_string(),
        patch_hash: patch_hash(patch),
        verbs,
        expires_unix_ms: now_unix_ms().saturating_add(ttl_seconds.saturating_mul(1000)),
        nonce: nonce.to_string(),
    };
    let token = token_id(&approval);
    let path = token_file(root, &token)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(approval.to_json(&token).render_compact().as_bytes())
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok((token, approval))
}

/// Check a token against the call and burn it. Nothing is removed until every
/// check passes, so a wrong call does not destroy a good approval.
pub fn consume(
    root: &Path,
    token: &str,
    tree_hash: &str,
    patch: &Value,
    verb: &str,
) -> Result<Approval, String> {
    let path = token_file(root, token)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "approval token is unknown or already used".to_string())?;
    let value = json_rpc::parse(&text).map_err(|error| format!("approval is not JSON: {error}"))?;
    let approval = Approval::from_json(&value).ok_or("approval is missing a field".to_string())?;
    if token_id(&approval) != token {
        return Err("approval token does not match its contents".to_string());
    }
    if now_unix_ms() >= approval.expires_unix_ms {
        return Err("approval token has expired".to_string());
    }
    if approval.tree_hash != tree_hash {
        return Err("approval token is bound to another tree".to_string());
    }
    if approval.patch_hash != patch_hash(patch) {
        return Err("approval token is bound to another patch".to_string());
    }
    if !approval.verbs.iter().any(|name| name == verb) {
        return Err(format!("approval token does not permit `{verb}`"));
    }
    std::fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(approval)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(text: &str) -> Value {
        json_rpc::parse(text).unwrap()
    }

    #[test]
    fn canonical_patch_ignores_key_order() {
        let first = patch("{\"a\":1,\"b\":[2,3]}");
        let second = patch("{\"b\":[2,3],\"a\":1}");
        assert_eq!(patch_hash(&first), patch_hash(&second));
    }

    #[test]
    fn a_token_is_bound_to_tree_patch_and_verb() {
        let dir = std::env::temp_dir().join("yashy-approval-unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (token, _) = mint(
            &dir,
            "tree-one",
            &patch("{\"edits\":[]}"),
            vec!["edit".to_string()],
            60,
            "nonce-1",
        )
        .unwrap();
        assert!(consume(&dir, &token, "tree-two", &patch("{\"edits\":[]}"), "edit").is_err());
        assert!(consume(&dir, &token, "tree-one", &patch("{\"edits\":[1]}"), "edit").is_err());
        assert!(consume(&dir, &token, "tree-one", &patch("{\"edits\":[]}"), "build").is_err());
        assert!(consume(&dir, &token, "tree-one", &patch("{\"edits\":[]}"), "edit").is_ok());
        assert!(consume(&dir, &token, "tree-one", &patch("{\"edits\":[]}"), "edit").is_err());
    }

    #[test]
    fn a_bad_token_spelling_is_refused_before_any_read() {
        let dir = std::env::temp_dir().join("yashy-approval-spelling");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let error = consume(&dir, "../secret", "tree", &Value::Null, "edit").unwrap_err();
        assert!(error.contains("not a 64-character hex id"), "{error}");
    }

    #[test]
    fn an_expired_token_is_refused() {
        let dir = std::env::temp_dir().join("yashy-approval-expired");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (token, _) = mint(
            &dir,
            "tree",
            &Value::Null,
            vec!["edit".to_string()],
            0,
            "nonce-2",
        )
        .unwrap();
        assert!(consume(&dir, &token, "tree", &Value::Null, "edit")
            .unwrap_err()
            .contains("expired"));
    }
}
