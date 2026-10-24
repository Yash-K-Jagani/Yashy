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
    emit_program(&hir, "main.ya", CodegenOptions::default())
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
    assert_eq!(pushes, 1, "one push per element of the source:\n{ir_text}");
    assert!(
        ir_text.contains("store { i8*, i64, i64 } zeroinitializer"),
        "the new array starts empty:\n{ir_text}"
    );
}
