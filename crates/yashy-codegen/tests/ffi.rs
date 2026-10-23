//! The C boundary's copying rules.
//!
//! Spec §8.1: crossing into C copies. A `string` is passed as a fresh
//! NUL-terminated buffer that C may keep after the call; a call that writes
//! `unsafe` opts out and lends C the string's own bytes instead. A raw pointer
//! is never copied — it is an address — so passing one requires the marker.
//!
//! These tests pin the IR shape, not the runtime behaviour; the runtime's copy
//! is exercised end to end by the libc program in Task 5.

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::{SourceMap, Span};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirExternDecl, HirParam, HirProgram, Type, TypeKind};

use yashy_codegen::{emit_program, gen_ffi_call, CodegenOptions, FnCtx, GenValue, Module};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("ffi_copy.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(&check_source(src), "ffi_copy.ya", CodegenOptions::default())
}

const STRLEN: &str = r#"extern "C" link "m" {
    unsafe fn strlen(s: *const u8) -> usize;
}

fn main() {
    strlen("hi");
}
"#;

#[test]
fn a_string_argument_is_copied_into_a_temporary() {
    let ir = emit(STRLEN);
    assert!(
        ir.contains("call i8* @yashy_str_cstr("),
        "the string must be copied before the C call:\n{ir}"
    );
    let copy = ir
        .find("call i8* @yashy_str_cstr(")
        .expect("the copy is present");
    let call = ir
        .find("call i64 @strlen(")
        .expect("the C call is present");
    assert!(
        copy < call,
        "the copy must be emitted before the call, not after:\n{ir}"
    );
    assert!(
        !ir.contains("@strlen(i8* @.str."),
        "the string's own header pointer must not cross the boundary:\n{ir}"
    );
}

#[test]
fn the_temporary_is_released_after_the_call() {
    let ir = emit(STRLEN);
    let call = ir
        .find("call i64 @strlen(")
        .expect("the C call is present");
    let free = ir
        .rfind("call void @yashy_free(i8*")
        .expect("the copy is freed");
    assert!(
        free > call,
        "the temporary must outlive the call and be freed after it:\n{ir}"
    );
}

#[test]
fn an_unsafe_call_lends_the_string_and_frees_nothing() {
    let ir = emit(
        r#"extern "C" link "m" {
            unsafe fn strlen(s: *const u8) -> usize;
        }

        fn main() {
            unsafe strlen("hi");
        }
        "#,
    );
    assert!(
        ir.contains("call i8* @yashy_str_ptr("),
        "an unsafe call passes the string's own bytes:\n{ir}"
    );
    assert!(
        !ir.contains("call i8* @yashy_str_cstr("),
        "an unsafe call must not copy:\n{ir}"
    );
    assert!(
        !ir.contains("call void @yashy_free(i8*"),
        "an unsafe call must not free a pointer C may have kept:\n{ir}"
    );
}

#[test]
fn a_raw_pointer_argument_is_passed_unchanged() {
    // A hand-built lowering context: obtaining a C pointer from Yashy code
    // needs `malloc`, and that call's numeric argument is Task 5's cast work.
    // What matters here is only that `gen_ffi_call` does not wrap a pointer.
    let mut module = Module::new();
    let decl = HirExternDecl {
        name: "touch".to_string(),
        params: vec![HirParam {
            name: "p".to_string(),
            ty: Type {
                kind: TypeKind::CPtr(Box::new(Type {
                    kind: TypeKind::CUInt(8),
                    span: Span::SYNTHETIC,
                })),
                span: Span::SYNTHETIC,
            },
            span: Span::SYNTHETIC,
        }],
        ret: Type {
            kind: TypeKind::Unit,
            span: Span::SYNTHETIC,
        },
        span: Span::SYNTHETIC,
    };
    let arg = GenValue {
        value: "%p".to_string(),
        ty: TypeKind::CPtr(Box::new(Type {
            kind: TypeKind::CUInt(8),
            span: Span::SYNTHETIC,
        })),
    };
    {
        let mut ctx = FnCtx::new(&mut module, TypeKind::Unit);
        let _ = gen_ffi_call(&mut ctx, &decl, &[arg], true);
    }
    let ir = module.finish();
    assert!(
        ir.contains("call void @touch(i8* %p)"),
        "a pointer crosses as itself:\n{ir}"
    );
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn updating_goldens() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some()
}

fn read_lf_only(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

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
fn golden_ffi_copy_matches_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("ffi_copy.ya"));
    let actual = normalize_host_header(&emit(&input));
    let expected_path = golden_dir().join("ffi_copy.expected.ll");
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
