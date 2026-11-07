//! Trait impls: coherence and completeness.
//!
//! A trait is a requirement set (Task 1); an `impl Trait for Type` block
//! must provide exactly its methods, with exactly their signatures, and one
//! pair of (trait, type) may have only one impl. These tests pin the codes
//! `E1500`–`E1502` and `E1511` and the emitted `Type.Trait.method` symbol.

use yashy_common::codes;
use yashy_common::{Diagnostic, SourceMap};
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

/// The `code` of the first diagnostic carrying one, if any.
fn code_of(diags: &[Diagnostic], code: &'static str) -> bool {
    diags.iter().any(|d| d.code == Some(code))
}

/// Assert that `src` fails with `code`, and return the diagnostics.
fn assert_code(src: &str, code: &'static str) -> Vec<Diagnostic> {
    let result = check_source(src);
    let diags = result.expect_err("the program must be rejected");
    assert!(
        code_of(&diags, code),
        "expected {code}, got {:?}",
        diags
            .iter()
            .map(|d| (d.code, &d.message))
            .collect::<Vec<_>>()
    );
    diags
}

#[test]
fn impl_drop_is_e1513() {
    let diags = check_source(
        "struct Counter { n: int }\n\
         impl Drop for Counter { fn drop(self: *mut Self) { self[0].n = 0; } }",
    )
    .expect_err("there is no Drop");
    assert!(
        diags.iter().any(|d| d.code == Some(codes::DROP_TRAIT)),
        "expected E1513, got {:?}",
        diags
            .iter()
            .map(|d| (d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_operator_on_a_type_with_a_same_named_method_is_e1515() {
    let diags = check_source(
        "struct V { n: int }\n\
         impl V { fn add(self, V other) -> V { return V { n: self.n + other.n }; } }\n\
         fn f() { let a = V { n: 1 }; let b = V { n: 2 }; let c = a + b; print(c.n); }",
    )
    .expect_err("operators cannot be overloaded");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(codes::OPERATOR_OVERLOAD)),
        "expected E1515, got {:?}",
        diags
            .iter()
            .map(|d| (d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

#[test]
fn two_impls_of_one_trait_for_one_type_conflict() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self) -> string { return \"a\"; } }\n\
         impl Show for Point { fn show(self) -> string { return \"b\"; } }",
        codes::CONFLICTING_IMPL,
    );
    assert!(diags
        .iter()
        .all(|d| d.code == Some(codes::CONFLICTING_IMPL)));
}

#[test]
fn an_impl_missing_a_method_is_rejected() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; fn short(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self) -> string { return \"a\"; } }",
        codes::INCOMPLETE_IMPL,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("missing `fn short(self) -> string`")),
        "the diagnostic names the missing method and its signature: {diags:?}"
    );
}

#[test]
fn an_impl_with_a_method_the_trait_does_not_declare_is_rejected() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self) -> string { return \"a\"; } fn extra(self) -> int { return 1; } }",
        codes::INCOMPLETE_IMPL,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("declares no `extra`")),
        "the diagnostic names the extra method: {diags:?}"
    );
}

#[test]
fn a_method_with_the_wrong_receiver_kind_is_rejected() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self: *mut Point) -> string { return \"a\"; } }",
        codes::INCOMPLETE_IMPL,
    );
    assert!(
        diags.iter().any(|d| d
            .message
            .contains("impl has `fn show(self: *mut Point) -> string`")
            && d.message
                .contains("trait requires `fn show(self) -> string`")),
        "the diagnostic prints both signatures: {diags:?}"
    );
}

#[test]
fn a_user_declared_generic_trait_is_e1511() {
    assert_code(
        "trait Pair[T] { fn first(self) -> T; }",
        codes::GENERIC_TRAIT,
    );
}

#[test]
fn an_impl_of_an_unknown_trait_is_e1500() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Shw for Point { fn show(self) -> string { return \"a\"; } }",
        codes::UNKNOWN_TRAIT,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.label.as_deref() == Some("did you mean `Show`?")),
        "the diagnostic suggests the nearest trait: {diags:?}"
    );
}

#[test]
fn a_correct_impl_checks_and_emits_the_trait_symbol() {
    let hir = check_source(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self) -> string { return \"a\"; } }",
    )
    .expect("a complete impl must check");
    assert!(
        hir.items
            .iter()
            .any(|item| matches!(item, HirItem::Fn(f) if f.name == "Point.Show.show")),
        "the method is emitted as Point.Show.show: {:?}",
        hir.items
            .iter()
            .map(|i| format!("{i:?}"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_inherent_method_shadows_a_trait_method() {
    let hir = check_source(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Point { fn show(self) -> string { return \"inherent\"; } }\n\
         impl Show for Point { fn show(self) -> string { return \"trait\"; } }\n\
         fn f(Point p) -> string { return p.show(); }",
    )
    .expect("the inherent method wins silently");
    let caller = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(format!("{:?}", f.body)),
            _ => None,
        })
        .expect("fn f");
    assert!(
        caller.contains("Point.show"),
        "the call resolves to the inherent method: {caller}"
    );
    assert!(
        !caller.contains("Point.Show.show"),
        "the trait impl is not the target: {caller}"
    );
}

#[test]
fn two_traits_providing_one_method_is_e1504() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         trait Debug2 { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self) -> string { return \"s\"; } }\n\
         impl Debug2 for Point { fn show(self) -> string { return \"d\"; } }\n\
         fn f(Point p) -> string { return p.show(); }",
        codes::AMBIGUOUS_METHOD,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("`Debug2`") && d.message.contains("`Show`")),
        "the diagnostic names both traits: {diags:?}"
    );
}

#[test]
fn a_trait_method_on_a_type_without_the_impl_is_e1503() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         struct Secret { x: int }\n\
         fn f(Secret s) -> string { return s.show(); }",
        codes::UNSATISFIED_BOUND,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("`Show`") && d.message.contains("`Secret`")),
        "the diagnostic names the trait and the type: {diags:?}"
    );
}

#[test]
fn a_single_trait_impl_is_the_target() {
    let hir = check_source(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self) -> string { return \"s\"; } }\n\
         fn f(Point p) -> string { return p.show(); }",
    )
    .expect("a single trait impl resolves");
    let caller = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(format!("{:?}", f.body)),
            _ => None,
        })
        .expect("fn f");
    assert!(
        caller.contains("Point.Show.show"),
        "the call names the trait symbol: {caller}"
    );
}

#[test]
fn an_associated_function_impl_checks() {
    let hir = check_source(
        "trait Factory { fn make() -> int; }\n\
         struct Point { x: int }\n\
         impl Factory for Point { fn make() -> int { return 1; } }",
    )
    .expect("an associated function impl must check");
    assert!(
        hir.items
            .iter()
            .any(|item| matches!(item, HirItem::Fn(f) if f.name == "Point.Factory.make")),
        "the method is emitted as Point.Factory.make"
    );
}

#[test]
fn a_pointer_receiver_impl_checks() {
    check_source(
        "trait Counter { fn inc(self: *mut Self); }\n\
         struct Point { x: int }\n\
         impl Counter for Point { fn inc(self: *mut Point) { } }",
    )
    .expect("a matching pointer receiver must check");
}

#[test]
fn an_inherent_impl_still_checks_alongside_a_trait_impl() {
    let hir = check_source(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Point { fn sum(self) -> int { return self.x; } }\n\
         impl Show for Point { fn show(self) -> string { return \"a\"; } }",
    )
    .expect("an inherent impl and a trait impl must coexist");
    assert!(hir
        .items
        .iter()
        .any(|item| matches!(item, HirItem::Fn(f) if f.name == "Point.sum")));
    assert!(hir
        .items
        .iter()
        .any(|item| matches!(item, HirItem::Fn(f) if f.name == "Point.Show.show")));
}

#[test]
fn a_bound_is_enforced_at_the_call_site() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         struct Secret { x: int }\n\
         fn show_it[T: Show](T x) -> string { return x.show(); }\n\
         fn go() -> string { return show_it(Secret { x: 1 }); }",
        codes::UNSATISFIED_BOUND,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("`Secret` does not implement `Show`")),
        "the diagnostic names the type and the trait: {diags:?}"
    );
}

#[test]
fn an_unbounded_type_parameter_cannot_call_a_trait_method() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self) -> string { return \"p\"; } }\n\
         fn show_it[T](T x) -> string { return x.show(); }\n\
         fn go() -> string { return show_it(Point { x: 1 }); }",
        codes::UNSATISFIED_BOUND,
    );
    assert!(
        diags.iter().any(|d| d.message.contains("needs `T`")),
        "the diagnostic names the missing bound: {diags:?}"
    );
}

#[test]
fn two_instantiations_produce_two_symbols_naming_their_impls() {
    let hir = check_source(
        "trait Show { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         struct Circle { r: int }\n\
         impl Show for Point { fn show(self) -> string { return \"p\"; } }\n\
         impl Show for Circle { fn show(self) -> string { return \"c\"; } }\n\
         fn show_it[T: Show](T x) -> string { return x.show(); }\n\
         fn a() -> string { return show_it(Point { x: 1 }); }\n\
         fn b() -> string { return show_it(Circle { r: 2 }); }",
    )
    .expect("both instantiations must check");
    let point_body = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "show_it_Point" => Some(format!("{f:?}")),
            _ => None,
        })
        .expect("a show_it_Point instantiation");
    assert!(
        point_body.contains("Point.Show.show"),
        "the Point instantiation calls Point's impl: {point_body}"
    );
    let circle_body = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "show_it_Circle" => Some(format!("{f:?}")),
            _ => None,
        })
        .expect("a show_it_Circle instantiation");
    assert!(
        circle_body.contains("Circle.Show.show"),
        "the Circle instantiation calls Circle's impl: {circle_body}"
    );
}

#[test]
fn a_bound_on_an_unknown_trait_is_e1500() {
    let diags = assert_code(
        "trait Show { fn show(self) -> string; }\n\
         fn f[T: Shw](T x) -> T { return x; }",
        codes::UNKNOWN_TRAIT,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.label.as_deref() == Some("did you mean `Show`?")),
        "the diagnostic suggests the nearest trait: {diags:?}"
    );
}

#[test]
fn a_type_path_call_checks_via_the_same_mangled_name_as_a_dot() {
    let hir = check_source(
        "struct Point { x: int, y: int }\n\
         impl Point { fn new(int x, int y) -> Point { return Point { x: x, y: y }; } }\n\
         fn f() -> Point { return Point::new(1, 2); }\n\
         fn g() -> Point { return Point.new(1, 2); }",
    )
    .expect("both spellings check");
    let callee_of = |name: &str| -> String {
        hir.items
            .iter()
            .find_map(|item| match item {
                HirItem::Fn(f) if f.name == name => Some(format!("{:?}", f.body)),
                _ => None,
            })
            .expect("the function under test")
    };
    assert!(callee_of("f").contains("\"Point.new\""));
    assert!(callee_of("g").contains("\"Point.new\""));
}

#[test]
fn a_method_called_through_a_type_path_is_e1506() {
    assert_code(
        "struct Point { x: int }\n\
         impl Point { fn get(self) -> int { return self.x; } }\n\
         fn f() -> int { return Point::get(); }",
        codes::METHOD_THROUGH_TYPE,
    );
}

#[test]
fn an_associated_function_called_on_a_value_is_e1507() {
    let diags = assert_code(
        "struct Point { x: int, y: int }\n\
         impl Point { fn origin() -> Point { return Point { x: 0, y: 0 }; } }\n\
         fn f(Point p) -> Point { return p.origin(); }",
        codes::ASSOC_THROUGH_VALUE,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("`Point::origin()`")),
        "the diagnostic names the type-path spelling: {diags:?}"
    );
}

#[test]
fn an_unknown_associated_function_is_e1508() {
    let diags = assert_code(
        "struct Point { x: int, y: int }\n\
         impl Point { fn origin() -> Point { return Point { x: 0, y: 0 }; } }\n\
         fn f() -> Point { return Point::origin2(); }",
        codes::UNKNOWN_ASSOCIATED_FN,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.label.as_deref() == Some("did you mean `origin`?")),
        "the diagnostic suggests the nearest associated function: {diags:?}"
    );
}

#[test]
fn a_qualified_trait_call_names_the_impl_symbol() {
    let hir = check_source(
        "trait Show { fn show(self) -> string; }\n\
         trait Debug2 { fn show(self) -> string; }\n\
         struct Point { x: int }\n\
         impl Show for Point { fn show(self) -> string { return \"s\"; } }\n\
         impl Debug2 for Point { fn show(self) -> string { return \"d\"; } }\n\
         fn f(Point p) -> string { return Show::show(p); }",
    )
    .expect("a qualified call disambiguates");
    let caller = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(format!("{:?}", f.body)),
            _ => None,
        })
        .expect("fn f");
    assert!(
        caller.contains("Point.Show.show") && !caller.contains("Point.Debug2.show"),
        "the Show impl is the target: {caller}"
    );
}
