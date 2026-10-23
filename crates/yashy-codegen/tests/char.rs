//! `char` lowering: one byte, compared in unsigned order.
//!
//! The golden program exists for the two comparison predicates. `'\xFF'` is
//! 255, which a signed `icmp slt` would read as -1, so the pinned IR is what
//! proves the comparison is unsigned rather than what proves the syntax parses.

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirFn, HirItem, HirProgram};

use yashy_codegen::{gen_block, run_defers, FnCtx, Flow, Module};

/// Lex, parse and check `source` into a typed HIR program.
fn check_source(src: &str) -> HirProgram {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    check_program(&ast, &sm, file).expect("these test sources must type-check")
}

/// The one function the helper programs define.
fn first_fn(src: &str) -> HirFn {
    check_source(src)
        .items
        .into_iter()
        .find_map(|item| match item {
            HirItem::Fn(f) => Some(f),
            _ => None,
        })
        .expect("the program defines one function")
}

/// Lower the first function of `src` into a complete module.
fn gen_body(src: &str) -> String {
    let f = first_fn(src);
    let mut module = Module::new();
    {
        let mut ctx = FnCtx::new(&mut module, f.ret.kind.clone());
        ctx.begin_entry();
        let _ = ctx.alloc_params(&f);
        let flow = gen_block(&mut ctx, &f.body);
        if flow == Flow::Fallthrough {
            run_defers(&mut ctx);
        }
        ctx.finish();
    }
    module.finish()
}

/// Directory holding the `.ya` inputs and `.ll` expectations.
fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// `UPDATE_GOLDEN=1` rewrites the expectation instead of comparing it.
fn updating_goldens() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some()
}

/// Read a file with `\n` line endings whatever the checkout uses.
fn read_lf_only(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// Replace the host-specific `target datalayout` and `target triple` lines.
fn normalize_host_header(ir: &str) -> String {
    ir.lines()
        .map(|line| {
            if line.starts_with("target triple =") {
                "target triple = \"<host>\""
            } else if line.starts_with("target datalayout =") {
                "target datalayout = \"<host>\""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn golden_char_matches_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("char.ya"));
    let actual = normalize_host_header(&gen_body(&input));
    let expected_path = golden_dir().join("char.expected.ll");
    if updating_goldens() {
        fs::write(&expected_path, &actual)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", expected_path.display()));
        return;
    }
    let expected = normalize_host_header(&read_lf_only(&expected_path));
    assert_eq!(
        actual, expected,
        "generated IR drifted from the pinned contract; \
         rerun with UPDATE_GOLDEN=1 only after checking the diff"
    );
}
