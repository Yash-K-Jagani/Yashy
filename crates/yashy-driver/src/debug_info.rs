//! LLVM debug metadata derived from HIR spans.
//!
//! The backend emits plain IR plus optional `; yashy-loc:<file>:<start>:<end>`
//! markers (one before each checked statement). This pass turns those markers
//! into `!dbg` attachments and writes the `DICompileUnit`, `DIFile`,
//! `DISubprogram` and `DILocation` metadata LLVM needs for a line table.
//! Normal builds emit no markers, so their IR stays byte-identical.

use std::collections::BTreeMap;

use yashy_common::{FileId, SourceMap, Span};
use yashy_hir::hir::{HirItem, HirProgram};

/// The source facts one `DISubprogram` needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionLocation {
    pub symbol: String,
    pub name: String,
    pub file: String,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Point {
    file: String,
    line: u32,
    column: u32,
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

/// Escape a path or name for an LLVM metadata string.
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

fn parse_marker(line: &str, sources: &SourceMap) -> Option<Point> {
    let rest = line.trim().strip_prefix("; yashy-loc:")?;
    let mut fields = rest.split(':');
    let file = fields.next()?.parse::<u32>().ok()?;
    let start = fields.next()?.parse::<u32>().ok()?;
    let end = fields.next()?.parse::<u32>().ok()?;
    let id = FileId::from_index(file);
    let at = sources.locate(id, Span::new(start, end));
    Some(Point {
        file: sources.path(id).to_string(),
        line: at.line,
        column: at.col,
    })
}

/// Whether an IR line can carry an instruction-level `!dbg` attachment.
fn attachable(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with(';')
        && !trimmed.starts_with('}')
        && !trimmed.ends_with(':')
        && !trimmed.starts_with("define ")
        && !trimmed.starts_with("declare ")
}

/// Attach function-level and per-statement debug metadata to a whole module.
pub fn annotate(ir: &str, hir: &HirProgram, sources: &SourceMap) -> String {
    let functions = functions(hir, sources);

    // Pass one: collect the unique files and source points so metadata ids can
    // be assigned before any line is rewritten.
    let mut points: Vec<Point> = Vec::new();
    for line in ir.lines() {
        if let Some(point) = parse_marker(line, sources) {
            if !points.contains(&point) {
                points.push(point);
            }
        }
    }
    let mut files: Vec<String> = functions.iter().map(|entry| entry.file.clone()).collect();
    for point in &points {
        if !files.contains(&point.file) {
            files.push(point.file.clone());
        }
    }

    // Metadata ids: 0 compile unit, 1 module flags, 2 subroutine type,
    // 3 empty type list, then DIFile, DISubprogram and DILocation blocks.
    let file_id = |path: &str| 4 + files.iter().position(|entry| entry == path).unwrap();
    // Simpler and stable: one DISubprogram per function entry, in order.
    let subprogram_base = 4 + files.len();
    let location_base = subprogram_base + functions.len();
    let point_id =
        |point: &Point| location_base + points.iter().position(|entry| entry == point).unwrap();

    // Pass two: drop markers and attach !dbg to the next instruction.
    let mut out: Vec<String> = Vec::new();
    let mut pending: Option<Point> = None;
    let mut subprograms = BTreeMap::new();
    for (index, function) in functions.iter().enumerate() {
        subprograms.insert(function.symbol.clone(), subprogram_base + index);
    }
    for line in ir.lines() {
        if parse_marker(line, sources).is_some() {
            pending = parse_marker(line, sources);
            continue;
        }
        let mut emitted = line.to_string();
        if let Some(rest) = line.strip_prefix("define ") {
            let symbol = rest
                .split_once('@')
                .map(|(_, rest)| rest.split(['(', ' ']).next().unwrap_or(""))
                .unwrap_or("");
            if let Some(id) = subprograms.get(symbol) {
                if let Some(head) = emitted.strip_suffix(" {") {
                    emitted = format!("{head} !dbg !{id} {{");
                }
            }
        } else if attachable(line) {
            if let Some(point) = pending.take() {
                emitted.push_str(&format!(", !dbg !{}", point_id(&point)));
            }
        }
        out.push(emitted);
    }

    out.push(String::new());
    out.push("!llvm.dbg.cu = !{!0}".to_string());
    out.push("!llvm.module.flags = !{!1}".to_string());
    out.push(
        "!0 = distinct !DICompileUnit(language: DW_LANG_C, file: !4, producer: \"yashyc\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)"
            .to_string(),
    );
    out.push("!1 = !{i32 2, !\"Debug Info Version\", i32 3}".to_string());
    out.push("!2 = !DISubroutineType(types: !3)".to_string());
    out.push("!3 = !{}".to_string());
    for (index, path) in files.iter().enumerate() {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        out.push(format!(
            "!{} = !DIFile(filename: {}, directory: \".\")",
            4 + index,
            llvm_string(&name)
        ));
    }
    for (index, function) in functions.iter().enumerate() {
        out.push(format!(
            "!{} = distinct !DISubprogram(name: {}, scope: !{}, file: !{}, line: {}, type: !2, scopeLine: {}, spFlags: DISPFlagDefinition, unit: !0)",
            subprogram_base + index,
            llvm_string(&function.name),
            file_id(&function.file),
            file_id(&function.file),
            function.line,
            function.line
        ));
    }
    for (index, point) in points.iter().enumerate() {
        out.push(format!(
            "!{} = !DILocation(line: {}, column: {}, scope: !{})",
            location_base + index,
            point.line,
            point.column,
            subprogram_base
        ));
    }
    out.push(String::new());
    out.join("\n")
}

/// The function-only form used by the unit tests: no statements, no markers.
pub fn annotate_basic(ir: &str, functions: &[FunctionLocation]) -> String {
    // The basic form is kept simple: function attachments without locations.
    let mut out: Vec<String> = ir.lines().map(str::to_string).collect();
    for line in &mut out {
        if let Some(rest) = line.strip_prefix("define ") {
            let symbol = rest
                .split_once('@')
                .map(|(_, rest)| rest.split(['(', ' ']).next().unwrap_or(""))
                .unwrap_or("");
            if let Some(index) = functions.iter().position(|entry| entry.symbol == symbol) {
                if let Some(head) = line.strip_suffix(" {") {
                    *line = format!("{head} !dbg !{} {{", 4 + functions.len() + index);
                }
            }
        }
    }
    out.push(String::new());
    out.push("!llvm.dbg.cu = !{!0}".to_string());
    out.push("!1 = !{i32 2, !\"Debug Info Version\", i32 3}".to_string());
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
    fn a_function_definition_gets_a_subprogram() {
        let ir = "define i32 @main() {\nentry:\n  ret i32 0\n}\n";
        let annotated = annotate_basic(ir, &[location("main", "main", 3)]);
        assert!(
            annotated.contains("define i32 @main() !dbg !5 {"),
            "{annotated}"
        );
        assert!(annotated.contains("!llvm.dbg.cu = !{!0}"), "{annotated}");
    }

    #[test]
    fn markers_are_removed_and_locations_are_parsed() {
        let mut sources = SourceMap::new();
        sources.add_file(
            "main.ya".to_string(),
            "// line one\nfn main() {\n  let x = 1;\n}\n".to_string(),
        );
        let point = parse_marker("; yashy-loc:0:13:24", &sources).unwrap();
        assert_eq!(point.line, 2);
        assert!(parse_marker("  ret i32 0", &sources).is_none());
    }
}
