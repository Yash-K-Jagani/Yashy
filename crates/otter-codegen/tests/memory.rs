//! Memory-lowering tests: `alloc`, `dealloc`, `move`, and what happens to a
//! linear binding when it is overwritten.
//!
//! The harness lowers a whole function body through [`gen_block`], the same way
//! `gen_fn` does, so statements (`var`, assignment, an expression statement) are
//! exercised rather than only the `const`/expression subset the expression tests
//! use.

use std::fs;
use std::path::{Path, PathBuf};

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirFn, HirItem, HirProgram};

use otter_codegen::{gen_block, FnCtx, Module};

/// Lex, parse and check `source` into a typed HIR program.
fn check_source(src: &str) -> HirProgram {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ot", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    check_program(&ast, &sm, file).expect("these test sources must type-check")
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
        let mut defers = Vec::new();
        let _ = gen_block(&mut ctx, &f.body, &mut defers);
        ctx.finish();
    }
    module.finish()
}

/// How many times the module frees a block.
fn free_count(ir: &str) -> usize {
    ir.matches("call void @otter_free").count()
}

#[test]
fn alloc_counts_elements_and_asks_for_i64_alignment() {
    // `alloc(16)` means sixteen `i64`s, not sixteen bytes: the runtime counts
    // bytes, so the multiply by the element size is the backend's job.
    let ir_text = gen_body("fn f() { const p = alloc(16); }");
    assert!(
        ir_text.contains("mul i64 16, 8"),
        "the element count must be scaled by the element size:\n{ir_text}"
    );
    assert!(
        ir_text.contains("@otter_alloc(i64 ") && ir_text.contains(", i64 8)"),
        "the call must take a byte count and an i64 alignment:\n{ir_text}"
    );
}

#[test]
fn dealloc_is_one_call_and_no_value() {
    let ir_text = gen_body("fn f() { const p = alloc(16); dealloc(p); }");
    assert_eq!(free_count(&ir_text), 1, "one free:\n{ir_text}");
    assert!(
        ir_text.contains("call void @otter_free(i8* "),
        "otter_free takes the pointer as i8*:\n{ir_text}"
    );
}

#[test]
fn move_poisons_the_source_slot() {
    // The checker already forbids reading a moved binding, so this is belt and
    // braces: if codegen ever drifts, the worst case is a `free` of null.
    let ir_text = gen_body("fn f() { var b = alloc(16); var o = move b; dealloc(o); }");
    assert!(
        ir_text.contains("store i8* null, ptr"),
        "moving a pointer must null the source slot:\n{ir_text}"
    );
}

#[test]
fn move_of_a_copy_value_does_not_poison() {
    // `i64` is not linear, so `move` is a plain read and no slot is written.
    let ir_text = gen_body("fn f() { var x = 1; var y = move x; print_i64(x + y); }");
    assert!(
        !ir_text.contains("store i64 null"),
        "a copy value has no owner to give away:\n{ir_text}"
    );
    assert_eq!(free_count(&ir_text), 0, "no frees here:\n{ir_text}");
}

#[test]
fn rebinding_a_live_linear_var_releases_the_old_pointer() {
    // Overwriting an owning binding discards the pointer, so the backend frees
    // it first -- one free for the old value, one for the `dealloc` at the end.
    let ir_text = gen_body("fn f() { var b = alloc(16); b = alloc(32); dealloc(b); }");
    assert_eq!(free_count(&ir_text), 2, "old value plus final free:\n{ir_text}");
}

#[test]
fn rebinding_after_a_dealloc_does_not_free_the_same_pointer_twice() {
    // `dealloc(b)` frees the block but leaves the address in the slot. The
    // checker knows `b` no longer owns anything, so `b = alloc(32)` must not
    // release it again -- otherwise this program would be a double free.
    let ir_text = gen_body("fn f() { var b = alloc(16); dealloc(b); b = alloc(32); dealloc(b); }");
    assert_eq!(
        free_count(&ir_text),
        2,
        "each allocation is freed exactly once:\n{ir_text}"
    );
}

#[test]
fn a_deallocated_slot_is_not_released_by_a_later_rebind() {
    // The same rule seen from the other side: one `dealloc`, then a rebind that
    // must emit no free of its own.
    let ir_text = gen_body("fn f() { var b = alloc(16); dealloc(b); b = alloc(32); }");
    assert_eq!(free_count(&ir_text), 1, "only the explicit free:\n{ir_text}");
}

// --- golden IR ---------------------------------------------------------------

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// True when `UPDATE_GOLDEN=1` asks for the `.expected.ll` file to be rewritten.
fn updating_goldens() -> bool {
    matches!(std::env::var("UPDATE_GOLDEN").as_deref(), Ok("1") | Ok("true"))
}

/// Read a golden file with line endings normalised to `\n`.
fn read_lf_only(path: &Path) -> String {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read golden file {}: {e}", path.display()));
    let body = raw.strip_suffix('\n').unwrap_or(raw.as_str());
    let mut out = String::new();
    for line in body.split('\n') {
        out.push_str(line.strip_suffix('\r').unwrap_or(line));
        out.push('\n');
    }
    out
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
fn golden_memory_matches_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("memory.ot"));
    let actual = normalize_host_header(&gen_body(&input));
    let expected_path = golden_dir().join("memory.expected.ll");
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
