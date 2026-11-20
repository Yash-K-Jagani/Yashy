//! Explicit closure captures. A plain lambda copies what it reads; a `move`
//! lambda transfers it, and the HIR records the mode on every capture so the
//! verifier and code generator see the same fact.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::hir::{CaptureMode, HirCapture};
use yashy_sema::{check_program_with, CheckOptions, HirExpr, HirItem, HirProgram, HirStmt};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("closures.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must accept it");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    check_program_with(&ast, &sm, file, CheckOptions { release: false })
}

fn captures(program: &HirProgram) -> Vec<HirCapture> {
    for item in &program.items {
        let HirItem::Fn(function) = item else { continue };
        if function.name != "main" {
            continue;
        }
        for statement in &function.body.stmts {
            let HirStmt::Const { init, .. } = statement else {
                continue;
            };
            if let HirExpr::Lambda { f, .. } = init {
                return f.captures.clone();
            }
        }
    }
    panic!("the lambda was not found in main")
}

#[test]
fn a_plain_lambda_copies_its_capture() {
    let program = check(
        "fn main() { let int x = 7; const f = || -> int { return x; }; print(f()); print(x); }",
    )
    .expect("a copy capture checks");
    let found = captures(&program);
    assert_eq!(found.len(), 1, "one capture: {found:?}");
    assert_eq!(found[0].name, "x");
    assert_eq!(found[0].mode, CaptureMode::Value);
}

#[test]
fn a_move_lambda_records_the_mode_and_uses_up_the_binding() {
    let program = check(
        "fn main() { let int x = 7; const f = move || -> int { return x; }; print(f()); }",
    )
    .expect("a move capture checks");
    let found = captures(&program);
    assert_eq!(found.len(), 1, "one capture: {found:?}");
    assert_eq!(found[0].mode, CaptureMode::Move);

    let diagnostics = check(
        "fn main() { let int x = 7; const f = move || -> int { return x; }; print(x); print(f()); }",
    )
    .expect_err("the binding was moved into the environment");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("moved")),
        "a use after the move must be refused: {diagnostics:?}"
    );
}
