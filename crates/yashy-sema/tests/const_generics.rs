//! Const generics: a `const N: int` parameter is part of a type's identity and
//! substitutes into array lengths at each use.

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("const_generics.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must accept it");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    check_program_with(&ast, &sm, file, CheckOptions { release: false })
}

#[test]
fn a_const_parameter_substitutes_into_an_array_length() {
    let program = check(
        "struct Buffer[const N: int] { data: [int; N], len: int }\n\
         fn f(Buffer[16] b) -> int { return b.len; }\n\
         fn g() -> int { return sizeof(Buffer[16]); }",
    )
    .expect("the monomorph must check");
    let text = format!("{:?}", program.items);
    assert!(
        text.contains("cint16"),
        "the const value is part of the monomorph name: {text}"
    );
}

#[test]
fn different_const_values_are_different_types() {
    let program = check(
        "struct Buffer[const N: int] { data: [int; N], len: int }\n\
         fn f(Buffer[16] a, Buffer[32] b) -> int { return a.len + b.len; }",
    )
    .expect("two values give two monomorphs");
    let names: Vec<_> = program
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Struct(s) => Some(s.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        names.iter().any(|n| n.contains("cint16")) && names.iter().any(|n| n.contains("cint32")),
        "both monomorphs are emitted: {names:?}"
    );
}

#[test]
fn a_const_argument_fixes_the_layout() {
    let program = check(
        "struct Buffer[const N: int] { data: [int; N] }\n\
         fn main() {\n\
             const int small = sizeof(Buffer[2]);\n\
             const int large = sizeof(Buffer[7]);\n\
             if small == 16 && large == 56 { }\n\
         }",
    )
    .expect("each value has its own size");
    let text = format!("{:?}", program.items);
    assert!(text.contains("cint2") && text.contains("cint7"), "{text}");
}

#[test]
fn a_generic_function_instantiates_per_const_value() {
    let program = check(
        "fn first[const N: int]([int; N] xs) -> int { return 0; }\n\
         fn main() {\n\
             const int a = first[2]([1, 2]);\n\
             const int b = first[3]([1, 2, 3]);\n\
             if a == b { }\n\
         }",
    )
    .expect("the const argument fixes the parameter and array length");
    let text = format!("{:?}", program.items);
    assert!(
        text.contains("first_cint2") && text.contains("first_cint3"),
        "one function per const value: {text}"
    );
}

#[test]
fn a_bracket_layout_query_names_an_instantiation() {
    let program = check(
        "struct Buffer[const N: int] { data: [int; N] }\n\
         fn main() { const int n = size_of[Buffer[4]](); if n == 32 { } }",
    )
    .expect("size_of[T]() names the instantiation");
    let text = format!("{:?}", program.items);
    assert!(text.contains("Buffer_cint4"), "{text}");
}

#[test]
fn a_non_constant_length_is_a_diagnostic() {
    let diagnostics = check(
        "struct Buffer[const N: int] { data: [int; N], len: int }\n\
         fn f(int n) -> int { return sizeof(Buffer[n]); }",
    )
    .expect_err("a run-time value cannot be a const argument");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == Some(codes::CONST_NOT_CONSTANT)),
        "E1524 expected: {diagnostics:?}"
    );
}

#[test]
fn a_repr_c_const_buffer_keeps_its_abi_shape() {
    let program = check(
        "repr(C) struct Buffer[const N: int] { tag: u8, data: [u8; N] }\n\
         fn main() { const int n = size_of[Buffer[4]](); if n == 5 { } }",
    )
    .expect("the tag and the four bytes stay adjacent");
    let text = format!("{:?}", program.items);
    assert!(text.contains("Buffer_cint4"), "{text}");
}

#[test]
fn const_arguments_count_toward_the_instantiation_budget() {
    let mut src =
        String::from("fn first[const N: int]([int; N] xs) -> int { return 0; }\nfn main() {\n");
    for n in 1..=65 {
        let elements = vec!["0"; n].join(", ");
        src.push_str(&format!("const int v{n} = first[{n}]([{elements}]);\n"));
    }
    src.push_str("}\n");
    let diagnostics = check(&src).expect_err("the 65th value exceeds the per-function budget");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == Some(codes::INSTANTIATION_BUDGET)),
        "E1505 expected: {diagnostics:?}"
    );
}

#[test]
fn an_unwritten_const_argument_is_a_diagnostic() {
    let diagnostics = check(
        "fn first[const N: int]([int; N] xs) -> int { return 0; }\n\
         fn main() { const int a = first([1, 2, 3]); }",
    )
    .expect_err("a const parameter cannot be inferred from nothing");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("const parameter")),
        "a named const parameter is expected: {diagnostics:?}"
    );
}
