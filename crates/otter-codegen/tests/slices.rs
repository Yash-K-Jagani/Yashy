//! Slice lowering: the three-word header, the backing allocation, and the two
//! runtime calls.
//!
//! The header layout is an ABI contract with `otter_slice` in the runtime
//! (`{ void *data; long long len; long long cap; }`), so these tests pin the
//! IR shapes the runtime expects rather than only that the program compiles.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram};

use otter_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("slices.ot", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(&check_source(src), "slices.ot", CodegenOptions::default())
}

#[test]
fn a_slice_literal_allocates_and_stores_its_elements() {
    let ir = emit("fn main() { var xs = []i64 { 10, 20 }; free_slice(xs); }");
    assert!(
        ir.contains("mul i64 2, 8"),
        "two `i64` elements are sixteen bytes:\n{ir}"
    );
    assert!(
        ir.contains("call i8* @otter_alloc(i64 %t") && ir.contains(", i64 8)"),
        "the backing array comes from the runtime allocator:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr i64, ptr") && ir.contains("store i64 20"),
        "each element is stored at its index:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue { i8*, i64, i64 } undef, i8*"),
        "the header starts from an undefined aggregate:\n{ir}"
    );
    assert!(
        ir.contains(", i64 2, 1") && ir.contains(", i64 2, 2"),
        "length and capacity are both the element count:\n{ir}"
    );
}

#[test]
fn an_empty_slice_literal_allocates_nothing() {
    let ir = emit("fn main() { var xs = []i64 {}; push(xs, 1); free_slice(xs); }");
    assert!(
        !ir.contains("call i8* @otter_alloc"),
        "an empty literal has a null backing pointer:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue { i8*, i64, i64 } undef, i8* null, 0"),
        "the empty header is null, zero, zero:\n{ir}"
    );
}

#[test]
fn pushing_passes_the_header_address_and_the_element_size() {
    let ir = emit("fn main() { var xs = []i64 { 1 }; push(xs, 2); free_slice(xs); }");
    assert!(
        ir.contains("call void @otter_slice_push(ptr %slot") && ir.contains(", i64 8)"),
        "push takes the binding's slot and an 8-byte element:\n{ir}"
    );
}

#[test]
fn freeing_a_slice_calls_the_runtime_with_the_binding() {
    let ir = emit("fn main() { var xs = []i64 { 1 }; free_slice(xs); }");
    assert!(
        ir.contains("call void @otter_slice_free(ptr %slot"),
        "free_slice passes the header's address:\n{ir}"
    );
}

/// Directory holding the `.ot` inputs and `.ll` expectations.
fn golden_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// `UPDATE_GOLDEN=1` rewrites the expectation instead of comparing it.
fn updating_goldens() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some()
}

/// Read a file with `\n` line endings whatever the checkout uses.
fn read_lf_only(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// Replace the host-specific `target datalayout` and `target triple` lines.
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
fn golden_slices_matches_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("slices.ot"));
    let actual = normalize_host_header(&emit(&input));
    let expected_path = golden_dir().join("slices.expected.ll");
    if updating_goldens() {
        std::fs::write(&expected_path, &actual)
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
