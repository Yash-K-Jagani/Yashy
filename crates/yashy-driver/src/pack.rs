//! Structured context packs: excerpts of the workspace, not a prompt.
//!
//! The caller decides what to do with a pack. This module only chooses which
//! excerpts of compiler-owned source are relevant, orders them deterministically,
//! and cuts them to a token budget. What did not fit is named with a reason, so
//! a small budget is visible rather than silently lossy. Every excerpt carries
//! the file, the byte span and a hash of its own text, and the pack names the
//! revision of the model it was cut from.

use std::path::Path;

use yashy_common::Json;

use crate::context::estimate_tokens;
use crate::edit::hash_bytes;
use crate::index::{build_index, IndexModel};

/// The five ways an agent can ask for context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackMode {
    Focus,
    Diff,
    Tests,
    Docs,
    Error,
}

impl PackMode {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "focus" => Some(Self::Focus),
            "diff" => Some(Self::Diff),
            "tests" => Some(Self::Tests),
            "docs" => Some(Self::Docs),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Diff => "diff",
            Self::Tests => "tests",
            Self::Docs => "docs",
            Self::Error => "error",
        }
    }
}

/// What the caller wants, in compiler terms rather than prompt text.
#[derive(Clone, Debug)]
pub struct PackRequest {
    pub focus: String,
    pub mode: PackMode,
    pub budget: usize,
    pub changed: Vec<String>,
    pub diagnostic: Option<String>,
}

/// One excerpt of one file, with the bytes it came from.
#[derive(Clone, Debug)]
pub struct Excerpt {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub hash: String,
    pub reason: String,
}

/// A ranked candidate that did not fit the budget.
#[derive(Clone, Debug)]
pub struct Dropped {
    pub file: String,
    pub reason: String,
}

/// The pack: a value the caller can render however it needs to.
#[derive(Clone, Debug)]
pub struct Pack {
    pub mode: PackMode,
    pub focus: String,
    pub revision: String,
    pub excerpts: Vec<Excerpt>,
    pub signatures: Vec<String>,
    pub diagnostics: Vec<String>,
    pub token_count: usize,
    pub budget: usize,
    pub dropped: Vec<Dropped>,
}

impl Pack {
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("mode", Json::string(self.mode.as_str()));
        root.set("focus", Json::string(&self.focus));
        root.set("revision", Json::string(&self.revision));
        root.set("tokens", Json::number(self.token_count as i64));
        root.set("budget", Json::number(self.budget as i64));
        root.set(
            "excerpts",
            Json::array(
                self.excerpts
                    .iter()
                    .map(|excerpt| {
                        let mut entry = Json::object();
                        entry.set("file", Json::string(&excerpt.file));
                        entry.set("start", Json::number(excerpt.start as i64));
                        entry.set("end", Json::number(excerpt.end as i64));
                        entry.set("text", Json::string(&excerpt.text));
                        entry.set("hash", Json::string(&excerpt.hash));
                        entry.set("reason", Json::string(&excerpt.reason));
                        entry
                    })
                    .collect(),
            ),
        );
        root.set(
            "signatures",
            Json::array(
                self.signatures
                    .iter()
                    .map(|signature| Json::string(signature))
                    .collect(),
            ),
        );
        root.set(
            "diagnostics",
            Json::array(
                self.diagnostics
                    .iter()
                    .map(|diagnostic| Json::string(diagnostic))
                    .collect(),
            ),
        );
        root.set(
            "dropped",
            Json::array(
                self.dropped
                    .iter()
                    .map(|dropped| {
                        let mut entry = Json::object();
                        entry.set("file", Json::string(&dropped.file));
                        entry.set("reason", Json::string(&dropped.reason));
                        entry
                    })
                    .collect(),
            ),
        );
        root
    }
}

/// A stable id for the model the pack was cut from. Two packs with the same
/// revision describe the same files, symbols, imports and tests.
fn revision_of(model: &IndexModel) -> String {
    let mut text = String::new();
    for file in &model.files {
        text.push_str("file:");
        text.push_str(file);
        text.push('\n');
    }
    for symbol in &model.symbols {
        text.push_str(&format!(
            "symbol:{}:{}:{}:{}\n",
            symbol.name, symbol.file, symbol.span.start, symbol.span.end
        ));
    }
    for import in &model.imports {
        text.push_str(&format!(
            "import:{}:{}\n",
            import.from,
            import.path.join(".")
        ));
    }
    for test in &model.tests {
        text.push_str(&format!("test:{}:{}\n", test.file, test.calls.join(",")));
    }
    hash_bytes(text.as_bytes())
}

fn file_text(root: &Path, file: &str) -> String {
    let path = root.join(file);
    std::fs::read_to_string(&path)
        .or_else(|_| std::fs::read_to_string(file))
        .unwrap_or_default()
}

/// The whole line a byte span sits on, so an excerpt is readable source.
fn lines_at(text: &str, start: u32, end: u32) -> (usize, usize, String) {
    let start = (start as usize).min(text.len());
    let end = (end as usize).min(text.len()).max(start);
    let from = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let to = text[end..].find('\n').map_or(text.len(), |at| end + at + 1);
    (from, to, text[from..to].to_string())
}

fn excerpt(root: &Path, file: &str, start: u32, end: u32, reason: &str) -> Option<Excerpt> {
    let text = file_text(root, file);
    if text.is_empty() {
        return None;
    }
    let (from, _, _) = lines_at(&text, start, end);
    // Enough trailing bytes for a normal function body without carrying the
    // whole workspace into a focused pack. The budget still applies on top.
    let tail = (end as usize + 800).min(text.len());
    let to = text[tail..]
        .find('\n')
        .map_or(text.len(), |at| tail + at + 1);
    let body = text[from..to].to_string();
    Some(Excerpt {
        file: file.to_string(),
        start: from,
        end: to,
        hash: hash_bytes(body.as_bytes()),
        text: body,
        reason: reason.to_string(),
    })
}

/// The candidates for one request, in fixed relevance order.
fn candidates(root: &Path, model: &IndexModel, req: &PackRequest) -> Vec<Excerpt> {
    let mut out = Vec::new();
    let target = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == req.focus || req.focus.ends_with(&symbol.name));
    let covering: Vec<&crate::index::TestProgram> = model
        .tests
        .iter()
        .filter(|test| {
            target.is_some_and(|target| test.calls.iter().any(|call| call == &target.name))
        })
        .collect();

    if let Some(diagnostic) = &req.diagnostic {
        out.push(Excerpt {
            file: "diagnostic".to_string(),
            start: 0,
            end: diagnostic.len(),
            text: format!("{diagnostic}\n"),
            hash: hash_bytes(diagnostic.as_bytes()),
            reason: "the diagnostic that caused this pack".to_string(),
        });
    }
    if let Some(target) = target {
        if let Some(piece) = excerpt(
            root,
            &target.file,
            target.span.start,
            target.span.end,
            "the focus symbol",
        ) {
            out.push(piece);
        }
    }
    match req.mode {
        PackMode::Focus => {
            for test in &covering {
                let text = file_text(root, &test.file);
                out.push(Excerpt {
                    file: test.file.clone(),
                    start: 0,
                    end: text.len(),
                    hash: hash_bytes(text.as_bytes()),
                    text: format!("{text}\n"),
                    reason: "a test that calls the focus symbol".to_string(),
                });
            }
        }
        PackMode::Tests => {
            for test in &covering {
                let text = file_text(root, &test.file);
                out.push(Excerpt {
                    file: test.file.clone(),
                    start: 0,
                    end: text.len(),
                    hash: hash_bytes(text.as_bytes()),
                    text: format!("{text}\n"),
                    reason: "a test that calls the focus symbol".to_string(),
                });
            }
        }
        PackMode::Docs => {
            for symbol in &model.symbols {
                if let Some(docs) = &symbol.docs {
                    out.push(Excerpt {
                        file: symbol.file.clone(),
                        start: symbol.span.start as usize,
                        end: symbol.span.end as usize,
                        text: format!("{}: {docs}\n", symbol.name),
                        hash: hash_bytes(docs.as_bytes()),
                        reason: "documentation on a symbol in the workspace".to_string(),
                    });
                }
            }
        }
        PackMode::Error => {
            // The diagnostic itself is first; the focus and its docs explain
            // the code it names. A rule page is a separate CLI view, not a
            // source excerpt, so this pack never invents one.
            if let (Some(symbol), Some(docs)) = (target, target.and_then(|t| t.docs.as_ref())) {
                out.push(Excerpt {
                    file: symbol.file.clone(),
                    start: symbol.span.start as usize,
                    end: symbol.span.end as usize,
                    text: format!("docs: {docs}\n"),
                    hash: hash_bytes(docs.as_bytes()),
                    reason: "documentation for the symbol in the diagnostic".to_string(),
                });
            }
        }
        PackMode::Diff => {
            for changed in &req.changed {
                let text = file_text(root, changed);
                if !text.is_empty() {
                    out.push(Excerpt {
                        file: changed.clone(),
                        start: 0,
                        end: text.len(),
                        hash: hash_bytes(text.as_bytes()),
                        text: format!("{text}\n"),
                        reason: "a changed file".to_string(),
                    });
                }
                for test in &model.tests {
                    if test.calls.iter().any(|call| {
                        model
                            .symbols
                            .iter()
                            .any(|symbol| symbol.file == *changed && symbol.name == *call)
                    }) {
                        let text = file_text(root, &test.file);
                        out.push(Excerpt {
                            file: test.file.clone(),
                            start: 0,
                            end: text.len(),
                            hash: hash_bytes(text.as_bytes()),
                            text: format!("{text}\n"),
                            reason: "a caller of a changed function".to_string(),
                        });
                    }
                }
            }
        }
    }
    out
}

/// Build the pack for `req` over the workspace at `root`.
pub fn context_pack(root: &Path, req: &PackRequest) -> Result<Pack, String> {
    let model = build_index(root)?;
    if model.symbols.is_empty() {
        return Err(format!(
            "the focus `{}` names nothing in the workspace",
            req.focus
        ));
    }
    let revision = revision_of(&model);
    let focus_file = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == req.focus || req.focus.ends_with(&symbol.name))
        .map(|symbol| symbol.file.clone());
    let mut signatures: Vec<String> = model
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.name == req.focus
                || req.focus.ends_with(&symbol.name)
                || (matches!(req.mode, PackMode::Focus | PackMode::Error)
                    && focus_file.as_deref() == Some(symbol.file.as_str()))
        })
        .map(|symbol| symbol.signature.clone())
        .collect();
    signatures.sort();
    signatures.dedup();
    let mut candidates = candidates(root, &model, req);
    candidates.sort_by(|a, b| (&a.file, a.start, &a.reason).cmp(&(&b.file, b.start, &b.reason)));
    let mut pack = Pack {
        mode: req.mode,
        focus: req.focus.clone(),
        revision,
        excerpts: Vec::new(),
        signatures,
        diagnostics: req.diagnostic.clone().into_iter().collect(),
        token_count: 0,
        budget: req.budget,
        dropped: Vec::new(),
    };
    for candidate in candidates {
        let cost = estimate_tokens(&candidate.text) + 4;
        if pack.token_count + cost <= req.budget {
            pack.token_count += cost;
            pack.excerpts.push(candidate);
        } else {
            pack.dropped.push(Dropped {
                file: candidate.file,
                reason: format!(
                    "{} (over the {} token budget)",
                    candidate.reason, req.budget
                ),
            });
        }
    }
    Ok(pack)
}
