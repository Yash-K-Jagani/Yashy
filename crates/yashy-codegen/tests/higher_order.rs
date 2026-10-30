//! The higher-order array methods: `map`, `filter`, and `fold`.
//!
//! Each of them is a loop that calls a function on the array's elements, so
//! the IR has three parts: the loop, the call through the function value, and
//! what the loop does with what the call returned.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and emit `src` as one module.
fn ir_of(src: &str) -> String {
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
    emit_program(&hir, "main.ya", CodegenOptions::default(), &yashy_common::ProofFacts::empty())
}

#[test]
fn map_builds_a_new_array_and_pushes_what_the_callback_returned() {
    let ir_text = ir_of(
        "fn main() { let xs = []int { 1, 2 }; const ys = xs.map(|x: int| -> int { return x * 2; }); }",
    );
    assert!(
        ir_text.contains("call i64 %"),
        "the callback is called through the function value:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @yashy_slice_push("),
        "the mapped values go into a new array:\n{ir_text}"
    );
    assert!(
        ir_text.contains("getelementptr { i8*, i64, i64 }, ptr"),
        "the loop reads the source array's header:\n{ir_text}"
    );
}

#[test]
fn filter_keeps_only_the_elements_that_pass() {
    let ir_text = ir_of(
        "fn main() { let xs = []int { 1, 2 }; const ys = xs.filter(|x: int| -> bool { return x > 1; }); }",
    );
    assert!(
        ir_text.contains("call i1 %"),
        "the callback says whether the element stays:\n{ir_text}"
    );
    assert!(
        ir_text.contains("br i1 %"),
        "the loop branches on what the callback said:\n{ir_text}"
    );
}

#[test]
fn fold_carries_one_value_through_the_array() {
    let ir_text = ir_of(
        "fn main() { let xs = []int { 1, 2 }; const n = xs.fold(0, |acc: int, x: int| -> int { return acc + x; }); }",
    );
    assert!(
        ir_text.contains("alloca i64"),
        "the accumulator has a slot of its own:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i64 %"),
        "the callback is called through the function value:\n{ir_text}"
    );
}

#[test]
fn every_block_of_the_three_methods_ends_in_one_terminator() {
    // The loop is three blocks -- the test, the body, the increment -- and
    // `filter` adds a fourth for the elements that pass. A block with two
    // terminators is not well-formed IR, and clang's verifier is the only
    // thing that says so.
    for source in [
        "fn main() { let xs = []int { 1, 2 }; const ys = xs.map(|x: int| -> int { return x; }); }",
        "fn main() { let xs = []int { 1, 2 }; const ys = xs.filter(|x: int| -> bool { return x > 1; }); }",
        "fn main() { let xs = []int { 1, 2 }; const n = xs.fold(0, |acc: int, x: int| -> int { return acc + x; }); }",
    ] {
        let ir_text = ir_of(source);
        assert_one_terminator_per_block(&ir_text, source);
    }
}

/// Assert every basic block in `ir` has exactly one terminator.
fn assert_one_terminator_per_block(ir: &str, source: &str) {
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
            "{source}: a block continued after its terminator: `{trimmed}`\n{ir}"
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
fn the_mapped_array_is_a_new_one() {
    // The source keeps its own elements: the mapped values go into a header
    // of their own, and the source's header is only read.
    let ir_text = ir_of(
        "fn main() { let xs = []int { 1, 2 }; const ys = xs.map(|x: int| -> int { return x; }); }",
    );
    let pushes = ir_text.matches("call void @yashy_slice_push(").count();
    assert_eq!(pushes, 1, "the fallback path has one push site:\n{ir_text}");
    assert!(
        ir_text.contains("store { i8*, i64, i64 } zeroinitializer"),
        "the new array starts empty:\n{ir_text}"
    );
}

#[test]
fn map_reserves_the_source_length_before_it_writes() {
    // The result cannot be longer than the source, so one reservation covers
    // every element and the loop never reallocates. The reservation has to
    // happen before the first write, and it must not write a capacity the
    // runtime will trust without bytes behind it.
    let ir_text = ir_of(
        "fn main() { let xs = []int { 1, 2 }; const ys = xs.map(|x: int| -> int { return x; }); }",
    );
    let reserve = ir_text
        .find("call void @yashy_slice_reserve(")
        .expect("the result's capacity is reserved");
    let push = ir_text
        .find("call void @yashy_slice_push(")
        .expect("the mapped value is pushed");
    assert!(
        reserve < push,
        "the reservation comes before the loop:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("9223372036854775807"),
        "the capacity is the source length, not a sentinel:\n{ir_text}"
    );
}

#[test]
fn map_writes_straight_into_the_reserved_buffer() {
    // The reservation is the output's capacity, so the common case is a store
    // through the data pointer: no runtime call per element. The push site
    // remains only as the fallback for a reservation that could not allocate.
    let ir_text = ir_of(
        "fn main() { let xs = []int { 1, 2 }; const ys = xs.map(|x: int| -> int { return x; }); }",
    );
    let store = ir_text
        .find("_store:")
        .expect("the reserved buffer has a write path");
    let push = ir_text
        .find("call void @yashy_slice_push(")
        .expect("the fallback push exists");
    assert!(
        store < push,
        "the write path is the branch that runs first:\n{ir_text}"
    );
    assert!(
        ir_text.contains("icmp ne i8* "),
        "the data pointer decides which path runs:\n{ir_text}"
    );
    assert!(
        ir_text.contains("getelementptr i8, ptr "),
        "the element is addressed inside the reserved bytes:\n{ir_text}"
    );
}
