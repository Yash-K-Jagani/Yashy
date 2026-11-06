//! Labeled `break` / `continue` after checking: the flag lowering, `E0604`
//! for an unknown label, and `E0605` for a duplicated one.

use yashy_common::{Diagnostic, FileId, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram, HirStmt};

fn check_source(src: &str) -> (Result<HirProgram, Vec<Diagnostic>>, SourceMap, FileId) {
    let mut sm = SourceMap::new();
    let file = sm.add_file("labeled.ya", src.to_string());
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
        (Err(diags), _, _) => panic!(
            "expected {src:?} to type-check, got {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        ),
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

fn assert_code(diags: &[Diagnostic], code: &str) {
    assert!(
        diags.iter().any(|d| d.code.as_deref() == Some(code)),
        "expected a {code} diagnostic, got {:?}",
        diags.iter().map(|d| d.code.clone()).collect::<Vec<_>>()
    );
}

/// Flatten every statement under `stmt`, so a test can find the flag
/// declarations and checks wherever the lowering put them.
fn collect<'a>(stmt: &'a HirStmt, out: &mut Vec<&'a HirStmt>) {
    out.push(stmt);
    match stmt {
        HirStmt::If {
            init,
            then_block,
            else_block,
            ..
        } => {
            if let Some(init) = init {
                collect(init, out);
            }
            for s in &then_block.stmts {
                collect(s, out);
            }
            if let Some(else_block) = else_block {
                for s in &else_block.stmts {
                    collect(s, out);
                }
            }
        }
        HirStmt::ForRange { body, .. }
        | HirStmt::ForIn { body, .. }
        | HirStmt::ForC { body, .. } => {
            for s in &body.stmts {
                collect(s, out);
            }
        }
        _ => {}
    }
}

fn all_stmts(hir: &HirProgram) -> Vec<&HirStmt> {
    let mut out = Vec::new();
    for stmt in main_body(hir) {
        collect(stmt, &mut out);
    }
    out
}

#[test]
fn a_labeled_loop_declares_mutable_flags_and_checks_them() {
    let hir = check_ok(
        "fn main() {\n\
             outer: for i in 0..3 {\n\
                 for j in 0..3 {\n\
                     break outer;\n\
                 }\n\
             }\n\
         }\n",
    );
    let stmts = all_stmts(&hir);
    // Mutable `let`, never `const`: assigning a `const` flag would be E0400.
    assert!(
        stmts.iter().any(|s| matches!(
            s,
            HirStmt::Var { name, mutability, .. }
                if name == "__brk_outer"
                    && *mutability == yashy_sema::Mutability::Var
        )),
        "{stmts:?}"
    );
    assert!(
        stmts.iter().any(|s| matches!(
            s,
            HirStmt::Var { name, mutability, .. }
                if name == "__cont_outer"
                    && *mutability == yashy_sema::Mutability::Var
        )),
        "{stmts:?}"
    );
}

#[test]
fn a_labeled_jump_sets_the_flag_and_breaks() {
    let hir = check_ok(
        "fn main() {\n\
             outer: for i in 0..3 {\n\
                 for j in 0..3 {\n\
                     break outer;\n\
                 }\n\
             }\n\
         }\n",
    );
    let stmts = all_stmts(&hir);
    assert!(
        stmts.iter().any(|s| matches!(
            s,
            HirStmt::Assign { name, .. } if name == "__brk_outer"
        )),
        "{stmts:?}"
    );
    // The labeled loop itself exits through a real `break` (through the
    // flag check at the end of its body), never through new HIR.
    assert!(
        stmts.iter().any(|s| matches!(s, HirStmt::Break(_))),
        "{stmts:?}"
    );
}

#[test]
fn continue_on_the_innermost_loop_is_a_plain_continue() {
    // Depth 0: the label is on the loop the statement sits in, so the flag
    // form would exit it instead of re-testing it.
    let hir = check_ok(
        "fn main() {\n\
             outer: for i in 0..3 {\n\
                 continue outer;\n\
             }\n\
         }\n",
    );
    let stmts = all_stmts(&hir);
    assert!(
        stmts.iter().any(|s| matches!(s, HirStmt::Continue(_))),
        "{stmts:?}"
    );
}

#[test]
fn break_nope_is_e0604_and_names_the_labels_in_scope() {
    let diags = check_err(
        "fn main() {\n\
             outer: for i in 0..3 {\n\
                 for j in 0..3 {\n\
                     break middle;\n\
                 }\n\
             }\n\
         }\n",
    );
    assert_code(&diags, "E0604");
    assert_message(&diags, "no enclosing loop is labeled `middle`");
    assert_message(&diags, "`outer`");
}

#[test]
fn a_duplicate_nested_label_is_e0605() {
    let diags = check_err(
        "fn main() {\n\
             outer: for i in 0..3 {\n\
                 outer: for j in 0..3 {\n\
                     print(j);\n\
                 }\n\
             }\n\
         }\n",
    );
    assert_code(&diags, "E0605");
    assert_message(&diags, "the label `outer` is already in scope");
}

#[test]
fn an_unlabeled_break_still_lowers_to_a_plain_break() {
    let hir = check_ok(
        "fn main() {\n\
             for i in 0..3 {\n\
                 break;\n\
             }\n\
         }\n",
    );
    assert!(matches!(
        main_body(&hir).get(0),
        Some(HirStmt::ForRange { .. })
    ));
    let stmts = all_stmts(&hir);
    assert!(
        stmts.iter().any(|s| matches!(s, HirStmt::Break(_))),
        "{stmts:?}"
    );
    assert!(
        !stmts.iter().any(|s| matches!(
            s,
            HirStmt::Var { name, .. } if name.starts_with("__brk_") || name.starts_with("__cont_")
        )),
        "an unlabeled loop must not gain flags: {stmts:?}"
    );
}
