//! Import aliases: `import std.io as io;` parses and names the alias.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{Item, Program};
use yashy_parser::Parser;

fn parse_ok(src: &str) -> Program {
    let mut sources = SourceMap::new();
    let file = sources.add_file("alias.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("must lex");
    let mut parser = Parser::new(tokens, &sources, file);
    match parser.parse_program() {
        Ok(program) => program,
        Err(diags) => panic!("{src:?} must parse, got {diags:?}"),
    }
}

#[test]
fn an_import_may_name_an_alias() {
    let program = parse_ok("import std.io as io;\nfn main() {}\n");
    match program.items.as_slice() {
        [Item::Import { path, alias, .. }, Item::Fn { .. }] => {
            assert_eq!(path, &["std".to_string(), "io".to_string()]);
            assert_eq!(alias.as_deref(), Some("io"));
        }
        other => panic!("expected an aliased import, got {other:?}"),
    }
}

#[test]
fn an_import_without_an_alias_has_none() {
    let program = parse_ok("import lib.util;\nfn main() {}\n");
    match program.items.as_slice() {
        [Item::Import { path, alias, .. }, ..] => {
            assert_eq!(path, &["lib".to_string(), "util".to_string()]);
            assert_eq!(*alias, None);
        }
        other => panic!("expected an import, got {other:?}"),
    }
}

#[test]
fn a_qualified_call_parses_with_a_dotted_callee() {
    let program = parse_ok("fn main() { let int x = l.print(1); }\n");
    assert_eq!(program.items.len(), 1);
}
