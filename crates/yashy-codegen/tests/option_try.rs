//! `?` on `Option`: the success tag is `Some`, not `Ok`.
//!
//! `gen_try` branches on the subject's variant tag. `Option` numbers `None` = 0
//! and `Some` = 1, while `Result` numbers `Ok` = 0 and `Err` = 1, so the two
//! success tags are different numbers. The lowering is shared, which makes a
//! copy-paste that hard-codes one tag easy and wrong: this test reads the
//! emitted comparison and pins `1` for `Option` and `0` for `Result`.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and emit `src` as one module.
fn ir_of(src: &str) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    let hir = check_program(&ast, &sm, file).expect("these test sources must type-check");
    emit_program(
        &hir,
        "main.ya",
        CodegenOptions::default(),
        &yashy_common::ProofFacts::empty(),
    )
}

/// The one `icmp eq i8` that decides whether `?` continues, as written.
fn success_comparison(ir: &str) -> &str {
    let mut hits = ir
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("icmp eq i8"));
    let first = hits
        .next()
        .unwrap_or_else(|| panic!("no tag comparison in the IR:\n{ir}"));
    assert!(
        hits.next().is_none(),
        "expected one tag comparison, found more:\n{ir}"
    );
    first
}

#[test]
fn an_option_question_mark_continues_on_some() {
    let ir = ir_of(
        "fn f(Option[int] o) -> Option[int] { const v = o?; return Some(v + 1); }\n\
         fn main() {}\n",
    );
    let comparison = success_comparison(&ir);
    assert!(
        comparison.ends_with(", 1"),
        "`Some` is tag 1, so `?` must continue on 1: {comparison}"
    );
}

#[test]
fn a_result_question_mark_continues_on_ok() {
    let ir = ir_of(
        "fn g() -> Result[int, string] { return Ok(7); }\n\
         fn f() -> Result[int, string] { const v = g()?; return Ok(v); }\n\
         fn main() {}\n",
    );
    let comparison = success_comparison(&ir);
    assert!(
        comparison.ends_with(", 0"),
        "`Ok` is tag 0, so `?` must continue on 0: {comparison}"
    );
}
