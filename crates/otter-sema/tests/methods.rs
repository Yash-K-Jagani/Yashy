//! Methods: what the checker makes of an `impl` block.
//!
//! A method is found by the type, not by the name, and the receiver is the
//! first argument. These tests pin the rules a method call follows and the
//! diagnostics a mistake produces.

use otter_common::{Diagnostic, SourceMap};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program_with, CheckOptions};

/// Lex, parse and check `source`.
fn check_source(source: &str) -> Result<(), Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ot", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    check_program_with(&ast, &sm, file, CheckOptions { release: false }).map(|_| ())
}

/// Assert that `source` fails to type-check with a message containing `needle`.
fn assert_error(source: &str, needle: &str) {
    let diags = match check_source(source) {
        Ok(()) => panic!("expected a type error containing `{needle}`"),
        Err(diags) => diags,
    };
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected an error containing `{needle}`, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_method_is_called_on_a_value_of_its_type() {
    check_source(
        "struct Point { x: i64, y: i64 } \
         impl Point { fn sum(self) -> i64 { return self.x + self.y; } } \
         fn main() { const p = Point { x: 1, y: 2 }; const n = p.sum(); }",
    )
    .expect("a method call should type-check");
}

#[test]
fn a_method_on_an_enum_switches_on_the_receiver() {
    check_source(
        "enum Shape { Circle(i64), Square(i64) } \
         impl Shape { fn area(self) -> i64 { return switch self { \
         case Circle(r) { r * r } case Square(s) { s * s } }; } } \
         fn main() { const c = Shape.Circle(2); const n = c.area(); }",
    )
    .expect("a method on an enum should type-check");
}

#[test]
fn a_method_that_is_not_there_is_reported() {
    assert_error(
        "struct Point { x: i64 } \
         impl Point { fn sum(self) -> i64 { return 1; } } \
         fn main() { const p = Point { x: 1 }; const n = p.area(); }",
        "no method `area` on `Point`",
    );
}

#[test]
fn a_method_called_with_the_wrong_arity_is_reported() {
    assert_error(
        "struct Point { x: i64 } \
         impl Point { fn sum(self) -> i64 { return 1; } } \
         fn main() { const p = Point { x: 1 }; const n = p.sum(2); }",
        "`sum` takes 0 arguments",
    );
}

#[test]
fn an_impl_block_for_a_type_that_does_not_exist_is_reported() {
    assert_error(
        "impl Missing { fn sum(self) -> i64 { return 1; } }",
        "no type named `Missing`",
    );
}

#[test]
fn a_method_may_not_be_declared_twice() {
    assert_error(
        "struct Point { x: i64 } \
         impl Point { fn sum(self) -> i64 { return 1; } } \
         impl Point { fn sum(self) -> i64 { return 2; } }",
        "already has a method named `sum`",
    );
}

#[test]
fn a_static_method_needs_the_type_and_not_a_value() {
    assert_error(
        "struct Point { x: i64 } \
         impl Point { fn origin() -> Point { return Point { x: 0 }; } } \
         fn main() { const p = Point { x: 1 }; const q = p.origin(); }",
        "is a static method of `Point` and needs the type, not a value",
    );
}

#[test]
fn an_object_declaration_carries_its_fields_and_its_methods() {
    check_source(
        "object Counter { value: i64, \
         fn get(self) -> i64 { return self.value; }, \
         fn add(self, n: i64) -> Counter { return Counter { value: self.value + n }; } } \
         fn main() { const c = Counter { value: 3 }; const n = c.add(4).get(); }",
    )
    .expect("an object with methods should type-check");
}

#[test]
fn an_object_is_constructed_like_a_struct() {
    check_source(
        "object Counter { value: i64 } \
         fn main() { const c = Counter { value: 3 }; }",
    )
    .expect("an object literal should type-check");
}

#[test]
fn a_static_method_is_called_on_the_type() {
    check_source(
        "object Counter { value: i64, \
         fn new(start: i64) -> Counter { return Counter { value: start }; }, \
         fn get(self) -> i64 { return self.value; } } \
         fn main() { const c = Counter.new(3); const n = c.get(); }",
    )
    .expect("a static method call should type-check");
}

#[test]
fn a_method_called_on_the_type_needs_a_receiver() {
    assert_error(
        "object Counter { value: i64, fn get(self) -> i64 { return self.value; } } \
         fn main() { const n = Counter.get(); }",
        "`Counter.get` is a method and needs a receiver",
    );
}

#[test]
fn a_static_method_with_the_wrong_argument_type_is_reported() {
    assert_error(
        "object Counter { value: i64, \
         fn new(start: i64) -> Counter { return Counter { value: start }; } } \
         fn main() { const c = Counter.new(\"three\"); }",
        "expected `i64`",
    );
}

#[test]
fn a_method_that_is_not_there_on_the_type_is_reported() {
    assert_error(
        "object Counter { value: i64 } \
         fn main() { const c = Counter.missing(); }",
        "no method `missing` on `Counter`",
    );
}

#[test]
fn a_pointer_receiver_mutates_a_var_binding() {
    check_source(
        "object Counter { value: i64, \
         fn bump(self: *mut Counter) { self[0].value = self[0].value + 1; } } \
         fn main() { var c = Counter { value: 0 }; c.bump(); }",
    )
    .expect("a pointer receiver called on a `var` should type-check");
}

#[test]
fn a_pointer_receiver_needs_a_mutable_binding() {
    assert_error(
        "object Counter { value: i64, \
         fn bump(self: *mut Counter) { self[0].value = self[0].value + 1; } } \
         fn main() { const c = Counter { value: 0 }; c.bump(); }",
        "`bump` needs a mutable object binding",
    );
}
