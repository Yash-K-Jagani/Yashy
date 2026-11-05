//! `if let` and `while let` after checking: the shapes they lower to and the
//! names their patterns bind.

use yashy_common::{Diagnostic, FileId, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{
    check_program_with, CheckOptions, HirItem, HirPattern, HirProgram, HirStmt, TypeKind,
};

/// Lex, parse and check `source`.
fn check_source(src: &str) -> (Result<HirProgram, Vec<Diagnostic>>, SourceMap, FileId) {
    let mut sm = SourceMap::new();
    let file = sm.add_file("if_let.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    let result = check_program_with(&ast, &sm, file, CheckOptions { release: false });
    (result, sm, file)
}

fn check_ok(src: &str) -> HirProgram {
    match check_source(src) {
        (Ok(hir), _, _) => hir,
        (Err(diags), _, _) => {
            panic!(
                "expected {src:?} to type-check, got {:?}",
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            )
        }
    }
}

fn check_err(src: &str) -> Vec<Diagnostic> {
    match check_source(src) {
        (Ok(_), _, _) => panic!("expected {src:?} to fail"),
        (Err(diags), _, _) => diags,
    }
}

fn main_body(hir: &HirProgram) -> &[HirStmt] {
    hir.items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "main" => Some(f.body.stmts.as_slice()),
            _ => None,
        })
        .expect("the fixture defines main")
}

fn assert_message(diags: &[Diagnostic], needle: &str) {
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected a diagnostic containing `{needle}`, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn if_let_binds_the_payload_for_the_then_block() {
    let hir = check_ok(
        "fn main() {\n\
             const Option[int] opt = Some(1);\n\
             if let Some(x) = opt { print(x); }\n\
         }\n",
    );
    match main_body(&hir).get(1) {
        Some(HirStmt::IfLet {
            pattern,
            then_block,
            else_block,
            ..
        }) => {
            assert!(
                matches!(
                    pattern,
                    HirPattern::TupleVariant { ref path, ref args, .. }
                        if path == &["Some".to_string()]
                            && matches!(
                                args.as_slice(),
                                [HirPattern::Binding { name, ty, .. }]
                                    if name == "x" && ty.kind == TypeKind::I64
                            )
                ),
                "{pattern:?}"
            );
            assert_eq!(then_block.stmts.len(), 1);
            assert!(else_block.is_none());
        }
        other => panic!("expected an `if let`, got {other:?}"),
    }
}

#[test]
fn a_pattern_binding_is_not_visible_after_the_if() {
    let diags = check_err(
        "fn main() {\n\
             const Option[int] opt = Some(1);\n\
             if let Some(x) = opt { print(x); }\n\
             print(x);\n\
         }\n",
    );
    assert_message(&diags, "cannot find value `x`");
}

#[test]
fn a_pattern_binding_is_not_visible_in_the_else_block() {
    // The else branch is the path where the pattern did *not* match, so it has
    // no payload to name.
    let diags = check_err(
        "fn main() {\n\
             const Option[int] opt = Some(1);\n\
             if let Some(x) = opt { print(x); } else { print(x); }\n\
         }\n",
    );
    assert_message(&diags, "cannot find value `x`");
}

#[test]
fn a_missing_else_still_merges_ownership() {
    // A value moved in the then branch is moved after the statement: the merge
    // has to assume the branch ran, because whether it did is a run-time
    // question.
    let diags = check_err(
        "fn main() {\n\
             let buf = alloc(16);\n\
             const Option[int] opt = Some(1);\n\
             if let Some(_) = opt { buf.free(); }\n\
             buf.free();\n\
         }\n",
    );
    assert!(
        diags.iter().any(|d| d.message.contains("moved")),
        "{:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn while_let_lowers_to_an_endless_loop_with_a_breaking_else() {
    let hir = check_ok(
        "fn main() {\n\
             const Option[int] opt = Some(1);\n\
             while let Some(x) = opt { print(x); }\n\
         }\n",
    );
    let Some(HirStmt::ForC {
        init,
        cond,
        update,
        body,
        ..
    }) = main_body(&hir).get(1)
    else {
        panic!("expected a `for` loop, got {:?}", main_body(&hir).get(1));
    };
    assert!(init.is_none() && cond.is_none() && update.is_none());
    let Some(HirStmt::IfLet {
        pattern,
        then_block,
        else_block,
        ..
    }) = body.stmts.first()
    else {
        panic!(
            "expected the loop body to be an `if let`, got {:?}",
            body.stmts
        );
    };
    assert!(
        matches!(
            pattern,
            HirPattern::TupleVariant { ref path, .. } if path == &["Some".to_string()]
        ),
        "{pattern:?}"
    );
    assert_eq!(then_block.stmts.len(), 1);
    let leave = else_block.as_ref().expect("the miss path breaks the loop");
    assert!(
        matches!(leave.stmts.as_slice(), [HirStmt::Break(_)]),
        "{:?}",
        leave.stmts
    );
}

#[test]
fn a_refutable_literal_pattern_type_checks() {
    let hir = check_ok(
        "fn main() {\n\
             const int n = 1;\n\
             if let 1 = n { print(n); }\n\
         }\n",
    );
    assert!(matches!(
        main_body(&hir).get(1),
        Some(HirStmt::IfLet { .. })
    ));
}

#[test]
fn a_binding_pattern_that_always_matches_is_e0603() {
    let diags = check_err("fn main() {\n    if let x = 1 { print(x); }\n}\n");
    assert_message(
        &diags,
        "this pattern matches every `int`: `if let` needs a pattern that can fail",
    );
}

#[test]
fn a_wildcard_pattern_that_always_matches_is_e0603() {
    let diags = check_err("fn main() {\n    if let _ = 1 { print(1); }\n}\n");
    assert_message(&diags, "`if let` needs a pattern that can fail");
}

#[test]
fn a_single_variant_payload_is_irrefutable() {
    let diags = check_err(
        "enum Wrap { Only(int) }\n\
         fn main() {\n\
             const Wrap w = Wrap.Only(1);\n\
             if let Only(x) = w { print(x); }\n\
         }\n",
    );
    assert_message(&diags, "this pattern matches every `Wrap`");
}

#[test]
fn a_single_variant_named_fields_pattern_is_irrefutable() {
    let diags = check_err(
        "enum Shape { Point { x: int, y: int } }\n\
         fn main() {\n\
             const Shape p = Shape.Point { x: 1, y: 2 };\n\
             if let Point { x: a, y: b } = p { print(a); print(b); }\n\
         }\n",
    );
    assert_message(&diags, "this pattern matches every `Shape`");
}

#[test]
fn a_two_variant_struct_pattern_is_refutable() {
    let hir = check_ok(
        "enum Shape { Point { x: int }, Empty }\n\
         fn main() {\n\
             const Shape p = Shape.Point { x: 1 };\n\
             if let Point { x: a } = p { print(a); }\n\
         }\n",
    );
    assert!(matches!(
        main_body(&hir).get(1),
        Some(HirStmt::IfLet { .. })
    ));
}

#[test]
fn a_while_let_that_always_matches_is_e0603() {
    let diags = check_err("fn main() {\n    while let x = 1 { print(x); }\n}\n");
    assert_message(
        &diags,
        "this pattern matches every `int`: `while let` would never end",
    );
}
