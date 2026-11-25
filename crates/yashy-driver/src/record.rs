//! Versioned execution traces for `yashyc record` / `yashyc replay`.
//!
//! A trace pins the program hash and target triple. Replay refuses a trace
//! whose header does not match the source it is given; it never approximates a
//! decision the recorder did not see.

use yashy_common::json_rpc::{number, object, parse, string, to_string, Value};

#[derive(Clone, Debug)]
pub struct Trace {
    pub program_hash: String,
    pub target: String,
    pub events: Vec<String>,
}

/// A stable FNV-1a hash of the program text, as lowercase hex.
pub fn program_hash(source: &str) -> String {
    let mut value: u64 = 0xcbf29ce484222325;
    for byte in source.as_bytes() {
        value ^= *byte as u64;
        value = value.wrapping_mul(0x100000001b3);
    }
    format!("{value:016x}")
}

/// The target triple of the running host, used as a portable header field.
pub fn target() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// Write a trace file. Return addresses are normalised by the caller so a
/// trace never depends on address-space layout.
pub fn write(program_hash: &str, events: &[String], path: &std::path::Path) -> Result<(), String> {
    let rendered: Vec<Value> = events.iter().map(|line| string(line.as_str())).collect();
    let value = object([
        ("schema_version", number(1)),
        ("tool", string("record")),
        ("program_hash", string(program_hash)),
        ("target", string(target())),
        ("events", Value::Arr(rendered)),
    ]);
    std::fs::write(path, format!("{}\n", to_string(&value))).map_err(|e| e.to_string())
}

/// Read and validate a trace file.
pub fn read(path: &std::path::Path) -> Result<Trace, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value = parse(&text).map_err(|e| e)?;
    let program_hash = match value.get("program_hash") {
        Some(Value::Str(text)) => text.clone(),
        _ => return Err("trace has no program_hash".to_string()),
    };
    let target = match value.get("target") {
        Some(Value::Str(text)) => text.clone(),
        _ => return Err("trace has no target".to_string()),
    };
    let events = match value.get("events") {
        Some(Value::Arr(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::Str(text) => Some(text.clone()),
                _ => None,
            })
            .collect(),
        _ => return Err("trace has no events".to_string()),
    };
    Ok(Trace { program_hash, target, events })
}

/// Replay checks the header before it trusts any event.
pub fn validate(trace: &Trace, source: &str) -> Result<(), String> {
    let actual = program_hash(source);
    if actual != trace.program_hash {
        return Err(format!(
            "program hash mismatch: trace has {}, source has {actual}",
            trace.program_hash
        ));
    }
    if trace.target != target() {
        return Err(format!(
            "target mismatch: trace has {}, host is {}",
            trace.target,
            target()
        ));
    }
    Ok(())
}

/// Normalise a raw runtime trace line by replacing address-shaped tokens.
pub fn normalise(line: &str) -> String {
    line.split_whitespace()
        .map(|token| {
            let head = token.split(',').next().unwrap_or(token);
            if head.len() > 4 && head.chars().all(|c| c.is_ascii_hexdigit()) {
                format!("ADDR{}", &token[head.len()..])
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn summary(trace: &Trace) -> String {
    format!(
        "trace: {} events, program {}, target {}",
        trace.events.len(),
        trace.program_hash,
        trace.target
    )
}
