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
fn lambda(src: &str) -> (Vec<(String, Option<TypeExpr>)>, Option<TypeExpr>) {
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
    let (params, ret) = lambda("fn main() { const f = |int x, string y| -> bool { return true; }; }");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].0, "x");
    assert!(matches!(&params[0].1, Some(TypeExpr::Name(name, _)) if name == "int"));
    assert_eq!(params[1].0, "y");
    assert!(matches!(&params[1].1, Some(TypeExpr::Name(name, _)) if name == "string"));
    assert!(matches!(ret, Some(TypeExpr::Name(name, _)) if name == "bool"));
}

#[test]
fn a_lambda_without_an_arrow_has_no_return_type() {
    let (params, ret) = lambda("fn main() { const f = |int x| { print(x); }; }");
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
fn a_lambda_parameter_may_omit_its_type() {
    let (params, ret) = lambda("fn main() { const f = |x, y| -> int { return 1; }; }");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].0, "x");
    assert!(params[0].1.is_none(), "`|x|` asks the callee for the type");
    assert_eq!(params[1].0, "y");
    assert!(params[1].1.is_none());
    assert!(matches!(ret, Some(TypeExpr::Name(name, _)) if name == "int"));
}

#[test]
fn a_trailing_block_is_the_last_argument_of_a_call() {
    let stmt = first_stmt("fn main() { with_file(path) { |f| return f; }; }");
    let Stmt::Expr(Expr::Call { args, .. }, _) = stmt else {
        panic!("expected a call, got {stmt:?}");
    };
    assert_eq!(args.len(), 2, "the block is one more argument");
    let Expr::Lambda { params, .. } = &args[1] else {
        panic!("expected a lambda as the last argument, got {:?}", args[1]);
    };
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "f");
    assert!(params[0].ty.is_none());
}

#[test]
fn a_trailing_block_attaches_to_a_method_without_parentheses() {
    let stmt = first_stmt("fn main() { xs.map { |x| return x; }; }");
    let Stmt::Expr(Expr::Call { callee, args, .. }, _) = stmt else {
        panic!("expected a call, got {stmt:?}");
    };
    assert_eq!(callee, vec!["xs".to_string(), "map".to_string()]);
    assert_eq!(args.len(), 1);
    assert!(matches!(args[0], Expr::Lambda { .. }));
}

#[test]
fn a_brace_after_a_condition_is_the_then_block() {
    // The trailing block is only an argument where no block is expected. A
    // condition's `{` still opens the body, even when the condition is a bare
    // name (`if ready { }`) or a call (`if ready() { }`).
    assert!(matches!(
        first_stmt("fn main() { if ready { print(1); } }"),
        Stmt::If { .. }
    ));
    assert!(matches!(
        first_stmt("fn main() { if ready() { print(1); } }"),
        Stmt::If { .. }
    ));
    assert!(matches!(
        first_stmt("fn main() { while ready { } }"),
        Stmt::ForC { .. }
    ));
    assert!(matches!(
        first_stmt("fn main() { for x in xs { } }"),
        Stmt::ForIn { .. }
    ));
}
#[test]
fn a_lambda_is_an_expression() {
    // The point of the syntax: a lambda is a value, so it can be an argument.
    let program = "fn main() { apply(|int x| -> int { return x; }, 1); }";
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
