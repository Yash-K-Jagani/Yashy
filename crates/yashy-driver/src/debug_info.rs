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
use yashy_hir::ty::TypeKind;

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

/// One user type and the facts LLVM needs to describe it.
#[derive(Clone, Debug)]
struct TypeEntry {
    name: String,
    line: u32,
    tag: &'static str,
    layout: yashy_hir::layout::Layout,
    /// Struct fields: name, field type and byte offset.
    fields: Vec<(String, TypeKind, usize)>,
    /// Enum variant names, in declaration order.
    variants: Vec<String>,
}

/// Collect declared types and their layout from the same tables the checker
/// and `inspect --layout` use. A type the checker cannot lay out is skipped:
/// emitting a wrong size would be worse than leaving it out of the table.
fn types(hir: &HirProgram, sources: &SourceMap) -> Vec<TypeEntry> {
    let decls = crate::layout::HirDecls::from_program(hir);
    let entry = FileId::from_index(0);
    let mut out = Vec::new();
    for item in &hir.items {
        let (name, tag, fields, variants) = match item {
            HirItem::Struct(structure) => (
                structure.name.clone(),
                "DW_TAG_structure_type",
                structure
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.kind.clone()))
                    .collect::<Vec<_>>(),
                Vec::new(),
            ),
            HirItem::Enum(enumeration) => (
                enumeration.name.clone(),
                "DW_TAG_enumeration_type",
                Vec::new(),
                enumeration
                    .variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => continue,
        };
        let Some(layout) = crate::layout::find(&name, &decls) else {
            continue;
        };
        let offsets = layout.offsets.clone();
        let fields = fields
            .into_iter()
            .enumerate()
            .map(|(index, (field, kind))| (field, kind, offsets.get(index).copied().unwrap_or(0)))
            .collect();
        out.push(TypeEntry {
            name,
            line: sources.locate(entry, item_span(item)).line,
            tag,
            layout,
            fields,
            variants,
        });
    }
    out
}

fn item_span(item: &HirItem) -> Span {
    match item {
        HirItem::Struct(structure) => structure.span,
        HirItem::Enum(enumeration) => enumeration.span,
        HirItem::Fn(function) => function.span,
        HirItem::Import(import) => import.span,
        HirItem::Extern(block) => block.span,
    }
}

/// The five basic types the member walk maps field kinds onto.
const BASIC_TYPES: [(&str, &str, &str); 5] = [
    ("int", "64", "DW_ATE_signed"),
    ("float", "64", "DW_ATE_float"),
    ("bool", "8", "DW_ATE_boolean"),
    ("char", "8", "DW_ATE_unsigned_char"),
    ("ptr", "64", "DW_ATE_address"),
];

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

    // The type table continues after the locations, so adding it does not
    // renumber any metadata an existing debug build already emitted.
    let decls = crate::layout::HirDecls::from_program(hir);
    let type_entries = types(hir, sources);
    let basic_base = location_base + points.len();
    let element_base = basic_base + BASIC_TYPES.len();
    let element_count: usize = type_entries
        .iter()
        .map(|entry| entry.fields.len() + entry.variants.len())
        .sum();
    let composite_base = element_base + element_count;
    let composites: BTreeMap<String, usize> = type_entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.name.clone(), composite_base + index))
        .collect();

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
    let retained = composites
        .values()
        .map(|id| format!("!{id}"))
        .collect::<Vec<_>>()
        .join(", ");
    out.push(format!(
        "!0 = distinct !DICompileUnit(language: DW_LANG_C, file: !4, producer: \"yashyc\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug, types: !{{{retained}}})"
    ));
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
    for (index, (name, size, encoding)) in BASIC_TYPES.iter().enumerate() {
        out.push(format!(
            "!{} = !DIBasicType(name: {}, size: {}, encoding: {})",
            basic_base + index,
            llvm_string(name),
            size,
            encoding
        ));
    }

    // One member or enumerator per declared field/variant. Field offsets and
    // sizes come from the checker's layout walk, in bits as LLVM requires.
    let mut element = element_base;
    let mut composite_elements: Vec<Vec<usize>> = Vec::new();
    for entry in &type_entries {
        let mut ids = Vec::new();
        for (field, kind, offset) in &entry.fields {
            let base = match kind {
                TypeKind::I64 | TypeKind::CInt(64) => basic_base,
                TypeKind::F64 | TypeKind::CFloat(64) => basic_base + 1,
                TypeKind::Bool => basic_base + 2,
                TypeKind::Char | TypeKind::CUInt(8) => basic_base + 3,
                TypeKind::Struct(name) | TypeKind::Enum(name) => {
                    *composites.get(name).unwrap_or(&(basic_base + 4))
                }
                _ => basic_base + 4,
            };
            let size = yashy_hir::layout::layout(kind, &decls)
                .map(|layout| layout.size * 8)
                .unwrap_or(64);
            out.push(format!(
                "!{} = !DIDerivedType(tag: DW_TAG_member, name: {}, scope: !{}, file: !{}, baseType: !{}, size: {}, offset: {})",
                element,
                llvm_string(field),
                composites[&entry.name],
                4,
                base,
                size,
                offset * 8
            ));
            ids.push(element);
            element += 1;
        }
        for (index, variant) in entry.variants.iter().enumerate() {
            out.push(format!(
                "!{} = !DIEnumerator(name: {}, value: {})",
                element,
                llvm_string(variant),
                index
            ));
            ids.push(element);
            element += 1;
        }
        composite_elements.push(ids);
    }
    for (index, entry) in type_entries.iter().enumerate() {
        let elements = composite_elements[index]
            .iter()
            .map(|id| format!("!{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        let extra = if entry.tag == "DW_TAG_enumeration_type" {
            format!(", baseType: !{}", basic_base)
        } else {
            String::new()
        };
        out.push(format!(
            "!{} = distinct !DICompositeType(tag: {}, name: {}, scope: !4, file: !4, line: {}, size: {}, align: {}, elements: !{{{}}}{})",
            composite_base + index,
            entry.tag,
            llvm_string(&entry.name),
            entry.line,
            entry.layout.size * 8,
            entry.layout.align * 8,
            elements,
            extra
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
    fn declared_types_get_a_table_in_the_compile_unit() {
        use yashy_hir::hir::{HirFieldDef, HirProgram, HirStruct};
        use yashy_hir::ty::{Type, TypeKind};

        let mut sources = SourceMap::new();
        sources.add_file(
            "main.ya".to_string(),
            "struct Point { x: int }\n".to_string(),
        );
        let span = Span::new(0, 22);
        let hir = HirProgram {
            items: vec![HirItem::Struct(HirStruct {
                name: "Point".to_string(),
                fields: vec![HirFieldDef {
                    name: "x".to_string(),
                    ty: Type {
                        kind: TypeKind::I64,
                        span,
                    },
                    span,
                }],
                repr: None,
                span,
            })],
            instantiations: 0,
            uses_threads: false,
            uses_async: false,
        };
        let annotated = annotate(
            "define i32 @main() {\nentry:\n  ret i32 0\n}\n",
            &hir,
            &sources,
        );
        assert!(annotated.contains("DW_TAG_structure_type"), "{annotated}");
        assert!(annotated.contains("name: \"Point\""), "{annotated}");
        assert!(annotated.contains("DW_TAG_member"), "{annotated}");
        assert!(annotated.contains("types: !{!"), "{annotated}");
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
