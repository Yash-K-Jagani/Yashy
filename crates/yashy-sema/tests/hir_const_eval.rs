//! Differential proof for the typed-HIR const evaluator.
//!
//! The const evaluator used to consume only the parser AST — a second compiler
//! alongside the real checker. The checker now lowers an accepted `const fn`
//! body to typed HIR, and `evaluate_hir` runs that typed tree. Until the two
//! paths are proven equal, the checker keeps the AST path as the oracle. This
//! test is the proof: for a corpus of const fns, the AST and HIR evaluators must
//! answer the same value for the same inputs.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with_const_fns, differential_evaluate, CheckOptions, ConstTypes};

fn evaluate_all(src: &str) -> Vec<(String, yashy_sema::ConstValue, yashy_sema::ConstValue)> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("hir_eval.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    let (_, consts) = check_program_with_const_fns(&ast, &sm, file, CheckOptions::default())
        .expect("the program must check");
    // The staged migration only proves anything when bodies were actually
    // lowered to typed HIR; a table of AST-only bodies would make the
    // differential vacuous.
    assert!(
        consts.values().all(|f| f.hir_body.is_some()),
        "every const fn body must be lowered to typed HIR"
    );
    let structs = ConstTypes::new();
    let mut results = Vec::new();
    for (name, function) in &consts {
        let (ast_value, hir_value) = differential_evaluate(function, &consts, &structs)
            .unwrap_or_else(|e| panic!("differential failed for {name}: {}", e.message));
        results.push((name.clone(), ast_value, hir_value));
    }
    results
}

fn assert_corpus(src: &str) {
    let results = evaluate_all(src);
    assert!(!results.is_empty(), "the corpus must declare const fns");
    for (name, ast, hir) in results {
        assert_eq!(ast, hir, "AST and HIR evaluators disagree for `{name}`");
    }
}

#[test]
fn arithmetic_and_literals_agree() {
    assert_corpus(
        "const fn one() -> int { return 1; }\n\
         const fn add(int a) -> int { return a + 4; }\n\
         const fn arith() -> int { return (2 + 3) * 4 - 1; }\n\
         const fn floaty() -> float { return 1.5 + 2.25; }\n\
         const fn flag() -> bool { return 2 < 3; }\n\
         fn main() { }",
    );
}

#[test]
fn branching_and_loops_agree() {
    assert_corpus(
        "const fn sign(int n) -> int { if n < 0 { return -1; } return 1; }\n\
         const fn sum(int n) -> int { let int t = 0; let i = 0; while i < n { t = t + i; i = i + 1; } return t; }\n\
         const fn switchy(int n) -> int { return switch n { case 0 { 10 } case 1..=5 { 20 } default { 30 } }; }\n\
         fn main() { }",
    );
}

#[test]
fn arrays_and_nested_calls_agree() {
    assert_corpus(
        "const fn square(int n) -> int { return n * n; }\n\
         const fn quad(int n) -> int { return square(n) + square(n); }\n\
         const fn table() -> [int; 4] { return [1, 2, 3, 4]; }\n\
         fn main() { }",
    );
}

#[test]
fn conversions_agree() {
    assert_corpus(
        "const fn code() -> int { return 'A'.to_int(); }\n\
         const fn letter() -> int { return 66.to_char().to_int(); }\n\
         fn main() { }",
    );
}
