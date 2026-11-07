//! Trait declarations parse as requirement sets: a name, optional type
//! parameters, and methods without bodies.
//!
//! The trait's methods are signatures, never implementations. A body inside a
//! trait is a parse error rather than a type-check error because the language
//! has no default methods (spec §3.1): the shape is visible before any name is
//! resolved.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{Item, TypeExpr};
use yashy_parser::Parser;

fn parse_src(src: &str) -> Result<yashy_parser::ast::Program, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("traits.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize()?;
    Parser::new(tokens, &sources, file).parse_program()
}

fn parse_ok(src: &str) -> yashy_parser::ast::Program {
    match parse_src(src) {
        Ok(program) => program,
        Err(diags) => panic!("{src:?} must parse, got {diags:?}"),
    }
}

fn parse_err(src: &str) -> Vec<Diagnostic> {
    match parse_src(src) {
        Ok(program) => panic!("{src:?} must not parse, got {program:?}"),
        Err(diags) => diags,
    }
}

fn messages(diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_trait_with_one_method_parses() {
    let program = parse_ok("trait Show { fn show(self) -> string; }");
    match &program.items[0] {
        Item::Trait {
            name,
            type_params,
            methods,
            ..
        } => {
            assert_eq!(name, "Show");
            assert!(type_params.is_empty());
            assert_eq!(methods.len(), 1);
            assert_eq!(methods[0].name, "show");
            assert_eq!(methods[0].params.len(), 1);
            assert_eq!(methods[0].params[0].name, "self");
            assert!(matches!(
                &methods[0].ret,
                Some(TypeExpr::Name(name, _)) if name == "string"
            ));
        }
        other => panic!("expected a trait item, got {other:?}"),
    }
}

#[test]
fn a_trait_with_several_methods_keeps_their_order() {
    let program = parse_ok(
        "trait Shape { fn area(self) -> float; fn name(self) -> string; fn sides(self) -> int; }",
    );
    let Item::Trait { methods, .. } = &program.items[0] else {
        panic!("expected a trait item");
    };
    let names: Vec<&str> = methods.iter().map(|method| method.name.as_str()).collect();
    assert_eq!(names, vec!["area", "name", "sides"]);
}

#[test]
fn a_trait_method_may_be_an_associated_function() {
    let program = parse_ok("trait Factory { fn make() -> int; }");
    let Item::Trait { methods, .. } = &program.items[0] else {
        panic!("expected a trait item");
    };
    assert!(
        methods[0].params.is_empty(),
        "a method with no receiver has no parameters"
    );
}

#[test]
fn a_trait_method_with_a_body_is_rejected() {
    let diags = parse_err("trait Show { fn show(self) -> string { return \"x\"; } }");
    assert!(
        messages(&diags).contains("trait method has no body"),
        "the diagnostic names the rule, got {diags:?}"
    );
}

#[test]
fn a_pub_method_inside_a_trait_is_rejected() {
    let diags = parse_err("trait Show { pub fn show(self) -> string; }");
    assert!(
        messages(&diags).contains("trait method takes the trait's visibility"),
        "the diagnostic explains where visibility belongs, got {diags:?}"
    );
}

#[test]
fn pub_trait_wraps_the_trait() {
    let program = parse_ok("pub trait Show { fn show(self) -> string; }");
    match &program.items[0] {
        Item::Public { item, .. } => {
            assert!(matches!(**item, Item::Trait { .. }));
        }
        other => panic!("expected a public wrapper, got {other:?}"),
    }
}

#[test]
fn type_parameters_carry_bounds() {
    let program = parse_ok("fn f[T: Show, U: Show + Display](T x, U y) -> T { return x; }");
    let Item::Fn {
        type_params, ..
    } = &program.items[0]
    else {
        panic!("expected a function item");
    };
    assert_eq!(type_params.len(), 2);
    assert_eq!(type_params[0].name, "T");
    assert_eq!(type_params[0].bounds, vec!["Show".to_string()]);
    assert_eq!(type_params[1].name, "U");
    assert_eq!(
        type_params[1].bounds,
        vec!["Show".to_string(), "Display".to_string()]
    );
}

#[test]
fn type_parameters_on_a_trait_parse() {
    let program = parse_ok("trait Iterator[T] { fn next(self: *mut Self) -> Option[T]; }");
    let Item::Trait {
        type_params,
        methods,
        ..
    } = &program.items[0]
    else {
        panic!("expected a trait item");
    };
    assert_eq!(type_params.len(), 1);
    assert_eq!(type_params[0].name, "T");
    assert!(matches!(&methods[0].params[0].ty, TypeExpr::Ptr { .. }));
}
