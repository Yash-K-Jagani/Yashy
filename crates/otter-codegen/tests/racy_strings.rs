//! Which reference-count operations the backend calls.
//!
//! Spec §5.6 promises the non-atomic retain/release path stays free for
//! strings that are never shared, so the atomic pair must not become the
//! default. Once a program can spawn a thread, though, *any* string in it can
//! reach another thread: through an argument, a channel-side copy, or a call
//! that passes one along. The rule this milestone implements is therefore
//! per-program, not per-binding:
//!
//! - no `thread_spawn` anywhere → plain retain/release, unchanged from M3;
//! - one or more spawns → every string operation in the program is atomic.
//!
//! That over-approximates (a thread body's own private strings pay for
//! atomics) and deliberately under-approximates nothing: atomic increments are
//! correct even where a plain one would have been, so the approximation is a
//! performance choice, never a correctness one. Interprocedural reachability —
//! "which bindings can a spawned body see?" — is not modelled here.
//!
//! The stress half of the plan lives in the runtime's own C suite
//! (`otter-runtime/tests/racy_strings.c`), because it has to call the two
//! symbols directly and run real OS threads; a codegen test only sees IR text.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram};

use otter_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("racy.ot", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(&check_source(src), "racy.ot", CodegenOptions::default())
}

#[test]
fn a_string_never_shared_across_threads_uses_the_plain_retain() {
    // Binding a literal retains it (a no-op for the static string, but the
    // call is the shape under test), and the scope exit releases it.
    let ir = emit("fn main() { var s: string = \"a\"; print_str(s); }");
    assert!(ir.contains("call i8* @otter_str_retain(i8*"), "{ir}");
    assert!(!ir.contains("@otter_str_retain_atomic"), "{ir}");
    assert!(ir.contains("call void @otter_str_release(i8*"), "{ir}");
    assert!(!ir.contains("@otter_str_release_atomic"), "{ir}");
}

#[test]
fn a_string_passed_to_spawn_uses_the_atomic_retain() {
    // The caller retains the thread's argument; that pair crosses threads, so
    // both halves are atomic even though the binding itself never leaks out.
    let ir = emit(
        "fn work(s: string) { print_str(s); } \
         fn main() { \
           var s: string = \"a\"; \
           var h: i64 = thread_spawn(work, s); \
           thread_join(h); \
         }",
    );
    assert!(ir.contains("call i8* @otter_str_retain_atomic(i8*"), "{ir}");
    assert!(!ir.contains("call i8* @otter_str_retain(i8*"), "{ir}");
}

#[test]
fn a_program_with_a_spawn_uses_atomic_refcounts_everywhere() {
    // One spawn flips the whole program: a string local in `main` and a
    // separate helper both use the atomic pair.
    let ir = emit(
        "fn helper() -> string { return \"b\"; } \
         fn work() { var t: string = helper(); print_str(t); } \
         fn main() { \
           var s: string = \"a\"; \
           var h: i64 = thread_spawn(work); \
           thread_join(h); \
           print_str(s); \
         }",
    );
    assert!(ir.contains("call i8* @otter_str_retain_atomic(i8*"), "{ir}");
    assert!(ir.contains("call void @otter_str_release_atomic(i8*"), "{ir}");
    assert!(!ir.contains("call i8* @otter_str_retain(i8*"), "{ir}");
    assert!(!ir.contains("call void @otter_str_release(i8*"), "{ir}");
}

#[test]
fn the_thread_entry_releases_its_argument_atomically() {
    let ir = emit(
        "fn work(s: string) { print_str(s); } \
         fn main() { var s: string = \"a\"; var h: i64 = thread_spawn(work, s); thread_join(h); }",
    );
    // The synthesized entry drops the thread's own reference after the body.
    assert!(ir.contains("call void @otter_str_release_atomic(i8* %arg0_v)"), "{ir}");
}
