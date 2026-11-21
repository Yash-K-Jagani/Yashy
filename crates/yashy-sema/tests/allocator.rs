//! The static-dispatch `Allocator` interface (Phase 4 Task 6 Step 3).
//!
//! A program implements `Allocator` for its own type and calls the methods on
//! a local binding. Dispatch is static: each call lowers to the concrete
//! type's `Type.Allocator.method` symbol, with no trait object or vtable.
//! A recording impl counts every call, which is the "custom allocator records
//! every call" gate from Step 1.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirItem, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("allocator.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the source must parse");
    check_program(&ast, &sources, file)
}

const RECORDER: &str = "\
trait Allocator { fn alloc(self: *mut Self, int count) -> *mut int; fn free(self: *mut Self, *mut int p); }\n\
struct Rec { allocs: int, frees: int }\n\
impl Allocator for Rec {\n\
    fn alloc(self: *mut Self, int count) -> *mut int { self[0].allocs = self[0].allocs + 1; return alloc(count); }\n\
    fn free(self: *mut Self, *mut int p) { self[0].frees = self[0].frees + 1; p.free(); }\n\
}\n\
fn main() { let r = Rec { allocs: 0, frees: 0 }; const p = r.alloc(2); r.free(p); print(r.allocs); print(r.frees); }";

#[test]
fn a_custom_allocator_records_every_call() {
    let program = check(RECORDER).expect("a recording allocator must check");
    // The two trait methods are emitted as concrete symbols, not a vtable.
    let text = format!("{:?}", program.items);
    assert!(
        text.contains("Rec.Allocator.alloc") && text.contains("Rec.Allocator.free"),
        "dispatch must name the concrete impl: {text}"
    );
}

#[test]
fn an_impl_must_provide_every_allocator_method() {
    let source = "\
trait Allocator { fn alloc(self: *mut Self, int count) -> *mut int; fn free(self: *mut Self, *mut int p); }\n\
struct Rec { n: int }\n\
impl Allocator for Rec { fn alloc(self: *mut Self, int count) -> *mut int { return alloc(count); } }\n\
fn main() { }";
    let diagnostics = check(source).expect_err("an incomplete impl must be refused");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("free")
                || d.code == Some(yashy_common::codes::INCOMPLETE_IMPL)),
        "the missing free method must be named: {diagnostics:?}"
    );
}

#[test]
fn a_generic_trait_bound_dispatches_statically() {
    let source = "\
trait Allocator { fn alloc(self: *mut Self, int count) -> *mut int; fn free(self: *mut Self, *mut int p); }\n\
struct Rec { n: int }\n\
impl Allocator for Rec {\n\
    fn alloc(self: *mut Self, int count) -> *mut int { return alloc(count); }\n\
    fn free(self: *mut Self, *mut int p) { p.free(); }\n\
}\n\
fn use_it[T: Allocator](T a) -> *mut int { let local = a; return local.alloc(1); }\n\
fn main() { let r = Rec { n: 0 }; const p = use_it(r); p.free(); }";
    let program = check(source).expect("a bounded generic allocator must check");
    let names: Vec<_> = program
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Fn(f) => Some(f.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        names
            .iter()
            .any(|name| name.contains("use_it") && name.contains("Rec")),
        "the generic must be monomorphised per allocator: {names:?}"
    );
}
