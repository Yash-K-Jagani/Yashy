//! The monomorphisation budget: 64 distinct substitutions per generic
//! function, 4096 per program; identical substitutions count once.

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirProgram, MAX_INSTANTIATIONS_PER_FN};

/// Lex, parse and check `source` as a single file.
fn check_source(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    check_program_with(&ast, &sm, file, CheckOptions::default())
}

/// A program calling the generic `pick[T]` once per named struct type.
fn many_types(count: usize) -> String {
    let mut src = String::from("fn pick[T](T x) -> T { return x; }\n");
    for i in 0..count {
        src.push_str(&format!("struct T{i} {{ n: int }}\n"));
    }
    src.push_str("fn f() -> int { let total = 0;\n");
    for i in 0..count {
        src.push_str(&format!("let v{i} = T{i} {{ n: 1 }}; total = total + pick(v{i}).n;\n"));
    }
    src.push_str("return total; }\n");
    src
}

#[test]
fn sixty_four_distinct_instantiations_pass() {
    let hir = check_source(&many_types(MAX_INSTANTIATIONS_PER_FN))
        .expect("64 distinct instantiations are within budget");
    assert_eq!(hir.instantiations, MAX_INSTANTIATIONS_PER_FN);
}

#[test]
fn sixty_five_distinct_instantiations_is_e1505() {
    let diags = check_source(&many_types(MAX_INSTANTIATIONS_PER_FN + 1))
        .expect_err("65 distinct instantiations exceed the budget");
    assert!(
        diags.iter().any(|d| d.code == Some(codes::INSTANTIATION_BUDGET)),
        "expected E1505, got {:?}",
        diags.iter().map(|d| (d.code, &d.message)).collect::<Vec<_>>()
    );
}

#[test]
fn the_same_substitution_from_many_call_sites_is_one_instantiation() {
    let mut src = String::from("fn id[T](T x) -> T { return x; }\nfn f() -> int { let total = 0;\n");
    for _ in 0..100 {
        src.push_str("total = total + id(1);\n");
    }
    src.push_str("return total; }\n");
    let hir = check_source(&src).expect("one substitution must check");
    assert_eq!(hir.instantiations, 1, "100 call sites, one instantiation: {hir:?}");
}
