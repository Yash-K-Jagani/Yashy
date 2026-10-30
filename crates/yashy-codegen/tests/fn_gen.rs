//! Function and statement codegen tests: `if`/`else`, both `for` forms, `break`
//! and `continue`, `switch`, `defer`, and function prologues/terminators.
//!
//! Everything here goes through [`emit_program`], so the tests exercise the
//! real pipeline (lex -> parse -> check -> emit) rather than a private helper.

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and emit `src` as one module, with default options.
fn ir_of(src: &str) -> String {
    ir_of_with(src, CodegenOptions::default())
}

/// Lex, parse, check and emit `src` as one module with the given options.
fn ir_of_with(src: &str, options: CodegenOptions) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    let hir = check_program(&ast, &sm, file).expect("these test sources must type-check");
    let ir_text = emit_program(&hir, "main.ya", options, &yashy_common::ProofFacts::empty());
    // Every test doubles as a structural check: an appended-after-terminator
    // instruction would still look plausible but be invalid LLVM.
    assert_one_terminator_per_block(&ir_text);
    ir_text
}

/// Assert every basic block in `ir` has exactly one terminator.
///
/// This is the cheapest way to catch the class of bug where lowering appends
/// an instruction to a block it already terminated with `br`/`ret`: the text
/// still looks plausible line by line, but `clang` rejects it.
fn assert_one_terminator_per_block(ir: &str) {
    let mut in_function = false;
    let mut terminated = false;
    for line in ir.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with("define ") {
            in_function = true;
            terminated = false;
            continue;
        }
        if !in_function {
            continue;
        }
        if trimmed == "}" {
            in_function = false;
            continue;
        }
        if trimmed.ends_with(':') {
            terminated = false;
            continue;
        }
        assert!(
            !terminated,
            "a block continued after its terminator: `{trimmed}`\n{ir}"
        );
        if trimmed.starts_with("br ")
            || trimmed.starts_with("ret ")
            || trimmed.starts_with("switch ")
            || trimmed == "unreachable"
        {
            terminated = true;
        }
    }
}

#[test]
fn if_else_emits_one_conditional_branch_and_a_merge_label() {
    let ir_text = ir_of("fn f(x: bool) { if x { print(1); } else { print(2); } }");
    assert_eq!(
        ir_text.matches("br i1").count(),
        1,
        "one condition, one conditional branch:\n{ir_text}"
    );
    assert!(
        ir_text.contains("_merge:"),
        "the two arms rejoin at a merge label:\n{ir_text}"
    );
}

#[test]
fn range_for_lowers_to_a_counted_loop() {
    let ir_text = ir_of("fn f() { for i in 0..10 { print(i); } }");
    // A counted loop: load the index, compare against the (exclusive) end.
    assert!(
        ir_text.contains("icmp slt i64"),
        "a signed less-than exit test:\n{ir_text}"
    );
    assert!(
        ir_text.contains("i64 10"),
        "the exclusive end is the loop bound:\n{ir_text}"
    );
    assert!(
        ir_text.contains("_done:"),
        "the loop has an exit label:\n{ir_text}"
    );
}

#[test]
fn three_clause_for_lowers_to_a_counted_loop() {
    let ir_text = ir_of("fn f() { for i := 0; i < 3; print(i) { print(i); } }");
    assert!(
        ir_text.contains("br i1"),
        "the condition is the loop test:\n{ir_text}"
    );
    assert!(
        ir_text.contains("_inc:"),
        "the update runs at the bottom of the body:\n{ir_text}"
    );
}

#[test]
fn break_and_continue_branch_to_the_loop_labels() {
    let ir_text = ir_of(
        "fn f() { for i in 0..10 { if i == 3 { break; } if i == 5 { continue; } } }",
    );
    // Both targets exist as labels, and at least one branch names each.
    assert!(ir_text.contains("_done:"), "loop exit label:\n{ir_text}");
    assert!(ir_text.contains("_inc:"), "loop increment label:\n{ir_text}");
    assert!(
        ir_text.lines().any(|l| l.contains("br label %") && l.contains("_done")),
        "`break` branches to the exit:\n{ir_text}"
    );
    assert!(
        ir_text.lines().any(|l| l.contains("br label %") && l.contains("_inc")),
        "`continue` branches to the increment:\n{ir_text}"
    );
}

#[test]
fn defer_emits_in_reverse_order_before_return() {
    let ir_text = ir_of(
        "fn f() { print(1); defer { print(2); } defer { print(3); } return; }",
    );
    let three = ir_text
        .find("call i8* @yashy_str_from_i64(i64 3)")
        .expect("the second defer must be emitted");
    let two = ir_text
        .find("call i8* @yashy_str_from_i64(i64 2)")
        .expect("the first defer must be emitted");
    assert!(
        three < two,
        "defers run last registered first:\n{ir_text}"
    );
}

#[test]
fn defer_runs_on_the_return_path_only_once() {
    let ir_text = ir_of("fn f() { defer { print(7); } return; }");
    assert_eq!(
        ir_text.matches("call i8* @yashy_str_from_i64(i64 7)").count(),
        1,
        "the deferred body runs exactly once:\n{ir_text}"
    );
}

#[test]
fn nested_scope_defer_does_not_leak_to_the_outer_scope() {
    let ir_text = ir_of(
        "fn f() { for i in 0..1 { defer { print(9); } } return; }",
    );
    assert_eq!(
        ir_text.matches("call i8* @yashy_str_from_i64(i64 9)").count(),
        1,
        "a body defer fires at the body's exit, not the function's:\n{ir_text}"
    );
}

#[test]
fn empty_function_with_unit_return_emits_ret_void() {
    let ir_text = ir_of("fn f() { }");
    assert!(
        ir_text.contains("define void @f()"),
        "a unit function returns void:\n{ir_text}"
    );
    assert!(
        ir_text.contains("ret void"),
        "and terminates with `ret void`:\n{ir_text}"
    );
}

#[test]
fn a_function_that_cannot_return_is_refused() {
    // The checker refuses a value-returning function that does not return on
    // every path, so the generator's `ret i64 0` fallback is only reachable
    // from a tree the checker let through.
    let source = "fn f() -> int { }";
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("the program must parse");
    assert!(
        check_program(&ast, &sm, file).is_err(),
        "a function that returns nothing on one path is refused"
    );
}

#[test]
fn parameters_are_spilled_into_entry_block_slots() {
    let ir_text = ir_of("fn f(a: int, b: float) -> int { return a; }");
    assert!(
        ir_text.contains("define i64 @f(i64 %arg0, double %arg1)"),
        "the signature carries the parameter types:\n{ir_text}"
    );
    assert!(
        ir_text.contains("store i64 %arg0, ptr %slot0"),
        "the first argument is spilled into a slot:\n{ir_text}"
    );
}

// --- assignment --------------------------------------------------------------

#[test]
fn assignment_stores_into_the_bindings_own_slot() {
    let ir_text = ir_of("fn f() { let x = 1; x = 2; }");
    let stores: Vec<&str> = ir_text
        .lines()
        .filter(|line| line.trim_start().starts_with("store i64"))
        .collect();
    assert_eq!(
        stores.len(),
        2,
        "declaring and assigning are two stores:\n{ir_text}"
    );
    // Both stores must go to the same address. An assignment that wrote to a
    // fresh slot would leave `x` reading its old value, and the IR would still
    // look perfectly reasonable line by line.
    let target = |line: &str| line.split("ptr ").nth(1).unwrap_or_default().to_string();
    assert_eq!(
        target(stores[0]),
        target(stores[1]),
        "the assignment must overwrite the binding's slot:\n{ir_text}"
    );
}

#[test]
fn assignment_inside_a_loop_body_accumulates() {
    // The reason the statement exists: a loop body that keeps a running total.
    let ir_text = ir_of(
        "fn f() -> int { let sum = 0; for i in 0..10 { sum = sum + i; } return sum; }",
    );
    assert!(
        ir_text.contains("add i64"),
        "expected the accumulation to be computed:\n{ir_text}"
    );
    // `return sum` must read the slot the loop body wrote, not a fresh value.
    assert!(
        ir_text.lines().any(|l| l.contains("load i64")),
        "expected the total to be read back out of its slot:\n{ir_text}"
    );
    let stores: usize = ir_text.matches("store i64").count();
    assert!(
        stores >= 3,
        "declaration, the parameter spill and the loop-body assignment are all stores:\n{ir_text}"
    );
}

#[test]
fn assigning_other_reprs_stores_their_width() {
    // The store's type comes from the binding, not from the value: a store of
    // the wrong width writes over the neighbouring slot.
    let bools = ir_of("fn f() { let b = true; b = false; }");
    assert_eq!(
        bools.matches("store i1").count(),
        2,
        "a bool binding is an i1 slot:\n{bools}"
    );
    let floats = ir_of("fn f() { let d = 1.0; d = 2.0; }");
    assert_eq!(
        floats.matches("store double").count(),
        2,
        "an f64 binding is a double slot:\n{floats}"
    );
}

// --- floating-point contraction ----------------------------------------------

/// `a * b + 1.0` with `a` and `b` behind bindings, so the checker cannot fold
/// the arithmetic and the backend really sees a multiply feeding an add.
const FMA_SOURCE: &str = "fn main() -> float { let a = 2.0; let b = 3.0; return a * b + 1.0; }";

#[test]
fn floating_point_contraction_is_off_until_it_is_asked_for() {
    // The default has to be the exact-semantics one. A compiler that quietly
    // fused the multiply and the add would change the number its programs
    // print, and it would do so per machine, depending on which CPU had a
    // fused multiply-add.
    let exact = ir_of(FMA_SOURCE);
    assert!(
        !exact.contains("contract"),
        "the default must emit no fast-math flag:\n{exact}"
    );
    assert!(
        exact.contains("fmul double"),
        "expected a plain fmul:\n{exact}"
    );
    assert!(
        exact.contains("fadd double"),
        "expected a plain fadd:\n{exact}"
    );
}

#[test]
fn floating_point_contraction_marks_the_multiply_and_the_add() {
    let contracted = ir_of_with(FMA_SOURCE, CodegenOptions { fp_contract: true });
    assert!(
        contracted.contains("fmul contract double"),
        "the multiply must carry the flag:\n{contracted}"
    );
    assert!(
        contracted.contains("fadd contract double"),
        "the add must carry the flag:\n{contracted}"
    );
}

#[test]
fn contraction_covers_subtraction_and_not_division() {
    // `a * b - c` is a fused multiply-subtract, so `fsub` is in scope. `fdiv`
    // can fuse with nothing, and a flag that cannot change the code is noise in
    // the IR, so it must not appear there.
    let difference = ir_of_with(
        "fn main() -> float { let a = 2.0; let b = 3.0; return a * b - 1.0; }",
        CodegenOptions { fp_contract: true },
    );
    assert!(
        difference.contains("fsub contract double"),
        "a multiply feeding a subtract can fuse:\n{difference}"
    );

    let quotient = ir_of_with(
        "fn main() -> float { let a = 2.0; let b = 3.0; return a / b; }",
        CodegenOptions { fp_contract: true },
    );
    assert!(
        !quotient.contains("contract"),
        "division can fuse with nothing, so it takes no flag:\n{quotient}"
    );
}

#[test]
fn contraction_leaves_integer_arithmetic_alone() {
    // `contract` is a floating-point flag; on `add`/`sub`/`mul`/`sdiv` it would
    // be an LLVM parse error, so enabling contraction must not reach them.
    let integers = ir_of_with(
        "fn main() -> int { let a = 2; let b = 3; return a * b + 1; }",
        CodegenOptions { fp_contract: true },
    );
    assert!(
        !integers.contains("contract"),
        "integer arithmetic takes no fast-math flag:\n{integers}"
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

/// Replace the host-specific `target datalayout` and `target triple` lines with
/// placeholders, so one golden file is valid on MSVC, GNU/Windows and Linux.
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
fn golden_control_flow_matches_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("control_flow.ya"));
    let actual = normalize_host_header(&ir_of(&input));
    let expected_path = golden_dir().join("control_flow.expected.ll");
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

#[test]
fn golden_panic_matches_the_pinned_contract() {
    // The diagnostic path is the one place lowering emits a terminator of its
    // own inside an expression, so its IR is pinned rather than only
    // spot-checked: the block structure around the call *is* the design.
    let input = read_lf_only(&golden_dir().join("panic.ya"));
    let actual = normalize_host_header(&ir_of(&input));
    let expected_path = golden_dir().join("panic.expected.ll");
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
