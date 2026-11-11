//! The stdlib is a product, not a stub.
//!
//! Three mechanical questions, and no judgement calls:
//!
//! - Does every module in `crates/yashy-stdlib/yashy/` have a documented
//!   `pub fn`, and does every `pub fn` in every module carry a `///`/`/**`
//!   block? A module without either is prose, not a library.
//! - Does every module have a program under `tests/programs/stdlib/` that
//!   imports it? A documented function nothing exercises is a claim, not a
//!   feature.
//! - Does any fallible `-> string`/`-> int` function document "or the empty
//!   string"/"or zero" as its failure value? Yashy answers failure with
//!   `Result`/`Option` (the phase 3b rule); a sentinel is a design defect even
//!   when it is documented.
//!
//! A fourth check is the one the flat root scope imposes: a public name may not
//! be declared by two modules. Importing either one would then be a duplicate
//! definition, so the name is a bug waiting for a caller.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// `crates/yashy-stdlib/yashy`, the directory holding `io.ya` and friends.
fn stdlib_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("yashy-stdlib")
        .join("yashy")
}

/// `crates/yashy-driver/tests/programs/stdlib`, one program per module.
fn programs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("programs")
        .join("stdlib")
}

/// Every `.ya` module, sorted by stem so a failure names the same file every
/// run.
fn modules() -> Vec<(String, String)> {
    let mut found = Vec::new();
    let entries = fs::read_dir(stdlib_dir()).expect("the stdlib directory must be readable");
    for entry in entries {
        let path = entry.expect("a stdlib entry must be readable").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("ya") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("a stdlib module has a UTF-8 stem")
            .to_string();
        let text = fs::read_to_string(&path).expect("a stdlib module must be readable");
        found.push((stem, text));
    }
    found.sort();
    assert!(!found.is_empty(), "the stdlib directory has no modules");
    found
}

/// One `pub fn` and the documentation written immediately above it.
struct PublicFn {
    name: String,
    returns: String,
    doc: String,
}

/// The `pub fn` declarations in `text`, with their doc blocks.
///
/// The doc comment is the contiguous run of `///` lines or one `/** ... */`
/// block directly above the declaration. Any other line ends the run, so a
/// comment separated from the function by a blank line does not count.
fn public_fns(text: &str) -> Vec<PublicFn> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut docs = String::new();
    let mut in_block = false;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if in_block {
            docs.push_str(trimmed.trim_start_matches('*').trim());
            docs.push('\n');
            if trimmed.contains("*/") {
                in_block = false;
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("///") {
            docs.push_str(rest.trim());
            docs.push('\n');
            continue;
        }
        if trimmed.starts_with("/**") {
            in_block = true;
            docs.push_str(trimmed.trim_start_matches("/**").trim());
            docs.push('\n');
            if trimmed.contains("*/") {
                in_block = false;
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("pub fn ") {
            let name = rest
                .split(['(', '<'])
                .next()
                .unwrap_or(rest)
                .trim()
                .to_string();
            // The return type is everything after `->` on the signature line(s).
            // Every stdlib declaration keeps its signature on one line, which
            // the next assertion makes a checked fact rather than an assumption.
            let signature = &lines[index];
            assert!(
                signature.contains('{'),
                "a `pub fn` signature must open its body on the same line: {signature}"
            );
            let returns = signature
                .split_once("->")
                .map(|(_, tail)| tail.split('{').next().unwrap_or("").trim().to_string())
                .unwrap_or_else(|| "void".to_string());
            found.push(PublicFn {
                name,
                returns,
                doc: std::mem::take(&mut docs),
            });
            continue;
        }
        // A blank line ends the run: the compiler attaches only the
        // contiguous block directly above the declaration.
        docs.clear();
    }
    found
}

/// Public item names declared by one module, for the uniqueness check.
fn public_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        for keyword in [
            "pub fn ",
            "pub struct ",
            "pub enum ",
            "pub trait ",
            "pub const ",
        ] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
                let name = rest
                    .split(['(', '<', '{', ':', '=', ' '])
                    .next()
                    .unwrap_or(rest)
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
    }
    names
}

#[test]
fn every_module_has_a_documented_public_function() {
    let mut failures = Vec::new();
    for (module, text) in modules() {
        let fns = public_fns(&text);
        if fns.is_empty() {
            failures.push(format!("{module}: declares no `pub fn`"));
            continue;
        }
        for function in &fns {
            if function.doc.trim().is_empty() {
                failures.push(format!(
                    "{module}: `pub fn {}` has no `///` or `/**` block",
                    function.name
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "every module is a library, not a stub:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_module_has_a_program_that_imports_it() {
    let mut failures = Vec::new();
    for (module, _) in modules() {
        let program = programs_dir().join(format!("{module}.ya"));
        match fs::read_to_string(&program) {
            Ok(text) => {
                let import = format!("import {module};");
                if !text.contains(&import) {
                    failures.push(format!(
                        "{}: does not contain `{import}`",
                        program.display()
                    ));
                }
            }
            Err(error) => {
                failures.push(format!("{}: no test program ({error})", program.display()))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "every stdlib module is exercised by one program:\n{}",
        failures.join("\n")
    );
}

#[test]
fn no_fallible_scalar_function_documents_a_sentinel() {
    // The failure values the phase 3b rule forbids a scalar signature from
    // using. A function that can fail returns `Result`/`Option`; these phrases
    // are the ones a sentinel hides behind.
    const SENTINELS: [&str; 4] = [
        "or the empty string",
        "or an empty string",
        "or zero",
        "or 0 when it fails",
    ];
    let mut failures = Vec::new();
    for (module, text) in modules() {
        for function in public_fns(&text) {
            let scalar = function.returns == "string"
                || function.returns == "int"
                || function.returns == "i64"
                || function.returns == "u64";
            if !scalar {
                continue;
            }
            let doc = function.doc.to_lowercase();
            for phrase in SENTINELS {
                if doc.contains(phrase) {
                    failures.push(format!(
                        "{module}: `pub fn {}` -> {} documents `{phrase}`; return Result/Option",
                        function.name, function.returns
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "a failure is a value, never a sentinel:\n{}",
        failures.join("\n")
    );
}

#[test]
fn no_public_name_is_declared_by_two_modules() {
    let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (module, text) in modules() {
        for name in public_names(&text) {
            owners.entry(name).or_default().push(module.clone());
        }
    }
    let clashes: Vec<String> = owners
        .into_iter()
        .filter(|(_, modules)| modules.len() > 1)
        .map(|(name, modules)| format!("{name}: {}", modules.join(", ")))
        .collect();
    assert!(
        clashes.is_empty(),
        "the modules share one root scope, so a name has one owner:\n{}",
        clashes.join("\n")
    );
}
