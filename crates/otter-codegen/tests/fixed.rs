//! Fixed arrays (`[T; N]`) and borrowed views (`[T]`): the aggregate in the
//! frame, the guarded index, and the pointer-and-length view header.
//!
//! These are IR-shape tests, not runtime tests. The end-to-end behaviour lives
//! in `crates/otter-driver/tests/programs/fixed_arrays.ot`, but that test needs
//! to spawn a compiler and an executable, and Application Control blocks a
//! freshly linked binary now and then. These tests always run: they pin the
//! shapes that make the runtime test meaningful.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram};

use otter_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("fixed.ot", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(&check_source(src), "fixed.ot", CodegenOptions::default())
}

/// The text of the basic block `name` starts with, up to the next block label.
fn block<'a>(ir: &'a str, name: &str) -> &'a str {
    let start = ir
        .find(&format!("{name}:"))
        .unwrap_or_else(|| panic!("the IR has no `{name}` block:\n{ir}"));
    let rest = &ir[start..];
    let next = rest[1..].find("\nbb").map(|offset| offset + 1);
    match next {
        Some(end) => &rest[..end],
        None => rest,
    }
}

#[test]
fn a_fixed_array_literal_lives_in_the_frame() {
    // The whole point of `[T; N]`: the elements are the local's own storage.
    // There is no allocation, so there is nothing to free and no owner.
    let ir = emit("fn main() { const grid = [1, 2, 3]; print_i64(grid.len()); }");
    assert!(
        ir.contains("%slot0 = alloca [3 x i64]"),
        "the elements are a stack aggregate:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue [3 x i64] undef, i64 1, 0")
            && ir.contains("insertvalue [3 x i64] %t0, i64 2, 1")
            && ir.contains("insertvalue [3 x i64] %t1, i64 3, 2"),
        "the literal is an insertvalue chain, one per element:\n{ir}"
    );
    assert!(
        !ir.contains("call i8* @otter_alloc"),
        "a fixed array never reaches the allocator:\n{ir}"
    );
    assert!(
        ir.contains("call void @otter_print_i64(i64 3)"),
        "`len` is the length in the type, folded to a constant:\n{ir}"
    );
}

#[test]
fn a_fixed_array_index_is_checked_and_guarded() {
    // `grid[1]` is an `Option[i64]`: the comparison is the tag, and the load
    // reads element 0 when the index is out of range, so no load is ever
    // outside the array.
    let ir = emit("fn main() { const grid = [1, 2, 3]; const at = grid[1]; }");
    assert!(
        ir.contains("icmp slt i64 1, 0") && ir.contains("icmp sge i64 1,"),
        "the index is compared against both ends:\n{ir}"
    );
    assert!(
        ir.contains("= select i1 %t8, i64 1, i64 0"),
        "an out-of-range index is replaced by 0 before the load:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr i64, ptr %t3, i64 %t9")
            && ir.contains("load i64, ptr %t10"),
        "the element is loaded through the guarded index:\n{ir}"
    );
    assert!(
        ir.contains("insertvalue { i8, i64 } undef, i8 %t13, 0")
            && ir.contains("insertvalue { i8, i64 } %t14, i64 %t15, 1"),
        "the answer is the `Option` tag plus the payload:\n{ir}"
    );
}

#[test]
fn a_view_is_a_pointer_and_a_clamped_length() {
    // `grid[0..2]` borrows: a pointer to the first element and a count. Both
    // bounds go through `select`s, so a range can never fault.
    let ir = emit("fn main() { const grid = [1, 2, 3]; const head = grid[0..2]; }");
    assert!(
        ir.contains("insertvalue { i8*, i64 } undef, i8*")
            && ir.contains("insertvalue { i8*, i64 } %t18, i64 %t15, 1"),
        "the view is one pointer and one length:\n{ir}"
    );
    assert!(
        ir.contains("icmp sgt i64 %t6, %t4") && ir.contains("icmp sgt i64 %t10, %t4"),
        "each bound is clamped to the source's length:\n{ir}"
    );
    assert!(
        ir.contains("sub i64 %t14, %t8"),
        "the length is `end - start`:\n{ir}"
    );
    assert!(
        ir.contains("mul i64 %t8, 8") && ir.contains("getelementptr i8, ptr %t3, i64 %t16"),
        "the pointer is the source's first element plus `start * size`:\n{ir}"
    );
    assert!(
        !ir.contains("call i8* @otter_alloc"),
        "a view owns nothing, so it allocates nothing:\n{ir}"
    );
}

#[test]
fn a_fixed_array_loop_uses_the_length_from_its_type() {
    // The length cannot change, so the loop's bound is a constant and the
    // element address is computed once, before the loop.
    let ir = emit("fn main() { const grid = [1, 2, 3]; var sum = 0; for v in grid { sum = sum + v; } }");
    assert!(
        ir.contains("icmp slt i64 %t4, 3"),
        "the loop compares its index with the constant length:\n{ir}"
    );
    assert!(
        !ir.contains("i32 0, i32 1"),
        "there is no header to read a length from:\n{ir}"
    );
    let entry = &ir[..ir.find("bb0_cond:").unwrap_or(0)];
    assert!(
        entry.contains("getelementptr [3 x i64], ptr %slot0, i64 0, i64 0"),
        "the element address is hoisted out of the loop:\n{ir}"
    );
}

#[test]
fn a_slice_loop_re_reads_its_header_each_iteration() {
    // A slice's header is live: a `push` inside the loop grows the backing
    // array, so the length and the data pointer must be read again on every
    // iteration. Hoisting them would stop the loop after one element (and read
    // freed memory after the reallocation).
    let ir = emit(
        "fn main() { var xs = []i64 { 1, 2 }; for v in xs { xs.push(v); } xs.free(); }",
    );
    let condition = block(&ir, "bb0_cond");
    assert!(
        condition.contains("{ i8*, i64, i64 }, ptr %slot0, i32 0, i32 1")
            && condition.contains("= load i64"),
        "the length is read in the condition block:\n{condition}"
    );
    let body = block(&ir, "bb1_body");
    assert!(
        body.contains("{ i8*, i64, i64 }, ptr %slot0, i32 0, i32 0")
            && body.contains("= load i8*"),
        "the data pointer is read in the body block:\n{body}"
    );
}
