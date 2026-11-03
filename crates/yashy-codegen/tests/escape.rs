//! Escape analysis and promotion: where a non-escaping allocation lives.
//!
//! Promotion never changes what a program prints, so the promise these tests
//! make is about the emitted IR: a site that provably never leaves its function
//! becomes a frame slot or a block-arena slot, and a site that escapes — or one
//! the pass cannot follow — keeps the allocator call. The conservative path is
//! the default when the option is off.

use yashy_common::{ProofFacts, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and emit `src` with promotion enabled.
fn promoted(src: &str) -> String {
    lower(src, true)
}

/// Lex, parse, check and emit `src`, choosing whether promotion is enabled.
fn lower(src: &str, escape: bool) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add_file("escape.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the fixture's lexer must succeed");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the fixture must parse");
    let hir = check_program(&ast, &sources, file).expect("the fixture must check");
    emit_program(
        &hir,
        "escape.ya",
        CodegenOptions {
            escape,
            ..CodegenOptions::default()
        },
        &ProofFacts::empty(),
    )
}

/// The lines of the `define` block for `name`, without the module header.
fn body(ir: &str, name: &str) -> String {
    let needle = format!("@{name}(");
    let start = ir
        .lines()
        .position(|line| line.starts_with("define ") && line.contains(&needle))
        .unwrap_or_else(|| panic!("no `define` for `{name}` in:\n{ir}"));
    let lines: Vec<&str> = ir.lines().skip(start).collect();
    let end = lines
        .iter()
        .position(|line| *line == "}")
        .map(|index| index + 1)
        .unwrap_or(lines.len());
    lines[..end].join("\n")
}

const STACK: &str = "\
fn f() -> int {
    let p = alloc(8);
    p[0] = 1;
    let v = p[0];
    p.free();
    return v;
}
fn main() { }
";

#[test]
fn an_allocation_whose_pointer_never_leaves_the_function_goes_on_the_stack() {
    let ir = promoted(STACK);
    let f = body(&ir, "f");
    assert!(
        f.contains("alloca [8 x i64]"),
        "expected a frame slot:\n{f}"
    );
    assert!(
        !f.contains("@yashy_alloc("),
        "the allocator call must go:\n{f}"
    );
    assert!(
        !f.contains("@yashy_free("),
        "the matching free must go:\n{f}"
    );
}

#[test]
fn an_allocation_returned_from_the_function_stays_on_the_heap() {
    let ir = promoted(
        "\
fn f() -> *mut int {
    let p = alloc(8);
    return p;
}
fn main() { }
",
    );
    let f = body(&ir, "f");
    assert!(
        f.contains("@yashy_alloc("),
        "a returned pointer escapes:\n{f}"
    );
    assert!(
        !f.contains("alloca [8 x i64]"),
        "nothing may be promoted:\n{f}"
    );
}

#[test]
fn an_allocation_stored_in_a_returned_struct_stays_on_the_heap() {
    let ir = promoted(
        "\
struct Box { p: *mut int }
fn f() -> Box {
    let p = alloc(8);
    return Box { p: p };
}
fn main() { }
",
    );
    let f = body(&ir, "f");
    assert!(
        f.contains("@yashy_alloc("),
        "the field carries it out:\n{f}"
    );
    assert!(
        !f.contains("alloca [8 x i64]"),
        "nothing may be promoted:\n{f}"
    );
}

#[test]
fn a_dynamic_allocation_that_stays_in_its_block_uses_the_block_arena() {
    let ir = promoted(
        "\
fn f(n: int) -> int {
    let total = 0;
    let i = 0;
    while i < n {
        let p = alloc(n);
        p[0] = i;
        total += p[0];
        p.free();
        i += 1;
    }
    return total;
}
fn main() { }
",
    );
    let f = body(&ir, "f");
    assert!(
        f.contains("@yashy_arena_alloc("),
        "expected a block arena:\n{f}"
    );
    assert!(
        f.contains("@yashy_arena_free_all("),
        "the arena must be freed:\n{f}"
    );
    assert!(
        !f.contains("@yashy_alloc("),
        "the allocator call must go:\n{f}"
    );
}

#[test]
fn a_constant_allocation_larger_than_four_kib_uses_the_arena() {
    let small = promoted(
        "\
fn f() -> int {
    let p = alloc(512);
    p[0] = 1;
    let v = p[0];
    p.free();
    return v;
}
fn main() { }
",
    );
    assert!(
        body(&small, "f").contains("alloca [512 x i64]"),
        "4096 bytes fit in a frame:\n{}",
        body(&small, "f")
    );

    let large = promoted(
        "\
fn f() -> int {
    let p = alloc(513);
    p[0] = 1;
    let v = p[0];
    p.free();
    return v;
}
fn main() { }
",
    );
    let f = body(&large, "f");
    assert!(
        f.contains("@yashy_arena_alloc("),
        "4104 bytes exceed the cap:\n{f}"
    );
    assert!(
        !f.contains("alloca [513 x i64]"),
        "the frame must not grow:\n{f}"
    );
}

#[test]
fn an_allocation_handed_to_a_callee_stays_on_the_heap() {
    let ir = promoted(
        "\
fn consume(p: *mut int) {
    p[0] = 1;
}
fn f() {
    let p = alloc(8);
    consume(p);
}
fn main() { }
",
    );
    let f = body(&ir, "f");
    assert!(f.contains("@yashy_alloc("), "a callee can keep it:\n{f}");
    assert!(
        !f.contains("alloca [8 x i64]"),
        "nothing may be promoted:\n{f}"
    );
}

#[test]
fn a_deferred_free_of_a_promoted_allocation_is_suppressed() {
    // `defer` snapshots every linear binding into a fresh slot at
    // registration. That snapshot holds the promoted pointer, so the mark has
    // to travel with it; otherwise the deferred `free` reaches the allocator
    // with a frame address.
    let ir = promoted(
        "\
fn f() {
    let p = alloc(8);
    p[0] = 1;
    defer { p.free(); }
}
fn main() { }
",
    );
    let f = body(&ir, "f");
    assert!(
        f.contains("alloca [8 x i64]"),
        "the allocation is promoted:\n{f}"
    );
    assert!(
        !f.contains("@yashy_free("),
        "the deferred free must be suppressed too:\n{f}"
    );
    assert!(
        !f.contains("@yashy_alloc("),
        "the allocator call must go:\n{f}"
    );
}

#[test]
fn promotion_can_be_turned_off_for_a_bisect() {
    let ir = lower(STACK, false);
    let f = body(&ir, "f");
    assert!(
        f.contains("@yashy_alloc("),
        "the default path is the heap:\n{f}"
    );
    assert!(f.contains("@yashy_free("), "and its free stays:\n{f}");
    assert!(
        !f.contains("alloca [8 x i64]"),
        "nothing may be promoted:\n{f}"
    );
}
