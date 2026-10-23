//! The IR contract for `extern` declarations.
//!
//! A declaration must become a `declare`, never a `define`: emitting an empty
//! body would link against the wrong symbol and silently call nothing. The
//! golden pins the signatures codegen hands the linker for the two libc
//! functions the milestone is named after.

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::{SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

use yashy_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("extern.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    let hir = check_source(src);
    emit_program(&hir, "extern.ya", CodegenOptions::default())
}

const PROGRAM: &str = r#"extern "C" link "m" {
    unsafe fn puts(s: *const u8) -> i32;
    unsafe fn strlen(s: *const u8) -> usize;
}

fn main() {
    strlen("hi");
}
"#;

#[test]
fn an_extern_declaration_emits_a_declare_not_a_define() {
    let ir = emit(PROGRAM);
    assert!(
        ir.contains("declare i32 @puts(i8*)"),
        "puts must be declared with its C width:\n{ir}"
    );
    assert!(
        ir.contains("declare i64 @strlen(i8*)"),
        "size_t lowers as i64:\n{ir}"
    );
    assert!(
        !ir.contains("define i32 @puts"),
        "an extern declaration must never gain a body:\n{ir}"
    );
}

#[test]
fn a_declaration_does_not_disturb_the_entry_point() {
    let ir = emit(PROGRAM);
    assert!(
        ir.contains("define i32 @main()"),
        "the program still has its own entry point:\n{ir}"
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
fn golden_extern_matches_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("extern.ya"));
    let actual = normalize_host_header(&emit(&input));
    let expected_path = golden_dir().join("extern.expected.ll");
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
