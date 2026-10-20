//! The C pointer spellings the FFI edge adds to the type grammar.
//!
//! `*mut T` already existed for Otter's owning pointer; `*T` and `*const T` are
//! new, and all three have to survive parsing as *distinct* AST nodes so the
//! checker can tell a shared C pointer from an owning one. That distinction is
//! the whole reason these tests inspect the AST rather than a checked type.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::ast::{Item, PtrKind, TypeExpr};
use otter_parser::Parser;

/// Parse `fn f(x: <spelling>) {}` and return the parameter's type.
fn param_type(spelling: &str) -> TypeExpr {
    let src = format!("fn f(x: {spelling}) {{ }}");
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ot", src.clone());
    let tokens = Lexer::new(&src)
        .tokenize()
        .expect("the lexer must accept a type spelling");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .unwrap_or_else(|error| panic!("`{spelling}` must parse: {error:?}"));
    let item = program
        .items
        .into_iter()
        .next()
        .expect("the program has one item");
    match item {
        Item::Fn { params, .. } => params
            .into_iter()
            .next()
            .expect("the function has one parameter")
            .ty,
        other => panic!("expected a function item, got {other:?}"),
    }
}

/// The pointer's mutability and pointee spelling, with spans stripped.
fn pointer_parts(spelling: &str) -> (PtrKind, String) {
    match param_type(spelling) {
        TypeExpr::Ptr { kind, pointee, .. } => {
            let name = match *pointee {
                TypeExpr::Name(name, _) => name,
                other => panic!("expected a named pointee, got {other:?}"),
            };
            (kind, name)
        }
        other => panic!("`{spelling}` must parse as a pointer, got {other:?}"),
    }
}

#[test]
fn pointers_parse_in_bare_mut_and_const_flavours() {
    assert_eq!(pointer_parts("*i8"), (PtrKind::Bare, "i8".to_string()));
    assert_eq!(pointer_parts("*mut i8"), (PtrKind::Mut, "i8".to_string()));
    assert_eq!(pointer_parts("*const i8"), (PtrKind::Const, "i8".to_string()));
}

#[test]
fn mutability_is_the_only_difference_between_the_spellings() {
    // Same pointee, three nodes: a checker that compared only the pointee
    // would call `*const u8` and `*mut u8` the same type, which at the FFI
    // boundary is exactly the mistake `unsafe` is supposed to make visible.
    let bare = pointer_parts("*u8");
    let mutable = pointer_parts("*mut u8");
    let shared = pointer_parts("*const u8");
    assert_eq!(bare.1, mutable.1);
    assert_eq!(bare.1, shared.1);
    assert_ne!(bare.0, mutable.0);
    assert_ne!(bare.0, shared.0);
    assert_ne!(mutable.0, shared.0);
}

#[test]
fn a_pointer_to_a_c_scalar_still_parses() {
    // The pointee grammar is not narrowed at parse time: `*const u8` is the
    // spelling of a C string, and `u8` is only meaningful at the FFI edge, so
    // deciding that is the checker's job, not the parser's.
    let (kind, pointee) = pointer_parts("*const u8");
    assert_eq!(kind, PtrKind::Const);
    assert_eq!(pointee, "u8");
}
