//! The alias corpus: one positive and one negative case for every ALIAS rule
//! that has an enforcing diagnostic, plus an explicit record of the rule that
//! does not. The cases are checked in-process, so no linked binary is needed.
//!
//! A rule with no code is not silently dropped: it is named as an orphan, so
//! `yashyc trace --fail-orphans` and this test agree about what is enforced.

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirProgram};
use yashy_verify::{inventory, AssumptionKind};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("alias_corpus.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the corpus must parse");
    check_program_with(&ast, &sm, file, CheckOptions::default())
}

fn has_code(diagnostics: &[Diagnostic], code: &str) -> bool {
    diagnostics.iter().any(|d| d.code == Some(code))
}

// ALIAS-1: an owning pointer is unique, so a bare copy is the mistake.
#[test]
fn alias_1_copies_are_refused_and_moves_are_accepted() {
    let rejected = check("fn main() { let p = alloc(8); const q = p; q.free(); p.free(); }")
        .expect_err("a bare copy creates two owners");
    assert!(
        has_code(&rejected, codes::POINTER_COPY_WITHOUT_MOVE),
        "ALIAS-1 must be named: {rejected:?}"
    );
    check("fn main() { let p = alloc(8); let q = move p; q.free(); }")
        .expect("an explicit move keeps one owner");
}

// ALIAS-2: a pointer from C may alias anything, so the deref needs `unsafe`.
#[test]
fn alias_2_c_pointer_reads_need_unsafe() {
    let source = |body: &str| {
        format!(
            "extern \"C\" {{ unsafe fn malloc() -> *mut i64; }}\n\
             fn main() {{ let p = malloc(); {body} }}"
        )
    };
    let rejected = check(&source("let v = p[0]; print(v);"))
        .expect_err("a C pointer cannot be read in safe code");
    assert!(
        has_code(&rejected, codes::FFI_POINTER_DEREF),
        "ALIAS-2 must be named: {rejected:?}"
    );
    check(&source("unsafe { print(p[0]); }")).expect("the unsafe block is the promise");
}

// ALIAS-3: a view must name storage the compiler can keep alive.
#[test]
fn alias_3_views_of_temporaries_are_refused_and_bindings_are_accepted() {
    let rejected = check(
        "fn make() -> []int { return []int { 1, 2 }; }\n\
                          fn main() { const v = make()[0..1]; }",
    )
    .expect_err("a view of a temporary has no source to keep alive");
    assert!(
        has_code(&rejected, codes::UNPROVED_VIEW_SOURCE),
        "ALIAS-3 must be named: {rejected:?}"
    );
    check("fn main() { let xs = []int { 1, 2 }; const v = xs[0..1]; xs.free(); }")
        .expect("a named binding is a proved source");
}

// ALIAS-4 is written in the architecture doc but has no diagnostic yet. The
// corpus says so rather than inventing a case that nothing enforces.
#[test]
fn alias_4_is_recorded_as_unenforced() {
    assert!(
        codes::PLANNED_RULES.contains(&"ALIAS-4"),
        "a rule without a code must be named as a planned rule"
    );
    assert!(
        !codes::CODES
            .iter()
            .any(|code| codes::rule_for(code.id) == "ALIAS-4"),
        "ALIAS-4 must not claim an enforcing diagnostic it does not have"
    );
}

// ALIAS-5: every trust assumption is listed, with a reason a reader can audit.
#[test]
fn alias_5_every_assumption_is_listed_with_a_reason() {
    let program = check(
        "extern \"C\" { unsafe fn malloc() -> *mut i64; }\n\
         fn main() { let p = malloc(); unsafe { print(p[0]); } }",
    )
    .expect("the program checks");
    let found = inventory(&program);
    assert!(
        found
            .iter()
            .any(|a| a.kind == AssumptionKind::ExternDecl && a.name == "malloc"),
        "the C declaration must be listed: {found:?}"
    );
    assert!(
        found.iter().any(|a| a.kind == AssumptionKind::UnsafeBlock),
        "the unsafe block must be listed: {found:?}"
    );
    for item in &found {
        assert!(
            !item.reason.is_empty(),
            "{} has no reason",
            item.kind.as_str()
        );
    }
}
