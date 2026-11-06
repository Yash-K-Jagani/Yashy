//! Doc comments (`///`): parsing and storing them on items, and `E2100` when
//! no item follows.
//!
//! The text is stored, not interpreted: a doc block above an item becomes
//! `Item::Documented` carrying the lines with `/// ` stripped.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{Item, Program};
use yashy_parser::Parser;

fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("doc.ya", src.to_string());
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

#[test]
fn a_doc_block_above_a_function_is_stored() {
    let program = parse_ok("/// Adds two numbers.\nfn add(int a, int b) -> int { return a + b; }\n");
    match program.items.as_slice() {
        [Item::Documented { docs, item, .. }] => {
            assert_eq!(docs, &["Adds two numbers.".to_string()]);
            assert!(matches!(**item, Item::Fn { .. }));
        }
        other => panic!("expected one documented item, got {other:?}"),
    }
}

#[test]
fn consecutive_doc_lines_form_one_block() {
    let program = parse_ok("/// First line.\n///Second line has no space.\n///\nfn f() {}\n");
    match program.items.as_slice() {
        [Item::Documented { docs, .. }] => {
            assert_eq!(
                docs,
                &[
                    "First line.".to_string(),
                    "Second line has no space.".to_string(),
                    "".to_string()
                ]
            );
        }
        other => panic!("expected one documented item, got {other:?}"),
    }
}

#[test]
fn a_doc_block_above_pub_fn_attaches_and_nests() {
    let program = parse_ok("/// Docs.\npub fn f() {}\n");
    match program.items.as_slice() {
        [Item::Documented { docs, item, .. }] => {
            assert_eq!(docs, &["Docs.".to_string()]);
            assert!(matches!(**item, Item::Public { .. }));
        }
        other => panic!("expected a documented `pub` item, got {other:?}"),
    }
}

#[test]
fn a_doc_block_above_a_method_attaches() {
    let program = parse_ok(
        "struct P { x: int }\nimpl P { /// Gets x.\n fn get(self) -> int { return self.x; } }\n",
    );
    match program.items.as_slice() {
        [Item::Struct { .. }, Item::Impl { methods, .. }] => match methods.as_slice() {
            [Item::Documented { docs, .. }] => assert_eq!(docs, &["Gets x.".to_string()]),
            other => panic!("expected a documented method, got {other:?}"),
        },
        other => panic!("expected a struct and an impl, got {other:?}"),
    }
}

#[test]
fn two_blocks_separated_by_a_blank_line_attach_only_the_second() {
    let program = parse_ok("/// First block.\n\n/// Second block.\nfn f() {}\n");
    match program.items.as_slice() {
        [Item::Documented { docs, .. }] => assert_eq!(docs, &["Second block.".to_string()]),
        other => panic!("expected one documented item, got {other:?}"),
    }
}

#[test]
fn four_slashes_is_an_ordinary_comment_and_does_not_attach() {
    let program = parse_ok("//// Not a doc comment.\nfn f() {}\n");
    assert!(matches!(program.items.as_slice(), [Item::Fn { .. }]));
}

#[test]
fn a_doc_comment_before_a_statement_is_e2100() {
    let diags = parse_err("fn main() {\n    /// Documents nothing.\n    let int x = 1;\n}\n");
    assert!(
        diags.iter().any(|d| d.code.as_deref() == Some("E2100")),
        "expected E2100, got {diags:?}"
    );
}

#[test]
fn a_doc_comment_at_the_end_of_the_file_is_e2100() {
    let diags = parse_err("fn main() {}\n/// Trailing docs.\n");
    assert!(
        diags.iter().any(|d| d.code.as_deref() == Some("E2100")),
        "expected E2100, got {diags:?}"
    );
}

#[test]
fn an_ordinary_comment_still_parses_as_nothing() {
    let program = parse_ok("// Just a comment.\nfn main() { let int x = 1; print(x); }\n");
    assert!(matches!(program.items.as_slice(), [Item::Fn { .. }]));
}
