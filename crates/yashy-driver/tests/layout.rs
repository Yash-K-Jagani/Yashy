//! The compiler must have one layout table, not three.
//!
//! `sizeof`/`alignof`/`offsetof` are folded by the checker from the HIR layout
//! walk, and `yashyc inspect --layout` renders the same walk through
//! `HirDecls`. This test reads both answers for the same source and fails if
//! they disagree, which is the compiler bug the representation plan warns
//! about.

use yashy_common::SourceMap;
use yashy_hir::hir::{HirExpr, HirItem, HirProgram, HirStmt, Literal};
use yashy_hir::layout::layout;
use yashy_hir::ty::TypeKind;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

fn check(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("layout.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the declaration");
    check_program(&program, &sources, file).expect("the struct and its queries must check")
}

fn const_int(program: &HirProgram, name: &str) -> i64 {
    for item in &program.items {
        let HirItem::Fn(function) = item else {
            continue;
        };
        for stmt in &function.body.stmts {
            let HirStmt::Const {
                name: bound,
                init: HirExpr::Literal(Literal::I64(value), _, _),
                ..
            } = stmt
            else {
                continue;
            };
            if bound == name {
                return *value;
            }
        }
    }
    panic!("no `const {name} = <integer>` in the checked program");
}

#[test]
fn folded_queries_agree_with_the_layout_table() {
    let src = "repr(C) struct Pair { a: u8, b: i32 }\n\
               fn main() {\n\
                   const size = sizeof(Pair);\n\
                   const align = alignof(Pair);\n\
                   const first = offsetof(Pair, a);\n\
                   const second = offsetof(Pair, b);\n\
               }";
    let program = check(src);
    let decls = yashy_driver::layout::HirDecls::from_program(&program);
    let expected = layout(&TypeKind::Struct("Pair".to_string()), &decls)
        .expect("the declared struct has a layout");

    assert_eq!(const_int(&program, "size"), expected.size as i64);
    assert_eq!(const_int(&program, "align"), expected.align as i64);
    assert_eq!(const_int(&program, "first"), expected.offsets[0] as i64);
    assert_eq!(const_int(&program, "second"), expected.offsets[1] as i64);
    // Pin the C rule too, so a shared wrong walk cannot pass by agreeing with
    // itself: 1 byte, 3 bytes of padding, then a 4-byte integer.
    assert_eq!((expected.size, expected.align), (8, 4));
}

#[test]
fn the_human_report_lists_every_type_with_field_offsets() {
    let src = "struct Alpha { x: int, y: char }\nstruct Beta { z: int }\nfn main() {}";
    let program = check(src);
    let decls = yashy_driver::layout::HirDecls::from_program(&program);
    let rendered = yashy_driver::layout::render_human(&decls);
    assert!(rendered.contains("type"), "{rendered}");
    assert!(rendered.contains("Alpha"), "{rendered}");
    assert!(rendered.contains("Beta"), "{rendered}");
    assert!(rendered.contains("x@0"), "{rendered}");
}

#[test]
fn the_report_and_the_queries_name_every_field_offset() {
    let src = "repr(C) struct Triple { a: int, b: char, c: int }\n\
               fn main() {\n\
                   const first = offsetof(Triple, a);\n\
                   const third = offsetof(Triple, c);\n\
               }";
    let program = check(src);
    let decls = yashy_driver::layout::HirDecls::from_program(&program);
    let expected = layout(&TypeKind::Struct("Triple".to_string()), &decls)
        .expect("the declared struct has a layout");
    assert_eq!(expected.offsets.len(), 3);
    assert_eq!(const_int(&program, "first"), expected.offsets[0] as i64);
    assert_eq!(const_int(&program, "third"), expected.offsets[2] as i64);

    let reported =
        yashy_driver::layout::find("Triple", &decls).expect("inspect --layout must find the type");
    assert_eq!(reported.offsets, expected.offsets);
}

/// A folded query has to reach the backend as the same literal a user would
/// have written. Comparing the emitted `main` bodies makes the agreement a
/// three-way one: the checker's fold, the layout table and the IR.
#[test]
fn a_folded_layout_query_emits_the_same_literal_as_a_written_number() {
    let src = "repr(C) struct Pair { a: u8, b: i32 }\n\
               fn main() {\n\
                   const size = sizeof(Pair);\n\
                   const align = alignof(Pair);\n\
                   const second = offsetof(Pair, b);\n\
                   print(size);\n\
                   print(align);\n\
                   print(second);\n\
               }";
    let program = check(src);
    let decls = yashy_driver::layout::HirDecls::from_program(&program);
    let expected = layout(&TypeKind::Struct("Pair".to_string()), &decls)
        .expect("the declared struct has a layout");

    let written = format!(
        "fn main() {{\n\
             const size = {};\n\
             const align = {};\n\
             const second = {};\n\
             print(size);\n\
             print(align);\n\
             print(second);\n\
         }}",
        expected.size, expected.align, expected.offsets[1]
    );
    let literal_program = check(&written);

    let body = |program: &HirProgram| {
        let ir = yashy_codegen::emit_program(
            program,
            "main",
            Default::default(),
            &yashy_common::ProofFacts::empty(),
        );
        let at = ir
            .find("define i32 @main")
            .expect("the entry point is emitted");
        ir[at..].to_string()
    };
    assert_eq!(
        body(&program),
        body(&literal_program),
        "sizeof/alignof/offsetof must fold to the literals the layout table reports"
    );
}

#[cfg(test)]
mod ir_cross_check {
    use super::*;
    use std::collections::HashMap;

    /// One type as the IR declares it: its element spellings in order.
    fn definitions(ir: &str) -> HashMap<String, Vec<String>> {
        let mut out = HashMap::new();
        for line in ir.lines() {
            let line = line.trim();
            let Some((name, rest)) = line.split_once(" = type { ") else {
                continue;
            };
            let Some(body) = rest.strip_suffix(" }") else {
                continue;
            };
            out.insert(name.to_string(), split_top_level(body));
        }
        out
    }

    /// Split a `{ ... }` body on the commas that are not inside brackets.
    fn split_top_level(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        let mut start = 0usize;
        for (index, ch) in body.char_indices() {
            match ch {
                '[' | '{' | '(' => depth += 1,
                ']' | '}' | ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    out.push(body[start..index].trim().to_string());
                    start = index + 1;
                }
                _ => {}
            }
        }
        let last = body[start..].trim();
        if !last.is_empty() {
            out.push(last.to_string());
        }
        out
    }

    fn align_up(value: usize, align: usize) -> usize {
        if align <= 1 {
            value
        } else {
            (value + align - 1) / align * align
        }
    }

    /// `(size, align)` of an LLVM type spelling, computed with the C aggregate
    /// rules the target uses, not by asking the layout table.
    fn size_align(
        spelling: &str,
        defs: &HashMap<String, Vec<String>>,
        seen: &mut Vec<String>,
    ) -> (usize, usize) {
        let (size, align, _) = layout_of(spelling, defs, seen);
        (size, align)
    }

    /// `(size, align, offsets)` of an LLVM type spelling.
    fn layout_of(
        spelling: &str,
        defs: &HashMap<String, Vec<String>>,
        seen: &mut Vec<String>,
    ) -> (usize, usize, Vec<usize>) {
        let spelling = spelling.trim();
        if let Some(inner) = spelling.strip_prefix('[') {
            let (count, rest) = inner.split_once(" x ").expect("an array is `[N x T]`");
            let count: usize = count.trim().parse().expect("the array length is a number");
            let rest = rest.strip_suffix(']').expect("the array is closed");
            let (size, align, _) = layout_of(rest, defs, seen);
            return (size * count, align, Vec::new());
        }
        if let Some(inner) = spelling.strip_prefix('{') {
            let inner = inner
                .strip_suffix('}')
                .expect("the literal struct is closed");
            return struct_layout(&split_top_level(inner), defs, seen);
        }
        if spelling.starts_with('%') {
            assert!(
                !seen.contains(&spelling.to_string()),
                "the IR types are not recursive through `{spelling}`"
            );
            let fields = defs
                .get(spelling)
                .unwrap_or_else(|| panic!("no definition for `{spelling}`"));
            seen.push(spelling.to_string());
            let found = struct_layout(fields, defs, seen);
            seen.pop();
            return found;
        }
        let scalar = match spelling {
            "i1" | "i8" => (1, 1),
            "i16" => (2, 2),
            "i32" | "float" => (4, 4),
            "i64" | "double" | "ptr" => (8, 8),
            other if other.ends_with('*') => (8, 8),
            other => panic!("the test does not know the IR type `{other}`"),
        };
        (scalar.0, scalar.1, Vec::new())
    }

    fn struct_layout(
        fields: &[String],
        defs: &HashMap<String, Vec<String>>,
        seen: &mut Vec<String>,
    ) -> (usize, usize, Vec<usize>) {
        let mut offsets = Vec::new();
        let mut size = 0usize;
        let mut align = 1usize;
        for field in fields {
            let (field_size, field_align) = size_align(field, defs, seen);
            size = align_up(size, field_align);
            offsets.push(size);
            size += field_size;
            align = align.max(field_align);
        }
        (align_up(size, align), align, offsets)
    }

    fn ir_body(ir: &str, name: &str) -> Vec<String> {
        let defs = definitions(ir);
        let spelling = format!("%struct.{name}");
        defs.get(&spelling)
            .or_else(|| defs.get(&format!("%enum.{name}")))
            .unwrap_or_else(|| panic!("the backend must define `{spelling}`:\n{ir}"))
            .clone()
    }

    /// The numbers in the report must be the numbers in the IR. The walk is
    /// re-derived here from the emitted aggregate, so agreeing with itself is
    /// not enough to pass.
    #[test]
    fn every_declared_type_matches_the_ir_aggregate() {
        let src = "repr(C) struct Pair { a: u8, b: i32 }\n\
                   struct Inner { tag: char, value: int }\n\
                   struct Outer { inner: Inner, flag: bool, tail: char }\n\
                   enum Shape { Dot, Line(int), Box { w: int, h: int } }\n\
                   struct Item { name: char, shape: Shape, count: int }\n\
                   struct Table { items: [int; 3], flag: bool, item: Item }\n\
                   fn main() {\n\
                       let p = Pair { a: 1 as u8, b: 2 as i32 };\n\
                       let t = Table {\n\
                           items: [1, 2, 3],\n\
                           flag: true,\n\
                           item: Item { name: 'x', shape: Shape.Line(2), count: 3 },\n\
                       };\n\
                       let o = Outer { inner: Inner { tag: 'x', value: 3 }, flag: true, tail: 'y' };\n\
                       print(t.item.count + o.inner.value);\n\
                   }";
        let program = check(src);
        let decls = yashy_driver::layout::HirDecls::from_program(&program);
        let ir = yashy_codegen::emit_program(
            &program,
            "main",
            Default::default(),
            &yashy_common::ProofFacts::empty(),
        );

        for (name, found, _) in yashy_driver::layout::all(&decls) {
            let fields = ir_body(&ir, &name);
            let defs = definitions(&ir);
            let mut seen = Vec::new();
            let from_ir = struct_layout(&fields, &defs, &mut seen);
            assert_eq!(
                from_ir,
                (found.size, found.align, found.offsets),
                "the report and the IR disagree about `{name}`"
            );
        }
    }

    /// Padding is a fact about the aggregate, not a second walk: a `repr(C)`
    /// struct pins the C rule (1 byte, 3 of padding, then 4 bytes).
    #[test]
    fn padding_is_the_bytes_no_field_occupies() {
        let src = "repr(C) struct Pair { a: u8, b: i32 }\n\
                   struct Notes { flag: bool, value: char }\n\
                   fn main() {\n\
                       let p = Pair { a: 1 as u8, b: 2 as i32 };\n\
                       let n = Notes { flag: true, value: 'x' };\n\
                   }";
        let program = check(src);
        let decls = yashy_driver::layout::HirDecls::from_program(&program);
        assert_eq!(yashy_driver::layout::padding_bytes("Pair", &decls), 3);
        assert_eq!(yashy_driver::layout::padding_bytes("Notes", &decls), 0);
        assert_eq!(
            yashy_driver::layout::field_sizes("Pair", &decls),
            vec![1, 4]
        );
    }
}

#[test]
fn a_const_generic_query_emits_the_instantiated_literal() {
    let src = "struct Buffer[const N: int] { data: [int; N] }\n\
               fn main() {\n\
                   const small = size_of[Buffer[2]]();\n\
                   const large = size_of[Buffer[7]]();\n\
                   print(small);\n\
                   print(large);\n\
               }";
    let program = check(src);
    let written = "fn main() {\n\
                   const small = 16;\n\
                   const large = 56;\n\
                   print(small);\n\
                   print(large);\n\
               }";
    let literal_program = check(written);
    let body = |program: &HirProgram| {
        let ir = yashy_codegen::emit_program(
            program,
            "main",
            Default::default(),
            &yashy_common::ProofFacts::empty(),
        );
        let at = ir
            .find("define i32 @main")
            .expect("the entry point is emitted");
        ir[at..].to_string()
    };
    assert_eq!(
        body(&program),
        body(&literal_program),
        "size_of[Buffer[N]]() must fold to the size of that instantiation"
    );
}
