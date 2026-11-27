#![forbid(unsafe_code)]
//! The Yashy SDK: the compiler's verbs as library calls, not a second pipeline.
//!
//! Every function here is a thin adapter over `yashy_driver`. The CLI, the LSP,
//! the MCP server and tests can all reach the same facts through the same
//! driver functions; this crate adds no parser, no checker and no code
//! generator of its own. Values are returned as JSON where the CLI prints JSON,
//! so a client and the command line cannot drift.

use std::path::{Path, PathBuf};

use yashy_common::{Json, SourceMap};
use yashy_verify::{ContractMode, VerifyOptions};

/// The result of a check: JSON in the same schema as `yashyc check --json`.
pub struct CheckResult {
    pub success: bool,
    pub diagnostics: Vec<yashy_common::Diagnostic>,
}

impl CheckResult {
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("success", Json::bool(self.success));
        root.set(
            "diagnostics",
            Json::array(
                self.diagnostics
                    .iter()
                    .map(|diagnostic| crate::diagnostic_json(diagnostic))
                    .collect(),
            ),
        );
        root
    }
}

fn diagnostic_json(diagnostic: &yashy_common::Diagnostic) -> Json {
    let mut entry = Json::object();
    entry.set(
        "severity",
        Json::string(format!("{:?}", diagnostic.severity).to_ascii_lowercase()),
    );
    entry.set(
        "code",
        match diagnostic.code {
            Some(code) => Json::string(code),
            None => Json::Null,
        },
    );
    entry.set("message", Json::string(&diagnostic.message));
    entry
}

/// Check and verify one source file, with the same options a build uses.
pub fn check(path: &Path, source: &str, enforce_contracts: bool) -> CheckResult {
    let mut sources = SourceMap::new();
    let options = VerifyOptions {
        contracts: if enforce_contracts {
            ContractMode::Enforce
        } else {
            ContractMode::Warn
        },
    };
    match yashy_driver::pipeline::check_and_verify(
        &mut sources,
        path,
        source.to_string(),
        false,
        &options,
        1,
    ) {
        Ok((_, report)) => CheckResult {
            success: report.error_count() == 0,
            diagnostics: report.diagnostics,
        },
        Err(diagnostics) => CheckResult {
            success: false,
            diagnostics,
        },
    }
}

/// Verify a source with contracts enforced. Same pipeline as `check`, with a
/// different promise mode; there is no second verifier.
pub fn verify(path: &Path, source: &str) -> CheckResult {
    check(path, source, true)
}

/// Build one source file into an executable at `out`.
pub fn build(
    source: &str,
    entry_name: &str,
    out: &Path,
    options: yashy_driver::pipeline::BuildOptions,
) -> Result<(), Vec<yashy_common::Diagnostic>> {
    let toolchain = yashy_driver::toolchain::detect().map_err(|error| {
        vec![yashy_common::Diagnostic::new(
            yashy_common::Severity::Error,
            yashy_common::FileId::from_index(0),
            yashy_common::Span::SYNTHETIC,
            error.to_string(),
        )]
    })?;
    yashy_driver::pipeline::compile_to_binary(source, entry_name, out, &toolchain, options)
}

/// The compiler's symbol index for a workspace, as JSON.
pub fn index(root: &Path) -> Result<Json, String> {
    yashy_driver::index::build_index(root).map(|model| yashy_driver::index::to_json(&model))
}

/// Semantic review against an index snapshot, as JSON.
pub fn review(root: &Path, baseline: &str) -> Result<Json, String> {
    yashy_driver::review::review(root, baseline).map(|review| review.to_json())
}

/// Impact from a review, as JSON.
pub fn impact(root: &Path, baseline: &str) -> Result<Json, String> {
    let review = yashy_driver::review::review(root, baseline)?;
    yashy_driver::impact::impact(root, baseline, &review).map(|impact| impact.to_json())
}

/// A token-budgeted context pack, as JSON.
pub fn context(root: &Path, task: &str, budget: usize) -> Result<Json, String> {
    yashy_driver::context::build_context(root, task, budget, None).map(|pack| pack.to_json())
}

/// The catalog entry for a code, as JSON, or `None` when the code is unknown.
pub fn explain(code: &str) -> Option<Json> {
    let entry = yashy_common::codes::find(code)?;
    let mut root = Json::object();
    root.set("schema_version", Json::number(1));
    root.set("tool", Json::string("yashyc"));
    root.set("code", Json::string(entry.id));
    root.set("title", Json::string(entry.title));
    root.set("summary", Json::string(entry.summary));
    root.set(
        "rule",
        Json::string(yashy_common::codes::rule_for(entry.id)),
    );
    root.set(
        "docs_url",
        Json::string(yashy_common::codes::docs_url(entry.id)),
    );
    Some(root)
}

/// The workspace root for a path: the nearest ancestor with `yashy.toml`.
pub fn workspace_root(path: &Path) -> PathBuf {
    let mut at = path.to_path_buf();
    loop {
        if at.join("yashy.toml").is_file() {
            return at;
        }
        if !at.pop() {
            return path.to_path_buf();
        }
    }
}
