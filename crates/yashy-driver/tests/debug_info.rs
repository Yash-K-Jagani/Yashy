//! Debug metadata and address lookup are derived from compiler facts.

use yashy_common::{SourceMap, Span};
use yashy_driver::addr::{nearest, parse_nm, Symbol};
use yashy_driver::debug_info::{annotate_basic, FunctionLocation};

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
    let annotated = annotate_basic("define i32 @main() {\nentry:\n  ret i32 0\n}\n", &symbols);
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
    assert_eq!(annotate_basic(ir, &[]), ir);
}

mod common;

use std::process::Command;

const DEBUG_PROGRAM: &str = concat!(
    "struct Point { x: int }\n",
    "\n",
    "fn total(int n) -> int {\n",
    "    let name = \"ada\";\n",
    "    let p = Point { x: n };\n",
    "    if n > 0 {\n",
    "        let inner = n + 1;\n",
    "        print(inner);\n",
    "    }\n",
    "    return p.x + n;\n",
    "}\n",
    "\n",
    "fn main() {\n",
    "    let answer = total(2);\n",
    "    print(\"done\");\n",
    "}\n",
    "\n",
    "async fn later() -> int {\n",
    "    return 7;\n",
    "}\n",
);

#[test]
fn a_debug_ir_dump_carries_locals_and_types_without_a_toolchain() {
    // `--debug-info` with `--emit-ir` stops before the backend, so this reads
    // the exact metadata a debugger will consume without needing clang, a
    // linker, or LLDB. It is also the only place the *shape* of that metadata
    // is pinned: a field LLVM does not accept (an earlier `types:` on the
    // compile unit) fails to compile a debug build and no unit test noticed.
    let dir = common::work_dir("debug-ir");
    common::write(&dir, "main.ya", DEBUG_PROGRAM);
    let output = Command::new(common::yashyc())
        .arg("build")
        .arg(dir.join("main.ya"))
        .arg("--emit-ir")
        .arg("--debug-info")
        .output()
        .expect("the yashyc CLI must be runnable");
    assert!(
        output.status.success(),
        "a debug IR dump failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ir = String::from_utf8_lossy(&output.stdout).into_owned();

    // The compile unit lists the declared types under the field LLVM defines.
    assert!(ir.contains("retainedTypes: !{"), "{ir}");
    assert!(
        !ir.contains("emissionKind: FullDebug, types:"),
        "`types` is not a DICompileUnit field:\n{ir}"
    );
    assert!(
        ir.contains("DW_TAG_structure_type, name: \"Point\""),
        "{ir}"
    );

    // A parameter and a `let` are declared variables with a debug location,
    // and the statement markers that carried them are gone.
    assert!(ir.contains("!DILocalVariable(name: \"n\", arg: 1,"), "{ir}");
    assert!(ir.contains("!DILocalVariable(name: \"answer\","), "{ir}");
    assert!(ir.contains("call void @llvm.dbg.declare("), "{ir}");
    assert!(
        ir.contains("declare void @llvm.dbg.declare(metadata, metadata, metadata)"),
        "{ir}"
    );
    assert!(!ir.contains("; yashy-loc:"), "a marker survived:\n{ir}");
    assert!(!ir.contains("; yashy-var:"), "a marker survived:\n{ir}");

    // A `string` is described by the runtime header behind its pointer, which
    // is what the shipped LLDB summary reads.
    assert!(ir.contains("name: \"yashy_str\""), "{ir}");
    assert!(ir.contains("DW_OP_deref"), "{ir}");
    assert!(ir.contains("name: \"data\""), "{ir}");

    // Every declared variable carries a location: LLVM's record form drops a
    // declaration with a null location, and with it the function's variables.
    let declares: Vec<&str> = ir
        .lines()
        .filter(|line| line.contains("call void @llvm.dbg.declare("))
        .collect();
    assert!(declares.len() >= 4, "expected one per binding:\n{ir}");
    for line in declares {
        assert!(
            line.contains("), !dbg !"),
            "a declare without a location is discarded: {line}"
        );
    }

    // Both ordinary functions and the async body's poll get a subprogram.
    // The async body is where the statements live; its factory only allocates
    // a frame, so giving the factory the subprogram would orphan every marker.
    assert_eq!(ir.matches("= distinct !DISubprogram(").count(), 3, "{ir}");
    assert!(
        ir.contains("@later.poll"),
        "the poll function is emitted: {ir}"
    );
    let later = ir
        .lines()
        .find(|line| line.contains("= distinct !DISubprogram(name: \"later\""))
        .expect("the async body has a subprogram");
    let later_id = later.trim_start_matches('!').split(' ').next().unwrap();
    assert!(
        ir.lines()
            .any(|line| line.contains("!DILocation(")
                && line.contains(&format!("scope: !{later_id}"))),
        "the async body's statement is scoped to its poll: {ir}"
    );

    // A `let` inside `if { ... }` is scoped to that lexical block, carries the
    // line it is declared on, and its `declare` sits at that line's location.
    let block_ids: Vec<&str> = ir
        .lines()
        .filter(|line| line.contains("= distinct !DILexicalBlock("))
        .map(|line| line.trim_start_matches('!').split(' ').next().unwrap())
        .collect();
    assert!(!block_ids.is_empty(), "no lexical block was emitted:\n{ir}");
    let inner = ir
        .lines()
        .find(|line| line.contains("!DILocalVariable(name: \"inner\""))
        .expect("the nested binding must have a variable");
    let scope = inner
        .split("scope: !")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .expect("the variable names a scope");
    assert!(
        block_ids.contains(&scope),
        "the nested binding is scoped to its block, not the function: {inner}"
    );
    assert!(
        !inner.contains("line: 0"),
        "a declared line is known: {inner}"
    );
    assert!(
        ir.lines().any(|line| line.contains("!DILocation(")
            && line.contains(&format!("scope: !{scope}"))),
        "the block owns at least one location:\n{ir}"
    );
}
