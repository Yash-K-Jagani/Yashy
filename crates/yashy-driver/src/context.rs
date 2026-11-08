//! `yashy context`: a token-budgeted pack for an agent.
//!
//! The pack answers one task ("fix the leak in file_cli") with the
//! compiler's own facts, added in a fixed relevance order until the next
//! section would exceed the budget. What did not fit is named in `dropped`,
//! never silently truncated away.
//!
//! The token estimator is documented and versioned: one token per four
//! bytes of ASCII, one per non-ASCII character, rounded up.

use std::path::Path;

use yashy_common::Json;

use crate::index::{build_index, IndexModel};

/// One section of the pack, in relevance order.
#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    pub body: String,
}

/// The pack: sections that fit, and the names of the ones that did not.
#[derive(Clone, Debug, Default)]
pub struct ContextPack {
    pub task: String,
    pub budget: usize,
    pub sections: Vec<Section>,
    pub dropped: Vec<String>,
}

impl ContextPack {
    /// The pack's own token count, by the versioned estimator.
    pub fn tokens(&self) -> usize {
        estimate_tokens(&self.render_markdown())
    }

    pub fn render_markdown(&self) -> String {
        let mut out = format!("# Context: {}\n", self.task);
        for section in &self.sections {
            out.push_str(&format!("\n## {}\n\n{}", section.name, section.body));
            if !section.body.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }

    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashy"));
        root.set("task", Json::string(&self.task));
        root.set("tokens", Json::number(self.tokens() as i64));
        root.set("budget", Json::number(self.budget as i64));
        root.set(
            "included",
            Json::array(
                self.sections
                    .iter()
                    .map(|section| Json::string(&section.name))
                    .collect(),
            ),
        );
        root.set(
            "dropped",
            Json::array(
                self.dropped
                    .iter()
                    .map(|dropped| Json::string(dropped))
                    .collect(),
            ),
        );
        root
    }
}

/// Estimate the token count of `text`: ceil(bytes/4) for ASCII, plus one
/// per non-ASCII character (which is also its byte count in UTF-8, so this
/// is the same number computed in two ways).
pub fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut bytes = 0usize;
    for character in text.chars() {
        if character.is_ascii() {
            bytes += 1;
        } else {
            if bytes > 0 {
                tokens += bytes.div_ceil(4);
                bytes = 0;
            }
            tokens += 1;
        }
    }
    tokens + bytes.div_ceil(4)
}

/// The relevance order: diagnostic, enclosing function, direct callees,
/// covering tests, stdlib signatures. A fixed table, not a model call.
fn sections_for(model: &IndexModel, task: &str, diagnostic: Option<&str>) -> Vec<Section> {
    let mut sections = Vec::new();
    // The symbol the task names: the word after "fix the ... in" is a guess,
    // so instead find the symbol whose name appears in the task.
    let target = model
        .symbols
        .iter()
        .find(|symbol| task.contains(&symbol.name));
    if let Some(diagnostic) = diagnostic {
        sections.push(Section {
            name: "diagnostic".to_string(),
            body: diagnostic.to_string(),
        });
    }
    if let Some(target) = target {
        let callees: Vec<String> = model
            .symbols
            .iter()
            .filter(|symbol| symbol.file == target.file && symbol.name != target.name)
            .map(|symbol| symbol.signature.clone())
            .collect();
        sections.push(Section {
            name: "enclosing function".to_string(),
            body: format!("{}\n", target.signature),
        });
        if !callees.is_empty() {
            sections.push(Section {
                name: "callees".to_string(),
                body: callees.join("\n") + "\n",
            });
        }
        let covering: Vec<String> = model
            .tests
            .iter()
            .filter(|test| test.calls.iter().any(|call| call == &target.name))
            .map(|test| test.file.clone())
            .collect();
        if !covering.is_empty() {
            sections.push(Section {
                name: "tests".to_string(),
                body: covering.join("\n") + "\n",
            });
        }
    }
    sections
}

/// Build the pack for `task` over the model at `root`, fitting `budget`.
pub fn build_context(
    root: &Path,
    task: &str,
    budget: usize,
    diagnostic: Option<&str>,
) -> Result<ContextPack, String> {
    let model = build_index(root)?;
    if model.symbols.is_empty() {
        return Err(format!("the task `{task}` names nothing in the workspace"));
    }
    let candidates = sections_for(&model, task, diagnostic);
    let mut pack = ContextPack {
        task: task.to_string(),
        budget,
        sections: Vec::new(),
        dropped: Vec::new(),
    };
    for section in candidates {
        let mut trial = pack.clone();
        trial.sections.push(section.clone());
        if trial.tokens() <= budget {
            pack.sections.push(section);
        } else {
            pack.dropped.push(section.name);
        }
    }
    Ok(pack)
}
