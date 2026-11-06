//! Named and default arguments: resolved at the call site to positional HIR.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirExpr, HirItem, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file)
}

fn assert_error_code(src: &str, code: &str) {
    match check(src) {
        Ok(_) => panic!("expected an error with code {code}"),
        Err(diagnostics) => assert!(
            diagnostics.iter().any(|d| d.code == Some(code)),
            "expected code {code}, got {diagnostics:?}"
        ),
    }
}

const F: &str = "fn f(int a, int b) -> int { return a + b; } ";

fn call_args(program: &HirProgram) -> Vec<HirExpr> {
    let HirItem::Fn(f) = program
        .items
        .iter()
        .find_map(|i| match i {
            HirItem::Fn(f) if f.name == "main" => Some(i.clone()),
            _ => None,
        })
        .expect("main")
    else {
        panic!("expected a function item");
    };
    match &f.body.stmts[0] {
        yashy_sema::HirStmt::Expr(HirExpr::Call { args, .. }, _, _) => args.clone(),
        other => panic!("expected a call statement, got {other:?}"),
    }
}

#[test]
fn named_arguments_resolve_to_parameter_positions() {
    let named = check(&format!("{F}fn main() {{ f(b: 2, a: 1); }}")).expect("named args check");
    let positional = check(&format!("{F}fn main() {{ f(1, 2); }}")).expect("positional args check");
    // The second item is `main` in both programs; its HIR call must be equal
    // modulo source spans (the two spellings sit at different positions).
    let named_values: Vec<_> = call_args(&named)
        .iter()
        .map(|a| match a {
            HirExpr::Literal(l, _, _) => format!("{l:?}"),
            other => format!("{other:?}"),
        })
        .collect();
    let positional_values: Vec<_> = call_args(&positional)
        .iter()
        .map(|a| match a {
            HirExpr::Literal(l, _, _) => format!("{l:?}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        named_values, positional_values,
        "named and positional spellings produce the same HIR call"
    );
}

#[test]
fn a_default_fills_a_missing_argument() {
    let src = "fn f(int a, int b = 2) -> int { return a + b; } fn main() { f(1); }";
    let program = check(src).expect("a default fills the missing argument");
    let args = call_args(&program);
    assert_eq!(args.len(), 2, "the HIR call is full: {args:?}");
    assert!(
        matches!(&args[1], HirExpr::Literal(_, _, _)),
        "the default is inserted as a literal: {args:?}"
    );
}

#[test]
fn a_named_argument_that_names_no_parameter_is_e0701() {
    assert_error_code(&format!("{F}fn main() {{ f(c: 1); }}"), "E0701");
}

#[test]
fn a_positional_argument_after_a_named_one_is_e0701() {
    assert_error_code(&format!("{F}fn main() {{ f(b: 2, 1); }}"), "E0701");
}

#[test]
fn a_duplicate_argument_is_e0702() {
    assert_error_code(&format!("{F}fn main() {{ f(a: 1, a: 2); }}"), "E0702");
    assert_error_code(&format!("{F}fn main() {{ f(1, a: 2); }}"), "E0702");
}

#[test]
fn a_default_that_calls_a_function_is_e0102() {
    assert_error_code(
        "fn g() -> int { return 1; } fn f(int a = g()) -> int { return a; } fn main() { }",
        "E0102",
    );
}

#[test]
fn a_missing_argument_without_a_default_is_e0700() {
    assert_error_code(&format!("{F}fn main() {{ f(1); }}"), "E0700");
}

#[test]
fn a_named_argument_may_be_mixed_after_positional() {
    check(&format!("{F}fn main() {{ f(1, b: 2); }}")).expect("positional then named is fine");
}
