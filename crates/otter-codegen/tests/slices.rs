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
    let ir = emit("fn main() { var xs = []i64 { 10, 20 }; xs.free(); }");
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
    let ir = emit("fn main() { var xs = []i64 {}; xs.push(1); xs.free(); }");
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
    let ir = emit("fn main() { var xs = []i64 { 1 }; xs.push(2); xs.free(); }");
    assert!(
        ir.contains("call void @otter_slice_push(ptr %slot") && ir.contains(", i64 8)"),
        "push takes the binding's slot and an 8-byte element:\n{ir}"
    );
}

#[test]
fn freeing_a_slice_calls_the_runtime_with_the_binding() {
    let ir = emit("fn main() { var xs = []i64 { 1 }; xs.free(); }");
    assert!(
        ir.contains("call void @otter_slice_free(ptr %slot"),
        "free_slice passes the header's address:\n{ir}"
    );
}

#[test]
fn len_of_a_slice_extracts_the_length_field() {
    let ir = emit("fn main() { var xs = []i64 { 1, 2 }; print_i64(xs.len()); }");
    assert!(
        ir.contains("extractvalue { i8*, i64, i64 } %t") && ir.contains(", 1"),
        "the length is field 1 of the header, with no call:\n{ir}"
    );
    assert!(
        !ir.contains("call i64 @otter_str_len"),
        "a slice length is not a string length:\n{ir}"
    );
}

#[test]
fn indexing_calls_the_runtime_and_builds_an_option() {
    let ir = emit(
        "fn main() { var xs = []i64 { 7 }; \
         var o = switch xs[0] { case Some(v) { v } case None { 0 } }; \
         print_i64(o); }",
    );
    assert!(
        ir.contains("call i64 @otter_slice_get(ptr") && ir.contains(", i64 8)"),
        "`get` takes the header, the index and the element size:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue { i8, i64 } undef, i8"),
        "the answer is an `Option[i64]` with its tag:\n{ir}"
    );
}

#[test]
fn a_negative_index_reaches_the_runtime_as_written() {
    let ir = emit("fn main() { var xs = []i64 { 7 }; var o = xs[0 - 1]; xs.free(); }");
    assert!(
        (ir.contains("sub i64 0, 1") || ir.contains("i64 -1"))
            && ir.contains("@otter_slice_get"),
        "the runtime decides whether a negative index is in range:\n{ir}"
    );
}

#[test]
fn pop_calls_the_runtime_with_the_binding() {
    let ir = emit("fn main() { var xs = []i64 { 7 }; var o = xs.pop(); xs.free(); }");
    assert!(
        ir.contains("call i64 @otter_slice_pop(ptr %slot") && ir.contains(", i64 8)"),
        "pop updates the header in place and copies the element out:\n{ir}"
    );
}

#[test]
fn for_in_branches_on_the_length_and_reloads_the_header() {
    let ir = emit(
        "fn main() { var xs = []i64 { 1 }; var s = 0; for x in xs { s = s + x; } print_i64(s); }",
    );
    let cond = ir
        .split("_cond:")
        .nth(1)
        .unwrap_or_else(|| panic!("a loop condition block:\n{ir}"));
    assert!(
        cond.contains("load i64") && cond.contains("icmp slt i64"),
        "the condition compares the index with the current length:\n{ir}"
    );
    let body = ir
        .split("_body:")
        .nth(1)
        .unwrap_or_else(|| panic!("a loop body block:\n{ir}"));
    assert!(
        body.contains("getelementptr i64, ptr"),
        "the body reloads the data pointer each iteration:\n{ir}"
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

/// Compare (or, under `UPDATE_GOLDEN=1`, rewrite) one pinned program.
fn golden_case(name: &str) {
    let input = read_lf_only(&golden_dir().join(format!("{name}.ot")));
    let actual = normalize_host_header(&emit(&input));
    let expected_path = golden_dir().join(format!("{name}.expected.ll"));
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

#[test]
fn golden_slices_matches_the_pinned_contract() {
    golden_case("slices");
}

#[test]
fn golden_slice_operations_match_the_pinned_contract() {
    golden_case("slices_ops");
}

#[test]
fn a_string_literal_element_is_retained_by_the_slice() {
    let ir = emit("fn main() { var xs = []string { \"a\" }; xs.push(\"b\"); xs.free(); }");
    assert_eq!(
        ir.matches("call i8* @otter_str_retain").count(),
        2,
        "the literal element and the pushed element each get one reference:\n{ir}"
    );
    assert!(
        ir.contains("call void @otter_slice_free_strings(ptr"),
        "a `[]string` is freed element by element:\n{ir}"
    );
    assert!(
        !ir.contains("call void @otter_slice_free(ptr"),
        "the plain free would leak every element's reference:\n{ir}"
    );
}

#[test]
fn a_string_temporary_pushed_into_a_slice_transfers_its_reference() {
    let ir = emit("fn main() { var xs = []string {}; xs.push(\"a\" + \"b\"); xs.free(); }");
    assert!(
        !ir.contains("call i8* @otter_str_retain"),
        "a concatenation already owns a reference; retaining again would leak one:\n{ir}"
    );
    assert!(
        ir.contains("call void @otter_slice_free_strings(ptr"),
        "the transferred reference is released when the slice is:\n{ir}"
    );
}
