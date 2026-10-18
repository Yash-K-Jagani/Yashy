//! Codegen scaffold tests: the LLVM module header, string interning, the type
//! mapping, and `main` discovery.
//!
//! The `check_source` helper is the same lex -> parse -> check pipeline the
//! driver will run, written once here so these tests do not depend on the
//! driver existing.

use otter_common::{Severity, SourceMap, Span};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram, TypeKind};

use otter_codegen::{check_main_exists, emit_program, find_main, llvm_type_string, CodegenOptions, Module};

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

#[test]
fn finish_emits_the_opaque_pointer_header() {
    let ir = Module::new().finish();
    assert!(
        ir.contains("target datalayout"),
        "missing datalayout:\n{ir}"
    );
    assert!(ir.contains("%otter_str = type"), "missing str type:\n{ir}");
    assert!(
        ir.contains("declare void @otter_print_str(i8*)"),
        "the print symbols must return void per ledger L1:\n{ir}"
    );
    // The triple follows the LLVM target the *driver* will pass to `clang`, so
    // assert against that one function rather than a second copy of the rule.
    let triple = otter_codegen::host_target_triple();
    assert!(
        ir.contains(&format!("target triple = \"{triple}\"")),
        "wrong target triple for this host:\n{ir}"
    );
}

#[test]
fn intern_string_dedupes_identical_bytes() {
    let mut module = Module::new();
    let first = module.intern_string(b"hi");
    let second = module.intern_string(b"hi");
    let third = module.intern_string(b"ho");
    assert_eq!(first, "@.str.0");
    assert_eq!(second, "@.str.0", "identical bytes must share one global");
    assert_eq!(
        third, "@.str.1",
        "different bytes must get their own global"
    );
    let ir = module.finish();
    let globals = ir.lines().filter(|line| line.starts_with("@.str.")).count();
    assert_eq!(globals, 2, "two globals for three interns:\n{ir}");
}

#[test]
fn string_global_is_nul_terminated_and_escaped() {
    let mut module = Module::new();
    module.intern_string(b"a\"b");
    let ir = module.finish();
    assert!(
        ir.contains("c\"a\\22b\\00\""),
        "quote must escape as \\22 and the global must end with \\00:\n{ir}"
    );
    assert!(
        ir.contains("[4 x i8]"),
        "a 3-byte string plus its NUL is 4 bytes:\n{ir}"
    );
}

#[test]
fn missing_main_is_a_diagnostic_not_a_link_failure() {
    let program = check_source("fn helper() -> i64 { return 1; }");
    assert!(find_main(&program.items).is_none());
    let err = check_main_exists(&program.items)
        .expect_err("a program with no main must fail here, not at link time");
    assert_eq!(err.severity, Severity::Error);
    assert!(
        err.message.contains("main"),
        "the diagnostic must name main: {}",
        err.message
    );
    assert_eq!(err.span, Span::new(0, 0));
}

#[test]
fn main_may_be_absent_or_return_i64() {
    // "absent" here means the `-> T` annotation is absent: `main` returns
    // nothing.
    let unit = check_source("fn main() {}");
    let found = find_main(&unit.items).expect("main must be found");
    assert_eq!(found.name, "main");
    assert!(check_main_exists(&unit.items).is_ok());

    let i64_ret = check_source("fn main() -> i64 { return 0; }");
    let _found = find_main(&i64_ret.items).expect("main must be found");
    assert!(check_main_exists(&i64_ret.items).is_ok());

    let bad_ret = check_source("fn main() -> bool { return true; }");
    let err = check_main_exists(&bad_ret.items)
        .expect_err("main returning bool is not a valid entry point");
    assert!(
        err.message.contains("i64") || err.message.contains("return"),
        "the diagnostic must explain the return type: {}",
        err.message
    );
}

#[test]
fn main_may_not_take_parameters() {
    // The C runtime calls the entry point with its own argument convention, and
    // the symbol this backend emits takes none. Accepting a parameter would
    // produce a `main` the loader cannot call.
    let with_param = check_source("fn main(argc: i64) { print_i64(argc); }");
    let err = check_main_exists(&with_param.items)
        .expect_err("a `main` with a parameter is not callable");
    assert!(
        err.message.contains("parameter") && err.message.contains("argc"),
        "the diagnostic must name the offending parameter: {}",
        err.message
    );
    assert_eq!(
        err.span,
        with_param
            .items
            .iter()
            .find_map(|item| match item {
                otter_sema::HirItem::Fn(f) => f.params.first().map(|p| p.span),
                _ => None,
            })
            .expect("the parameter must have a span"),
        "the diagnostic must point at the parameter, not the function"
    );
}

/// Lower a whole program's source to one IR module.
fn ir_of(src: &str) -> String {
    emit_program(&check_source(src), "main.ot", CodegenOptions::default())
}

#[test]
fn a_unit_main_is_emitted_as_i32_and_exits_zero() {
    // `void main` leaves the process exit status as whatever the last call left
    // in the return register, so `fn main() { print_str("hello"); }` exits with
    // the length its last call happened to return. The entry point therefore
    // returns `i32` even when the Otter signature is `()`.
    let ir_text = ir_of("fn main() { print_str(\"hi\"); }");
    assert!(
        ir_text.contains("define i32 @main()"),
        "the entry point is an `i32` function:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("define void @main"),
        "and never a `void` one:\n{ir_text}"
    );
    assert!(
        ir_text.contains("\nret i32 0"),
        "the fall-through terminator yields zero:\n{ir_text}"
    );
}

#[test]
fn an_explicit_unit_return_in_main_still_yields_zero() {
    let ir_text = ir_of("fn main() { print_str(\"hi\"); return; }");
    assert!(
        ir_text.contains("ret i32 0"),
        "`return;` in `main` is the entry point's zero:\n{ir_text}"
    );
    assert!(!ir_text.contains("ret void"), "no `ret void` in `main`:\n{ir_text}");
}

#[test]
fn an_i64_main_truncates_to_the_entry_point_width() {
    let ir_text = ir_of("fn main() -> i64 { return 7; }");
    assert!(
        ir_text.contains("define i32 @main()"),
        "the entry point is still `i32`:\n{ir_text}"
    );
    assert!(
        ir_text.contains("trunc i64 ") && ir_text.contains("to i32"),
        "the `i64` result narrows to the exit code's width:\n{ir_text}"
    );
}

#[test]
fn an_ordinary_function_keeps_its_own_return_type() {
    // The `i32` rule is the entry point's, not every function's.
    let ir_text = ir_of("fn seven() -> i64 { return 7; }\nfn main() { print_i64(seven()); }");
    assert!(
        ir_text.contains("define i64 @seven()"),
        "a helper keeps `i64`:\n{ir_text}"
    );
    assert!(
        ir_text.contains("ret i64 7"),
        "and so does its `ret`:\n{ir_text}"
    );
}

#[test]
fn input_calls_use_the_runtime_symbols() {
    // The Otter names are `read_line`/`parse_i64`, and the runtime's carry the
    // `otter_` prefix. Nothing else changes: the argument and result types are
    // already the IR types, so the call is the whole lowering.
    let ir_text = ir_of(
        "fn main() {\n\
         \x20   const line = read_line();\n\
         \x20   print_i64(parse_i64(line));\n\
         \x20   print_f64(parse_f64(line));\n\
         }",
    );
    assert!(
        ir_text.contains("call i8* @otter_read_line()"),
        "a no-argument call to the runtime reader:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i64 @otter_parse_i64(i8* "),
        "the line is passed through unchanged:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call double @otter_parse_f64(i8* "),
        "and the floating-point parser is the same shape:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("call i8* @read_line"),
        "the bare Otter name is not a symbol the runtime defines:\n{ir_text}"
    );
}

#[test]
fn a_struct_field_is_read_with_the_declaration_order_index() {
    // A struct value is an aggregate, so reading a field is `extractvalue` and
    // needs no address. The index is the declaration order position: this
    // literal writes `y` first, and `p.x` must still extract field 0.
    let ir_text = ir_of(
        "struct P { x: i64, y: f64 }\n\
         fn main() {\n\
         \x20   const p = P { y: 2.0, x: 1 };\n\
         \x20   print_i64(p.x);\n\
         \x20   print_f64(p.y);\n\
         }",
    );
    assert!(
        ir_text.contains("%struct.P = type { i64, double }"),
        "the aggregate follows the declaration:\n{ir_text}"
    );
    let extracts: Vec<&str> = ir_text
        .lines()
        .filter(|line| line.contains("extractvalue"))
        .collect();
    assert_eq!(extracts.len(), 2, "one extract per field read:\n{ir_text}");
    assert!(
        extracts[0].trim_end().ends_with(", 0"),
        "`p.x` is field 0: {}",
        extracts[0]
    );
    assert!(
        extracts[1].trim_end().ends_with(", 1"),
        "`p.y` is field 1: {}",
        extracts[1]
    );
}

#[test]
fn the_number_to_string_conversions_call_the_runtime() {
    let ir_text = ir_of(
        "fn main() {\n\
         \x20   const a = str_from_i64(1);\n\
         \x20   const b = str_from_f64(1.5);\n\
         \x20   const c = str_from_bool(true);\n\
         \x20   print_str(a + b + c);\n\
         }",
    );
    assert!(
        ir_text.contains("call i8* @otter_str_from_i64(i64 "),
        "the integer conversion is a direct call:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @otter_str_from_f64(double "),
        "and so is the float one:\n{ir_text}"
    );
    // The bool takes a different path: the runtime declares an `i64` because C
    // has no one-bit type, so the `i1` is widened first rather than handed over
    // as a value the callee would read differently.
    assert!(
        ir_text.contains("zext i1 "),
        "the bool is widened:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @otter_str_from_bool(i64 "),
        "and the call passes the widened value:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("@otter_str_from_bool(i1 "),
        "never as an `i1`, which the declaration does not take:\n{ir_text}"
    );
}

#[test]
fn llvm_type_mapping_matches_the_spec() {
    assert_eq!(llvm_type_string(&TypeKind::I64), "i64");
    assert_eq!(llvm_type_string(&TypeKind::F64), "double");
    assert_eq!(llvm_type_string(&TypeKind::Bool), "i1");
    assert_eq!(llvm_type_string(&TypeKind::Str), "{ i32, i32, i8* }");
    assert_eq!(llvm_type_string(&TypeKind::Unit), "void");
}
