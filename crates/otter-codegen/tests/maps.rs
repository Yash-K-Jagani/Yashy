//! Map lowering: one opaque table pointer and six runtime calls.
//!
//! The runtime owns the layout, so these tests pin the *call shapes*: the key
//! kind and strides at creation, slot addresses for insert, and the 0/1 flag
//! turned into an `Option` for lookups.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram};

use otter_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("maps.ot", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(&check_source(src), "maps.ot", CodegenOptions::default())
}

#[test]
fn map_new_carries_the_key_kind_and_the_two_strides() {
    let ir = emit("fn main() { var m: map[i64]i64 = map_new(); map_free(m); }");
    assert!(
        ir.contains("call i8* @otter_map_new(i64 0, i64 8, i64 8)"),
        "an `i64` key is kind 0 with an eight-byte key and value:\n{ir}"
    );
}

#[test]
fn a_string_key_map_is_created_as_kind_two() {
    let ir = emit("fn main() { var m: map[string]i64 = map_new(); map_free(m); }");
    assert!(
        ir.contains("call i8* @otter_map_new(i64 2, i64 8, i64 8)"),
        "a `string` key is kind 2 and still one word wide:\n{ir}"
    );
}

#[test]
fn a_bool_key_map_is_created_as_kind_one() {
    let ir = emit("fn main() { var m: map[bool]i64 = map_new(); map_free(m); }");
    assert!(
        ir.contains("call i8* @otter_map_new(i64 1, i64 1, i64 8)"),
        "a `bool` key is kind 1 with a one-byte key:\n{ir}"
    );
}

#[test]
fn insert_hands_the_runtime_two_slots() {
    let ir = emit(
        "fn main() { var m: map[i64]i64 = map_new(); map_insert(m, 7, 9); map_free(m); }",
    );
    assert!(
        ir.contains("store i64 7, ptr %slot") && ir.contains("store i64 9, ptr %slot"),
        "the key and the value are stored before the call:\n{ir}"
    );
    assert!(
        ir.contains("call void @otter_map_insert(i8* %t") && ir.contains(", ptr %slot"),
        "insert takes the table and two addresses:\n{ir}"
    );
}

#[test]
fn a_lookup_turns_the_flag_into_an_option() {
    let ir = emit(
        "fn main() { var m: map[i64]i64 = map_new(); var v = map_get(m, 1); map_free(m); }",
    );
    assert!(
        ir.contains("@otter_map_get(i8* %t") && ir.contains(", ptr %slot"),
        "get takes the table, the key slot and an out slot:\n{ir}"
    );
    assert!(
        ir.contains("trunc i64 %t") && ir.contains("insertvalue { i8, i64 }"),
        "the 0/1 answer becomes the Option tag and payload:\n{ir}"
    );
}

#[test]
fn remove_uses_the_remove_entry_point() {
    let ir = emit(
        "fn main() { var m: map[i64]i64 = map_new(); var v = map_remove(m, 1); map_free(m); }",
    );
    assert!(
        ir.contains("call i64 @otter_map_remove("),
        "remove is a distinct runtime call:\n{ir}"
    );
}

#[test]
fn len_calls_the_runtime() {
    let ir = emit(
        "fn main() { var m: map[i64]i64 = map_new(); var n = map_len(m); print_i64(n); \
         map_free(m); }",
    );
    assert!(
        ir.contains("call i64 @otter_map_len(i8* %t"),
        "the length is a runtime question, not a header field:\n{ir}"
    );
}

#[test]
fn free_passes_the_table_pointer() {
    let ir = emit("fn main() { var m: map[i64]i64 = map_new(); map_free(m); }");
    assert!(
        ir.contains("call void @otter_map_free(i8* %t"),
        "free takes the binding's table value:\n{ir}"
    );
}

/// Directory holding the `.ot` inputs and `.ll` expectations.
fn golden_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Compare (or, under `UPDATE_GOLDEN=1`, rewrite) one pinned program.
fn golden_case(name: &str) {
    let input = std::fs::read_to_string(golden_dir().join(format!("{name}.ot")))
        .unwrap_or_else(|e| panic!("cannot read {name}.ot: {e}"))
        .replace("\r\n", "\n");
    let actual = normalize_host_header(&emit(&input));
    let expected_path = golden_dir().join(format!("{name}.expected.ll"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&expected_path, &actual)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", expected_path.display()));
        return;
    }
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", expected_path.display()))
        .replace("\r\n", "\n");
    assert_eq!(
        actual,
        normalize_host_header(&expected),
        "generated IR drifted from the pinned contract; \
         rerun with UPDATE_GOLDEN=1 only after checking the diff"
    );
}

#[test]
fn golden_maps_matches_the_pinned_contract() {
    golden_case("maps");
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
