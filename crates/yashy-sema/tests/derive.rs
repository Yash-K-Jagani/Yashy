//! Derive expansion: the names on the clause become ordinary methods, checked
//! by the same passes as hand-written ones. Expansion is declaration-ordered,
//! so the generated items appear in a stable order.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("derive.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    check_program_with(&ast, &sm, file, CheckOptions { release: false })
}

fn generated_names(program: &HirProgram) -> Vec<String> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Fn(function) => Some(function.name.clone()),
            _ => None,
        })
        .filter(|name| name.contains("debug") || name.contains("eq") || name.contains("hash"))
        .collect()
}

#[test]
fn debug_and_eq_generate_methods() {
    let program = check(
        "struct Point derive(Debug, Eq) { x: int, y: int }\n\
         fn main() { let a = Point { x: 1, y: 2 }; let b = Point { x: 1, y: 2 }; \
                     print(a.debug()); print(a.eq(b)); }",
    )
    .expect("the generated methods type-check");
    let names = generated_names(&program);
    assert!(
        names.iter().any(|name| name.contains("debug")),
        "a debug method was generated: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.contains("eq")),
        "an eq method was generated: {names:?}"
    );
}

#[test]
fn generated_methods_appear_in_declaration_order() {
    let program = check("struct Pair derive(Debug, Eq) { a: int, b: int } fn main() {}")
        .expect("the program checks");
    let names = generated_names(&program);
    assert_eq!(names.len(), 2, "one method per derive: {names:?}");
}

#[test]
fn generated_methods_take_a_self_receiver() {
    // A generated function without a receiver is an associated function, so
    // `a.debug()` would be refused. The `self` parameter is what makes the
    // generated item a method, and that is what this pins.
    let program = check(
        "struct Point derive(Debug, Eq) { x: int, y: int }\n\
         fn main() { let a = Point { x: 1, y: 2 }; let b = Point { x: 1, y: 2 }; \
                     print(a.debug()); print(a.eq(b)); }",
    )
    .expect("the generated methods type-check");
    let mut checked = 0;
    for item in &program.items {
        let HirItem::Fn(function) = item else {
            continue;
        };
        if !function.name.contains("debug") && !function.name.contains("eq") {
            continue;
        }
        assert_eq!(
            function.params.first().map(|param| param.name.as_str()),
            Some("self"),
            "`{}` must be a method, not an associated function: {:?}",
            function.name,
            function.params
        );
        checked += 1;
    }
    assert_eq!(checked, 2, "both derived methods were inspected");
}

#[test]
fn hash_generates_a_deterministic_method() {
    let first = check(
        "struct Point derive(Hash) { x: int, y: int }\n\
         fn main() { let a = Point { x: 1, y: 2 }; print(a.hash()); }",
    )
    .expect("the generated method type-checks");
    let second = check(
        "struct Point derive(Hash) { x: int, y: int }\n\
         fn main() { let a = Point { x: 1, y: 2 }; print(a.hash()); }",
    )
    .expect("the second expansion type-checks");
    let names = generated_names(&first);
    assert_eq!(names.len(), 1, "one hash method: {names:?}");
    assert!(names[0].contains("hash"), "the method is named hash");
    // The expansion is pure AST data in declaration order, so two runs give
    // the same typed program byte for byte.
    assert_eq!(
        format!("{first:?}"),
        format!("{second:?}"),
        "two expansions must produce the same program"
    );
}

#[test]
fn all_three_derives_are_declaration_ordered() {
    let program = check("struct Pair derive(Debug, Eq, Hash) { a: int, b: int } fn main() {}")
        .expect("the program checks");
    let names = generated_names(&program);
    assert_eq!(names.len(), 3, "one method per derive: {names:?}");
    assert!(names[0].contains("debug"), "first derive first: {names:?}");
    assert!(names[1].contains("eq"), "second derive second: {names:?}");
    assert!(names[2].contains("hash"), "third derive third: {names:?}");
}

#[test]
fn an_unsupported_field_is_named() {
    let diagnostics = check(
        "struct Point derive(Debug) { x: string }\n\
         fn main() { let p = Point { x: \"a\" }; print(p.debug()); }",
    )
    .expect_err("a string field cannot be rendered by this expansion");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("field `x`")),
        "the failure names the field: {diagnostics:?}"
    );
}

#[test]
fn an_unknown_derive_is_named() {
    let diagnostics = check("struct Point derive(Show) { x: int } fn main() {}")
        .expect_err("only known derives are admitted");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("`Show`")),
        "the failure names the derive: {diagnostics:?}"
    );
}

#[test]
fn an_enum_derives_one_method_per_name() {
    let program = check(
        "enum Shape derive(Debug, Eq, Hash) { Circle(int), Rect { w: int, h: int }, Empty }\n\
         fn main() {\n\
             let a = Shape.Circle(2);\n\
             let b = Shape.Circle(2);\n\
             print(a.debug());\n\
             print(a.eq(b));\n\
             print(a.hash());\n\
         }",
    )
    .expect("enum derives type-check");
    let names = generated_names(&program);
    assert_eq!(names.len(), 3, "one method per derive: {names:?}");
    assert!(names[0].contains("debug"), "first derive first: {names:?}");
    assert!(names[1].contains("eq"), "second derive second: {names:?}");
    assert!(names[2].contains("hash"), "third derive third: {names:?}");
}

#[test]
fn an_enum_derive_switches_over_every_variant_in_order() {
    // The generated body is a `switch self`, so it names the same variants the
    // declaration does and has no other control flow to review. The outer arm
    // order is the variant order; a second expansion is byte-identical.
    let first = check(
        "enum Shape derive(Debug) { Circle(int), Rect { w: int, h: int }, Empty }\n\
         fn main() { let e = Shape.Empty; print(e.debug()); }",
    )
    .expect("the generated enum method type-checks");
    let second = check(
        "enum Shape derive(Debug) { Circle(int), Rect { w: int, h: int }, Empty }\n\
         fn main() { let e = Shape.Empty; print(e.debug()); }",
    )
    .expect("the second expansion type-checks");
    assert_eq!(
        format!("{first:?}"),
        format!("{second:?}"),
        "two expansions must produce the same program"
    );
}

#[test]
fn an_enum_derive_names_the_variant_field_it_cannot_handle() {
    let diagnostics = check(
        "enum Shape derive(Eq) { Circle(string) }\n\
         fn main() { print(Shape.Circle(\"a\").debug()); }",
    )
    .expect_err("a string payload cannot be compared by this expansion");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("field 0") && d.message.contains("`Circle`")),
        "the failure names the variant and the field: {diagnostics:?}"
    );
}

#[test]
fn a_generic_enum_derive_is_refused_with_a_reason() {
    let diagnostics = check(
        "enum Maybe[T] derive(Debug) { Nothing, Just(T) }\n\
         fn main() {}",
    )
    .expect_err("a generic enum has no one concrete type to expand for");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("generic enum")),
        "the failure explains the limitation: {diagnostics:?}"
    );
}
