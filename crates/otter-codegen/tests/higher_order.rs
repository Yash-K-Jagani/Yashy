//! The higher-order array methods: `map`, `filter`, and `fold`.
//!
//! Each of them is a loop that calls a function on the array's elements, so
//! the IR has three parts: the loop, the call through the function value, and
//! what the loop does with what the call returned.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::check_program;

use otter_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and emit `src` as one module.
fn ir_of(src: &str) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ot", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    let hir = check_program(&ast, &sm, file).expect("these test sources must type-check");
    emit_program(&hir, "main.ot", CodegenOptions::default())
}

#[test]
fn map_builds_a_new_array_and_pushes_what_the_callback_returned() {
    let ir_text = ir_of(
        "fn main() { var xs = []i64 { 1, 2 }; const ys = xs.map(|x: i64| -> i64 { return x * 2; }); }",
    );
    assert!(
        ir_text.contains("call i64 %"),
        "the callback is called through the function value:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @otter_slice_push("),
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
        "fn main() { var xs = []i64 { 1, 2 }; const ys = xs.filter(|x: i64| -> bool { return x > 1; }); }",
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
        "fn main() { var xs = []i64 { 1, 2 }; const n = xs.fold(0, |acc: i64, x: i64| -> i64 { return acc + x; }); }",
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
fn the_mapped_array_is_a_new_one() {
    // The source keeps its own elements: the mapped values go into a header
    // of their own, and the source's header is only read.
    let ir_text = ir_of(
        "fn main() { var xs = []i64 { 1, 2 }; const ys = xs.map(|x: i64| -> i64 { return x; }); }",
    );
    let pushes = ir_text.matches("call void @otter_slice_push(").count();
    assert_eq!(pushes, 1, "one push per element of the source:\n{ir_text}");
    assert!(
        ir_text.contains("store { i8*, i64, i64 } zeroinitializer"),
        "the new array starts empty:\n{ir_text}"
    );
}
