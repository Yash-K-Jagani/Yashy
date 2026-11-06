//! Aliased imports: a qualified call through an alias resolves to the same
//! item as the unqualified one, with the same visibility rules.

use yashy_common::{Diagnostic, FileId, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_modules_with, CheckOptions, HirProgram};

fn check(modules: &[(&str, &str)]) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let mut parsed = Vec::new();
    for (name, text) in modules {
        let file = sources.add_file(*name, text.to_string());
        let tokens = Lexer::new(text)
            .tokenize()
            .expect("the lexer must not panic");
        let mut parser = Parser::new(tokens, &sources, file);
        let program = parser
            .parse_program()
            .expect("the parser must accept the program");
        parsed.push((file, program));
    }
    let modules: Vec<(FileId, &yashy_parser::ast::Program)> = parsed
        .iter()
        .map(|(file, program)| (*file, program))
        .collect();
    check_modules_with(&modules, &sources, CheckOptions::default())
}

#[test]
fn a_qualified_call_through_an_alias_resolves() {
    check(&[
        (
            "main.ya",
            "import lib as l;\nfn main() { print(l.seven()); }\n",
        ),
        ("lib.ya", "pub fn seven() -> int { return 7; }\n"),
    ])
    .expect("`l.seven()` must resolve through the alias");
}

#[test]
fn a_qualified_call_to_a_private_item_is_e0801() {
    match check(&[
        (
            "main.ya",
            "import lib as l;\nfn main() { print(l.seven()); }\n",
        ),
        ("lib.ya", "fn seven() -> int { return 7; }\n"),
    ]) {
        Ok(_) => panic!("expected E0801"),
        Err(diags) => assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("E0801")),
            "expected E0801, got {diags:?}"
        ),
    }
}

#[test]
fn a_qualified_call_naming_a_missing_item_is_e0001() {
    match check(&[
        (
            "main.ya",
            "import lib as l;\nfn main() { print(l.nine()); }\n",
        ),
        ("lib.ya", "pub fn seven() -> int { return 7; }\n"),
    ]) {
        Ok(_) => panic!("expected an unknown-name diagnostic"),
        Err(diags) => assert!(
            diags.iter().any(|d| d.message.contains("nine")),
            "must name the missing item, got {diags:?}"
        ),
    }
}
