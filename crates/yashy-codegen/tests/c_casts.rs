//! Cast lowering across the C numeric widths.
//!
//! An `extern` value can be narrower or unsigned (`strlen` returns `usize`,
//! `strchr`-style functions return `i32`, a `char` parameter is `u8`). Moving
//! one into ordinary Yashy arithmetic needs an `as`, and the instruction has to
//! respect the source's signedness: `sext` vs `zext`, `sitofp` vs `uitofp`,
//! `fptosi` vs `fptoui`. The widths here are the ones Task 5's libc program
//! actually uses, plus the neighbours that would hide a wrong choice.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

use yashy_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("casts.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(&check_source(src), "casts.ya", CodegenOptions::default(), &yashy_common::ProofFacts::empty())
}

#[test]
fn a_narrowing_cast_truncates_and_a_signed_widening_cast_sign_extends() {
    // 258 does not fit in an `i32` parameter's narrower neighbour, so the value
    // must be truncated rather than carried as 258. The return is `i32`, so
    // widening it back to `i64` must sign-extend: `strlen`-shaped functions
    // return 32 bits and the upper half has to be the sign bit.
    let ir = emit(
        r#"extern "C" {
               unsafe fn take(x: i32) -> i32;
           }

           fn main() {
               print(take(258 as i32) as int);
           }
           "#,
    );
    assert!(ir.contains("trunc i64"), "the cast must truncate:\n{ir}");
    assert!(
        ir.contains("call i32 @take(i32"),
        "the parameter crosses as `i32`:\n{ir}"
    );
    assert!(
        ir.contains("sext i32"),
        "an `i32` is signed, so widening sign-extends:\n{ir}"
    );
}

#[test]
fn an_unsigned_source_zero_extends_and_floats_choose_their_instruction() {
    // `u8` is unsigned: widening must zero-extend. `1 as f32` is an integer to
    // float, so it is `sitofp`; widening that `float` to `double` is `fpext`;
    // and a `f64` to `i32` is `fptosi` because the *target* is signed.
    let ir = emit(
        r#"extern "C" {
               unsafe fn byte_of(x: u8) -> u8;
           }

           fn main() {
               print(byte_of(1 as u8) as int);
               print(1 as f32 as float);
               print(2.5 as i32 as int);
           }
           "#,
    );
    assert!(
        ir.contains("zext i8"),
        "a `u8` is unsigned, so widening zero-extends:\n{ir}"
    );
    assert!(
        ir.contains("sitofp i64"),
        "an integer to `f32` uses the source's signedness:\n{ir}"
    );
    assert!(ir.contains("fpext float"), "`float` to `double`:\n{ir}");
    assert!(
        ir.contains("fptosi double"),
        "a float to a signed integer:\n{ir}"
    );
}
