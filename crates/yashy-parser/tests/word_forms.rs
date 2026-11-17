//! `and`, `or` and `not` are the word spellings of `&&`, `||` and `!`.
//! They produce the same binary/unary nodes; only the token kind differs.

use yashy_common::SourceMap;
use yashy_lexer::{Lexer, TokenKind};
use yashy_parser::ast::{Expr, Item, Stmt};
use yashy_parser::Parser;

fn body(src: &str) -> Vec<Stmt> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("words.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("must lex");
    let program = Parser::new(tokens, &sources, file)
        .parse_program()
        .expect("must parse");
    let Item::Fn { body, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function");
    };
    body.stmts
}

fn expr_of(statements: &[Stmt]) -> Expr {
    match &statements[0] {
        Stmt::Expr(expression, _) => expression.clone(),
        other => panic!("expected an expression, got {other:?}"),
    }
}

#[test]
fn word_forms_lex_to_their_own_keyword_kinds() {
    let tokens = Lexer::new("and or not").tokenize().expect("must lex");
    let kinds: Vec<TokenKind> = tokens.iter().map(|token| token.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::KwAnd,
            TokenKind::KwOr,
            TokenKind::KwNot,
            TokenKind::Eof
        ]
    );
}

#[test]
fn and_equals_symbolic_and() {
    let words = body("fn main() { a and b; }");
    let symbols = body("fn main() { a && b; }");
    assert_eq!(
        expr_of(&words),
        expr_of(&symbols),
        "the tree must not know which spelling was used"
    );
}

#[test]
fn or_equals_symbolic_or() {
    let words = body("fn main() { a or b; }");
    let symbols = body("fn main() { a || b; }");
    assert_eq!(expr_of(&words), expr_of(&symbols));
}

#[test]
fn not_equals_symbolic_not() {
    let words = body("fn main() { not a; }");
    let symbols = body("fn main() { !a; }");
    assert_eq!(expr_of(&words), expr_of(&symbols));
}

#[test]
fn word_forms_keep_symbolic_precedence() {
    // `a or b and c` is `a || (b && c)`, exactly as with the symbols.
    let words = body("fn main() { a or b and c; }");
    let symbols = body("fn main() { a || b && c; }");
    assert_eq!(expr_of(&words), expr_of(&symbols));
}

#[test]
fn is_and_is_not_are_their_own_nodes() {
    let yes = expr_of(&body("fn main() { x is Some; }"));
    let no = expr_of(&body("fn main() { x is not Some; }"));
    match (yes, no) {
        (Expr::Is { negate: false, .. }, Expr::Is { negate: true, .. }) => {}
        (a, b) => panic!("expected is nodes, got {a:?} and {b:?}"),
    }
}
