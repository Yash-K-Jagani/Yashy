//! Tuples, destructuring, and pattern guards: the aggregate in registers, the
//! per-element extraction, and the extra branch a guard adds.
//!
//! These are IR-shape tests, not runtime tests. The end-to-end behaviour lives
//! in `crates/yashy-driver/tests/programs/tuples_guards.ya`, which needs to
//! spawn a compiler and an executable; these always run and pin the shapes
//! that make that program mean what it says.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

use yashy_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("tuples.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(
        &check_source(src),
        "tuples.ya",
        CodegenOptions::default(),
    )
}

#[test]
fn a_tuple_literal_is_an_insertvalue_chain() {
    // A tuple is an LLVM literal struct whose field list *is* the type, so the
    // value is built where it is used: one `insertvalue` per element onto
    // `undef`, and nothing allocated.
    let ir = emit("fn main() { const pair = (1, 2.0); }");
    assert!(
        ir.contains("%slot0 = alloca { i64, double }"),
        "the boxed local is the aggregate itself:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue { i64, double } undef, i64 1, 0")
            && ir.contains("insertvalue { i64, double } %t0, double 2.0, 1"),
        "one insertvalue per element, in order:\n{ir}"
    );
    assert!(
        !ir.contains("call i8* @yashy_alloc"),
        "a tuple owns nothing, so it allocates nothing:\n{ir}"
    );
}

#[test]
fn a_nested_tuple_is_a_nested_struct() {
    // The inner tuple is an element like any other, so the representation
    // nests with it -- position is the whole of the type's structure.
    let ir = emit("fn main() { const nested = (1, (2, 3)); }");
    assert!(
        ir.contains("alloca { i64, { i64, i64 } }"),
        "the element's own representation is its type:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue { i64, i64 } undef, i64 2, 0")
            && ir.contains("insertvalue { i64, { i64, i64 } } %t0, { i64, i64 } %t2, 1"),
        "the inner tuple is built first, then inserted as the outer element:\n{ir}"
    );
}

#[test]
fn destructuring_extracts_each_element_into_its_own_slot() {
    // Each name gets the element its position names, and `_` gets nothing at
    // all: no slot, no extraction, no store.
    let ir = emit("fn main() { const pair = (1, true); const (n, _) = pair; print_i64(n); }");
    assert!(
        ir.contains("extractvalue { i64, i1 }"),
        "element 0 is extracted from the aggregate:\n{ir}"
    );
    assert_eq!(
        ir.matches("extractvalue { i64, i1 }").count(),
        1,
        "`_` extracts nothing, so there is no element-1 read:\n{ir}"
    );
    assert!(
        !ir.contains("alloca i1"),
        "`_` binds no slot either:\n{ir}"
    );
    assert!(
        ir.contains("store i64 %t"),
        "the extracted element is stored into the binding's own slot:\n{ir}"
    );
}

#[test]
fn a_tuple_is_passed_and_returned_by_value() {
    // The aggregate is the calling convention here: one field per element, the
    // same `{ i1, i64 }` on both sides of the call, and no allocation.
    let ir = emit(
        "fn swap(p: (int, bool)) -> (bool, int) { const (n, b) = p; return (b, n); } \
         fn main() { const out = swap((1, true)); }",
    );
    assert!(
        ir.contains("define { i1, i64 } @swap({ i64, i1 } %arg0)"),
        "a tuple parameter is passed as its fields:\n{ir}"
    );
    assert!(
        ir.contains("ret { i1, i64 } %"),
        "the tuple return is the aggregate return:\n{ir}"
    );
    assert!(
        ir.contains("call { i1, i64 } @swap({ i64, i1 }"),
        "the call site names the aggregate result type:\n{ir}"
    );
}

#[test]
fn a_guard_branches_back_into_the_search_when_it_fails() {
    // A guard that says no must behave exactly like a pattern that did not
    // match. That is one extra block per guarded arm: the body on the accepted
    // edge, the next test on the rejected one. The unguarded arm gets neither.
    let ir = emit(
        "enum Shape { Circle(float), Dot } \
         fn area(s: Shape) -> float { return switch s { \
             case Circle(r) if r > 10.0 { 1.0 } \
             case Circle(r) { 2.0 } \
             case Dot { 3.0 } \
         }; }",
    );
    assert!(
        ir.contains("_accept:"),
        "the accepted edge of the guard is its own block:\n{ir}"
    );
    assert_eq!(
        ir.matches("_accept:").count(),
        1,
        "only the guarded arm has an accept block:\n{ir}"
    );
    // The rejected edge is the block the *next* test lives in, not the result:
    // the search continues with the unguarded `Circle` arm and then `Dot`.
    let accept_at = ir.find("_accept:").expect("the accept block exists");
    let rejected = ir[..accept_at]
        .lines()
        .rev()
        .find(|line| line.contains("br i1"))
        .expect("the guard ends in a conditional branch");
    assert!(
        !rejected.contains("result"),
        "a failed guard continues the search rather than ending it: {rejected}"
    );
    assert!(
        ir.contains("fcmp ogt double"),
        "the guard is the comparison, separate from the tag test:\n{ir}"
    );
}

#[test]
fn two_guards_on_one_variant_get_two_accept_blocks() {
    // Each guarded arm owns its accept block and its own rejected edge, so the
    // second guard is reached from the first guard's failure -- the same path
    // a tag mismatch would take. The unguarded arm below them is the one that
    // finally stores the result.
    let ir = emit(
        "enum E { A(int), B } \
         fn f(e: E) -> int { return switch e { \
             case A(v) if v > 10 { 1 } \
             case A(v) if v > 0 { 2 } \
             case A(v) { 3 } \
             case B { 4 } \
         }; }",
    );
    assert_eq!(
        ir.matches("_accept:").count(),
        2,
        "one accept block per guarded arm:\\n{ir}"
    );
    assert_eq!(
        ir.matches("icmp sgt i64").count(),
        2,
        "both guards are their own comparison:\\n{ir}"
    );
}

#[test]
fn a_guarded_catch_all_still_has_a_place_in_the_chain() {
    // `case _ if n < 0` matches everything, but the guard can say no, so the
    // arm cannot be the chain's fall-through: it gets a block of its own and a
    // rejected edge to the `default`, which is what makes `bucket(-5)` and
    // `bucket(5)` differ.
    let ir = emit(
        "fn bucket(n: int) -> int { return switch n { \
             case 0 { 100 } \
             case _ if n < 0 { 200 } \
             default { 300 } \
         }; }",
    );
    assert!(
        ir.contains("_accept:"),
        "the guarded catch-all has an accept block like any other guard:\n{ir}"
    );
    assert!(
        ir.contains("icmp slt i64") || ir.contains("icmp slt"),
        "the guard is the `n < 0` comparison:\n{ir}"
    );
    // The chain still has the literal test for `case 0` and an unconditional
    // entry for the catch-all guard, so the `default` is reachable when both
    // decline. Three blocks' worth of branching, not a truncated search.
    assert_eq!(
        ir.matches("_accept:").count(),
        1,
        "one guarded arm, one accept block:\n{ir}"
    );
}
