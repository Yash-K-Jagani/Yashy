//! Structured edits: the only way an agent changes the workspace.
//!
//! An [`Edit`] names a byte span, the hash of the file it was computed from,
//! and the replacement. A [`Patch`] is all-or-nothing: every hash and span is
//! checked before anything is written, the whole tree is re-checked afterwards,
//! and a failing check restores the original bytes. There is no
//! write-anything tool and no shell.
//!
//! The hash is FNV-1a over the file bytes, spelled as a decimal string. It is a
//! staleness check, not a signature: the workspace has no hashing dependency,
//! and an agent cannot gain permission by choosing a different digest.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use yashy_common::Span;

/// A single replacement in one workspace-relative file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub file: String,
    pub span: Span,
    pub expected_hash: String,
    pub replacement: String,
    pub rationale: String,
}

/// Several edits applied as one transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch {
    pub edits: Vec<Edit>,
    pub expected_tree_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    OutsideWorkspace(String),
    StaleHash(String),
    StaleTree,
    InvalidSpan(String),
    Overlapping(String),
    CheckFailed(Vec<String>),
    Io(String),
}

impl EditError {
    pub fn message(&self) -> String {
        match self {
            EditError::OutsideWorkspace(path) => format!("`{path}` is outside the workspace"),
            EditError::StaleHash(file) => format!("`{file}` changed since the edit was computed"),
            EditError::StaleTree => "the workspace changed since the patch was computed".into(),
            EditError::InvalidSpan(file) => format!("the span is outside `{file}`"),
            EditError::Overlapping(file) => format!("two edits overlap in `{file}`"),
            EditError::CheckFailed(diagnostics) => {
                format!(
                    "the patched tree does not check: {}",
                    diagnostics.join("; ")
                )
            }
            EditError::Io(message) => message.clone(),
        }
    }
}

/// FNV-1a over bytes, as a decimal string.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash.to_string()
}

/// Hash one file's contents.
pub fn file_hash(path: &Path) -> Result<String, EditError> {
    let bytes = std::fs::read(path).map_err(|error| EditError::Io(error.to_string()))?;
    Ok(hash_bytes(&bytes))
}

fn relative_path(root: &Path, file: &str) -> Result<PathBuf, EditError> {
    if file.is_empty() || Path::new(file).is_absolute() {
        return Err(EditError::OutsideWorkspace(file.to_string()));
    }
    if Path::new(file)
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(EditError::OutsideWorkspace(file.to_string()));
    }
    let root = root
        .canonicalize()
        .map_err(|error| EditError::Io(error.to_string()))?;
    let path = root.join(file);
    let parent = path
        .parent()
        .unwrap_or(&root)
        .canonicalize()
        .map_err(|error| EditError::Io(error.to_string()))?;
    if !parent.starts_with(&root) {
        return Err(EditError::OutsideWorkspace(file.to_string()));
    }
    Ok(path)
}

/// Hash the whole tree: one stable row per source file, sorted by path.
pub fn tree_hash(root: &Path) -> Result<String, EditError> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
                    continue;
                }
                collect(&path, out)?;
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("ya") {
                out.push(path);
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    collect(root, &mut files).map_err(|error| EditError::Io(error.to_string()))?;
    files.sort();
    let mut rows = BTreeMap::new();
    for file in files {
        let relative = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        rows.insert(relative, file_hash(&file)?);
    }
    let mut text = String::new();
    for (file, hash) in rows {
        text.push_str(&file);
        text.push('\u{0}');
        text.push_str(&hash);
        text.push('\n');
    }
    Ok(hash_bytes(text.as_bytes()))
}

/// Validate a patch against the tree without writing anything.
pub fn validate(root: &Path, patch: &Patch) -> Result<BTreeMap<String, String>, EditError> {
    if tree_hash(root)? != patch.expected_tree_hash {
        return Err(EditError::StaleTree);
    }
    let mut contents: BTreeMap<String, String> = BTreeMap::new();
    let mut spans: BTreeMap<&str, Vec<Span>> = BTreeMap::new();
    for edit in &patch.edits {
        let path = relative_path(root, &edit.file)?;
        let text = contents
            .entry(edit.file.clone())
            .or_insert_with(|| std::fs::read_to_string(&path).unwrap_or_default());
        if file_hash(&path)? != edit.expected_hash {
            return Err(EditError::StaleHash(edit.file.clone()));
        }
        let end = edit.span.end as usize;
        if edit.span.start as usize > end || end > text.len() {
            return Err(EditError::InvalidSpan(edit.file.clone()));
        }
        let taken = spans.entry(&edit.file).or_default();
        if taken
            .iter()
            .any(|other| edit.span.start < other.end && other.start < edit.span.end)
        {
            return Err(EditError::Overlapping(edit.file.clone()));
        }
        taken.push(edit.span);
    }
    for (file, edits) in &patch.edits.iter().fold(BTreeMap::new(), |mut map, edit| {
        map.entry(edit.file.clone())
            .or_insert_with(Vec::new)
            .push(edit.clone());
        map
    }) {
        let text = contents.get_mut(file).expect("validated above");
        let mut sorted: Vec<&Edit> = edits.iter().collect();
        sorted.sort_by_key(|edit| edit.span.start);
        let mut result = String::with_capacity(text.len());
        let mut cursor = 0usize;
        for edit in sorted {
            let start = edit.span.start as usize;
            result.push_str(&text[cursor..start]);
            result.push_str(&edit.replacement);
            cursor = edit.span.end as usize;
        }
        result.push_str(&text[cursor..]);
        *text = result;
    }
    Ok(contents)
}

/// Apply a patch atomically, re-checking the changed files before committing.
///
/// `check` receives the path and the new text for each changed file and returns
/// the diagnostics that must prevent the write. A failure restores the bytes it
/// already wrote, so a refusal leaves the tree byte-identical.
pub fn apply_patch<F>(root: &Path, patch: &Patch, mut check: F) -> Result<Vec<String>, EditError>
where
    F: FnMut(&Path, &str) -> Result<(), Vec<String>>,
{
    let changed = validate(root, patch)?;
    let mut originals: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let result: Result<(), EditError> = (|| {
        for (file, text) in &changed {
            let path = relative_path(root, file)?;
            let bytes = std::fs::read(&path).map_err(|error| EditError::Io(error.to_string()))?;
            originals.push((path.clone(), bytes));
            std::fs::write(&path, text).map_err(|error| EditError::Io(error.to_string()))?;
        }
        let mut diagnostics = Vec::new();
        for (file, text) in &changed {
            let path = relative_path(root, file)?;
            if let Err(found) = check(&path, text) {
                diagnostics.extend(found);
            }
        }
        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(EditError::CheckFailed(diagnostics))
        }
    })();
    if result.is_err() {
        for (path, bytes) in originals.into_iter().rev() {
            let _ = std::fs::write(path, bytes);
        }
    }
    result.map(|_| changed.keys().cloned().collect())
}

/// An edit that replaces every token named `from` in `text` with `to`.
///
/// Spans come from the lexer, so a string literal or a comment carrying the same
/// word is never renamed. This is the one refactor offered through MCP: it is
/// computed from the token stream and verified by the same re-check as any
/// other patch.
pub fn rename_tokens(
    text: &str,
    from: &str,
    to: &str,
    rationale: &str,
) -> Result<Vec<Edit>, EditError> {
    let tokens = yashy_lexer::Lexer::new(text)
        .tokenize()
        .map_err(|diagnostics| {
            EditError::CheckFailed(diagnostics.iter().map(|d| d.message.clone()).collect())
        })?;
    let mut edits = Vec::new();
    for token in tokens {
        if !matches!(token.kind, yashy_lexer::TokenKind::Ident) {
            continue;
        }
        let start = token.span.start as usize;
        let end = token.span.end as usize;
        if end <= text.len() && &text[start..end] == from {
            edits.push(Edit {
                file: String::new(),
                span: token.span,
                expected_hash: String::new(),
                replacement: to.to_string(),
                rationale: rationale.to_string(),
            });
        }
    }
    Ok(edits)
}
