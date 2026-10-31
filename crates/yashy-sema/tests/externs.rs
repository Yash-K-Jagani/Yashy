//! Type checking `extern` declarations and their call sites.
//!
//! The rules pinned here are the FFI trust boundary's: a declaration's C types
//! resolve, its `void` is a return type and nothing else, a `string` may stand
//! where a `*const u8` is wanted (the copying itself is codegen's job), and a C
//! pointer returned across the boundary is linear.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirItem, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
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

const STRLEN: &str =
    r#"extern "C" { unsafe fn strlen(s: *const u8) -> usize; } fn main() { }"#;

#[test]
fn an_extern_declaration_reaches_the_hir() {
    let hir = check(STRLEN).expect("the declaration checks");
    let block = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Extern(block) => Some(block),
            _ => None,
        })
        .expect("the extern item is in the HIR");
    assert_eq!(block.abi.as_deref(), Some("C"));
    assert_eq!(block.decls.len(), 1);
    assert_eq!(block.decls[0].name, "strlen");
}

#[test]
fn an_extern_function_is_callable() {
    // The literal is a `string`; the parameter is a `*const u8`. Accepting it
    // is the FFI boundary's one deliberate coercion (spec §8.1).
    check(
        r#"extern "C" { unsafe fn strlen(s: *const u8) -> usize; }
           fn main() { strlen("hi"); }"#,
    )
    .expect("a string may stand where a C string pointer is wanted");
}

#[test]
fn extern_parameters_are_checked() {
    assert_rejected(
        r#"extern "C" { unsafe fn strlen(s: *const u8) -> usize; }
           fn main() { strlen(1); }"#,
        "expected",
    );
}

#[test]
fn a_call_to_an_undeclared_extern_function_is_an_error() {
    assert_rejected(r#"fn main() { puts("x"); }"#, "cannot find function");
}

#[test]
fn an_extern_function_is_linear_if_its_return_is_a_pointer() {
    assert_rejected(
        r#"extern "C" { unsafe fn malloc() -> *mut u8; }
           fn main() { let p = malloc(); let q = p; }"#,
        "move",
    );
}

#[test]
fn void_is_allowed_only_as_a_return_type() {
    check(r#"extern "C" { unsafe fn f() -> void; } fn main() { }"#)
        .expect("`void` is a legal return type");
    assert_rejected(
        r#"extern "C" { unsafe fn f(x: void) -> i32; } fn main() { }"#,
        "return type",
    );
}

#[test]
fn a_duplicate_extern_name_is_a_duplicate_definition() {
    assert_rejected(
        r#"extern "C" { unsafe fn puts(s: *const u8) -> i32; }
           fn puts() { } fn main() { }"#,
        "duplicate definition",
    );
}

#[test]
fn a_raw_pointer_argument_requires_an_unsafe_call() {
    // A pointer cannot be copied into a fresh buffer, so the call site has to
    // say that it is lending C the address and whatever it names.
    let program = |call: &str| {
        format!(
            r#"extern "C" {{
                   unsafe fn malloc() -> *mut u8;
                   unsafe fn touch(p: *mut u8) -> void;
               }}
               fn main() {{ const p = malloc(); {call} }}"#
        )
    };
    assert_rejected(&program("touch(p);"), "requires an `unsafe` call");
    check(&program("unsafe touch(p);")).expect("an unsafe call may lend the pointer");
}

#[test]
fn a_wrong_signature_is_caught_by_yashy_not_c() {
    // C would hand `i32` to a variadic printf and print garbage; Yashy's
    // checker refuses the mismatch at the call site, before anything is linked.
    assert_rejected(
        r#"extern "C" { unsafe fn strlen(s: *const u8) -> i32; }
           fn main() { print(strlen("hi")); }"#,
        "i32",
    );
}

/// A program declaring variadic `printf` and a `main` whose body is `body`.
fn printf_program(body: &str) -> String {
    format!(
        "extern \"C\" {{ unsafe fn printf(fmt: *const u8, ...) -> i32; }}\n\
         fn main() {{ {body} }}"
    )
}

#[test]
fn a_variadic_declaration_reaches_the_hir() {
    let hir = check(&printf_program("")).expect("the declaration checks");
    let decl = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Extern(block) => block.decls.first(),
            _ => None,
        })
        .expect("the declaration reaches the HIR");
    assert!(decl.variadic, "the marker must survive into the HIR");
    assert_eq!(decl.params.len(), 1, "the fixed parameter stays checked");
}

#[test]
fn a_variadic_call_accepts_arguments_beyond_the_fixed_ones() {
    check(&printf_program(r#"unsafe printf("%d\n", 42 as i32);"#))
        .expect("one fixed argument and one variadic argument check");
    check(&printf_program(
        r#"unsafe printf("%d %s\n", 42 as i32, "x");"#,
    ))
    .expect("two variadic arguments check");
}

#[test]
fn a_variadic_call_still_needs_its_fixed_arguments() {
    assert_rejected(
        &printf_program("unsafe printf();"),
        "expected at least 1 arguments",
    );
}

#[test]
fn a_variadic_call_needs_unsafe_for_the_extra_arguments() {
    // The fixed format argument alone is a safe call; once a value past it is
    // passed, the callee reads bytes Yashy did not check, so the call says so.
    assert_rejected(
        &printf_program(r#"printf("%d\n", 42 as i32);"#),
        "requires an `unsafe` call",
    );
}

#[test]
fn a_variadic_argument_must_have_a_c_width() {
    assert_rejected(
        &printf_program(r#"unsafe printf("%d\n", 42);"#),
        "needs an explicit C width",
    );
}

#[test]
fn a_variadic_argument_must_be_a_type_c_can_pass() {
    assert_rejected(
        &printf_program(r#"unsafe printf("%d\n", true);"#),
        "cannot have type",
    );
}

#[test]
fn two_declarations_that_disagree_about_the_marker_are_e0900() {
    let program = r#"extern "C" {
        unsafe fn printf(fmt: *const u8) -> i32;
        unsafe fn also_printf(fmt: *const u8, ...) -> i32 link_name "printf";
    }
    fn main() { }"#;
    assert_rejected(program, "`(*u8) -> i32` and `(*u8, ...) -> i32`");
}
