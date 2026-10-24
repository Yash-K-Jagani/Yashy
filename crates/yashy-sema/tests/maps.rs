//! The map type at the checking edge: key/value restrictions, inference at
//! `map_new`, linearity, and the binding requirement shared with arrays.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

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
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "expected an error containing `{needle}`, got {diagnostics:?}"
        ),
    }
}

#[test]
fn a_map_round_trips_keys_of_every_primitive_key_type() {
    check(
        "fn main() { \
           let a: map[int]int = map_new(); \
           a.insert(1, 10); \
           let x: Option[int] = a.get(1); \
           let y: Option[int] = a.remove(1); \
           let n: int = a.len(); \
           a.free(); \
           let b: map[bool]int = map_new(); \
           b.insert(true, 1); \
           let p: Option[int] = b.get(true); \
           b.free(); \
           let c: map[string]int = map_new(); \
           c.insert(\"k\", 2); \
           let q: Option[int] = c.get(\"k\"); \
           c.free(); \
         }",
    )
    .expect("all three key kinds are supported");
}

#[test]
fn map_new_is_typed_by_a_parameter_or_a_binding_annotation() {
    check(
        "fn fill(m: map[bool]float) { m.insert(true, 1.5); } \
         fn main() { fill(map_new()); let c: map[int]int = map_new(); c.free(); }",
    )
    .expect("a parameter and a binding annotation both supply the expected type");
}

#[test]
fn map_new_needs_an_annotation_to_infer_from() {
    assert_rejected(
        "fn main() { let m = map_new(); }",
        "cannot infer the map's key and value types",
    );
}

#[test]
fn a_map_key_must_be_hashable() {
    assert_rejected(
        "fn main() { let m: map[float]int = map_new(); m.free(); }",
        "a map key must be `int`, `bool`, or `string`",
    );
}

#[test]
fn a_map_value_must_be_copyable() {
    assert_rejected(
        "fn main() { let m: map[int]string = map_new(); m.free(); }",
        "a map value must be a number or `bool`",
    );
}

#[test]
fn map_free_moves_the_binding() {
    assert_rejected(
        "fn main() { let m: map[int]int = map_new(); m.free(); m.free(); }",
        "moved",
    );
}

#[test]
fn a_map_is_linear() {
    assert_rejected(
        "fn main() { let m: map[int]int = map_new(); let n = m; }",
        "move",
    );
}

#[test]
fn map_get_and_remove_yield_options() {
    check(
        "fn main() { let m: map[int]int = map_new(); \
         let a: Option[int] = m.get(1); \
         let b: Option[int] = m.remove(1); \
         m.free(); }",
    )
    .expect("missing keys are values, not errors");
}

#[test]
fn a_key_of_the_wrong_type_is_reported() {
    assert_rejected(
        "fn main() { let m: map[int]int = map_new(); m.insert(\"k\", 1); m.free(); }",
        "expected `int`, found `string`",
    );
}

#[test]
fn a_value_of_the_wrong_type_is_reported() {
    assert_rejected(
        "fn main() { let m: map[int]int = map_new(); m.insert(1, true); m.free(); }",
        "expected `int`, found `bool`",
    );
}

#[test]
fn a_method_on_a_temporary_is_rejected() {
    // The parser keeps the receiver as the expression it is, so this is
    // rejected because a method needs a binding it can update -- not because
    // the name is unknown. It still cannot run: a method needs a binding
    // whose header it can update.
    assert_rejected(
        "fn make() -> map[int]int { return map_new(); } \
         fn main() { let v = make().get(1); }",
        "a method needs a binding as the receiver",
    );
}

#[test]
fn a_field_map_cannot_be_used_where_a_binding_is_needed() {
    assert_rejected(
        "struct S { m: map[int]int } \
         fn inspect(s: S) { let v = s.m.get(1); } \
         fn main() { }",
        "field access is not a binding",
    );
}

#[test]
fn len_measures_a_map() {
    check(
        "fn main() { let m: map[int]int = map_new(); let n: int = m.len(); m.free(); }",
    )
    .expect("a map has the same `len` method as an array and a string");
}

#[test]
fn the_map_methods_answer_with_the_types_the_table_holds() {
    check(
        "fn main() { let m: map[int]int = map_new(); \
         m.insert(1, 10); \
         let keys: []int = m.keys(); \
         let values: []int = m.values(); \
         let has: bool = m.contains_key(1); \
         keys.free(); values.free(); m.free(); }",
    )
    .expect("the keys and values are arrays of the table's own types");
}

#[test]
fn a_string_key_map_answers_with_a_string_array() {
    check(
        "fn main() { let m: map[string]int = map_new(); \
         m.insert(\"k\", 10); \
         let keys: []string = m.keys(); \
         let values: []int = m.values(); \
         keys.free(); values.free(); m.free(); }",
    )
    .expect("a string key map answers with a string array");
}

#[test]
fn a_key_of_the_wrong_type_is_reported_by_contains_key() {
    assert_rejected(
        "fn main() { let m: map[int]int = map_new(); let has: bool = m.contains_key(\"k\"); m.free(); }",
        "expected `int`, found `string`",
    );
}

#[test]
fn contains_key_needs_a_key() {
    assert_rejected(
        "fn main() { let m: map[int]int = map_new(); let has: bool = m.contains_key(); m.free(); }",
        "takes 2 arguments",
    );
}

#[test]
fn keys_and_values_take_no_arguments() {
    assert_rejected(
        "fn main() { let m: map[int]int = map_new(); let keys: []int = m.keys(1); m.free(); }",
        "takes 1 argument",
    );
}

#[test]
fn the_map_methods_are_not_available_on_other_types() {
    assert_rejected(
        "fn main() { let xs = []int { 1 }; let keys: []int = xs.keys(); }",
        "no method `keys` on `[]int`",
    );
}
