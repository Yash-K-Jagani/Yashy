//! Tuple syntax: `(A, B)` in type position, `(a, b)` in value position,
//! `const (a, b) = pair;` in statement position, and `if guard` after a
//! `switch` pattern.
//!
//! The one ambiguity in the whole feature is the parenthesis: `(1)` groups and
//! `(1, 2)` builds a tuple, so the comma is the only thing that decides which
//! node comes out. These tests pin that, and pin the fact that the pattern of a
//! destructuring statement is a list of names -- not an expression that happens
//! to look like one.

use yashy_common::{Diagnostic, SourceMap, Span};
use yashy_lexer::Lexer;
use yashy_parser::ast::{
    BindingKeyword, Expr, Item, Literal, Pattern, Program, Stmt, SwitchArm, TypeExpr,
};
use yashy_parser::Parser;

// -- helpers ---------------------------------------------------------------

/// Parses `src`, having lexed it, and returns whatever the parser decided.
fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut map = SourceMap::new();
    let file = map.add_file("main.ya", src.to_string());
    let tokens = match Lexer::with_file(src, file).tokenize() {
        Ok(tokens) => tokens,
        Err(diags) => panic!("test source must lex, got {diags:?}"),
    };
    Parser::new(tokens, &map, file).parse_program()
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
        Err(diags) => {
            assert!(
                !diags.is_empty(),
                "{src:?} must report at least one diagnostic"
            );
            diags
        }
    }
}

fn all_messages(diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| format!("[{}] {}", d.severity.as_str(), d.message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The span of `text` inside `src`, computed rather than written down.
fn span_of(src: &str, text: &str) -> Span {
    let start = match src.find(text) {
        Some(start) => start,
        None => panic!("the fixture must contain {text:?}"),
    };
    Span::new(
        u32::try_from(start).unwrap(),
        u32::try_from(start + text.len()).unwrap(),
    )
}

/// The first statement of the first function in `src`.
fn first_stmt(src: &str) -> Stmt {
    let program = parse_ok(src);
    let item = program.items.into_iter().next().expect("one item");
    let Item::Fn { body, .. } = item else {
        panic!("expected a function");
    };
    body.stmts.into_iter().next().expect("one statement")
}

/// The arms of the first `switch` in `src`, in source order.
fn switch_arms(src: &str) -> Vec<SwitchArm> {
    match first_stmt(src) {
        Stmt::Switch { arms, .. } => arms,
        other => panic!("expected a switch statement, got {other:?}"),
    }
}

// -- tuple literals --------------------------------------------------------

#[test]
fn a_comma_is_what_makes_a_tuple_and_grouping_stays_grouping() {
    // `(1)` is the integer: parentheses group, they do not build.
    match first_stmt("fn main() { const a = (1); }") {
        Stmt::Const { init, .. } => assert!(
            matches!(init, Expr::Literal(Literal::I64(1), _)),
            "`(1)` groups, so it is the literal itself: {init:?}"
        ),
        other => panic!("expected a const, got {other:?}"),
    }

    // Two elements, and the order they were written in is the order they are
    // stored in -- position is part of a tuple's type.
    match first_stmt("fn main() { const b = (1, 2); }") {
        Stmt::Const { init, .. } => match init {
            Expr::TupleLit { elements, .. } => {
                assert_eq!(elements.len(), 2, "{elements:?}");
                assert!(
                    matches!(&elements[0], Expr::Literal(Literal::I64(1), _)),
                    "{elements:?}"
                );
                assert!(
                    matches!(&elements[1], Expr::Literal(Literal::I64(2), _)),
                    "{elements:?}"
                );
            }
            other => panic!("`(1, 2)` is a tuple literal, got {other:?}"),
        },
        other => panic!("expected a const, got {other:?}"),
    }

    // A trailing comma is the one-element spelling: `(1,)` cannot be grouping,
    // because the comma has already said it is a tuple.
    match first_stmt("fn main() { const c = (1,); }") {
        Stmt::Const { init, .. } => match init {
            Expr::TupleLit { elements, .. } => {
                assert_eq!(elements.len(), 1, "a trailing comma is not an element: {elements:?}")
            }
            other => panic!("`(1,)` is a one-element tuple, got {other:?}"),
        },
        other => panic!("expected a const, got {other:?}"),
    }

    // Nested: the inner parentheses build their own tuple, and the outer one
    // holds it as an element.
    let src = "fn main() { const d = (1, (2, 3)); }";
    match first_stmt(src) {
        Stmt::Const { init, .. } => match init {
            Expr::TupleLit { elements, span } => {
                assert_eq!(elements.len(), 2, "{elements:?}");
                assert_eq!(span, span_of(src, "(1, (2, 3))"), "the whole literal");
                assert!(
                    matches!(&elements[1], Expr::TupleLit { elements, .. } if elements.len() == 2),
                    "the second element is its own tuple: {elements:?}"
                );
            }
            other => panic!("expected a tuple, got {other:?}"),
        },
        other => panic!("expected a const, got {other:?}"),
    }

    // An empty `()` is not an expression, so it is a diagnostic rather than a
    // zero-element tuple value.
    let message = all_messages(&parse_err("fn main() { const e = (); }"));
    assert!(message.contains("expected an expression"), "{message}");
}

// -- destructuring ---------------------------------------------------------

#[test]
fn destructuring_names_each_element_in_order() {
    match first_stmt("fn main() { const (a, _, c) = pair; }") {
        Stmt::Destructure {
            kind,
            names,
            init,
            span,
        } => {
            assert_eq!(kind, BindingKeyword::Const, "the keyword is kept as written");
            assert_eq!(
                names,
                vec![Some("a".to_string()), None, Some("c".to_string())],
                "`_` is a hole, not a binding named `_`"
            );
            assert!(
                matches!(init, Expr::Ident(ref path, _) if path == &vec!["pair".to_string()]),
                "{init:?}"
            );
            let src = "fn main() { const (a, _, c) = pair; }";
            assert_eq!(span, span_of(src, "const (a, _, c) = pair;"), "the whole statement");
        }
        other => panic!("expected a destructuring statement, got {other:?}"),
    }
}

#[test]
fn all_three_binding_keywords_can_destructure() {
    for (source, expected) in [
        ("fn main() { var (a, b) = pair; }", BindingKeyword::Var),
        ("fn main() { let (a, b) = pair; }", BindingKeyword::Let),
    ] {
        match first_stmt(source) {
            Stmt::Destructure { kind, names, .. } => {
                assert_eq!(kind, expected, "{source}");
                assert_eq!(names.len(), 2, "{source}");
            }
            other => panic!("{source}: expected a destructuring statement, got {other:?}"),
        }
    }
}

#[test]
fn a_destructuring_pattern_accepts_a_trailing_comma_and_nothing_else() {
    // A trailing comma before `)` is a separator, not a name.
    match first_stmt("fn main() { const (a, b,) = pair; }") {
        Stmt::Destructure { names, .. } => assert_eq!(names.len(), 2, "{names:?}"),
        other => panic!("expected a destructuring statement, got {other:?}"),
    }

    // An element position is a name or `_`. A literal there is a mistake worth
    // naming, not something the parser should try to read as an expression.
    let message = all_messages(&parse_err("fn main() { const (1, b) = pair; }"));
    assert!(message.contains("list of names"), "{message}");

    // A pattern with no elements has no tuple to unpack: `()` is the unit
    // type, so this is a diagnostic rather than a zero-binding statement.
    let message = all_messages(&parse_err("fn main() { const () = pair; }"));
    assert!(message.contains("at least one name"), "{message}");
}

// -- guards ----------------------------------------------------------------

#[test]
fn a_guard_is_parsed_after_the_pattern_in_a_switch_statement() {
    let arms = switch_arms(
        "fn main() { switch n { case 1 if n > 0 { 1 } case _ { 0 } } }",
    );
    assert_eq!(arms.len(), 2);
    match &arms[0].guard {
        Some(Expr::Binary { op, .. }) => {
            assert_eq!(*op, yashy_parser::ast::BinOp::Gt, "the guard is the comparison");
        }
        other => panic!("the first arm must carry a guard, got {other:?}"),
    }
    assert!(
        matches!(&arms[0].pattern, Pattern::Literal(Literal::I64(1), _)),
        "the guard is not folded into the pattern: {:?}",
        arms[0].pattern
    );
    assert_eq!(arms[1].guard, None, "no `if`, no guard");
}

#[test]
fn a_guard_is_parsed_in_a_switch_expression_too() {
    let src = "fn f(n: i64) -> i64 { return switch n { case 1 if n > 0 { 1 } default { 0 } }; }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected a function");
    };
    let Stmt::Return(Some(Expr::SwitchExpr { arms, .. }), _) = &body.stmts[0] else {
        panic!("expected a returned switch expression: {:?}", body.stmts[0]);
    };
    assert_eq!(arms.len(), 2);
    assert!(arms[0].guard.is_some(), "the first arm has a guard");
    assert_eq!(arms[1].guard, None, "`default` has none");
    assert!(arms[1].is_default, "`default` is still marked as the catch-all");
}

#[test]
fn a_guard_keeps_the_span_it_was_parsed_from() {
    let src = "fn main() { switch n { case 1 if n > 0 { 1 } } }";
    let arms = switch_arms(src);
    let Some(guard) = &arms[0].guard else {
        panic!("expected a guard");
    };
    assert_eq!(
        yashy_parser::exprs::expr_span(guard),
        span_of(src, "n > 0"),
        "a diagnostic about the guard points at the guard"
    );
    // The arm's span still runs from the pattern to the body, so the guard sits
    // inside the text the arm owns.
    assert!(arms[0].span.start <= span_of(src, "n > 0").start);
    assert!(arms[0].span.end >= span_of(src, "n > 0").end);
}

// -- tuple types -----------------------------------------------------------

#[test]
fn a_tuple_type_is_written_in_parentheses() {
    let program = parse_ok("fn f(p: (i64, bool)) -> (i64, bool) {}");
    let Item::Fn { params, ret, .. } = &program.items[0] else {
        panic!("expected a function");
    };
    match &params[0].ty {
        TypeExpr::Tuple(elements, _) => {
            assert_eq!(elements.len(), 2, "{elements:?}");
            assert!(
                matches!(&elements[0], TypeExpr::Name(name, _) if name == "i64"),
                "{elements:?}"
            );
            assert!(
                matches!(&elements[1], TypeExpr::Name(name, _) if name == "bool"),
                "{elements:?}"
            );
        }
        other => panic!("expected a tuple type, got {other:?}"),
    }
    assert!(
        matches!(ret, Some(TypeExpr::Tuple(elements, _)) if elements.len() == 2),
        "the return type is a tuple too: {ret:?}"
    );
}
