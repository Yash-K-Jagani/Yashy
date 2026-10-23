//! Threads, mutexes and channels at the checking edge.
//!
//! `thread_spawn`, `thread_join`, `mutex_new` and `chan_new` are free
//! constructors; a mutex or channel handle then carries `lock`, `unlock`,
//! `send`, `recv`, `close` and `free` as methods. The method layer lowered
//! those operations to the same runtime entry points the old flat spellings
//! used, so the ownership rules did not change with the spelling.
//!
//! What these pin, in order: a spawned body is an ordinary function checked on
//! its own; `thread_spawn` needs that function and its arguments to agree; a
//! channel's element type is fixed by its binding and checked on every send and
//! receive; handles are linear; and receiving answers `Option[T]`, because the
//! only value a closed and drained channel can give back is `None`.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file)
}

fn assert_rejected(src: &str, needle: &str) {
    match check(src) {
        Ok(_) => panic!("expected an error containing `{needle}`"),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "expected an error containing `{needle}`, got {diagnostics:?}"
        ),
    }
}

#[test]
fn a_thread_body_is_checked_like_any_other_function() {
    check(
        "fn work(n: int) { print_i64(n); } \
         fn main() { var h: int = thread_spawn(work, 7); thread_join(h); }",
    )
    .expect("a body with a matching parameter is accepted");

    // The body is an ordinary function, so an error inside it is reported even
    // though nothing but `thread_spawn` refers to it.
    assert_rejected(
        "fn work(n: int) { print_i64(true); } \
         fn main() { var h: int = thread_spawn(work, 7); thread_join(h); }",
        "expected `int`, found `bool`",
    );
}

#[test]
fn spawn_requires_a_function_and_matching_arguments() {
    assert_rejected(
        "fn main() { var x: int = 1; var h: int = thread_spawn(x); }",
        "`thread_spawn` needs a function",
    );
    assert_rejected(
        "fn work(n: int) { print_i64(n); } fn main() { var h: int = thread_spawn(work); }",
        "expected 1 arguments, found 0",
    );
    assert_rejected(
        "fn work(n: int) { print_i64(n); } fn main() { var h: int = thread_spawn(work, true); }",
        "expected `int`, found `bool`",
    );
    assert_rejected(
        "fn work(n: int) -> int { return n; } \
         fn main() { var h: int = thread_spawn(work, 1); }",
        "a thread body must return `void`",
    );
}

#[test]
fn channel_element_type_is_fixed_at_creation() {
    check(
        "fn main() { \
           var c: Channel[int] = chan_new(4); \
           var ok: bool = c.send(1); \
           var got: Option[int] = c.recv(); \
           c.close(); \
           c.free(); \
         }",
    )
    .expect("a channel of int round-trips");

    assert_rejected(
        "fn main() { var c: Channel[int] = chan_new(4); c.send(true); }",
        "expected `int`, found `bool`",
    );
    assert_rejected(
        "fn main() { var c: Channel[float] = chan_new(4); var n: int = c.recv(); }",
        "found `Option[float]`",
    );
    assert_rejected(
        "fn main() { var c = chan_new(4); }",
        "cannot infer the channel's element type",
    );
    // A string slot would be one pointer with one refcount, shared by sender
    // and receiver with no owner; the first release would dangle the other.
    assert_rejected(
        "fn main() { var c: Channel[string] = chan_new(1); }",
        "cannot carry `string`",
    );
}

#[test]
fn mutex_and_channel_handles_are_linear() {
    check(
        "fn main() { \
           var m: Mutex = mutex_new(); \
           m.lock(); \
           m.unlock(); \
           m.free(); \
         }",
    )
    .expect("lock and unlock borrow, free consumes");

    assert_rejected(
        "fn main() { var m: Mutex = mutex_new(); var n: Mutex = m; m.free(); }",
        "without `move`",
    );
    assert_rejected(
        "fn main() { var m: Mutex = mutex_new(); m.free(); m.free(); }",
        "moved",
    );
    assert_rejected(
        "fn main() { var c: Channel[int] = chan_new(1); var d: Channel[int] = c; }",
        "without `move`",
    );
}

#[test]
fn recv_yields_an_option_not_a_blocking_value() {
    check("fn main() { var c: Channel[int] = chan_new(1); var got: Option[int] = c.recv(); }")
        .expect("recv is an Option");
    assert_rejected(
        "fn main() { var c: Channel[int] = chan_new(1); var n: int = c.recv(); }",
        "found `Option[int]`",
    );
}

#[test]
fn a_spawn_argument_may_share_a_handle_or_a_string() {
    // A channel argument is borrowed, not moved: the parent still frees it
    // after the join, which is the only way two threads share a handle.
    check(
        "fn work(c: Channel[int]) { c.close(); } \
         fn main() { \
           var c: Channel[int] = chan_new(1); \
           var h: int = thread_spawn(work, c); \
           thread_join(h); \
           c.free(); \
         }",
    )
    .expect("a channel handle is shared with the thread and freed by the parent");

    // A string is copy-typed, so the parent keeps its binding while the thread
    // holds a retained reference.
    check(
        "fn work(s: string) { print_str(s); } \
         fn main() { \
           var s: string = \"hello\"; \
           var h: int = thread_spawn(work, s); \
           thread_join(h); \
           print_str(s); \
         }",
    )
    .expect("a string is shared by retain");
}

#[test]
fn a_linear_container_cannot_cross_a_thread_boundary_yet() {
    assert_rejected(
        "fn work(xs: []int) { } \
         fn main() { var xs: []int = []int { 1, 2 }; var h: int = thread_spawn(work, xs); }",
        "cannot cross a thread boundary",
    );
}
