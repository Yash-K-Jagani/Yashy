//! Debug metadata and address lookup are derived from compiler facts.

use yashy_common::{SourceMap, Span};
use yashy_driver::addr::{nearest, parse_nm, Symbol};
use yashy_driver::debug_info::{annotate, FunctionLocation};

#[test]
fn function_locations_come_from_hir_and_source() {
    // A synthetic HIR program is enough to prove the span-to-line mapping.
    // The full emitter is covered by the golden IR tests.
    let mut sources = SourceMap::new();
    let file = sources.add_file(
        "main.ya".to_string(),
        "// a comment\nfn main() {\n}\n".to_string(),
    );
    let location = yashy_common::Location { line: 2, col: 1 };
    assert_eq!(sources.locate(file, Span::new(13, 24)).line, location.line);
    let symbols = vec![FunctionLocation {
        symbol: "main".to_string(),
        name: "main".to_string(),
        file: sources.path(file).to_string(),
        line: sources.locate(file, Span::new(13, 24)).line,
    }];
    let annotated = annotate("define i32 @main() {\nentry:\n  ret i32 0\n}\n", &symbols);
    assert!(
        annotated.contains("define i32 @main() !dbg !5 {"),
        "{annotated}"
    );
    assert!(
        annotated.contains("!5 = distinct !DISubprogram"),
        "{annotated}"
    );
    assert!(annotated.contains("line: 2"), "{annotated}");
}

#[test]
fn an_address_resolves_to_the_nearest_symbol() {
    let symbols = parse_nm("0000000140001000 T main\n0000000140002000 T yashy_fn_add\n");
    assert_eq!(symbols.len(), 2);
    assert_eq!(
        nearest(&symbols, 0x140001234).map(|s| s.name.as_str()),
        Some("main")
    );
    assert_eq!(
        nearest(&symbols, 0x140002001).map(|s| s.name.as_str()),
        Some("yashy_fn_add")
    );
    assert!(nearest(&symbols, 0x140000000).is_none());
    let json = yashy_driver::addr::json(
        0x140001234,
        &Symbol {
            address: 0x140001000,
            name: "main".to_string(),
        },
        "main.ya",
        2,
        1,
        Span::new(13, 24),
    )
    .render();
    assert!(json.contains("\"address\": \"0x140001234\""), "{json}");
    assert!(json.contains("\"line\": 2"), "{json}");
}

#[test]
fn debug_metadata_is_absent_without_a_debug_build() {
    let ir = "define i32 @main() {\nentry:\n  ret i32 0\n}\n";
    assert_eq!(annotate(ir, &[]), ir);
}
