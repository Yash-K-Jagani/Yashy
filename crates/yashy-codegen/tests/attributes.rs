//! Parameter attributes are a promise the optimizer may rely on, so they come
//! from verifier facts and from nowhere else.
//!
//! Every positive test has a counterpart that must *not* get the attribute:
//! a parameter that escapes gets neither `noalias` nor `readonly`, an
//! `Option`-carried pointer is not a bare pointer, and a caller with no
//! verifier at all gets no attributes. The conservative path is the default.

use yashy_common::{ProofFacts, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};
use yashy_verify::{verify, VerifyOptions};

/// Lex, parse, check and emit `src` as one module, then emit it again with the
/// facts the verifier proved.
///
/// Returning both IRs is what lets the last test compare the two paths: the
/// same program with no facts must be attribute-free.
fn lower(src: &str) -> (String, String) {
    let mut sources = SourceMap::new();
    let file = sources.add_file("attributes.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the fixture's lexer must succeed");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the fixture must parse");
    let hir = check_program(&ast, &sources, file).expect("the fixture must check");

    let report = verify(&hir, &VerifyOptions::default());
    assert_eq!(
        report.error_count(),
        0,
        "the fixture must verify: {:?}",
        report.diagnostics
    );

    let verified = emit_program(
        &hir,
        "attributes.ya",
        CodegenOptions::default(),
        &report.facts,
    );
    let bare = emit_program(
        &hir,
        "attributes.ya",
        CodegenOptions::default(),
        &ProofFacts::empty(),
    );
    (verified, bare)
}

/// The `define` line that declares `name`.
fn signature<'a>(ir: &'a str, name: &str) -> &'a str {
    let needle = format!("@{name}(");
    ir.lines()
        .find(|line| line.starts_with("define ") && line.contains(&needle))
        .unwrap_or_else(|| panic!("no `define` for `{name}` in:\n{ir}"))
}

const READ_ONLY: &str = "\
fn read(*mut int p, int n) -> int {
    let sum = 0;
    let i = 0;
    while i < n {
        sum += p[i];
        i += 1;
    }
    return sum;
}
fn main() { }
";

const WRITES: &str = "\
fn fill(*mut int p, int n) {
    let i = 0;
    while i < n {
        p[i] = i;
        i += 1;
    }
}
fn main() { }
";

const RETURNS: &str = "\
fn give(*mut int p) -> *mut int {
    return p;
}
fn main() { }
";

const STORES: &str = "\
struct Holder {
    p: *mut int,
}
fn keep(*mut int p) -> Holder {
    return Holder { p: p };
}
fn main() { }
";

const OPTION: &str = "\
fn maybe(Option[*mut int] p) -> int {
    return 0;
}
fn main() { }
";

#[test]
fn a_read_only_pointer_gets_noalias_nonnull_readonly_and_alignment() {
    let (ir, _) = lower(READ_ONLY);
    assert_eq!(
        signature(&ir, "read"),
        "define i64 @read(i8* noalias nonnull readonly align 8 %arg0, i64 %arg1) {"
    );
}

#[test]
fn a_pointer_written_through_gets_noalias_but_not_readonly() {
    let (ir, _) = lower(WRITES);
    assert_eq!(
        signature(&ir, "fill"),
        "define void @fill(i8* noalias nonnull align 8 %arg0, i64 %arg1) {"
    );
}

#[test]
fn a_returned_pointer_escapes_and_loses_noalias_and_readonly() {
    let (ir, _) = lower(RETURNS);
    assert_eq!(
        signature(&ir, "give"),
        "define i8* @give(i8* nonnull align 8 %arg0) {"
    );
}

#[test]
fn a_pointer_stored_in_a_struct_escapes_too() {
    let (ir, _) = lower(STORES);
    assert_eq!(
        signature(&ir, "keep"),
        "define %struct.Holder @keep(i8* nonnull align 8 %arg0) {"
    );
}

#[test]
fn an_option_carried_pointer_is_not_a_bare_pointer() {
    let (ir, _) = lower(OPTION);
    let line = signature(&ir, "maybe");
    assert!(
        line.contains("{ i8, i8* } %arg0"),
        "the payload is a struct, not a pointer: {line}"
    );
    for attribute in ["noalias", "nonnull", "readonly", "align"] {
        assert!(
            !line.contains(attribute),
            "`{attribute}` is a claim about a pointer parameter: {line}"
        );
    }
}

#[test]
fn a_lambda_parameter_gets_the_same_attributes_as_a_named_one() {
    let source = "\
fn main() {
    let f = |*mut int p| -> int {
        let x = p[0];
        p.free();
        return x;
    };
    let buf = alloc(1);
    buf[0] = 7;
    print(f(buf));
    println(\"\");
}
";
    let (ir, _) = lower(source);
    assert_eq!(
        signature(&ir, "__lambda_0"),
        "define i64 @__lambda_0(i8* %env, i8* noalias nonnull align 8 %arg0) {"
    );
}

#[test]
fn without_facts_no_attribute_is_emitted() {
    let (_, bare) = lower(READ_ONLY);
    let line = signature(&bare, "read");
    assert_eq!(line, "define i64 @read(i8* %arg0, i64 %arg1) {");
}
