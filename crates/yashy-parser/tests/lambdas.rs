//! The lambda syntax: `|x: int| -> int { ... }`.
//!
//! A lambda is a function without a name, so its parameters and its return
//! type are written out: there is no declaration for the checker to take them
//! from. These tests pin the shape of the node, because a lambda that parsed
//! as a block of statements would still look plausible and would mean
//! something else entirely.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{Expr, Item, Stmt, TypeExpr};
use yashy_parser::Parser;

/// Parse `fn main() { <stmt> }` and return the statement.
fn first_stmt(src: &str) -> Stmt {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must accept the program");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .unwrap_or_else(|error| panic!("`{src}` must parse: {error:?}"));
    let item = program
        .items
        .into_iter()
        .next()
        .expect("the program has one item");
    let Item::Fn { body, .. } = item else {
        panic!("expected a function item, got {item:?}");
    };
    body.stmts
        .into_iter()
        .next()
        .expect("the function has one statement")
}

/// The expression a `const` binding is initialised with.
fn const_init(src: &str) -> Expr {
    match first_stmt(src) {
        Stmt::Const { init, .. } => init,
        other => panic!("expected a const binding, got {other:?}"),
    }
}

/// The lambda in `src`, which must be the whole initialiser.
fn lambda(src: &str) -> (Vec<(String, TypeExpr)>, Option<TypeExpr>) {
    match const_init(src) {
        Expr::Lambda {
            params, ret, body, ..
        } => {
            assert!(!body.stmts.is_empty(), "the lambda has a body");
            (
                params
                    .into_iter()
                    .map(|param| (param.name, param.ty))
                    .collect(),
                ret,
            )
        }
        other => panic!("expected a lambda, got {other:?}"),
    }
}

#[test]
fn a_lambda_keeps_its_parameters_and_return_type() {
    let (params, ret) = lambda("fn main() { const f = |x: int, y: string| -> bool { return true; }; }");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].0, "x");
    assert!(matches!(&params[0].1, TypeExpr::Name(name, _) if name == "int"));
    assert_eq!(params[1].0, "y");
    assert!(matches!(&params[1].1, TypeExpr::Name(name, _) if name == "string"));
    assert!(matches!(ret, Some(TypeExpr::Name(name, _)) if name == "bool"));
}

#[test]
fn a_lambda_without_an_arrow_has_no_return_type() {
    let (params, ret) = lambda("fn main() { const f = |x: int| { print(x); }; }");
    assert_eq!(params.len(), 1);
    assert!(ret.is_none(), "no arrow means no declared return type");
}

#[test]
fn a_lambda_may_take_no_parameters() {
    let (params, ret) = lambda("fn main() { const f = || -> int { return 1; }; }");
    assert!(params.is_empty());
    assert!(matches!(ret, Some(TypeExpr::Name(name, _)) if name == "int"));
}

#[test]
fn a_lambda_is_an_expression() {
    // The point of the syntax: a lambda is a value, so it can be an argument.
    let program = "fn main() { apply(|x: int| -> int { return x; }, 1); }";
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", program.to_string());
    let tokens = Lexer::new(program)
        .tokenize()
        .expect("the lexer must accept the program");
    let mut parser = Parser::new(tokens, &sources, file);
    let parsed = parser
        .parse_program()
        .unwrap_or_else(|error| panic!("`{program}` must parse: {error:?}"));
    let Item::Fn { body, .. } = &parsed.items[0] else {
        panic!("expected a function item");
    };
    let Stmt::Expr(Expr::Call { args, .. }, _) = &body.stmts[0] else {
        panic!("expected a call, got {:?}", body.stmts[0]);
    };
    assert!(matches!(args[0], Expr::Lambda { .. }));
}
