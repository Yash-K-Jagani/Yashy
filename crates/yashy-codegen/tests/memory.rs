//! Memory-lowering tests: `alloc`, `dealloc`, `move`, and what happens to a
//! linear binding when it is overwritten.
//!
//! The harness lowers a whole function body through [`gen_block`], the same way
//! `gen_fn` does, so statements (`let`, assignment, an expression statement) are
//! exercised rather than only the `const`/expression subset the expression tests
//! use.

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
///
/// Pending `defer` bodies are run at the end, the way [`yashy_codegen::gen_fn`]
/// does, so a test can exercise the exit sequence as well as the body.
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

/// How many times the module frees a block.
fn free_count(ir: &str) -> usize {
    ir.matches("call void @yashy_free").count()
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
        ir_text.contains("@yashy_alloc(i64 ") && ir_text.contains(", i64 8)"),
        "the call must take a byte count and an i64 alignment:\n{ir_text}"
    );
}

#[test]
fn dealloc_is_one_call_and_no_value() {
    let ir_text = gen_body("fn f() { const p = alloc(16); dealloc(p); }");
    assert_eq!(free_count(&ir_text), 1, "one free:\n{ir_text}");
    assert!(
        ir_text.contains("call void @yashy_free(i8* "),
        "yashy_free takes the pointer as i8*:\n{ir_text}"
    );
}

#[test]
fn move_poisons_the_source_slot() {
    // The checker already forbids reading a moved binding, so this is belt and
    // braces: if codegen ever drifts, the worst case is a `free` of null.
    let ir_text = gen_body("fn f() { let b = alloc(16); let o = move b; dealloc(o); }");
    assert!(
        ir_text.contains("store i8* null, ptr"),
        "moving a pointer must null the source slot:\n{ir_text}"
    );
}

#[test]
fn move_of_a_copy_value_does_not_poison() {
    // `i64` is not linear, so `move` is a plain read and no slot is written.
    let ir_text = gen_body("fn f() { let x = 1; let y = move x; print(x + y); }");
    assert!(
        !ir_text.contains("store i64 null"),
        "a copy value has no owner to give away:\n{ir_text}"
    );
    assert_eq!(free_count(&ir_text), 0, "no frees here:\n{ir_text}");
}

#[test]
fn rebinding_a_live_linear_let_releases_the_old_pointer() {
    // Overwriting an owning binding discards the pointer, so the backend frees
    // it first -- one free for the old value, one for the `dealloc` at the end.
    let ir_text = gen_body("fn f() { let b = alloc(16); b = alloc(32); dealloc(b); }");
    assert_eq!(free_count(&ir_text), 2, "old value plus final free:\n{ir_text}");
}

#[test]
fn rebinding_after_a_dealloc_does_not_free_the_same_pointer_twice() {
    // `dealloc(b)` frees the block but leaves the address in the slot. The
    // checker knows `b` no longer owns anything, so `b = alloc(32)` must not
    // release it again -- otherwise this program would be a double free.
    let ir_text = gen_body("fn f() { let b = alloc(16); dealloc(b); b = alloc(32); dealloc(b); }");
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
    let ir_text = gen_body("fn f() { let b = alloc(16); dealloc(b); b = alloc(32); }");
    assert_eq!(free_count(&ir_text), 1, "only the explicit free:\n{ir_text}");
}

// --- defer ------------------------------------------------------------------

/// The slot named by the `load` that produced the last free's argument.
///
/// A free takes one operand, and it always comes from a `load` of the binding's
/// slot or of the snapshot slot a `defer` captured it into — so which slot it
/// names is exactly the question these tests are asking.
fn last_free_source(ir_text: &str) -> Option<String> {
    let lines: Vec<&str> = ir_text.lines().collect();
    let free_index = lines
        .iter()
        .rposition(|line| line.contains("call void @yashy_free"))?;
    let load_line = lines[..free_index]
        .iter()
        .rev()
        .find(|line| line.contains("= load i8*, ptr"))?;
    load_line.split("ptr ").nth(1).map(|slot| slot.trim().to_string())
}

#[test]
fn a_deferred_free_targets_the_value_live_at_registration() {
    // The checker validates the deferred body when the `defer` is reached but
    // the body runs at exit, and this program's `buf` changes identity in
    // between. Without the capture, the deferred body would read the slot at
    // exit -- the second block -- and free it a second time after the explicit
    // `dealloc`, which is heap corruption rather than a diagnostic.
    let ir_text = gen_body(
        "fn f() {\n\
         \x20   let buf = alloc(16);\n\
         \x20   defer { dealloc(buf); }\n\
         \x20   buf = alloc(32);\n\
         \x20   dealloc(buf);\n\
         }",
    );
    assert_eq!(
        ir_text.matches("alloca i8*").count(),
        2,
        "one slot for `buf` and one for the snapshot:\n{ir_text}"
    );
    assert_eq!(
        free_count(&ir_text),
        2,
        "one free per allocation, not a second free of the same block:\n{ir_text}"
    );
    assert_eq!(
        last_free_source(&ir_text).as_deref(),
        Some("%slot1"),
        "the deferred free must read the registration-time snapshot:\n{ir_text}"
    );
}

#[test]
fn a_defer_captures_only_linear_bindings() {
    // Copy types carry no ownership, so snapshotting them would cost a load and
    // a store per binding per registration and buy nothing. `x` keeps its one
    // slot.
    let ir_text = gen_body("fn f() { const x = 1; defer { print(x); } print(x); }");
    assert_eq!(
        ir_text.matches("alloca i64").count(),
        1,
        "only the binding's own slot:\n{ir_text}"
    );
    assert_eq!(
        ir_text.matches("alloca i8*").count(),
        0,
        "nothing linear is in scope:\n{ir_text}"
    );
}

/// The names of the slots a module allocates for `of`, in allocation order.
fn slot_names(ir_text: &str, of: &str) -> Vec<String> {
    ir_text
        .lines()
        .filter(|line| line.contains(of))
        .filter_map(|line| line.split('=').next().map(|name| name.trim().to_string()))
        .collect()
}

#[test]
fn a_defer_inside_a_loop_snapshots_once_per_iteration() {
    // The registration runs every iteration, so the snapshot store is inside the
    // loop body and the free happens at the end of each iteration rather than
    // once at the end of the loop.
    let ir_text = gen_body("fn f() { for i in 0..3 { let buf = alloc(8); defer { dealloc(buf); } } }");
    assert_eq!(free_count(&ir_text), 1, "one free in the body:\n{ir_text}");
    let pointers = slot_names(&ir_text, "alloca i8*");
    assert_eq!(
        pointers.len(),
        2,
        "the binding and its snapshot:\n{ir_text}"
    );
    let snapshot = pointers.last().cloned().unwrap_or_default();

    let lines: Vec<&str> = ir_text.lines().collect();
    let capture = lines
        .iter()
        .position(|line| line.contains("store i8*") && line.contains(&snapshot))
        .expect("the snapshot store must be in the loop body");
    let increment = lines
        .iter()
        .position(|line| line.trim_end().ends_with("_inc:"))
        .expect("the loop must have an increment block");
    let free = lines
        .iter()
        .position(|line| line.contains("call void @yashy_free"))
        .expect("the body must free");
    assert!(
        capture < free && free < increment,
        "the snapshot store precedes the free, which precedes the loop's \
         increment block -- so both happen per iteration:\n{ir_text}"
    );
}

#[test]
fn a_defer_after_checked_arithmetic_still_registers_and_runs() {
    // Checked arithmetic is the default, so almost every function lowers a fault
    // block before its first `defer`. That block includes the pending deferred
    // bodies (they must fire if the process is ending there); emitting them used
    // to *consume* the compile-time scope stack, so a `defer` written after a
    // checked operation panicked the compiler with "a scope is always active".
    // The scope stack now survives the fault block.
    let ir_text = gen_body(
        "fn f() {\n\
         \x20   let i = 0;\n\
         \x20   while i < 2 { i += 1; }\n\
         \x20   defer { println(\"done\"); }\n\
         \x20   println(\"ok\");\n\
         }",
    );
    assert!(
        ir_text.contains("@yashy_panic_overflow"),
        "the loop's `i += 1` must still be a checked add:\n{ir_text}"
    );
    assert!(
        ir_text.contains("done"),
        "the deferred body must reach the module, not be lost with the stack:\n{ir_text}"
    );
    assert!(
        ir_text.matches("@yashy_print_line_str").count() >= 2,
        "the deferred print and the body's print must both be lowered:\n{ir_text}"
    );
}

#[test]
fn a_fault_inside_a_deferred_body_does_not_re_enter_the_fault_lowering() {
    // The fault block of the division includes the pending defers -- including
    // the very body being lowered, if re-entry were allowed. The guard makes the
    // body's own fault go straight to the runtime, so lowering terminates. This
    // test hanging (or overflowing the compiler's stack) is the failure it
    // guards against.
    let ir_text = gen_body(
        "fn f(x: int) {\n\
         \x20   defer { print(10 / x); }\n\
         \x20   println(\"ok\");\n\
         }",
    );
    assert!(
        ir_text.contains("@yashy_panic_division"),
        "the deferred division must carry its fault check:\n{ir_text}"
    );
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
    let input = read_lf_only(&golden_dir().join("memory.ya"));
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
