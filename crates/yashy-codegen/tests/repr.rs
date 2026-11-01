//! `repr(C)`'s one visible effect on the IR: the enum tag's width.
//!
//! A `struct`'s fields are the same walk under C's rules and Yashy's, so the
//! clause changes nothing about a struct's emitted type. An `enum` is different:
//! C's `enum` is an `int`, so a `repr(C)` enum's tag is `i32` and every place
//! that reads or writes it has to agree with that width.

use yashy_common::{ProofFacts, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};

fn emit(src: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add_file("repr.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    let hir = check_program(&program, &sources, file).expect("the program must check");
    emit_program(&hir, "repr.ya", CodegenOptions::default(), &ProofFacts::empty())
}

#[test]
fn a_repr_c_enum_has_a_c_int_tag() {
    let ir = emit(
        "repr(C) enum Kind { A, B }\n\
         fn main() {\n\
             const k = Kind.A;\n\
             switch k {\n\
                 case A { print(1); }\n\
                 case B { print(2); }\n\
             }\n\
         }",
    );
    assert!(
        ir.contains("%enum.Kind = type { i32 }"),
        "a C enum is an `int`, so its tag is four bytes:\n{ir}"
    );
}

#[test]
fn an_ordinary_enum_keeps_its_one_byte_tag() {
    // The two spellings must differ in the IR, or `repr(C)` would be decorative
    // and a C function reading a Yashy enum would read the wrong width.
    let ir = emit(
        "enum Kind { A, B }\n\
         fn main() {\n\
             const k = Kind.A;\n\
             switch k {\n\
                 case A { print(1); }\n\
                 case B { print(2); }\n\
             }\n\
         }",
    );
    assert!(
        ir.contains("%enum.Kind = type { i8 }"),
        "a Yashy enum's tag is one byte:\n{ir}"
    );
    assert!(
        !ir.contains("%enum.Kind = type { i32 }"),
        "the clause was not written, so the tag must not widen:\n{ir}"
    );
}

#[test]
fn a_match_on_a_repr_c_enum_compares_the_wide_tag() {
    // The tag extraction and the comparison both name the tag's width. A
    // mismatch here is invalid IR, not a wrong answer, so the two must be read
    // from the same place.
    let ir = emit(
        "repr(C) enum Kind { A, B }\n\
         fn main() {\n\
             const k = Kind.A;\n\
             switch k {\n\
                 case A { print(1); }\n\
                 case B { print(2); }\n\
             }\n\
         }",
    );
    assert!(
        ir.contains("icmp eq i32"),
        "the tag is compared at its own width:\n{ir}"
    );
    assert!(
        !ir.contains("icmp eq i8"),
        "an `i8` comparison against an `i32` tag is invalid IR:\n{ir}"
    );
}

#[test]
fn a_layout_query_is_folded_and_never_reaches_the_ir() {
    // The answer is a constant computed by the checker. Emitting a call would
    // name a function the linker has never heard of.
    let ir = emit("fn main() { const n = sizeof(int) + alignof(int); print(n); }");
    assert!(
        !ir.contains("sizeof") && !ir.contains("alignof"),
        "the queries are constants, not calls:\n{ir}"
    );
}
