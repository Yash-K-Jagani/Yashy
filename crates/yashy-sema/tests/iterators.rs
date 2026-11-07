//! The iterator protocol: `for x in it` over a user type lowers through
//! its `Iterator` impl as `ForC` + `IfLet`, and a type without one is
//! `E1503` at the `for`.

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram, HirStmt};

const ITER: &str = "pub trait Iterator[T] { fn next(self: *mut Self) -> Option[T]; }";

/// Lex, parse and check `src` as the stdlib's `iter.ya` module (so the
/// generic `Iterator` trait is allowed) plus a program.
fn check_with_iter(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let iter_file = sm.add_file("iter.ya", ITER.to_string());
    let file = sm.add_file("test.ya", src.to_string());
    let mut parsed = Vec::new();
    for (f, text) in [(iter_file, ITER), (file, src)] {
        let tokens = Lexer::with_file(text, f)
            .tokenize()
            .expect("lexer should never panic");
        let mut parser = Parser::new(tokens, &sm, f);
        let ast = parser
            .parse_program()
            .expect("parser should never panic on these inputs");
        parsed.push((f, ast));
    }
    let modules: Vec<_> = parsed.iter().map(|(f, a)| (*f, a)).collect();
    yashy_sema::check_modules_with(&modules, &sm, CheckOptions::default())
}

const COUNTER: &str = "struct Counter { n: int }\n\
     impl Iterator for Counter {\n\
         fn next(self: *mut Self) -> Option[int] {\n\
             if self[0].n <= 0 { return None; }\n\
             self[0].n = self[0].n - 1;\n\
             return Some(self[0].n);\n\
         }\n\
     }\n";

#[test]
fn a_user_iterator_lowers_to_for_c_and_if_let() {
    let hir = check_with_iter(&format!(
        "{COUNTER}\
         fn main() -> int {{ let total = 0; let c = Counter {{ n: 3 }}; for x in c {{ total = total + x; }} return total; }}"
    ))
    .expect("a user iterator must check");
    let body = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "main" => Some(&f.body),
            _ => None,
        })
        .expect("main");
    let loop_stmt = body
        .stmts
        .iter()
        .find(|s| matches!(s, HirStmt::ForC { .. }))
        .expect("the protocol loop lowers to ForC");
    match loop_stmt {
        HirStmt::ForC { init, body, .. } => {
            assert!(init.is_some(), "the iterator is moved into a mutable binding");
            assert!(
                body.stmts
                    .iter()
                    .any(|s| matches!(s, HirStmt::IfLet { .. })),
                "the loop body is an IfLet over next(): {body:?}"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn a_type_without_an_iterator_impl_is_e1503_at_the_for() {
    let diags = check_with_iter(
        "struct Plain { n: int }\n\
         fn main() { let p = Plain { n: 1 }; for x in p { print(x); } }",
    )
    .expect_err("no Iterator impl must be rejected");
    assert!(
        diags.iter().any(|d| d.code == Some(codes::UNSATISFIED_BOUND)),
        "expected E1503, got {:?}",
        diags.iter().map(|d| (d.code, &d.message)).collect::<Vec<_>>()
    );
}
