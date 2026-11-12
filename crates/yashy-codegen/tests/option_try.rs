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

/// The failure path when the return type is a *different* `Result` shape.
///
/// Only the error types have to agree, so `Result[int, string]` may be
/// propagated out of a function returning `Result[string, string]`. The two
/// aggregates have different LLVM reprs, so the failure has to be rebuilt as
/// the return type's `Err`; the earlier lowering answered `unreachable` here
/// and the run continued past the failed `?` with the wrong payload.
#[test]
fn a_result_question_mark_rebuilds_a_different_ok_payload() {
    let ir = ir_of(
        "fn g() -> Result[int, string] { return Ok(7); }\n\
         fn f() -> Result[string, string] { const v = g()?; return Ok(\"x\" + str_from_i64(v)); }\n\
         fn main() {}\n",
    );
    assert!(
        !ir.contains("TODO: `?`"),
        "the failure path must build a value, not give up:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue { i8, i8*, i8* } undef, i8 1, 0"),
        "the failure must be tag 1 (`Err`) in the return type:\n{ir}"
    );
    assert!(
        ir.contains("extractvalue { i8, i64, i8* }") && ir.contains("insertvalue { i8, i8*, i8* }"),
        "the error payload must be carried from the subject's shape into the \
         return shape:\n{ir}"
    );
}

/// The same rule for `Option`, whose failure carries no payload at all.
#[test]
fn an_option_question_mark_rebuilds_a_different_payload() {
    let ir = ir_of(
        "fn g() -> Option[int] { return Some(7); }\n\
         fn f() -> Option[string] { const v = g()?; return Some(str_from_i64(v)); }\n\
         fn main() {}\n",
    );
    assert!(
        !ir.contains("TODO: `?`"),
        "the failure path must build a value, not give up:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue { i8, i8* } undef, i8 0, 0"),
        "`None` is tag 0 with no payload:\n{ir}"
    );
}
