//! Methods: the receiver, the mangled name, and the call.
//!
//! A method is an ordinary function with a receiver, so what the backend
//! emits is an ordinary function under a mangled name and an ordinary call
//! with the receiver as the first argument.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and emit `src` as one module.
fn ir_of(src: &str) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    let hir = check_program(&ast, &sm, file).expect("these test sources must type-check");
    emit_program(&hir, "main.ya", CodegenOptions::default())
}

#[test]
fn a_method_is_emitted_under_its_mangled_name() {
    let ir_text = ir_of(
        "struct Point { x: int, y: int } \
         impl Point { fn sum(self) -> int { return self.x + self.y; } } \
         fn main() { const p = Point { x: 1, y: 2 }; print(p.sum()); }",
    );
    assert!(
        ir_text.contains("define i64 @Point.sum(%struct.Point"),
        "the method takes the receiver as its first argument:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i64 @Point.sum(%struct.Point"),
        "the call passes the receiver:\n{ir_text}"
    );
}

#[test]
fn a_method_reads_the_receivers_fields() {
    let ir_text = ir_of(
        "struct Point { x: int, y: int } \
         impl Point { fn sum(self) -> int { return self.x + self.y; } } \
         fn main() { const p = Point { x: 1, y: 2 }; print(p.sum()); }",
    );
    assert!(
        ir_text.contains("extractvalue %struct.Point"),
        "the receiver's fields are read out of the aggregate:\n{ir_text}"
    );
}

#[test]
fn an_enum_method_switches_on_the_receiver() {
    let ir_text = ir_of(
        "enum Shape { Circle(int), Square(int) } \
         impl Shape { fn area(self) -> int { return switch self { \
         case Circle(r) { r * r } case Square(s) { s * s } }; } } \
         fn main() { const c = Shape.Circle(2); print(c.area()); }",
    );
    assert!(
        ir_text.contains("define i64 @Shape.area("),
        "the method is emitted under the enum's name:\n{ir_text}"
    );
    assert!(
        ir_text.contains("icmp eq i8"),
        "the body compares the receiver's tag:\n{ir_text}"
    );
}

#[test]
fn a_pointer_receiver_takes_the_binding_and_stores_through_it() {
    let ir_text = ir_of(
        "struct Counter { value: int } \
         impl Counter { fn bump(self: *mut Counter) { self[0].value = self[0].value + 1; } } \
         fn main() { let c = Counter { value: 0 }; c.bump(); }",
    );
    assert!(
        ir_text.contains("define void @Counter.bump(i8*"),
        "a pointer receiver is a pointer parameter, not the aggregate:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @Counter.bump(i8* %slot0)"),
        "the call passes the binding's own storage:\n{ir_text}"
    );
    assert!(
        ir_text.contains("store i64") && ir_text.contains("getelementptr %struct.Counter"),
        "the field write is a store at the field's offset:\n{ir_text}"
    );
}
