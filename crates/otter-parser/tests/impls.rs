//! Methods declared in an `impl` block.
//!
//! A method is a function with a receiver: the block says which type it
//! belongs to, and the method's first parameter is the value it is called on.
//! These tests pin the three things that make it a method: the block parses,
//! the method is found by the type, and the receiver is the first argument.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::ast::{Expr, Item, Stmt, TypeExpr};
use otter_parser::Parser;

/// Parse `src` and return its items.
fn items_of(src: &str) -> Vec<Item> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ot", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    parser
        .parse_program()
        .unwrap_or_else(|error| panic!("`{src}` must parse: {error:?}"))
        .items
}

#[test]
fn an_impl_block_names_its_type_and_holds_its_methods() {
    let items = items_of(
        "struct Point { x: i64 } impl Point { fn sum(self) -> i64 { return 1; } }",
    );
    let impl_block = items
        .iter()
        .find_map(|item| match item {
            Item::Impl {
                type_name, methods, ..
            } => Some((type_name, methods)),
            _ => None,
        })
        .expect("the program has an `impl` block");
    assert_eq!(impl_block.0, "Point");
    assert_eq!(impl_block.1.len(), 1);
    let Item::Fn { name, params, .. } = &impl_block.1[0] else {
        panic!("a method is a function item");
    };
    assert_eq!(name, "sum");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "self");
}

#[test]
fn the_receiver_needs_no_annotation() {
    // `self` is written without a type, because the block already says what
    // the receiver is. The type it stands for is `Self`.
    let items = items_of("impl Point { fn sum(self) -> i64 { return 1; } }");
    let Item::Impl { methods, .. } = &items[0] else {
        panic!("the program has an `impl` block");
    };
    let Item::Fn { params, .. } = &methods[0] else {
        panic!("a method is a function item");
    };
    assert!(matches!(&params[0].ty, TypeExpr::Name(name, _) if name == "Self"));
}

#[test]
fn an_impl_block_may_hold_several_methods() {
    let items = items_of(
        "impl Point { fn a(self) -> i64 { return 1; } fn b(self) -> i64 { return 2; } }",
    );
    let Item::Impl { methods, .. } = &items[0] else {
        panic!("the program has an `impl` block");
    };
    assert_eq!(methods.len(), 2);
}

#[test]
fn an_object_holds_its_fields_and_its_methods() {
    let items = items_of(
        "object Counter { value: i64, fn get(self) -> i64 { return self.value; } }",
    );
    let Item::Object {
        name,
        fields,
        methods,
        ..
    } = &items[0]
    else {
        panic!("the program has an object");
    };
    assert_eq!(name, "Counter");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "value");
    assert_eq!(methods.len(), 1);
}

#[test]
fn a_method_called_on_a_value_is_not_a_path() {
    // `f().g()` has no path to write down, so the parser keeps the receiver
    // as the expression it is.
    let items = items_of("fn main() { const n = f().g(); }");
    let Item::Fn { body, .. } = &items[0] else {
        panic!("the program has a function");
    };
    let Stmt::Const { init, .. } = &body.stmts[0] else {
        panic!("the function has a const binding");
    };
    let Expr::MethodCall { method, .. } = init else {
        panic!("the call is on a value, not a path: {init:?}");
    };
    assert_eq!(method, "g");
}
