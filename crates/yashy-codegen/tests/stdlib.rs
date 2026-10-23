//! The M6 Task 10 builtins at the lowering edge: the boundary copies for file
//! and environment calls, and the owned substring.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

use yashy_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("stdlib.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(&check_source(src), "stdlib.ya", CodegenOptions::default())
}

#[test]
fn read_file_copies_the_path_and_frees_the_copy() {
    let ir = emit("fn main() { var text = read_file(\"a.txt\"); print_str(text); }");
    assert_eq!(
        ir.matches("call i8* @yashy_str_cstr").count(),
        1,
        "the path crosses as one fresh C string:\n{ir}"
    );
    assert!(
        ir.contains("call i8* @yashy_read_file(i8* "),
        "the copied pointer is what the callee reads:\n{ir}"
    );
    assert_eq!(
        ir.matches("call void @yashy_free").count(),
        1,
        "the copy is freed once the call returns:\n{ir}"
    );
}

#[test]
fn write_file_lends_the_body_and_copies_only_the_path() {
    let ir = emit("fn main() { var ok = write_file(\"a.txt\", \"body\"); print_i64(ok); }");
    assert_eq!(
        ir.matches("call i8* @yashy_str_cstr").count(),
        1,
        "only the path needs a NUL-terminated copy:\n{ir}"
    );
    assert!(
        ir.contains("call i8* @yashy_str_ptr"),
        "the body is lent, because it is a byte range and not a C string:\n{ir}"
    );
    assert!(
        ir.contains("call i64 @yashy_str_len"),
        "the length travels beside the pointer:\n{ir}"
    );
    assert!(
        ir.contains("call i64 @yashy_write_file("),
        "the runtime writes exactly that many bytes:\n{ir}"
    );
}

#[test]
fn getenv_copies_the_name() {
    let ir = emit("fn main() { var home = getenv(\"HOME\"); print_str(home); }");
    assert_eq!(
        ir.matches("call i8* @yashy_str_cstr").count(),
        1,
        "the name is a string argument at the C boundary:\n{ir}"
    );
    assert!(
        ir.contains("call i8* @yashy_getenv(i8* "),
        "the runtime resolves the name:\n{ir}"
    );
}

#[test]
fn a_substring_is_owned_not_retained() {
    // A parameter is borrowed, so the only string operation in the body is the
    // slice itself. Binding a literal instead would add the literal's own
    // retain and hide whether the slice result was retained again.
    let ir = emit(
        "fn take(hello: string) { var word = hello.slice(1, 3); print_str(word); } \
         fn main() { take(\"hello\"); }",
    );
    assert!(
        ir.contains("call i8* @yashy_str_slice(i8* "),
        "one runtime call builds the substring:\n{ir}"
    );
    assert!(
        !ir.contains("call i8* @yashy_str_retain"),
        "the fresh string is taken over by the binding, not retained again:\n{ir}"
    );
}

#[test]
fn a_temporary_argument_to_a_user_function_is_released_after_the_call() {
    // `"a" + "b"` builds an owned string. `show` borrows it: the caller releases
    // the one reference after the call returns, and never before, because the
    // callee reads it during the call.
    let ir = emit(
        "fn show(text: string) { print_str(text); }\n\
         fn main() { show(\"a\" + \"b\"); }",
    );
    let call_at = ir
        .find("call void @show(")
        .unwrap_or_else(|| panic!("the user call is emitted:\n{ir}"));
    let release_at = ir
        .find("call void @yashy_str_release(i8* ")
        .unwrap_or_else(|| panic!("the temporary is released:\n{ir}"));
    assert!(
        release_at > call_at,
        "the release follows the call, not precedes it:\n{ir}"
    );
    assert_eq!(
        ir.matches("call void @yashy_str_release").count(),
        1,
        "one owned temporary, one release:\n{ir}"
    );
}

#[test]
fn a_user_function_string_result_is_taken_over_without_a_retain() {
    // The callee returns a fresh string (`return` transfers the temporary's
    // reference), so the caller owns it already: binding it must not retain, and
    // the binding's scope end releases the one reference.
    let ir = emit(
        "fn make() -> string { return str_from_i64(7); }\n\
         fn main() { var text = make(); print_str(text); }",
    );
    assert!(
        !ir.contains("call i8* @yashy_str_retain"),
        "a returned string is owned, so the binding takes it over:\n{ir}"
    );
    assert_eq!(
        ir.matches("call void @yashy_str_release").count(),
        1,
        "the local releases the reference it took over:\n{ir}"
    );
}
