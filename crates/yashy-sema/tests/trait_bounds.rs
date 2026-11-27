//! A generic helper bounded by a trait.
//!
//! The bound is checked at the call site, so the body is checked once without
//! knowing the concrete recorder type.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirProgram};

fn check_source(src: &str) -> Result<HirProgram, Vec<yashy_common::Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("bounds.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("lexer must succeed");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("parser must succeed");
    check_program_with(&ast, &sm, file, CheckOptions::default())
}

#[test]
fn a_generic_helper_can_call_a_bounded_method() {
    let source = "
pub trait Recorder {
    fn record(self: *mut Self, string name);
}
pub struct Mock { calls: string }
impl Recorder for Mock {
    fn record(self: *mut Self, string name) {
        const current = self[0].calls;
        self[0].calls = current + name;
    }
}
pub fn call_twice[T: Recorder](T value) {
    let local = value;
    local.record(\"a\");
    local.record(\"b\");
}
fn main() {
    let m = Mock { calls: \"\" };
    call_twice(m);
}
";
    let hir = check_source(source).expect("the bounded generic helper must check");
    assert!(!hir.items.is_empty());
}

#[test]
fn an_unbounded_argument_cannot_call_the_method() {
    let source = "
pub trait Recorder {
    fn record(self: *mut Self, string name);
}
pub fn call_once[T](T value) {
    value.record(\"a\");
}
fn main() { call_once(42); }
";
    let diags = check_source(source).expect_err("an unbounded parameter must not call the method");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("no method `record`")),
        "expected the method to be refused, got {:?}",
        diags
            .iter()
            .map(|d| (d.code, &d.message))
            .collect::<Vec<_>>()
    );
}
