//! String-operation lowering: `+` (concatenation), `==`/`!=` (byte equality),
//! and `len`.
//!
//! A `string` value is an `i8*` to NUL-terminated bytes, so none of these is an
//! instruction — each one is a call into the runtime half of ledger L1. These
//! tests pin which call, and that the compiler emits exactly one per operator.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::check_program;

use otter_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and lower `source` to one LLVM IR module.
fn ir(source: &str) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ot", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    let hir = check_program(&ast, &sm, file).expect("these test sources must type-check");
    emit_program(&hir, "main.ot", CodegenOptions::default())
}

/// How many calls to `symbol` the module makes, not counting its declaration.
fn call_count(ir_text: &str, symbol: &str) -> usize {
    ir_text
        .lines()
        .filter(|line| !line.starts_with("declare") && line.contains(symbol))
        .count()
}

#[test]
fn concat_is_one_call_and_does_not_touch_its_operands() {
    let ir_text = ir("fn main() { const s = \"Hello, \" + \"Otter!\"; print_str(s); }");
    assert_eq!(
        call_count(&ir_text, "@otter_str_concat"),
        1,
        "one concat, one call:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @otter_str_concat(i8* "),
        "both operands are `i8*` values:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("@otter_str_release"),
        "M3 strings have no owner to release:\n{ir_text}"
    );
}

#[test]
fn chained_concat_is_one_call_per_operator() {
    // `"a" + "b" + "c"` groups left, so it is two concatenations and the result
    // of the first feeds the second.
    let ir_text = ir("fn main() { const s = \"a\" + \"b\" + \"c\"; print_str(s); }");
    assert_eq!(
        call_count(&ir_text, "@otter_str_concat"),
        2,
        "two operators, two calls:\n{ir_text}"
    );
}

#[test]
fn equality_compares_bytes_through_the_runtime() {
    // `icmp eq i8*` would compare the two addresses, and two identical literals
    // are not required to be the same address. The runtime compares contents.
    let ir_text = ir("fn main() { if \"a\" == \"b\" { print_str(\"same\"); } }");
    assert_eq!(call_count(&ir_text, "@otter_str_eq"), 1, "{ir_text}");
    assert!(
        !ir_text.contains("icmp eq i8*"),
        "string equality must not compare addresses:\n{ir_text}"
    );
}

#[test]
fn inequality_is_the_equality_call_negated() {
    let ir_text = ir("fn main() { if \"a\" != \"b\" { print_str(\"differ\"); } }");
    assert_eq!(call_count(&ir_text, "@otter_str_eq"), 1, "{ir_text}");
    assert!(
        ir_text.contains("xor i1 ") && ir_text.contains(", true"),
        "`!=` inverts the `==` result:\n{ir_text}"
    );
}

#[test]
fn len_is_a_runtime_call_and_yields_i64() {
    let ir_text = ir("fn main() { print_i64(len(\"hello\")); }");
    assert_eq!(call_count(&ir_text, "@otter_str_len"), 1, "{ir_text}");
    assert!(
        ir_text.contains("call i64 @otter_str_len(i8* "),
        "the length is an `i64`:\n{ir_text}"
    );
}

#[test]
fn a_string_operation_result_can_be_bound_and_reassigned() {
    // The result of a runtime call is an ordinary `string` value: it can be
    // stored in a slot, loaded back, and stored again.
    let ir_text = ir(
        "fn main() {\n\
         \x20   var s = \"a\" + \"b\";\n\
         \x20   s = s + \"c\";\n\
         \x20   print_str(s);\n\
         }",
    );
    assert_eq!(call_count(&ir_text, "@otter_str_concat"), 2, "{ir_text}");
    assert!(
        ir_text.contains("store i8* ") && ir_text.contains("load i8*, ptr"),
        "string values live in slots like any other value:\n{ir_text}"
    );
}

#[test]
fn interpolation_is_a_conversion_call_and_a_concatenation() {
    // `${n}` is lowered by the *checker* into `str_from_i64(n)`, so by the time
    // code generation sees the program it is an ordinary call and an ordinary
    // `+`. One conversion, one concat -- and nothing named `interp` for the
    // backend to know about.
    let ir_text = ir("fn main() { const n = 7; print_str(\"n = ${n}\"); }");
    assert_eq!(call_count(&ir_text, "@otter_str_from_i64"), 1, "{ir_text}");
    assert_eq!(call_count(&ir_text, "@otter_str_concat"), 1, "{ir_text}");
    assert!(
        !ir_text.to_lowercase().contains("interp"),
        "there is no interpolation instruction to emit:\n{ir_text}"
    );
}

#[test]
fn interpolating_a_string_needs_no_conversion() {
    // A `string` already *is* text, so `${s}` is just `s`: the module calls no
    // `str_from_*` at all. A conversion here would be a needless copy and a
    // second place for the value and its text to disagree.
    let ir_text = ir("fn main() { const s = \"x\"; print_str(\"${s}\"); }");
    assert_eq!(
        call_count(&ir_text, "@otter_str_from_"),
        0,
        "a string is already text:\n{ir_text}"
    );
}
