//! Generic structs and enums: schemes, one monomorph per argument tuple.
//!
//! `Pair[T]` is a scheme -- a set of fields written against its type
//! parameters -- and `Pair[int]` picks out one concrete instantiation of it.
//! Every instantiation is a monomorphic type named by the same mangling
//! generic functions use, and every existing rule about types applies to it.

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram};

/// Lex, parse and check `source` as a single file.
fn check_source(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    check_program_with(&ast, &sm, file, CheckOptions::default())
}

/// Assert that `src` fails with `code`, and return the diagnostics.
fn assert_code(src: &str, code: &'static str) -> Vec<Diagnostic> {
    let diags = check_source(src).expect_err("the program must be rejected");
    assert!(
        diags.iter().any(|d| d.code == Some(code)),
        "expected {code}, got {:?}",
        diags.iter().map(|d| (d.code, &d.message)).collect::<Vec<_>>()
    );
    diags
}

/// The HirStruct items emitted, by name.
fn emitted_types(hir: &HirProgram) -> Vec<String> {
    hir.items
        .iter()
        .filter_map(|item| match item {
            HirItem::Struct(s) => Some(s.name.clone()),
            HirItem::Enum(e) => Some(e.name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_pair_of_ints_constructs_and_reads() {
    check_source(
        "struct Pair[T] { first: T, second: T }\n\
         fn f() -> int { let Pair[int] p = Pair { first: 1, second: 2 }; return p.first + p.second; }",
    )
    .expect("Pair[int] constructs and reads");
}

#[test]
fn an_entry_of_string_and_int_constructs_and_reads() {
    check_source(
        "struct Entry[K, V] { key: K, value: V }\n\
         fn f() -> int { let Entry[string, int] e = Entry { key: \"a\", value: 3 }; return e.value; }",
    )
    .expect("Entry[string, int] constructs and reads");
}

#[test]
fn a_maybe_matches_exhaustively_at_each_instantiation() {
    check_source(
        "enum Maybe[T] { Nothing, Just(T) }\n\
         fn f(Maybe[int] m) -> int { return switch m { case Nothing { 0 } case Just(x) { x } }; }",
    )
    .expect("all variants covered");
}

#[test]
fn a_generic_enum_constructs_and_matches() {
    let hir = check_source(
        "enum Maybe[T] { Nothing, Just(T) }\n\
         fn f() -> int {\n\
             let Maybe[int] m = Maybe.Just(2);\n\
             return switch m { case Nothing { 0 } case Just(x) { x } };\n\
         }",
    )
    .expect("Maybe[int] constructs from its payload");
    let emitted = emitted_types(&hir);
    assert_eq!(
        emitted.iter().filter(|name| name.as_str() == "Maybe_i64").count(),
        1,
        "one Maybe_i64 definition, got {emitted:?}"
    );
}

#[test]
fn a_generic_enum_that_cannot_infer_its_argument_is_refused() {
    let diags = check_source(
        "enum Either[T, U] { Left(T), Right(U) }\n\
         fn f() -> int { let Either[int, string] e = Either.Left(1); return 0; }",
    )
    .expect_err("the payload does not determine every argument");
    assert!(
        diags.iter().any(|d| d.message.contains("cannot infer type argument")),
        "the diagnostic names the uninferred parameter: {diags:?}"
    );
}

#[test]
fn a_missing_arm_of_a_generic_enum_is_e0600() {
    assert_code(
        "enum Maybe[T] { Nothing, Just(T) }\n\
         fn f(Maybe[int] m) -> int { return switch m { case Just(x) { x } }; }",
        codes::NON_EXHAUSTIVE_MATCH,
    );
}

#[test]
fn one_instantiation_emits_one_type_irrespective_of_use_count() {
    let hir = check_source(
        "struct Pair[T] { first: T, second: T }\n\
         fn f() -> Pair[int] { return Pair { first: 1, second: 2 }; }\n\
         fn g() -> Pair[int] { return Pair { first: 3, second: 4 }; }",
    )
    .expect("used twice, one monomorph");
    let emitted = emitted_types(&hir);
    assert_eq!(
        emitted.iter().filter(|name| name.as_str() == "Pair_i64").count(),
        1,
        "one Pair_i64 definition, got {emitted:?}"
    );
}

#[test]
fn two_instantiations_emit_two_types() {
    let hir = check_source(
        "struct Pair[T] { first: T, second: T }\n\
         fn f() -> Pair[int] { return Pair { first: 1, second: 2 }; }\n\
         fn g() -> Pair[string] { return Pair { first: \"a\", second: \"b\" }; }",
    )
    .expect("both instantiations");
    let emitted = emitted_types(&hir);
    assert!(
        emitted.contains(&"Pair_i64".to_string()),
        "Pair_i64 is emitted: {emitted:?}"
    );
    assert!(
        emitted.contains(&"Pair_string".to_string()),
        "Pair_string is emitted: {emitted:?}"
    );
}

#[test]
fn too_many_arguments_is_e1509_with_both_arities() {
    let diags = assert_code(
        "struct Pair[T] { first: T, second: T }\n\
         fn f(Pair[int, string] p) -> int { return 0; }",
        codes::WRONG_TYPE_ARG_COUNT,
    );
    assert!(
        diags.iter().any(|d| d.message.contains("declares 1 type parameter") && d.message.contains("given 2")),
        "the message names both arities: {diags:?}"
    );
}

#[test]
fn a_bare_generic_type_is_e1509() {
    assert_code(
        "struct Pair[T] { first: T, second: T }\n\
         fn f(Pair p) -> int { return 0; }",
        codes::WRONG_TYPE_ARG_COUNT,
    );
}

#[test]
fn an_unused_type_parameter_is_e1510() {
    let diags = assert_code(
        "struct Wrapper[T] { n: int }",
        codes::UNUSED_TYPE_PARAMETER,
    );
    assert!(
        diags.iter().any(|d| d.message.contains("`T`")),
        "the diagnostic names the unused parameter: {diags:?}"
    );
}

#[test]
fn a_bound_on_a_struct_parameter_is_checked_at_instantiation() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         struct Pair[T: Show] { first: T, second: T }\n\
         fn f() -> int { let Pair[int] p = Pair { first: 1, second: 2 }; return 0; }",
        codes::UNSATISFIED_BOUND,
    );
    assert!(
        diags.iter().any(|d| d.message.contains("`int` does not implement `Show`")),
        "the bound fires at the instantiation: {diags:?}"
    );
}

#[test]
fn two_instantiations_do_not_unify() {
    let diags = check_source(
        "struct Pair[T] { first: T, second: T }\n\
         fn take(Pair[int] p) -> int { return 0; }\n\
         fn f() -> int { return take(Pair { first: \"a\", second: \"b\" }); }",
    )
    .expect_err("Pair[string] is not a Pair[int]");
    assert!(
        diags.iter().any(|d| d.message.contains("expected `Pair_i64`, found `Pair_string`")),
        "the two instantiations stay distinct: {diags:?}"
    );
}

#[test]
fn a_generic_type_inside_a_generic_function_monomorphises_per_t() {
    let hir = check_source(
        "struct Pair[T] { first: T, second: T }\n\
         fn first[T](Pair[T] p) -> T { return p.first; }\n\
         fn f() -> int { return first(Pair { first: 1, second: 2 }); }\n\
         fn g() -> string { return first(Pair { first: \"a\", second: \"b\" }); }",
    )
    .expect("per-instantiation monomorphs");
    let names: Vec<&str> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Fn(f) => Some(f.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        names.contains(&"first_i64"),
        "first_i64 exists: {names:?}"
    );
    assert!(
        names.contains(&"first_string"),
        "first_string exists: {names:?}"
    );
}

#[test]
fn a_type_that_names_itself_in_a_field_is_rejected_once() {
    let diags = check_source(
        "struct Node[T] { next: Node[T] }\n\
         fn f(Node[int] n) -> Node[int] { return n; }",
    )
    .expect_err("a self-naming field cannot instantiate");
    assert!(
        diags.iter().any(|d| d.message.contains("names itself")),
        "the recursion is one diagnostic, not a stack overflow: {diags:?}"
    );
}
