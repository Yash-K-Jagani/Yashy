//! The agent policy and its audit log.
//!
//! An MCP session is a series of tool calls made by a program, not a person.
//! Two facts keep that honest: the session's policy names the verbs it may
//! call and the paths it may touch, and every call -- allowed, refused or
//! failed -- is one line in an append-only, hash-chained log.
//!
//! The policy file is `.yashy/policy.toml`. It is deliberately small:
//!
//! ```toml
//! allow_verbs = ["check", "build", "edit"]
//! protected_paths = ["yashy.lock", ".github/**"]
//! max_edits = 20
//! audit_max_bytes = 1048576
//! ```
//!
//! A missing file is the strict default: the verbs that do not change the
//! source (`check`, `build`, `test`, `verify`, `explain`, `context`, `index`,
//! `lint`, `doc`) are allowed, and the mutating verbs (`edit`, `refactor`,
//! `format`, `fix`) are not. A file that names `allow_verbs` is the complete
//! set. The policy itself is always protected, so a session cannot widen its
//! own mandate. The policy can only narrow what the compiler already does --
//! it never grants a verb the tool table does not contain.
//!
//! The audit log's chain is over `seq`, tool, outcome, code, detail and the
//! previous hash, spelled as a decimal sequence. Removing or reordering an
//! entry breaks the chain and `yashyc agent audit --verify` names the first
//! bad sequence number ([`AUDIT_FAILURE`]).

use std::path::{Path, PathBuf};

use yashy_common::Json;
use yashy_manifest::signature::{self, Keyring};
use yashy_manifest::toml::{self, Value};

use crate::mcp::TOOLS;

/// The diagnostic code a refused policy or a broken audit chain carries.
pub const POLICY_DENIED: &str = yashy_common::codes::POLICY_DENIED;
pub const AUDIT_FAILURE: &str = yashy_common::codes::AUDIT_FAILURE;

/// The verbs that do not change the workspace. A missing policy allows these
/// and refuses the rest.
pub const READ_ONLY_VERBS: [&str; 9] = [
    "check", "build", "test", "verify", "explain", "context", "index", "lint", "doc",
];

/// The verbs that change source, and the only verbs an approval can name.
pub const MUTATING_VERBS: [&str; 4] = ["edit", "refactor", "format", "fix"];

/// The policy an MCP session runs under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPolicy {
    /// Always a complete set; a missing policy is [`READ_ONLY_VERBS`].
    pub allow: Vec<String>,
    /// Glob patterns matched against a request's workspace-relative paths.
    pub protected_paths: Vec<String>,
    /// The largest patch, in edits, one call may carry.
    pub max_edits: Option<usize>,
    /// Verbs that need a short-lived token before they may run.
    pub require_approval: Vec<String>,
    /// Stop the session after this many milliseconds of tool time.
    pub max_wall_ms: Option<u64>,
    /// Stop the session after this many tokens (bytes/4, rounded up).
    pub max_tokens: Option<u64>,
    /// Stop the session after this many output bytes from tools.
    pub max_output_bytes: Option<u64>,
    /// Rotate the active segment before an append would exceed this size.
    pub audit_max_bytes: u64,
}

impl Default for AgentPolicy {
    fn default() -> Self {
        AgentPolicy {
            allow: READ_ONLY_VERBS
                .iter()
                .map(|verb| verb.to_string())
                .collect(),
            protected_paths: Vec::new(),
            max_edits: None,
            require_approval: Vec::new(),
            max_wall_ms: None,
            max_tokens: None,
            max_output_bytes: None,
            audit_max_bytes: 1024 * 1024,
        }
    }
}

impl AgentPolicy {
    /// Load `.yashy/policy.toml` under `root`. A missing file is the strict
    /// default; a malformed one is an error, never a silent widening.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(".yashy").join("policy.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        Self::parse(&text).map_err(|message| format!("{}: {message}", path.display()))
    }

    /// Parse a policy document. Every key is known; an unknown key is an
    /// error so a misspelling cannot silently widen the policy.
    pub fn parse(text: &str) -> Result<Self, String> {
        let document = toml::parse_document(text).map_err(|error| {
            format!(
                "line {}, column {}: {}",
                error.line, error.column, error.message
            )
        })?;
        let mut policy = Self::default();
        for (key, value) in &document.root {
            match key.as_str() {
                "allow_verbs" => {
                    let names = string_array(key, value)?;
                    for name in &names {
                        if !TOOLS.contains(&name.as_str()) {
                            return Err(format!("`allow_verbs` names unknown tool `{name}`"));
                        }
                    }
                    policy.allow = names;
                }
                "protected_paths" => policy.protected_paths = string_array(key, value)?,
                "max_edits" => policy.max_edits = Some(positive(key, value)? as usize),
                "require_approval" => {
                    let verbs = string_array(key, value)?;
                    for verb in &verbs {
                        if !MUTATING_VERBS.contains(&verb.as_str()) {
                            return Err(format!(
                                "`require_approval` names non-mutating verb `{verb}`"
                            ));
                        }
                    }
                    policy.require_approval = verbs;
                }
                "max_wall_ms" => policy.max_wall_ms = Some(positive(key, value)? as u64),
                "max_tokens" => policy.max_tokens = Some(positive(key, value)? as u64),
                "max_output_bytes" => policy.max_output_bytes = Some(positive(key, value)? as u64),
                "audit_max_bytes" => policy.audit_max_bytes = positive(key, value)? as u64,
                other => return Err(format!("unknown policy key `{other}`")),
            }
        }
        Ok(policy)
    }

    /// Whether `tool` may be called at all.
    pub fn allows(&self, tool: &str) -> bool {
        self.allow.iter().any(|name| name == tool)
    }

    /// The first protected path a request names, if any. The policy file is
    /// always protected: widening the mandate is not a tool call.
    pub fn protected_hit<'a>(&self, paths: &'a [String]) -> Option<&'a str> {
        paths.iter().find_map(|path| {
            let normalized = normalize(path);
            (normalized == ".yashy/policy.toml"
                || self
                    .protected_paths
                    .iter()
                    .any(|pattern| glob_match(pattern, &normalized)))
            .then_some(path.as_str())
        })
    }

    /// The effective policy as JSON, so an agent can read what it may do.
    pub fn to_json(&self) -> Json {
        let allow: Vec<Json> = self.allow.iter().map(|name| Json::string(name)).collect();
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("allow", Json::array(allow));
        root.set(
            "protected_paths",
            Json::array(
                self.protected_paths
                    .iter()
                    .map(|path| Json::string(path))
                    .collect(),
            ),
        );
        root.set(
            "max_edits",
            match self.max_edits {
                Some(limit) => Json::number(limit as i64),
                None => Json::Null,
            },
        );
        root.set(
            "require_approval",
            Json::array(
                self.require_approval
                    .iter()
                    .map(|verb| Json::string(verb))
                    .collect(),
            ),
        );
        let budget_field = |value: Option<u64>| match value {
            Some(number) => Json::number(number as i64),
            None => Json::Null,
        };
        root.set("max_wall_ms", budget_field(self.max_wall_ms));
        root.set("max_tokens", budget_field(self.max_tokens));
        root.set("max_output_bytes", budget_field(self.max_output_bytes));
        root.set("audit_max_bytes", Json::number(self.audit_max_bytes as i64));
        root
    }
}

fn string_array(key: &str, value: &Value) -> Result<Vec<String>, String> {
    let Value::Array(items) = value else {
        return Err(format!("`{key}` must be an array of strings"));
    };
    let mut out = Vec::new();
    for item in items {
        let Value::String(text) = item else {
            return Err(format!("`{key}` must be an array of strings"));
        };
        out.push(text.clone());
    }
    Ok(out)
}

fn positive(key: &str, value: &Value) -> Result<i64, String> {
    let Value::Integer(number) = value else {
        return Err(format!("`{key}` must be a whole number"));
    };
    if *number <= 0 {
        return Err(format!("`{key}` must be positive"));
    }
    Ok(*number)
}

/// A request path as the policy compares it: forward slashes, no leading
/// `./`, and no drive prefix.
pub fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path);
    path.to_string()
}

/// `*` matches any run without `/`; `**` matches any run, including `/`;
/// `?` matches one character. This is the glob a policy author expects.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn matches(pattern: &[u8], text: &[u8]) -> bool {
        match pattern.split_first() {
            None => text.is_empty(),
            Some((b'*', rest)) => {
                if rest.first() == Some(&b'*') {
                    let rest = &rest[1..];
                    // `**/` also matches zero directories.
                    if rest.first() == Some(&b'/') && matches(&rest[1..], text) {
                        return true;
                    }
                    (0..=text.len()).any(|skip| matches(rest, &text[skip..]))
                } else {
                    (0..=text.len())
                        .take_while(|skip| *skip == 0 || text[skip - 1] != b'/')
                        .any(|skip| matches(rest, &text[skip..]))
                }
            }
            Some((b'?', rest)) => match text.split_first() {
                Some((character, tail)) if *character != b'/' => matches(rest, tail),
                _ => false,
            },
            Some((literal, rest)) => match text.split_first() {
                Some((character, tail)) if character == literal => matches(rest, tail),
                _ => false,
            },
        }
    }
    matches(pattern.as_bytes(), text.as_bytes())
}

/// An Ed25519 signature over the entry's sequence number and hash, made with
/// the session key. Unlike the hash chain, this cannot be recomputed by
/// someone who can rewrite the workspace after the session ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditSignature {
    pub key_id: String,
    pub digest: String,
    /// Raw 64-byte signature in hex.
    pub bytes: String,
}

/// One audit entry. The fields are fixed so the hash is over a stable shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    pub seq: u64,
    pub tool: String,
    pub outcome: String,
    pub code: Option<String>,
    pub detail: String,
    pub prev_hash: String,
    pub hash: String,
    pub signature: Option<AuditSignature>,
}

impl AuditEntry {
    /// The bytes the hash covers, in a fixed order with a separator no field
    /// can confuse.
    fn chained(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            self.seq,
            self.tool,
            self.outcome,
            self.code.clone().unwrap_or_default(),
            self.detail,
            self.prev_hash
        )
    }

    pub fn compute_hash(&self) -> String {
        yashy_manifest::sha256_hex(self.chained().as_bytes())
    }

    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("seq", Json::number(self.seq as i64));
        root.set("tool", Json::string(&self.tool));
        root.set("outcome", Json::string(&self.outcome));
        root.set(
            "code",
            match &self.code {
                Some(code) => Json::string(code),
                None => Json::Null,
            },
        );
        root.set("detail", Json::string(&self.detail));
        root.set("prev_hash", Json::string(&self.prev_hash));
        root.set("hash", Json::string(&self.hash));
        root.set(
            "signature",
            match &self.signature {
                Some(signature) => {
                    let mut value = Json::object();
                    value.set("key_id", Json::string(&signature.key_id));
                    value.set("digest", Json::string(&signature.digest));
                    value.set("bytes", Json::string(&signature.bytes));
                    value
                }
                None => Json::Null,
            },
        );
        root
    }

    pub fn from_json(value: &yashy_common::json_rpc::Value) -> Option<AuditEntry> {
        use yashy_common::json_rpc::Value;
        let field = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_string);
        let seq = value.get("seq").and_then(Value::as_i64)?;
        if seq < 0 {
            return None;
        }
        let signature = value.get("signature").and_then(|value| {
            let field = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_string);
            Some(AuditSignature {
                key_id: field("key_id")?,
                digest: field("digest")?,
                bytes: field("bytes")?,
            })
        });
        Some(AuditEntry {
            seq: seq as u64,
            tool: field("tool")?,
            outcome: field("outcome")?,
            code: field("code"),
            detail: field("detail")?,
            prev_hash: field("prev_hash")?,
            hash: field("hash")?,
            signature,
        })
    }
}

/// The hash before the first entry: 32 zero bytes in hex.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The bytes an entry's signature covers: its sequence number and its hash.
/// The hash already binds every visible field, so signing it is enough.
fn signed_bytes(seq: u64, hash: &str) -> String {
    format!("{seq}\n{hash}")
}

/// The session key lives in the environment, never in the workspace: someone
/// with write access to the tree must not be able to replace it along with the
/// log. An unset key leaves older sessions unsigned; a malformed one is a
/// configuration error rather than a silent downgrade.
fn session_signer() -> Result<Option<(String, [u8; 32])>, String> {
    let Some(value) = std::env::var_os("YASHY_AGENT_SEED") else {
        return Ok(None);
    };
    let text = value.to_string_lossy();
    if text.trim().is_empty() {
        return Ok(None);
    }
    let seed = decode_hex_32(&text).ok_or("YASHY_AGENT_SEED must be 64 hex characters")?;
    let key_id = std::env::var("YASHY_AGENT_KEY_ID").unwrap_or_else(|_| "session".to_string());
    Ok(Some((key_id, seed)))
}

fn decode_hex_32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, chunk) in text.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).ok()?;
        out[index] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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

/// Decode the public key passed to `agent audit --verify --key`.
pub fn public_key_from_hex(text: &str) -> Result<Vec<u8>, String> {
    decode_hex(text).ok_or_else(|| "the session public key is not hex".to_string())
}

fn failed(entries: u64, seq: Option<u64>, message: &str) -> AuditVerification {
    AuditVerification {
        entries,
        ok: false,
        first_bad_seq: seq,
        message: message.to_string(),
    }
}

/// The append-only log for one workspace.
#[derive(Clone, Debug)]
pub struct AuditLog {
    directory: PathBuf,
    active: PathBuf,
    max_bytes: u64,
    last_hash: String,
    next_seq: u64,
    /// The session key and its id, read once when the session opens. A log
    /// written without one stays unsigned, which is what older workspaces
    /// contain; verification with a public key refuses those entries.
    signer: Option<(String, [u8; 32])>,
}

/// What a verification found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditVerification {
    pub entries: u64,
    pub ok: bool,
    pub first_bad_seq: Option<u64>,
    pub message: String,
}

impl AuditVerification {
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("entries", Json::number(self.entries as i64));
        root.set("ok", Json::bool(self.ok));
        root.set(
            "first_bad_seq",
            match self.first_bad_seq {
                Some(seq) => Json::number(seq as i64),
                None => Json::Null,
            },
        );
        root.set("message", Json::string(&self.message));
        root.set(
            "code",
            if self.ok {
                Json::Null
            } else {
                Json::string(AUDIT_FAILURE)
            },
        );
        root
    }
}

fn audit_dir(root: &Path) -> PathBuf {
    root.join(".yashy").join("audit")
}

fn active_path(root: &Path) -> PathBuf {
    audit_dir(root).join("session.jsonl")
}

/// Segments oldest first: `session.5.jsonl` ... `session.1.jsonl`, then the
/// active `session.jsonl`. A missing segment is not an error; a gap in the
/// chain is caught by the seq and prev-hash checks.
fn segments(root: &Path) -> Vec<PathBuf> {
    let directory = audit_dir(root);
    let mut rotated: Vec<(u32, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&directory) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let generation = name
                .strip_prefix("session.")
                .and_then(|rest| rest.strip_suffix(".jsonl"))
                .and_then(|number| number.parse::<u32>().ok());
            if let Some(generation) = generation {
                rotated.push((generation, entry.path()));
            }
        }
    }
    rotated.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out: Vec<PathBuf> = rotated.into_iter().map(|(_, path)| path).collect();
    let active = active_path(root);
    if active.exists() {
        out.push(active);
    }
    out
}

fn last_entry(path: &Path) -> Option<AuditEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let last = text.lines().rev().find(|line| !line.trim().is_empty())?;
    let value = yashy_common::json_rpc::parse(last).ok()?;
    AuditEntry::from_json(&value)
}

impl AuditLog {
    /// Open the log under `root`, continuing the chain from the newest entry
    /// in any existing segment.
    pub fn open(root: &Path, max_bytes: u64) -> Result<Self, String> {
        Self::open_with_signer(root, max_bytes, session_signer()?)
    }

    /// The injectable form of [`Self::open`], so a test can sign a session
    /// without changing the process environment.
    pub fn open_with_signer(
        root: &Path,
        max_bytes: u64,
        signer: Option<(String, [u8; 32])>,
    ) -> Result<Self, String> {
        let directory = audit_dir(root);
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        let (last_hash, next_seq) = match segments(root).last().and_then(|path| last_entry(path)) {
            Some(entry) => (entry.hash, entry.seq + 1),
            None => (GENESIS.to_string(), 1),
        };
        Ok(AuditLog {
            directory,
            active: active_path(root),
            max_bytes: max_bytes.max(1),
            last_hash,
            next_seq,
            signer,
        })
    }

    /// Append one entry. Rotation happens before the write that would exceed
    /// the size cap, so no segment is larger than the cap plus one entry.
    pub fn record(
        &mut self,
        tool: &str,
        outcome: &str,
        code: Option<&str>,
        detail: &str,
    ) -> Result<(), String> {
        let detail = redact(detail);
        let mut entry = AuditEntry {
            seq: self.next_seq,
            tool: tool.to_string(),
            outcome: outcome.to_string(),
            code: code.map(str::to_string),
            detail,
            prev_hash: self.last_hash.clone(),
            hash: String::new(),
            signature: None,
        };
        entry.hash = entry.compute_hash();
        if let Some((key_id, seed)) = &self.signer {
            let signed = signature::sign(
                signed_bytes(entry.seq, &entry.hash).as_bytes(),
                key_id,
                seed,
            )
            .map_err(|error| format!("cannot sign an audit entry: {error}"))?;
            entry.signature = Some(AuditSignature {
                key_id: signed.key_id,
                digest: signed.digest,
                bytes: encode_hex(&signed.bytes),
            });
        }
        let line = format!("{}\n", entry.to_json().render_compact());
        let size = std::fs::metadata(&self.active)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if size > 0 && size + line.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.active)
            .map_err(|error| format!("{}: {error}", self.active.display()))?;
        file.write_all(line.as_bytes())
            .map_err(|error| format!("{}: {error}", self.active.display()))?;
        self.last_hash = entry.hash;
        self.next_seq += 1;
        Ok(())
    }

    /// Shift `session.N.jsonl` up by one and start a fresh active segment.
    /// The chain carries: the new segment's first `prev_hash` is the last
    /// entry's hash, which the log already holds.
    fn rotate(&mut self) -> Result<(), String> {
        const KEEP: u32 = 5;
        let oldest = self.directory.join(format!("session.{KEEP}.jsonl"));
        if oldest.exists() {
            std::fs::remove_file(&oldest)
                .map_err(|error| format!("{}: {error}", oldest.display()))?;
        }
        for generation in (1..KEEP).rev() {
            let from = self.directory.join(format!("session.{generation}.jsonl"));
            let to = self
                .directory
                .join(format!("session.{}.jsonl", generation + 1));
            if from.exists() {
                std::fs::rename(&from, &to)
                    .map_err(|error| format!("{}: {error}", from.display()))?;
            }
        }
        if self.active.exists() {
            let first = self.directory.join("session.1.jsonl");
            std::fs::rename(&self.active, &first)
                .map_err(|error| format!("{}: {error}", self.active.display()))?;
        }
        Ok(())
    }

    /// Every entry, oldest segment first and oldest entry first.
    pub fn entries(root: &Path) -> Result<Vec<AuditEntry>, String> {
        let mut out = Vec::new();
        for segment in segments(root) {
            let text = std::fs::read_to_string(&segment)
                .map_err(|error| format!("{}: {error}", segment.display()))?;
            for (index, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let value = yashy_common::json_rpc::parse(line).map_err(|error| {
                    format!(
                        "{}:{}: not one JSON object: {error}",
                        segment.display(),
                        index + 1
                    )
                })?;
                let entry = AuditEntry::from_json(&value).ok_or_else(|| {
                    format!("{}:{}: a field is missing", segment.display(), index + 1)
                })?;
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// Recompute the chain over every segment and name the first entry that
    /// does not belong.
    pub fn verify(root: &Path) -> AuditVerification {
        let entries = match Self::entries(root) {
            Ok(entries) => entries,
            Err(message) => {
                return AuditVerification {
                    entries: 0,
                    ok: false,
                    first_bad_seq: None,
                    message,
                }
            }
        };
        // A rotated log may have pruned the oldest segments. The first entry
        // we still hold is the chain's anchor: its own hash must still match,
        // and its `prev_hash` names an entry that is outside the window.
        let mut expected_seq = entries.first().map_or(1, |entry| entry.seq);
        let mut prev = entries
            .first()
            .map_or(GENESIS.to_string(), |entry| entry.prev_hash.clone());
        let mut seen = 0;
        for entry in &entries {
            seen += 1;
            if entry.seq != expected_seq {
                return AuditVerification {
                    entries: seen,
                    ok: false,
                    first_bad_seq: Some(entry.seq),
                    message: format!(
                        "entry out of order: expected seq {expected_seq}, found {}",
                        entry.seq
                    ),
                };
            }
            if entry.prev_hash != prev {
                return AuditVerification {
                    entries: seen,
                    ok: false,
                    first_bad_seq: Some(entry.seq),
                    message: "entry does not chain to the one before it".to_string(),
                };
            }
            if entry.compute_hash() != entry.hash {
                return AuditVerification {
                    entries: seen,
                    ok: false,
                    first_bad_seq: Some(entry.seq),
                    message: "entry content does not match its hash".to_string(),
                };
            }
            expected_seq += 1;
            prev = entry.hash.clone();
        }
        AuditVerification {
            entries: seen,
            ok: true,
            first_bad_seq: None,
            message: "chain ok".to_string(),
        }
    }

    /// Verify the chain and every session signature against a public key.
    ///
    /// The chain alone proves that no entry was removed or reordered; it does
    /// not prove that a whole chain was not rewritten by someone who can write
    /// the workspace. The signature covers each entry's hash with a key read
    /// from the process environment, so a rewritten chain cannot be resigned.
    pub fn verify_signed(root: &Path, key_id: &str, public_key: &[u8]) -> AuditVerification {
        let chain = Self::verify(root);
        if !chain.ok {
            return chain;
        }
        let mut keys = Keyring::default();
        if let Err(error) = keys.insert(key_id, public_key) {
            return failed(chain.entries, None, &format!("bad session key: {error}"));
        }
        let entries = match Self::entries(root) {
            Ok(entries) => entries,
            Err(message) => return failed(0, None, &message),
        };
        for entry in &entries {
            let Some(signature) = &entry.signature else {
                return failed(
                    chain.entries,
                    Some(entry.seq),
                    "entry has no session signature",
                );
            };
            if signature.key_id != key_id {
                return failed(
                    chain.entries,
                    Some(entry.seq),
                    "entry was signed by another session key",
                );
            }
            let Some(bytes) = decode_hex(&signature.bytes) else {
                return failed(chain.entries, Some(entry.seq), "signature is not hex");
            };
            let parsed = signature::Signature {
                key_id: signature.key_id.clone(),
                algorithm: "Ed25519",
                digest: signature.digest.clone(),
                bytes,
            };
            let signed = signed_bytes(entry.seq, &entry.hash);
            if let Err(error) = signature::verify(signed.as_bytes(), &parsed, &keys) {
                return failed(
                    chain.entries,
                    Some(entry.seq),
                    &format!("session signature: {error}"),
                );
            }
        }
        AuditVerification {
            entries: chain.entries,
            ok: true,
            first_bad_seq: None,
            message: "chain and signatures ok".to_string(),
        }
    }
}

/// Replace the value of a secret-looking key with `<redacted>`. The rule is
/// the trace redactor's: a word that is a known key, then `=` or `:`, then a
/// value. Anything else is left alone.
pub fn redact(detail: &str) -> String {
    const KEYS: [&str; 8] = [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "authorization",
        "credential",
    ];
    let mut out = String::new();
    for word in detail.split_inclusive(char::is_whitespace) {
        let trimmed = word.trim_end_matches(char::is_whitespace);
        let lowercase = trimmed.to_ascii_lowercase();
        let found = KEYS.iter().any(|key| {
            lowercase == *key
                || lowercase.starts_with(&format!("{key}="))
                || lowercase.starts_with(&format!("{key}:"))
        });
        if found {
            out.push_str("<redacted>");
            if word.len() != trimmed.len() {
                out.push(' ');
            }
        } else {
            out.push_str(word);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_policy_is_the_strict_default() {
        let policy = AgentPolicy::default();
        for verb in READ_ONLY_VERBS {
            assert!(policy.allows(verb), "{verb}");
        }
        for verb in ["edit", "refactor", "format", "fix", "run"] {
            assert!(!policy.allows(verb), "{verb}");
        }
    }

    #[test]
    fn an_allow_list_is_the_complete_set() {
        let policy = AgentPolicy::parse("allow_verbs = [\"check\", \"edit\"]\n").unwrap();
        assert!(policy.allows("check"));
        assert!(policy.allows("edit"));
        assert!(!policy.allows("build"));
    }

    #[test]
    fn an_unknown_tool_in_allow_is_refused() {
        let error = AgentPolicy::parse("allow_verbs = [\"shell\"]\n").unwrap_err();
        assert!(error.contains("unknown tool `shell`"), "{error}");
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        let error = AgentPolicy::parse("allowed = [\"check\"]\n").unwrap_err();
        assert!(error.contains("unknown policy key `allowed`"), "{error}");
    }

    #[test]
    fn approval_only_names_mutating_verbs() {
        let policy = AgentPolicy::parse("require_approval = [\"edit\", \"fix\"]\n").unwrap();
        assert_eq!(policy.require_approval, vec!["edit", "fix"]);
        let error = AgentPolicy::parse("require_approval = [\"check\"]\n").unwrap_err();
        assert!(error.contains("non-mutating verb `check`"), "{error}");
    }

    #[test]
    fn protected_paths_match_globs() {
        let policy =
            AgentPolicy::parse("protected_paths = [\"yashy.lock\", \".github/**\"]\n").unwrap();
        assert!(policy
            .protected_hit(&[".github/workflows/ci.yml".to_string()])
            .is_some());
        assert!(policy.protected_hit(&["yashy.lock".to_string()]).is_some());
        assert!(policy.protected_hit(&["src/main.ya".to_string()]).is_none());
        // The policy itself is protected even when no pattern names it.
        assert!(policy
            .protected_hit(&[".yashy/policy.toml".to_string()])
            .is_some());
        assert!(AgentPolicy::default()
            .protected_hit(&[".yashy/policy.toml".to_string()])
            .is_some());
    }

    #[test]
    fn a_star_does_not_cross_a_slash() {
        assert!(glob_match("src/*.ya", "src/main.ya"));
        assert!(!glob_match("src/*.ya", "src/deep/main.ya"));
        assert!(glob_match("src/**/*.ya", "src/deep/main.ya"));
        assert!(glob_match("**/main.ya", "main.ya"));
    }

    #[test]
    fn a_secret_is_redacted_by_key() {
        assert_eq!(redact("token=abc123"), "<redacted>");
        assert_eq!(
            redact("edit main.ya token=abc123 done"),
            "edit main.ya <redacted> done"
        );
        assert_eq!(redact("edit main.ya"), "edit main.ya");
    }

    #[test]
    fn the_chain_verifies_and_a_removed_entry_is_named() {
        let dir = std::env::temp_dir().join("yashy-agent-audit-unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = AuditLog::open(&dir, 1024).unwrap();
        log.record("check", "ok", None, "check main.ya").unwrap();
        log.record("edit", "ok", None, "edit main.ya").unwrap();
        log.record("build", "ok", None, "build main.ya").unwrap();
        assert!(AuditLog::verify(&dir).ok);

        // Remove the middle entry: the third no longer chains.
        let path = active_path(&dir);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        std::fs::write(&path, format!("{}\n{}\n", lines[0], lines[2])).unwrap();
        let verification = AuditLog::verify(&dir);
        assert!(!verification.ok);
        assert_eq!(verification.first_bad_seq, Some(3));
    }

    #[test]
    fn a_signed_chain_refuses_a_rewrite_that_recomputes_the_hash() {
        let dir = std::env::temp_dir().join("yashy-agent-audit-signed");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let seed = [5u8; 32];
        let mut log =
            AuditLog::open_with_signer(&dir, 1024, Some(("session".to_string(), seed))).unwrap();
        log.record("check", "ok", None, "check main.ya").unwrap();
        log.record("edit", "ok", None, "edit main.ya").unwrap();
        let public = signature::public_key(&seed).unwrap();
        let verified = AuditLog::verify_signed(&dir, "session", &public);
        assert!(verified.ok, "{}", verified.message);
        assert_eq!(verified.entries, 2);

        // Someone with write access can recompute a hash. They cannot make a
        // new signature, so verification must still refuse the edited entry.
        let path = active_path(&dir);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let value = yashy_common::json_rpc::parse(lines.last().unwrap()).unwrap();
        let mut entry = AuditEntry::from_json(&value).unwrap();
        entry.detail = "edit other.ya".to_string();
        entry.hash = entry.compute_hash();
        *lines.last_mut().unwrap() = entry.to_json().render_compact();
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        assert!(AuditLog::verify(&dir).ok, "the hash chain was recomputed");
        let bad = AuditLog::verify_signed(&dir, "session", &public);
        assert!(!bad.ok);
        assert_eq!(bad.first_bad_seq, Some(2));
    }

    #[test]
    fn an_unsigned_chain_is_refused_when_a_key_is_expected() {
        let dir = std::env::temp_dir().join("yashy-agent-audit-unsigned");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = AuditLog::open_with_signer(&dir, 1024, None).unwrap();
        log.record("check", "ok", None, "check main.ya").unwrap();
        let public = signature::public_key(&[7u8; 32]).unwrap();
        let bad = AuditLog::verify_signed(&dir, "session", &public);
        assert!(!bad.ok);
        assert_eq!(bad.first_bad_seq, Some(1));
    }

    #[test]
    fn rotation_keeps_the_chain_continuous() {
        let dir = std::env::temp_dir().join("yashy-agent-audit-rotation");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = AuditLog::open(&dir, 1000).unwrap();
        for index in 0..12 {
            log.record("check", "ok", None, &format!("check main.ya pass {index}"))
                .unwrap();
        }
        assert!(segments(&dir).len() > 1, "the log must rotate");
        let verification = AuditLog::verify(&dir);
        assert!(verification.ok, "{:?}", verification);
        assert_eq!(verification.entries, 12);
    }

    #[test]
    fn a_pruned_log_verifies_the_retained_window() {
        let dir = std::env::temp_dir().join("yashy-agent-audit-pruned");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = AuditLog::open(&dir, 240).unwrap();
        for index in 0..12 {
            log.record("check", "ok", None, &format!("check main.ya pass {index}"))
                .unwrap();
        }
        let entries = AuditLog::entries(&dir).unwrap();
        assert_eq!(entries.len(), 6, "only five rotated segments are kept");
        assert_eq!(entries[0].seq, 7);
        let verification = AuditLog::verify(&dir);
        assert!(verification.ok, "{:?}", verification);
        assert_eq!(verification.entries, 6);
    }
}
