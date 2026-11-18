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
fn the_report_and_the_queries_name_every_field_offset() {
    let src = "struct Triple { a: int, b: char, c: int }\n\
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
