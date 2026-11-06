//! `if let` and `while let`: parsing the two forms.
//!
//! Both reuse the `switch` pattern grammar, so these tests check only what the
//! statement parser adds: the keyword sequence, where the pattern, value and
//! blocks land, and the span that covers the whole statement.

use yashy_common::{Diagnostic, SourceMap, Span};
use yashy_lexer::Lexer;
use yashy_parser::ast::{Expr, Item, Pattern, Program, Stmt};
use yashy_parser::Parser;

fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("if_let.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize()?;
    let mut parser = Parser::new(tokens, &sources, file);
    parser.parse_program()
}

fn parse_ok(src: &str) -> Program {
    match parse_src(src) {
        Ok(program) => program,
        Err(diags) => panic!("{src:?} must parse, got {diags:?}"),
    }
}

fn parse_err(src: &str) -> Vec<Diagnostic> {
    match parse_src(src) {
        Ok(program) => panic!("{src:?} must not parse, got {program:?}"),
        Err(diags) => diags,
    }
}

/// The first statement of `fn main`, together with the full source the span
/// indexes.
fn first_main_stmt(stmt: &str) -> (String, Stmt) {
    let source = format!("fn main() {{ {stmt} }}");
    let program = parse_ok(&source);
    let Item::Fn { body, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function item");
    };
    (
        source,
        body.stmts.into_iter().next().expect("main has a statement"),
    )
}

fn source_of(src: &str, span: Span) -> String {
    src[span.start as usize..span.end as usize].to_string()
}

#[test]
fn if_let_parses_a_variant_pattern_a_value_and_two_blocks() {
    let stmt = "if let Some(x) = opt { use(x); }";
    let (source, statement) = first_main_stmt(stmt);
    match statement {
        Stmt::IfLet {
            pattern,
            value,
            then_block,
            else_block,
            span,
        } => {
            assert!(
                matches!(
                    pattern,
                    Pattern::TupleVariant { ref path, ref args, .. }
                        if path == &["Some".to_string()]
                            && matches!(args.as_slice(), [Pattern::Binding { name, .. }] if name == "x")
                ),
                "{pattern:?}"
            );
            assert!(
                matches!(value, Expr::Ident(ref path, _) if path == &["opt".to_string()]),
                "{value:?}"
            );
            assert_eq!(then_block.stmts.len(), 1);
            assert!(else_block.is_none());
            assert_eq!(source_of(&source, span), stmt);
        }
        other => panic!("expected an `if let`, got {other:?}"),
    }
}

#[test]
fn if_let_keeps_the_else_block() {
    let stmt = "if let None = opt { a(); } else { b(); }";
    let (source, statement) = first_main_stmt(stmt);
    match statement {
        Stmt::IfLet {
            else_block, span, ..
        } => {
            assert_eq!(else_block.map(|block| block.stmts.len()), Some(1));
            assert_eq!(source_of(&source, span), stmt);
        }
        other => panic!("expected an `if let`, got {other:?}"),
    }
}

#[test]
fn if_let_accepts_a_struct_variant_pattern() {
    let stmt = "if let Point { x: a, y: b } = p { use(a); use(b); }";
    let (_, statement) = first_main_stmt(stmt);
    match statement {
        Stmt::IfLet { pattern, .. } => {
            assert!(
                matches!(
                    pattern,
                    Pattern::StructVariant { ref path, ref fields, .. }
                        if path == &["Point".to_string()] && fields.len() == 2
                ),
                "{pattern:?}"
            );
        }
        other => panic!("expected an `if let`, got {other:?}"),
    }
}

#[test]
fn an_ordinary_if_is_still_an_if() {
    let stmt = "if ready { go(); }";
    let (_, statement) = first_main_stmt(stmt);
    match statement {
        Stmt::If { cond, .. } => {
            assert!(
                matches!(cond, Expr::Ident(ref path, _) if path == &["ready".to_string()]),
                "{cond:?}"
            );
        }
        other => panic!("expected an `if`, got {other:?}"),
    }
}

#[test]
fn while_let_parses_a_variant_pattern_a_value_and_a_body() {
    let stmt = "while let Some(line) = next() { use(line); }";
    let (source, statement) = first_main_stmt(stmt);
    match statement {
        Stmt::WhileLet {
            pattern,
            value,
            body,
            label,
            span,
        } => {
            assert!(
                matches!(
                    pattern,
                    Pattern::TupleVariant { ref path, ref args, .. }
                        if path == &["Some".to_string()]
                            && matches!(args.as_slice(), [Pattern::Binding { name, .. }] if name == "line")
                ),
                "{pattern:?}"
            );
            assert!(matches!(value, Expr::Call { .. }), "{value:?}");
            assert!(label.is_none());
            assert_eq!(source_of(&source, span), stmt);
        }
        other => panic!("expected a `while let`, got {other:?}"),
    }
}

#[test]
fn while_let_keeps_an_invariant_at_the_head_of_the_body() {
    let stmt = "while let Some(x) = next() invariant count >= 0 { count = count + 1; }";
    let (_, statement) = first_main_stmt(stmt);
    match statement {
        Stmt::WhileLet { body, .. } => {
            assert!(
                matches!(body.stmts.first(), Some(Stmt::Invariant(_))),
                "{:?}",
                body.stmts
            );
        }
        other => panic!("expected a `while let`, got {other:?}"),
    }
}

#[test]
fn if_let_without_a_pattern_is_a_parse_error() {
    assert!(!parse_err("fn main() { if let = opt { } }").is_empty());
}

#[test]
fn while_let_without_a_value_is_a_parse_error() {
    assert!(!parse_err("fn main() { while let Some(x) { } }").is_empty());
}
