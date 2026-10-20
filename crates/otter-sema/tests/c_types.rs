//! The C type spellings at the type-checking edge.
//!
//! Two rules are pinned here: how a C spelling maps to a checked type, and the
//! fact that those spellings are rejected outside an `extern` declaration.
//! The second rule is what keeps the C type names from quietly becoming a
//! second numeric tower inside ordinary Otter code.

use otter_common::{Diagnostic, SourceMap};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram, TypeKind};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ot", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the declaration");
    check_program(&program, &sources, file)
}

fn assert_rejected(src: &str, needle: &str) {
    match check(src) {
        Ok(_) => panic!("expected an error containing `{needle}`"),
        Err(diagnostics) => assert!(
            diagnostics.iter().any(|d| d.message.contains(needle)),
            "expected an error containing `{needle}`, got {:?}",
            diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
        ),
    }
}

#[test]
fn c_integer_spellings_map_to_widths() {
    assert_eq!(TypeKind::from_c_spelling("i8"), Some(TypeKind::CInt(8)));
    assert_eq!(TypeKind::from_c_spelling("i16"), Some(TypeKind::CInt(16)));
    assert_eq!(TypeKind::from_c_spelling("i32"), Some(TypeKind::CInt(32)));
    assert_eq!(TypeKind::from_c_spelling("u8"), Some(TypeKind::CUInt(8)));
    assert_eq!(TypeKind::from_c_spelling("u16"), Some(TypeKind::CUInt(16)));
    assert_eq!(TypeKind::from_c_spelling("u32"), Some(TypeKind::CUInt(32)));
    assert_eq!(TypeKind::from_c_spelling("u64"), Some(TypeKind::CUInt(64)));
    assert_eq!(TypeKind::from_c_spelling("f32"), Some(TypeKind::CFloat(32)));
    assert_eq!(TypeKind::from_c_spelling("usize"), Some(TypeKind::CSize));
    assert_eq!(TypeKind::from_c_spelling("void"), Some(TypeKind::CVoid));
}

#[test]
fn the_shared_widths_name_the_existing_language_types() {
    // An Otter `i64` and a C `int64_t` are the same machine value; giving them
    // separate type kinds would make every `extern` call a cast for no gain.
    assert_eq!(TypeKind::from_c_spelling("i64"), Some(TypeKind::I64));
    assert_eq!(TypeKind::from_c_spelling("f64"), Some(TypeKind::F64));
}

#[test]
fn only_the_ffi_only_spellings_are_marked_as_such() {
    for spelling in ["i8", "i16", "i32", "u8", "u16", "u32", "u64", "f32", "usize", "void"] {
        assert!(
            TypeKind::is_c_only_spelling(spelling),
            "`{spelling}` is an FFI-only spelling"
        );
    }
    for spelling in ["i64", "f64", "bool", "string", "Point"] {
        assert!(
            !TypeKind::is_c_only_spelling(spelling),
            "`{spelling}` is an ordinary Otter spelling"
        );
    }
}

#[test]
fn c_types_are_rejected_outside_an_extern_block() {
    assert_rejected("fn f(x: i32) {}", "extern");
    assert_rejected("fn f() -> u8 { }", "extern");
    assert_rejected("fn f(x: *const u8) {}", "extern");
    assert_rejected("fn f() -> void { }", "extern");
}

#[test]
fn an_otter_pointer_is_still_an_otter_pointer() {
    // `*mut T` predates the FFI edge and must keep its old meaning in ordinary
    // code: an owning, linear pointer. Only `*const`/bare pointers are new.
    check("fn f(x: *mut i64) {}").expect("an owning pointer is ordinary Otter code");
}

#[test]
fn a_c_numeric_type_is_allowed_as_a_cast_target() {
    // `as` is the one place outside an `extern` block where a C spelling is
    // legal: it is how an Otter program produces the `usize` `malloc` wants.
    check(r#"fn main() { const n = 64 as usize; print_i64(n as i64); }"#)
        .expect("an explicit cast may name a C type");
}

#[test]
fn a_c_non_numeric_type_is_not_a_cast_target() {
    assert_rejected(r#"fn main() { const x = 1 as *mut u8; }"#, "cannot cast");
}
