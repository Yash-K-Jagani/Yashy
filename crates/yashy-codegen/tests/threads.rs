//! Thread, mutex and channel lowering: the call shapes the runtime declares
//! and the one synthesized function a spawn needs.
//!
//! A spawn cannot hand the runtime a Yashy function directly: its parameters
//! are ordinary values and the runtime's entry takes one pointer. The backend
//! therefore copies the arguments into a struct, passes its address and size,
//! and synthesizes `@yashy_thread_entry_N` to unpack the copy and call the body.
//! These tests pin that struct, the per-site entry, and every runtime call.

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

use yashy_codegen::{emit_program, CodegenOptions};

fn check_source(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("threads.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).expect("the program must check")
}

fn emit(src: &str) -> String {
    emit_program(
        &check_source(src),
        "threads.ya",
        CodegenOptions::default(),
    )
}

#[test]
fn spawning_emits_a_call_to_yashy_thread_spawn_with_the_function_pointer() {
    let ir = emit(
        "fn work(n: int) { print(n); } \
         fn main() { var h: int = thread_spawn(work, 7); thread_join(h); }",
    );
    assert!(
        ir.contains("declare i32 @yashy_thread_spawn(i8*, i8*, i64)"),
        "{ir}"
    );
    assert!(
        ir.contains("call i32 @yashy_thread_spawn(i8* @yashy_thread_entry_0, i8* "),
        "{ir}"
    );
    // The runtime handle is `i32`; the source value is `i64`.
    assert!(ir.contains("sext i32"), "{ir}");
}

#[test]
fn the_thread_body_is_emitted_as_its_own_function() {
    let ir = emit(
        "fn work(n: int) { print(n); } \
         fn main() { var h: int = thread_spawn(work, 7); thread_join(h); }",
    );
    assert!(
        ir.contains("define void @yashy_thread_entry_0(i8* %raw)"),
        "{ir}"
    );
    // The entry unpacks the argument and calls the body as an ordinary
    // function; the body keeps its own definition.
    assert!(ir.contains("call void @work(i64 %arg0_v)"), "{ir}");
    assert!(ir.contains("define void @work(i64"), "{ir}");
    assert!(ir.contains("@yashy_thread_entry_0(i8* %raw)"), "{ir}");
}

#[test]
fn arguments_are_copied_into_the_thread_entry_struct() {
    let ir = emit(
        "fn work(n: int, flag: bool) { print(n); if flag { print(\"y\"); } } \
         fn main() { var h: int = thread_spawn(work, 7, true); thread_join(h); }",
    );
    // The parent builds the struct in its own frame: one store per argument.
    assert!(
        ir.contains("= getelementptr inbounds { i64, i1 }, ptr %slot"),
        "{ir}"
    );
    assert!(ir.contains("store i64 7, ptr %t"), "{ir}");
    assert!(ir.contains("store i1 1, ptr %t"), "{ir}");
    // The size handed to the runtime is the struct's own size, read from the
    // layout LLVM computes rather than from hand arithmetic.
    assert!(ir.contains("= getelementptr { i64, i1 }, ptr null, i32 1"), "{ir}");
    assert!(ir.contains("= ptrtoint ptr %t"), "{ir}");
}

#[test]
fn join_emits_a_blocking_call() {
    let ir = emit(
        "fn work() { } fn main() { var h: int = thread_spawn(work); thread_join(h); }",
    );
    assert!(ir.contains("define void @yashy_thread_entry_0(i8* %raw)"), "{ir}");
    // A body with no arguments passes no struct at all.
    assert!(
        ir.contains("call i32 @yashy_thread_spawn(i8* @yashy_thread_entry_0, i8* null, i64 0)"),
        "{ir}"
    );
    assert!(ir.contains("= trunc i64 %t"), "{ir}");
    assert!(ir.contains("call i32 @yashy_thread_join(i32 %t"), "{ir}");
}

#[test]
fn a_locked_mutex_makes_lock_block() {
    let ir = emit(
        "fn main() { \
           var m: Mutex = mutex_new(); \
           m.lock(); \
           m.unlock(); \
           m.free(); \
         }",
    );
    assert!(ir.contains("declare i8* @yashy_mutex_new()"), "{ir}");
    assert!(ir.contains("call i8* @yashy_mutex_new()"), "{ir}");
    assert!(ir.contains("call void @yashy_mutex_lock(i8* %t"), "{ir}");
    assert!(ir.contains("call void @yashy_mutex_unlock(i8* %t"), "{ir}");
    assert!(ir.contains("call void @yashy_mutex_free(i8* %t"), "{ir}");
}

#[test]
fn channels_send_a_slot_and_receive_an_option() {
    let ir = emit(
        "fn main() { \
           var c: Channel[int] = chan_new(4); \
           var ok: bool = c.send(7); \
           var got: Option[int] = c.recv(); \
           c.close(); \
           c.free(); \
         }",
    );
    assert!(ir.contains("declare i8* @yashy_chan_new(i64)"), "{ir}");
    assert!(ir.contains("call i8* @yashy_chan_new(i64 4)"), "{ir}");
    // Sending copies the value into a slot; the runtime reads bytes.
    assert!(ir.contains("= alloca i64"), "{ir}");
    assert!(ir.contains("= call i8 @yashy_chan_send(i8* %t"), "{ir}");
    assert!(ir.contains("icmp ne i8"), "{ir}");
    // Receiving answers an `Option`: a flag plus the element the runtime wrote.
    assert!(ir.contains("= call i8 @yashy_chan_recv(i8* %t"), "{ir}");
    assert!(ir.contains("insertvalue { i8, i64 }"), "{ir}");
    assert!(ir.contains("call void @yashy_chan_close(i8* %t"), "{ir}");
    assert!(ir.contains("call void @yashy_chan_free(i8* %t"), "{ir}");
}

/// Directory holding the `.ya` inputs and `.ll` expectations.
fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// `UPDATE_GOLDEN=1` rewrites the expectation instead of comparing it.
fn updating_goldens() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some()
}

/// Read a file with `\n` line endings whatever the checkout uses.
fn read_lf_only(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// Replace the host-specific `target datalayout` and `target triple` lines.
fn normalize_host_header(ir: &str) -> String {
    ir.lines()
        .map(|line| {
            if line.starts_with("target triple =") {
                "target triple = \"<host>\""
            } else if line.starts_with("target datalayout =") {
                "target datalayout = \"<host>\""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn golden_threads_matches_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("threads.ya"));
    let actual = normalize_host_header(&emit(&input));
    let expected_path = golden_dir().join("threads.expected.ll");
    if updating_goldens() {
        fs::write(&expected_path, &actual)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", expected_path.display()));
        return;
    }
    let expected = normalize_host_header(&read_lf_only(&expected_path));
    assert_eq!(
        actual, expected,
        "generated IR drifted from the pinned contract; \
         rerun with UPDATE_GOLDEN=1 only after checking the diff"
    );
}
