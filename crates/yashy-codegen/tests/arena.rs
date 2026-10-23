//! Arena lowering: `arena_new`, `arena_alloc`, `arena_free_all`.
//!
//! An arena is a bump allocator whose value is an ordinary `*mut i64` handle.
//! The backend emits one call per builtin and nothing else: in particular it
//! never inserts a release, because releasing an arena is a decision the
//! program makes (there is no per-allocation free to insert).

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::{Diagnostic, SourceMap};
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

/// Lex, parse and check `source`, expecting at least one diagnostic.
fn check_diagnostics(src: &str) -> Vec<Diagnostic> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    check_program(&ast, &sm, file).expect_err("expected the source to be rejected")
}

/// The first function in `src`.
fn first_fn(src: &str) -> HirFn {
    check_source(src)
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) => Some(f.clone()),
            _ => None,
        })
        .expect("the source must define a function")
}

/// Lower the first function's body into a finished module.
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

/// How many calls (not declarations) name `symbol`.
fn call_count(ir_text: &str, symbol: &str) -> usize {
    ir_text
        .lines()
        .filter(|line| line.contains("call ") && line.contains(symbol))
        .count()
}

#[test]
fn arena_new_emits_a_call_to_yashy_arena_new() {
    let ir_text = gen_body("fn f() { const arena = arena_new(); arena_free_all(arena); }");
    assert_eq!(
        call_count(&ir_text, "@yashy_arena_new"),
        1,
        "one call to the constructor:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @yashy_arena_new()"),
        "the constructor takes no arguments and yields the opaque handle:\n{ir_text}"
    );
}

#[test]
fn arena_alloc_calls_yashy_arena_alloc_with_the_arena_pointer() {
    // `arena_alloc(arena, 4)` counts four `i64` elements, exactly as `alloc`
    // does, so the byte count is the backend's multiply. The handle must be the
    // binding's value -- passing `null` here would compile and fault at run
    // time instead of refusing to allocate.
    let ir_text = gen_body(
        "fn f() { const arena = arena_new(); const p = arena_alloc(arena, 4); arena_free_all(arena); }",
    );
    assert!(
        ir_text.contains("mul i64 4, 8"),
        "the element count must be scaled by the element size:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @yashy_arena_alloc(i8* ")
            && ir_text.contains(", i64 8)"),
        "the call takes the handle, a byte count and an i64 alignment:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("@yashy_arena_alloc(i8* null"),
        "the binding's handle must be passed, not a null fallback:\n{ir_text}"
    );
}

#[test]
fn an_arena_handle_can_be_used_for_many_allocations() {
    // `arena_alloc` is the one builtin whose handle parameter does not consume
    // it: an arena exists to hand out many blocks. Only `arena_free_all` takes
    // ownership, which is what makes a second free a type error.
    let ir_text = gen_body(
        "fn f() { const arena = arena_new(); const a = arena_alloc(arena, 2); const b = arena_alloc(arena, 3); arena_free_all(arena); }",
    );
    assert_eq!(
        call_count(&ir_text, "@yashy_arena_alloc"),
        2,
        "both allocations reach the runtime:\n{ir_text}"
    );
}

#[test]
fn arena_free_all_is_not_called_automatically() {
    // A program that creates an arena and never frees it emits exactly one
    // `arena_new` and zero `arena_free_all` -- the language does not insert a
    // release the programmer did not write.
    let ir_text = gen_body("fn f() { const arena = arena_new(); const p = arena_alloc(arena, 4); }");
    assert_eq!(call_count(&ir_text, "@yashy_arena_new"), 1);
    assert_eq!(
        call_count(&ir_text, "@yashy_arena_free_all"),
        0,
        "releasing an arena is the program's decision:\n{ir_text}"
    );
}

#[test]
fn freeing_an_arena_twice_is_a_type_error() {
    // `arena_free_all` consumes the handle, so the second call reads a moved
    // binding. The alternative is a double free the checker could see and let
    // through.
    let diags = check_diagnostics(
        "fn main() { var a = arena_new(); arena_free_all(a); arena_free_all(a); }",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("already been moved")),
        "expected a moved-value diagnostic, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
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
fn golden_arena_matches_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("arena.ya"));
    let actual = normalize_host_header(&gen_body(&input));
    let expected_path = golden_dir().join("arena.expected.ll");
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
