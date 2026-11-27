//! The Yashy compiler driver, usable both as a library and as a binary so that
//! integration tests can call into it directly.
//!
//! `modules` finds and loads imported files, `pipeline` owns the whole build —
//! lex, parse, check, emit IR, invoke the toolchain — and `toolchain` is the
//! only place that knows how to find `clang`. No argument parsing lives here:
//! that is `main.rs`, so the library surface stays a set of functions a test
//! can call with plain values.

pub mod addr;
pub mod affected;
pub mod advisory;
pub mod agent;
pub mod audit;
pub mod approval;
pub mod bench;
pub mod capability_check;
pub mod cfg;
pub mod cli_spec;
pub mod context;
pub mod conformance;
pub mod coverage;
pub mod daemon;
pub mod daemon_protocol;
pub mod debug;
pub mod debug_info;
pub mod doc;
pub mod doc_server;
pub mod edit;
pub mod features;
pub mod fetch;
pub mod fix;
pub mod fuzz;
pub mod grammar;
pub mod impact;
pub mod index;
pub mod layout;
pub mod lint;
pub mod mcp;
pub mod migrate;
pub mod modules;
pub mod opt_report;
pub mod outline;
pub mod pack;
pub mod pipeline;
pub mod pkg_cmd;
pub mod playground;
pub mod policy;
pub mod profile;
pub mod registry;
pub mod repl;
pub mod resolve;
pub mod review;
pub mod sample;
pub mod sandbox;
pub mod sarif;
pub mod scaffold;
pub mod target;
pub mod task;
pub mod stack_report;
pub mod term;
pub mod test_harness;
pub mod tokens;
pub mod toolchain;
pub mod trace;
pub mod bench_measure;
pub mod differential;
pub mod dev;
pub mod openapi;
pub mod record;
pub mod versioned;
pub mod watch;
pub mod workspace;

use yashy_common::{Diagnostic, FileId, Json, SourceMap, Span};

/// Render one diagnostic as a JSON object for schema v1 output.
pub(crate) fn diagnostics_json(sources: &SourceMap, diagnostics: &[Diagnostic]) -> Vec<Json> {
    diagnostics
        .iter()
        .map(|d| diagnostic_json(sources, d))
        .collect()
}

pub fn diagnostic_json(sources: &SourceMap, diagnostic: &Diagnostic) -> Json {
    let mut object = Json::object();
    object.set(
        "code",
        match diagnostic.code {
            Some(code) => Json::string(code),
            None => Json::null(),
        },
    );
    object.set("severity", Json::string(diagnostic.severity.as_str()));
    object.set("message", Json::string(diagnostic.message.as_str()));
    object.set("file", Json::string(sources.path(diagnostic.file)));
    object.set("span", span_json(sources, diagnostic.file, diagnostic.span));
    if let Some(label) = &diagnostic.label {
        let mut label_object = Json::object();
        label_object.set("span", span_range_json(diagnostic.span));
        label_object.set("message", Json::string(label.as_str()));
        object.set("labels", Json::array(vec![label_object]));
    }
    if !diagnostic.notes.is_empty() {
        object.set(
            "notes",
            Json::array(
                diagnostic
                    .notes
                    .iter()
                    .map(|note| match note.strip_prefix("expansion: ") {
                        Some(message) => {
                            let mut expansion = Json::object();
                            expansion.set("kind", Json::string("expansion"));
                            expansion.set("message", Json::string(message));
                            expansion
                        }
                        None => Json::string(note.as_str()),
                    })
                    .collect(),
            ),
        );
    }
    if let Some(url) = diagnostic.docs_url() {
        object.set("docs_url", Json::string(url));
    }
    if let Some(proof) = &diagnostic.proof {
        let mut proof_object = Json::object();
        proof_object.set("obligation", Json::string(proof.obligation));
        proof_object.set("missing_fact", Json::string(proof.missing_fact.as_str()));
        proof_object.set("remedy", Json::string(proof.remedy.as_str()));
        object.set("proof", proof_object);
    }
    if !diagnostic.suggestions.is_empty() {
        let suggestions: Vec<Json> = diagnostic
            .suggestions
            .iter()
            .map(|suggestion| {
                let mut value = Json::object();
                value.set("span", span_range_json(suggestion.span));
                value.set("replacement", Json::string(&suggestion.replacement));
                value.set("message", Json::string(&suggestion.message));
                value.set(
                    "applicability",
                    Json::string(match suggestion.applicability {
                        yashy_common::Applicability::MachineApplicable => "machine-applicable",
                        yashy_common::Applicability::MaybeIncorrect => "maybe-incorrect",
                    }),
                );
                value
            })
            .collect();
        object.set("suggestions", Json::array(suggestions));
    }
    object
}

pub fn span_json(sources: &SourceMap, file: FileId, span: Span) -> Json {
    let location = sources.locate(file, span);
    let mut object = span_range_json(span);
    object.set("line", Json::number(i64::from(location.line)));
    object.set("column", Json::number(i64::from(location.col)));
    object
}

pub(crate) fn span_range_json(span: Span) -> Json {
    let mut object = Json::object();
    object.set("start", Json::number(i64::from(span.start)));
    object.set("end", Json::number(i64::from(span.end)));
    object
}

pub use fetch::{default_store_root, FetchError, Store};
pub use modules::{
    default_stdlib_root, load, load_sources, module_roots, resolve_import, stdlib_fingerprint,
    stdlib_version, Module, ModuleSource,
};
pub use pipeline::{
    check_and_verify, check_and_verify_timed, check_and_verify_timed_with_features, compile_hir,
    compile_hir_timed, compile_to_binary, ir_cache_key, ir_cache_store, load_and_check,
    load_and_check_exports_timed, load_and_check_exports_timed_with_features, load_and_check_timed,
    resolve_opt, timed, BuildOptions, CommandKind, CpuTarget, OptLevel, TimingRow,
};
pub use resolve::{resolve, ResolveError, ResolvedGraph, ResolvedPackage};
pub use toolchain::{detect, detect_with, Toolchain};
