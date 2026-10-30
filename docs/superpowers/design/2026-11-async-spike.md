# Async Spike: Lowering, Executor, and Verifier Interaction

## Decisions

### 1. State-machine lowering shape

Where the state machine is built: `crates/yashy-codegen/src/fn_gen.rs`.

An `async fn` becomes a state machine struct with:
- A `state: i32` field (0 = start, 1..N = suspended at await point N, -1 = done)
- One field per local that lives across an `await` point
- A `poll` method that drives the state machine forward

Locals that do NOT live across an await point stay on the LLVM stack inside `poll`.
Locals that DO live across an await are stored in the frame struct.

The compiler determines liveness-across-await by walking the function body:
every binding assigned before an await and read after it becomes a frame field.

### 2. Executor model

- Single-threaded `block_on` for the main thread: a loop calling `poll()` until `Ready`.
- `spawn` dispatches to a thread pool (std::thread) for CPU-bound work.
- No async traits, no generic runtime in v1.
- One executor per process.

### 3. Cancellation representation

- Cancellation is cooperative: a dropped future sets a `cancelled: bool` flag.
- The state machine checks the flag at each await point and returns `Ready(Cancelled)` early.
- No implicit cancellation of in-flight I/O.

### 4. Verifier interaction

- An `await` point is NOT an `io` effect: the suspension itself does no I/O.
- A suspended borrow IS a typestate question: a `&mut` held across an await
  must remain valid, which means the verifier tracks the borrow as live
  until the await completes.
- `async fn` may be declared `pure` only if it contains no `io` calls between
  await points.

### Cut from v1

- Async traits
- Generic executors
- Structured concurrency (nurseries)
- Cancellation tokens beyond cooperative drop
- `select!` macro
- Async iterators

### Where in the code

- State machine: `fn_gen.rs` (codegen)
- Executor: new `crates/yashy-async` (runtime + executor)
- Verifier: `yashy-verify/src/pass/effects.rs` (await is not io)
- Typestate: `yashy-verify/src/pass/typestate.rs` (suspended borrows)
