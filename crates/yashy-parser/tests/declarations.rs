//! Type-first declarations: `let int x = 5`, `fn add(int a, int b) -> int`.
//!
//! A binding and a parameter write the type before the name. The AST does not
//! change -- `Stmt::Let`/`Stmt::Const` still carry `name` and `Option<TypeExpr>`,
//! and `Param` still carries `name` and `ty` -- so these tests check the two
//! halves the parser is responsible for: that the name and the type land in the
//! right fields, and that every type shape `parse_type_expr` accepts is
//! reachable from the declaration grammar.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{Expr, Item, Program, Stmt, TypeExpr};
use yashy_parser::Parser;

fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("declarations.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize()?;
    let mut parser = Parser::new(tokens, &sources, file);
    parser.parse_program()
}

fn parse_ok(src: &str) -> Program {
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

/// The first statement of `fn main`, which the fixture writes as `stmt`.
fn first_main_stmt(stmt: &str) -> Stmt {
    let program = parse_ok(&format!("fn main() {{ {stmt} }}"));
    let Item::Fn { body, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function item");
    };
    body.stmts.into_iter().next().expect("main has a statement")
}

/// The `let` binding of `stmt`, as `(name, Option<TypeExpr>)`.
fn let_of(stmt: &str) -> (String, Option<TypeExpr>) {
    match first_main_stmt(stmt) {
        Stmt::Let { name, ty, .. } => (name, ty),
        other => panic!("expected a let binding, got {other:?}"),
    }
}

fn const_of(stmt: &str) -> (String, Option<TypeExpr>) {
    match first_main_stmt(stmt) {
        Stmt::Const { name, ty, .. } => (name, ty),
        other => panic!("expected a const binding, got {other:?}"),
    }
}

fn name_is(ty: &TypeExpr, expected: &str) -> bool {
    matches!(ty, TypeExpr::Name(name, _) if name == expected)
}

#[test]
fn a_let_writes_the_type_before_the_name() {
    let (name, ty) = let_of("let int x = 5;");
    assert_eq!(name, "x");
    assert!(matches!(ty, Some(ref ty) if name_is(ty, "int")));
}

#[test]
fn a_const_writes_the_type_before_the_name() {
    let (name, ty) = const_of("const string name = \"x\";");
    assert_eq!(name, "name");
    assert!(matches!(ty, Some(ref ty) if name_is(ty, "string")));
}

#[test]
fn an_untyped_binding_still_has_no_written_type() {
    let (name, ty) = let_of("let x = 5;");
    assert_eq!(name, "x");
    assert!(ty.is_none(), "no written type means the checker infers it");
}

#[test]
fn a_pointer_type_comes_before_its_name() {
    let (name, ty) = let_of("let *mut int p = q;");
    assert_eq!(name, "p");
    assert!(matches!(ty, Some(TypeExpr::Ptr { .. })));
}

#[test]
fn a_slice_type_comes_before_its_name() {
    let (name, ty) = let_of("let []int xs = ys;");
    assert_eq!(name, "xs");
    assert!(matches!(ty, Some(TypeExpr::Slice(..))));
}

#[test]
fn a_fixed_array_type_comes_before_its_name() {
    let (name, ty) = let_of("let [int; 3] xs = ys;");
    assert_eq!(name, "xs");
    assert!(matches!(ty, Some(TypeExpr::FixedArray(..))));
}

#[test]
fn an_option_type_comes_before_its_name() {
    let (name, ty) = let_of("let Option[int] o = n;");
    assert_eq!(name, "o");
    assert!(matches!(ty, Some(TypeExpr::Option(..))));
}

#[test]
fn a_map_type_comes_before_its_name() {
    let (name, ty) = let_of("let map[string]int m = n;");
    assert_eq!(name, "m");
    assert!(matches!(ty, Some(TypeExpr::Map(..))));
}

#[test]
fn a_tuple_type_comes_before_its_name() {
    let (name, ty) = let_of("let (int, string) pair = p;");
    assert_eq!(name, "pair");
    assert!(matches!(ty, Some(TypeExpr::Tuple(..))));
}

#[test]
fn a_function_type_comes_before_its_name() {
    let (name, ty) = let_of("let fn(int) -> int f = g;");
    assert_eq!(name, "f");
    assert!(matches!(ty, Some(TypeExpr::Fn { .. })));
}

#[test]
fn a_destructuring_binding_is_not_a_tuple_type() {
    // The `(` after `const` could start a tuple type; the `=` after the
    // matching `)` is what says it is a list of names instead.
    match first_main_stmt("const (a, b) = pair;") {
        Stmt::Destructure { names, .. } => assert_eq!(names.len(), 2),
        other => panic!("expected a destructuring binding, got {other:?}"),
    }
}

#[test]
fn a_function_parameter_writes_the_type_first() {
    let program = parse_ok("fn add(int a, int b) -> int { return a + b; }");
    let Item::Fn { params, .. } = &program.items[0] else {
        panic!("expected a function item");
    };
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "a");
    assert!(name_is(&params[0].ty, "int"));
    assert_eq!(params[1].name, "b");
    assert!(name_is(&params[1].ty, "int"));
}

#[test]
fn a_parameter_may_name_a_slice_or_a_pointer_type_first() {
    let program = parse_ok("fn f([]int xs, *mut int p) { }");
    let Item::Fn { params, .. } = &program.items[0] else {
        panic!("expected a function item");
    };
    assert!(matches!(params[0].ty, TypeExpr::Slice(..)));
    assert!(matches!(params[1].ty, TypeExpr::Ptr { .. }));
}

#[test]
fn a_lambda_parameter_writes_the_type_first() {
    let program = parse_ok("fn main() { const f = |int x| -> int { return x; }; }");
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected a function item");
    };
    let Stmt::Const {
        init: Expr::Lambda { params, .. },
        ..
    } = &body.stmts[0]
    else {
        panic!("expected a lambda, got {:?}", body.stmts[0]);
    };
    assert_eq!(params[0].name, "x");
    assert!(name_is(&params[0].ty, "int"));
}

#[test]
fn a_pointer_receiver_keeps_its_one_spelling() {
    // `self: *mut T` is a receiver, not a named parameter: the name is fixed
    // and the annotation says how the object is passed.
    let program =
        parse_ok("object Counter { value: int }\nimpl Counter { fn bump(self: *mut Counter) { } }");
    let Item::Impl { methods, .. } = &program.items[1] else {
        panic!("expected an impl block");
    };
    let Item::Fn { params, .. } = &methods[0] else {
        panic!("expected a method");
    };
    assert_eq!(params[0].name, "self");
    assert!(matches!(params[0].ty, TypeExpr::Ptr { .. }));
}

#[test]
fn a_struct_field_keeps_the_colon() {
    // Fields are a field list, not a variable declaration: `x: int` stays.
    let program = parse_ok("struct Point { x: int, y: int }");
    let Item::Struct { fields, .. } = &program.items[0] else {
        panic!("expected a struct item");
    };
    assert_eq!(fields[0].name, "x");
}

#[test]
fn a_non_receiver_parameter_still_needs_a_type() {
    // `fn f(x)` names a parameter with no type. The old grammar required the
    // colon; the new one tries to read `x` as the type and finds no name
    // after it, so it is still an error either way.
    let diagnostics = parse_err("fn f(x) { }");
    assert!(!diagnostics.is_empty());
}
