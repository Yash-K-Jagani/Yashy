//! String-operation lowering: `+` (concatenation), `==`/`!=` (byte equality),
//! `len`, and the two places a reference is dropped — a temporary an operation
//! consumed, and a local at the end of the scope that owns it.
//!
//! A `string` value is a pointer to a refcounted `%yashy_str`, so none of these
//! is an instruction — each one is a call into the runtime half of ledger L1.
//! These tests pin which call, that the compiler emits exactly one per operator,
//! and which operands are owned (and therefore released) and which are not.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and lower `source` to one LLVM IR module.
fn ir(source: &str) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    let hir = check_program(&ast, &sm, file).expect("these test sources must type-check");
    emit_program(&hir, "main.ya", CodegenOptions::default())
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
    // Two literals: neither is an owned temporary, so the concatenation releases
    // neither. The module's one release is the *binding* — the concatenation's
    // result was taken over by `s` and dropped when `s`'s scope ended.
    let ir_text = ir("fn main() { const s = \"Hello, \" + \"Yashy!\"; print_str(s); }");
    assert_eq!(
        call_count(&ir_text, "@yashy_str_concat"),
        1,
        "one concat, one call:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @yashy_str_concat(i8* @.str."),
        "a literal operand is addressed by its global:\n{ir_text}"
    );
    assert_eq!(
        call_count(&ir_text, "@yashy_str_release"),
        1,
        "the operands are static; only the binding is released:\n{ir_text}"
    );
}

#[test]
fn a_temporary_an_operation_built_is_released_when_consumed() {
    // `"n = ${n}"` is a literal and a `str_from_i64` result, so the concatenation
    // owns one of its operands and releases it; `print_str` then consumes the
    // concatenation itself and releases that. Two releases, and a program that
    // formats a number in a loop stops growing.
    let ir_text = ir("fn main() { const n = 7; print_str(\"n = ${n}\"); }");
    assert_eq!(
        call_count(&ir_text, "@yashy_str_release"),
        2,
        "the conversion and the concatenation are both released:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @yashy_str_release(i8* "),
        "the release is a call into the runtime:\n{ir_text}"
    );
}

#[test]
fn a_bound_string_is_released_at_scope_exit_not_by_the_read() {
    // `s` belongs to the local: the comparison and `print_str` read its bytes and
    // keep nothing, so neither releases it. The one release in the module is the
    // scope exit, and it comes after the last read.
    let ir_text = ir(
        "fn main() {\n\
         \x20   const s = \"x\" + \"y\";\n\
         \x20   if s == \"xy\" { print_str(s); }\n\
         }",
    );
    assert_eq!(
        call_count(&ir_text, "@yashy_str_release"),
        1,
        "one release, at scope exit:\n{ir_text}"
    );
    let release = ir_text
        .find("call void @yashy_str_release")
        .expect("a scope-exit release");
    let last_read = ir_text
        .find("call void @yashy_print_str")
        .expect("the binding is read");
    assert!(
        last_read < release,
        "the read comes before the release:\n{ir_text}"
    );
}

#[test]
fn reassignment_keeps_the_new_reference_before_dropping_the_old() {
    // `s = s + "x"` reads `s`, builds the replacement, and only then drops what
    // `s` held. Dropping first would free the bytes the concatenation was about
    // to read.
    let ir_text = ir(
        "fn main() {\n\
         \x20   var s = \"a\";\n\
         \x20   s = s + \"x\";\n\
         \x20   print_str(s);\n\
         }",
    );
    let concat = ir_text
        .find("call i8* @yashy_str_concat")
        .expect("the replacement is built");
    let release = ir_text
        .find("call void @yashy_str_release")
        .expect("the old value is dropped");
    assert!(
        concat < release,
        "the new value is built before the old is dropped:\n{ir_text}"
    );
}

#[test]
fn chained_concat_is_one_call_per_operator() {
    // `"a" + "b" + "c"` groups left, so it is two concatenations and the result
    // of the first feeds the second.
    let ir_text = ir("fn main() { const s = \"a\" + \"b\" + \"c\"; print_str(s); }");
    assert_eq!(
        call_count(&ir_text, "@yashy_str_concat"),
        2,
        "two operators, two calls:\n{ir_text}"
    );
}

#[test]
fn equality_compares_bytes_through_the_runtime() {
    // `icmp eq i8*` would compare the two addresses, and two identical literals
    // are not required to be the same address. The runtime compares contents.
    let ir_text = ir("fn main() { if \"a\" == \"b\" { print_str(\"same\"); } }");
    assert_eq!(call_count(&ir_text, "@yashy_str_eq"), 1, "{ir_text}");
    assert!(
        !ir_text.contains("icmp eq i8*"),
        "string equality must not compare addresses:\n{ir_text}"
    );
}

#[test]
fn inequality_is_the_equality_call_negated() {
    let ir_text = ir("fn main() { if \"a\" != \"b\" { print_str(\"differ\"); } }");
    assert_eq!(call_count(&ir_text, "@yashy_str_eq"), 1, "{ir_text}");
    assert!(
        ir_text.contains("xor i1 ") && ir_text.contains(", true"),
        "`!=` inverts the `==` result:\n{ir_text}"
    );
}

#[test]
fn len_is_a_runtime_call_and_yields_i64() {
    let ir_text = ir("fn main() { var hello = \"hello\"; print_i64(hello.len()); }");
    assert_eq!(call_count(&ir_text, "@yashy_str_len"), 1, "{ir_text}");
    assert!(
        ir_text.contains("call i64 @yashy_str_len(i8* "),
        "the length is an `i64`:\n{ir_text}"
    );
}

#[test]
fn byte_at_is_a_runtime_call_and_yields_i64() {
    // `byte_at` exists so the runtime's `yashy_str_byte_at` (ledger L1) is
    // reachable from Yashy: a declared symbol nothing calls is a contract with
    // no implementation behind it in practice. Out-of-range answers zero, which
    // the runtime test pins; this pins the call and the type.
    let ir_text = ir("fn main() { var abc = \"abc\"; print_i64(abc.byte_at(1)); }");
    assert_eq!(call_count(&ir_text, "@yashy_str_byte_at"), 1, "{ir_text}");
    assert!(
        ir_text.contains("call i64 @yashy_str_byte_at(i8* ") && ir_text.contains(", i64 1)"),
        "the byte is an `i64` and the index is passed through:\n{ir_text}"
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
    assert_eq!(call_count(&ir_text, "@yashy_str_concat"), 2, "{ir_text}");
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
    assert_eq!(call_count(&ir_text, "@yashy_str_from_i64"), 1, "{ir_text}");
    assert_eq!(call_count(&ir_text, "@yashy_str_concat"), 1, "{ir_text}");
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
        call_count(&ir_text, "@yashy_str_from_"),
        0,
        "a string is already text:\n{ir_text}"
    );
}
