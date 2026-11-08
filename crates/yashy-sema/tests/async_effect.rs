//! Async effects at the checking edge: `Task[T]`, `E1700`, `E1703`.
//!
//! The parser accepts `await` and `spawn` everywhere; the checker is what
//! refuses them outside an `async fn` (`E1700`) and refuses an `async`
//! method in a trait (`E1703`).

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirItem, HirProgram};

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

fn codes_of(diags: &[Diagnostic]) -> Vec<&str> {
    diags
        .iter()
        .map(|d| d.code.unwrap_or("<uncoded>"))
        .collect()
}

fn assert_codes(src: &str, expected: &[&str]) {
    let diags = check(src).expect_err("the program must be rejected");
    assert_eq!(codes_of(&diags), expected, "diagnostics: {diags:?}");
}

#[test]
fn await_outside_an_async_function_is_e1700() {
    assert_codes(
        "async fn f() -> int { return 1; } fn main() { let int n = f().await; }",
        &["E1700"],
    );
}

#[test]
fn spawn_outside_an_async_function_is_e1700() {
    assert_codes(
        "async fn w() -> int { return 1; } fn main() { scope { let t = spawn(w); } }",
        &["E1700"],
    );
}

#[test]
fn an_async_function_call_yields_task_and_await_yields_the_result() {
    let hir = check(
        "async fn f() -> int { return 1; } \
         async fn g() -> int { return f().await; } \
         fn main() { }",
    )
    .expect("an awaited task is the result type");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(f),
            _ => None,
        })
        .expect("f is emitted");
    assert!(f.is_async, "the async declaration is on the HirFn");
    assert_eq!(
        f.ret.kind,
        yashy_sema::TypeKind::Task(Box::new(yashy_sema::Type {
            kind: yashy_sema::TypeKind::I64,
            span: yashy_common::Span::new(0, 0),
        }))
    );
    assert!(hir.uses_async(), "the program uses async");
}

#[test]
fn printing_a_task_is_e0100() {
    let diags = check("async fn f() -> int { return 1; } fn main() { print(f()); }")
        .expect_err("a task has no text form");
    assert!(
        diags.iter().any(|d| d.code == Some(codes::TYPE_MISMATCH)),
        "printing a Task is E0100, got {:?}",
        codes_of(&diags)
    );
    assert!(
        diags.iter().any(|d| d.message.contains("await")),
        "the remedy names await: {diags:?}"
    );
}

#[test]
fn an_async_method_in_a_trait_is_e1703() {
    assert_codes("trait T { async fn m(self); } fn main() { }", &["E1703"]);
}

#[test]
fn await_on_a_non_task_is_e0100() {
    let diags = check("async fn f() -> int { let int n = 1; return n.await; } fn main() { }")
        .expect_err("await needs a task");
    assert!(
        diags.iter().any(|d| d.code == Some(codes::TYPE_MISMATCH)),
        "await on a non-task is E0100, got {:?}",
        codes_of(&diags)
    );
}
