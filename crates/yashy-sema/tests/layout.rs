//! `repr(C)` and the layout queries: `sizeof`, `alignof`, `offsetof`.
//!
//! The queries are folded by the checker, so the test that matters is the
//! number the compiler computed. A query that reached the backend would be a
//! function that does not exist, and a number that disagreed with C would make
//! every `extern` call that passes a struct a silent memory error. These tests
//! pin both the numbers and the diagnostics for the cases where there is no
//! answer to give.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirExpr, HirItem, HirProgram, HirStmt, Literal};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the declaration");
    check_program(&program, &sources, file)
}

fn assert_rejected(src: &str, needle: &str) -> Vec<Diagnostic> {
    match check(src) {
        Ok(_) => panic!("expected an error containing `{needle}`"),
        Err(diagnostics) => {
            assert!(
                diagnostics.iter().any(|d| d.message.contains(needle)),
                "expected an error containing `{needle}`, got {:?}",
                diagnostics
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
            );
            diagnostics
        }
    }
}

/// The integer a `const` was folded to, read from the checked program.
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

/// Run a program of `const` queries and read the value of each binding.
fn queries(body: &str) -> Vec<i64> {
    let src = format!("fn main() {{\n{body}\n}}");
    let program = check(&src).unwrap_or_else(|diagnostics| {
        panic!(
            "the queries must check: {:?}",
            diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
        )
    });
    let mut values = Vec::new();
    for item in &program.items {
        let HirItem::Fn(function) = item else {
            continue;
        };
        for stmt in &function.body.stmts {
            if let HirStmt::Const {
                init: HirExpr::Literal(Literal::I64(value), _, _),
                ..
            } = stmt
            {
                values.push(*value);
            }
        }
    }
    values
}

#[test]
fn scalar_sizes_and_alignments_are_the_machine_numbers() {
    let values = queries(
        "    const a = sizeof(u8);\n\
         const b = sizeof(i16);\n\
         const c = sizeof(i32);\n\
         const d = sizeof(int);\n\
         const e = sizeof(float);\n\
         const f = alignof(u8);\n\
         const g = alignof(i16);\n\
         const h = alignof(int);",
    );
    assert_eq!(
        values,
        vec![1, 2, 4, 8, 8, 1, 2, 8],
        "a scalar is a table entry, and the table is the machine's"
    );
}

#[test]
fn a_repr_c_struct_is_laid_out_by_cs_rules() {
    // `u8` then `i32`: the integer starts at the next multiple of four, and the
    // whole is rounded up to its largest alignment. This is the layout C uses,
    // which is the point of the clause.
    let src = "repr(C) struct Pair { a: u8, b: i32 }\n\
               fn main() {\n\
                   const size = sizeof(Pair);\n\
                   const align = alignof(Pair);\n\
                   const first = offsetof(Pair, a);\n\
                   const second = offsetof(Pair, b);\n\
               }";
    let program = check(src).expect("the struct and its queries must check");
    assert_eq!(const_int(&program, "size"), 8, "1 byte, 3 padding, 4 bytes");
    assert_eq!(const_int(&program, "align"), 4, "the largest field alignment");
    assert_eq!(const_int(&program, "first"), 0, "the first field starts at 0");
    assert_eq!(const_int(&program, "second"), 4, "the integer is aligned to 4");
}

#[test]
fn a_nested_repr_c_struct_keeps_its_inner_padding() {
    // The inner struct's own size already includes its padding, so the outer
    // walk sees one 8-byte field and not its pieces. A walk that flattened the
    // fields would put `c` at offset 5 and disagree with C.
    let src = "repr(C) struct Inner { a: u8, b: i32 }\n\
               repr(C) struct Outer { inner: Inner, c: u8 }\n\
               fn main() {\n\
                   const size = sizeof(Outer);\n\
                   const inner = offsetof(Outer, inner);\n\
                   const c = offsetof(Outer, c);\n\
               }";
    let program = check(src).expect("the nested structs must check");
    assert_eq!(const_int(&program, "size"), 12, "8 + 1, rounded to 4");
    assert_eq!(const_int(&program, "inner"), 0);
    assert_eq!(const_int(&program, "c"), 8);
}

#[test]
fn a_repr_c_enum_tag_is_a_c_int() {
    // A C `enum` is a name for an `int`, so the whole value is four bytes. The
    // Yashy enum's tag is one byte, which is what makes the two different.
    let src = "repr(C) enum CK { A, B }\n\
               enum YK { A, B }\n\
               fn main() {\n\
                   const c = sizeof(CK);\n\
                   const y = sizeof(YK);\n\
               }";
    let program = check(src).expect("both enums must check");
    assert_eq!(const_int(&program, "c"), 4, "a C enum is an `int`");
    assert_eq!(const_int(&program, "y"), 1, "a Yashy enum's tag is one byte");
}

#[test]
fn a_repr_c_enum_with_a_payload_is_reported() {
    let diagnostics = assert_rejected(
        "repr(C) enum Shape { Empty, Circle(f64) }",
        "has a payload",
    );
    assert!(
        diagnostics.iter().any(|d| d.code == Some("E0106")),
        "the diagnostic carries the catalog code: {diagnostics:?}"
    );
}

#[test]
fn the_queries_answer_about_a_type_this_module_can_name() {
    // A bare name is a type. A value, a call, or a dotted path is not, and
    // each is reported rather than guessed at.
    assert_rejected("fn main() { const x = sizeof(1); }", "not a value");
    assert_rejected("fn main() { const x = sizeof(NoSuchType); }", "no known type");
    assert_rejected("fn main() { const x = sizeof(); }", "takes one type name");
    assert_rejected(
        "fn main() { const x = sizeof(a.b); }",
        "is not a type",
    );
}

#[test]
fn offsetof_needs_a_repr_c_struct() {
    let diagnostics = assert_rejected(
        "struct P { a: int }\nfn main() { const x = offsetof(P, a); }",
        "needs a `repr(C)` struct",
    );
    assert!(
        diagnostics.iter().any(|d| d.code == Some("E0108")),
        "the diagnostic carries the catalog code: {diagnostics:?}"
    );
}

#[test]
fn offsetof_names_a_field_that_exists() {
    assert_rejected(
        "repr(C) struct P { a: int }\nfn main() { const x = offsetof(P, b); }",
        "has no field `b`",
    );
    assert_rejected(
        "repr(C) struct P { a: int }\nfn main() { const x = offsetof(P); }",
        "takes a type name and a field name",
    );
}

#[test]
fn a_non_repr_c_struct_still_refuses_c_field_spellings() {
    // The clause is what licenses the C spellings, so removing it must restore
    // the ordinary rule. Otherwise `repr(C)` would be decorative.
    assert_rejected("struct P { a: u8 }", "only allowed in an `extern`");
    check("repr(C) struct P { a: u8 }").expect("the clause licenses the C spelling");
}

#[test]
fn a_repr_c_object_and_enum_are_accepted_like_a_struct() {
    // The clause is read once and attached to whichever type follows, so the
    // object form and the enum form need no separate rule.
    check("repr(C) object O { v: int }").expect("an object has a layout");
    check("repr(C) enum K { A, B }").expect("a fieldless enum has a layout");
}
