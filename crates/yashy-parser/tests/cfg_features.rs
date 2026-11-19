//! `cfg(feature = "name")` is source sugar for the flat `feature:name` key the
//! driver's cfg filter already reads. One key shape keeps the filter, cache
//! key and lockfile in agreement.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{CfgExpr, Item};
use yashy_parser::Parser;

fn parse(src: &str) -> Result<Item, Vec<yashy_common::Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("cfg_feature.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    Parser::new(tokens, &sources, file)
        .parse_program()
        .map(|program| program.items.into_iter().next().expect("one item"))
}

fn key(src: &str) -> String {
    let item = parse(src).expect("the clause must parse");
    let Item::Cfg { key, .. } = item else {
        panic!("expected a cfg item, got {item:?}");
    };
    let CfgExpr::Key(name, _) = key else {
        panic!("expected one key, got {key:?}");
    };
    name
}

#[test]
fn a_feature_becomes_the_flat_key() {
    assert_eq!(key("cfg(feature = \"net\") import net;"), "feature:net");
}

#[test]
fn a_feature_works_inside_combinators() {
    assert_eq!(
        key("cfg(all(windows, feature = \"tls\")) import tls;"),
        "all(windows, feature:tls)"
    );
    assert_eq!(
        key("cfg(any(feature = \"a\", debug)) import a;"),
        "any(feature:a, debug)"
    );
}

#[test]
fn a_feature_needs_a_string_literal() {
    assert!(
        parse("cfg(feature = tls) import tls;").is_err(),
        "a bare name is not a feature spelling"
    );
}

#[test]
fn an_empty_feature_name_is_refused() {
    assert!(parse("cfg(feature = \"\") import tls;").is_err());
}

#[test]
fn ordinary_keys_are_unchanged() {
    assert_eq!(key("cfg(windows) import win;"), "windows");
}
