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

use otter_codegen::{check_main_exists, find_main, llvm_type_string, Module};

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
fn llvm_type_mapping_matches_the_spec() {
    assert_eq!(llvm_type_string(&TypeKind::I64), "i64");
    assert_eq!(llvm_type_string(&TypeKind::F64), "double");
    assert_eq!(llvm_type_string(&TypeKind::Bool), "i1");
    assert_eq!(llvm_type_string(&TypeKind::Str), "{ i32, i32, i8* }");
    assert_eq!(llvm_type_string(&TypeKind::Unit), "void");
}
