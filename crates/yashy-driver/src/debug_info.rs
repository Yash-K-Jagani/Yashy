//! LLVM debug metadata derived from HIR spans.
//!
//! The backend emits plain IR with no `!dbg` attachments. This pass adds the
//! smallest honest subset: one `DISubprogram` and one `DILocation` per named
//! user function, at the function's source line. A linker that consumes the IR
//! then produces a real line table with function entries. Statement-level
//! locations are not invented here: doing that needs a location attached to
//! every instruction during emission, which is a different change.

use std::collections::BTreeMap;
use std::path::Path;

use yashy_common::SourceMap;
use yashy_hir::hir::{HirItem, HirProgram};

/// The source facts one `DISubprogram` needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionLocation {
    pub symbol: String,
    pub name: String,
    pub file: String,
    pub line: u32,
}

/// Collect one location per named user function in source order.
pub fn functions(hir: &HirProgram, sources: &SourceMap) -> Vec<FunctionLocation> {
    let mut out = Vec::new();
    for item in &hir.items {
        let HirItem::Fn(function) = item else {
            continue;
        };
        if function.name.contains('$') || function.is_async {
            continue;
        }
        let symbol = yashy_codegen::ir::fn_symbol(&function.name);
        let line = sources.locate(function.file, function.span).line;
        out.push(FunctionLocation {
            symbol,
            name: function.name.clone(),
            file: sources.path(function.file).to_string(),
            line,
        });
    }
    out
}

/// Escape a path for an LLVM metadata string.
fn llvm_string(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\0A"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Attach function-level debug metadata to a whole IR module.
pub fn annotate(ir: &str, functions: &[FunctionLocation]) -> String {
    if functions.is_empty() {
        return ir.to_string();
    }
    // Metadata ids: 0 = compile unit, 1 = module flags, 2 = DIFile,
    // 3 = DISubroutineType, 4 = empty type list, then one DISubprogram and one
    // DILocation per function.
    let mut out: Vec<String> = Vec::with_capacity(ir.lines().count() + functions.len() * 2 + 10);
    let by_symbol: BTreeMap<&str, (usize, &FunctionLocation)> = functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.symbol.as_str(), (index, function)))
        .collect();
    for line in ir.lines() {
        let mut emitted = line.to_string();
        if let Some(rest) = line.strip_prefix("define ") {
            if let Some(symbol) = rest
                .split_once("@")
                .map(|(_, rest)| rest.split(['(', ' ']).next().unwrap_or(""))
            {
                if let Some((index, _)) = by_symbol.get(symbol) {
                    let id = 5 + index * 2;
                    if let Some(head) = emitted.strip_suffix(" {") {
                        emitted = format!("{head} !dbg !{id} {{");
                    }
                }
            }
        }
        out.push(emitted);
    }
    out.push(String::new());
    out.push("!llvm.dbg.cu = !{!0}".to_string());
    out.push("!llvm.module.flags = !{!1}".to_string());
    out.push(
        "!0 = distinct !DICompileUnit(language: DW_LANG_C, file: !2, producer: \"yashyc\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)"
            .to_string(),
    );
    out.push("!1 = !{i32 2, !\"Debug Info Version\", i32 3}".to_string());
    let file_name = functions
        .first()
        .map(|function| {
            Path::new(&function.file)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| function.file.clone())
        })
        .unwrap_or_else(|| "main.ya".to_string());
    out.push(format!(
        "!2 = !DIFile(filename: {}, directory: \".\")",
        llvm_string(&file_name)
    ));
    out.push("!3 = !DISubroutineType(types: !4)".to_string());
    out.push("!4 = !{}".to_string());
    for (index, function) in functions.iter().enumerate() {
        let program = 5 + index * 2;
        let location = program + 1;
        out.push(format!(
            "!{program} = distinct !DISubprogram(name: {}, scope: !2, file: !2, line: {}, type: !3, scopeLine: {}, spFlags: DISPFlagDefinition, unit: !0)",
            llvm_string(&function.name),
            function.line,
            function.line
        ));
        out.push(format!(
            "!{location} = !DILocation(line: {}, column: 1, scope: !{program})",
            function.line
        ));
    }
    out.push(String::new());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(symbol: &str, name: &str, line: u32) -> FunctionLocation {
        FunctionLocation {
            symbol: symbol.to_string(),
            name: name.to_string(),
            file: "main.ya".to_string(),
            line,
        }
    }

    #[test]
    fn a_function_definition_gets_a_subprogram_and_location() {
        let ir = "define i32 @main() {\nentry:\n  ret i32 0\n}\n";
        let annotated = annotate(ir, &[location("main", "main", 3)]);
        assert!(
            annotated.contains("define i32 @main() !dbg !5 {"),
            "{annotated}"
        );
        assert!(annotated.contains("!llvm.dbg.cu = !{!0}"), "{annotated}");
        assert!(
            annotated.contains("!5 = distinct !DISubprogram"),
            "{annotated}"
        );
        assert!(
            annotated.contains("!6 = !DILocation(line: 3"),
            "{annotated}"
        );
        assert!(annotated.contains("filename: \"main.ya\""), "{annotated}");
    }

    #[test]
    fn without_functions_the_ir_is_unchanged() {
        let ir = "define i32 @main() {\nentry:\n  ret i32 0\n}\n";
        assert_eq!(annotate(ir, &[]), ir);
    }
}
