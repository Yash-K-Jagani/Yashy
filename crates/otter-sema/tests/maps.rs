//! The map type at the checking edge: key/value restrictions, inference at
//! `map_new`, linearity, and the binding requirement shared with slices.

use otter_common::{Diagnostic, SourceMap};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ot", src.to_string());
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
           var a: map[i64]i64 = map_new(); \
           map_insert(a, 1, 10); \
           var x: Option[i64] = map_get(a, 1); \
           var y: Option[i64] = map_remove(a, 1); \
           var n: i64 = map_len(a); \
           map_free(a); \
           var b: map[bool]i64 = map_new(); \
           map_insert(b, true, 1); \
           var p: Option[i64] = map_get(b, true); \
           map_free(b); \
           var c: map[string]i64 = map_new(); \
           map_insert(c, \"k\", 2); \
           var q: Option[i64] = map_get(c, \"k\"); \
           map_free(c); \
         }",
    )
    .expect("all three key kinds are supported");
}

#[test]
fn map_new_is_typed_by_a_parameter_or_a_binding_annotation() {
    check(
        "fn fill(m: map[bool]f64) { map_insert(m, true, 1.5); } \
         fn main() { fill(map_new()); var c: map[i64]i64 = map_new(); map_free(c); }",
    )
    .expect("a parameter and a binding annotation both supply the expected type");
}

#[test]
fn map_new_needs_an_annotation_to_infer_from() {
    assert_rejected(
        "fn main() { var m = map_new(); }",
        "cannot infer the map's key and value types",
    );
}

#[test]
fn a_map_key_must_be_hashable() {
    assert_rejected(
        "fn main() { var m: map[f64]i64 = map_new(); map_free(m); }",
        "a map key must be `i64`, `bool`, or `string`",
    );
}

#[test]
fn a_map_value_must_be_copyable() {
    assert_rejected(
        "fn main() { var m: map[i64]string = map_new(); map_free(m); }",
        "a map value must be a number or `bool`",
    );
}

#[test]
fn map_free_moves_the_binding() {
    assert_rejected(
        "fn main() { var m: map[i64]i64 = map_new(); map_free(m); map_free(m); }",
        "moved",
    );
}

#[test]
fn a_map_is_linear() {
    assert_rejected(
        "fn main() { var m: map[i64]i64 = map_new(); var n = m; }",
        "move",
    );
}

#[test]
fn map_get_and_remove_yield_options() {
    check(
        "fn main() { var m: map[i64]i64 = map_new(); \
         var a: Option[i64] = map_get(m, 1); \
         var b: Option[i64] = map_remove(m, 1); \
         map_free(m); }",
    )
    .expect("missing keys are values, not errors");
}

#[test]
fn a_key_of_the_wrong_type_is_reported() {
    assert_rejected(
        "fn main() { var m: map[i64]i64 = map_new(); map_insert(m, \"k\", 1); map_free(m); }",
        "expected `i64`, found `string`",
    );
}

#[test]
fn a_value_of_the_wrong_type_is_reported() {
    assert_rejected(
        "fn main() { var m: map[i64]i64 = map_new(); map_insert(m, 1, true); map_free(m); }",
        "expected `i64`, found `bool`",
    );
}

#[test]
fn map_operations_need_a_binding_not_a_temporary() {
    assert_rejected(
        "fn make() -> map[i64]i64 { return map_new(); } \
         fn main() { var v = map_get(make(), 1); }",
        "needs a map binding, not a temporary",
    );
}

#[test]
fn a_field_map_cannot_be_used_where_a_binding_is_needed() {
    assert_rejected(
        "struct S { m: map[i64]i64 } \
         fn inspect(s: S) { var v = map_get(s.m, 1); } \
         fn main() { }",
        "needs a map binding, not a field access",
    );
}

#[test]
fn len_does_not_measure_a_map() {
    assert_rejected(
        "fn main() { var m: map[i64]i64 = map_new(); var n = len(m); map_free(m); }",
        "expects a string or a slice",
    );
}
