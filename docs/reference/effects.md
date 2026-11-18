# Effects and async

## Effects are declared

**EFFECT-1.** A function's effects are part of its signature. A call whose
effects are not declared at the call site is [E1300](../errors/E1300.md); the
compiler does not infer an effect from the callee and quietly allow it.

**EFFECT-2.** Calling an `extern` function without the `unsafe` marker at the
call site is [E1301](../errors/E1301.md). The declaration being `extern` is not
enough: handing a raw pointer across the C boundary is the dangerous act, and
the act is the call.

`unsafe` is a block, not a mode. Dereferencing an FFI pointer, pointer
arithmetic and other unsafe operations are confined to a block whose extent is
visible in the source. The verifier inventory counts the unsafe sites that
exist today.

## Tasks and scopes

Async is one executor. `async fn` returns `Task[T]` and suspends at `.await`;
`scope` joins or cancels its children before it is left.

**ASYNC-1.** An `await` outside an `async fn` is [E1700](../errors/E1700.md).

**ASYNC-2.** A task escaping its scope is [E1701](../errors/E1701.md).

**ASYNC-3.** A lock held across an `await` is [E1702](../errors/E1702.md). The
checker reports the lock and the await point, because either one alone is fine.

**ASYNC-4.** An `async` trait method is [E1703](../errors/E1703.md). There is no
`Future` trait and there are no async closures; the executor is not pluggable.

## What this buys the verifier

Because effects are declared and tasks are scoped, the verifier can prove that
a pure function has no hidden I/O and that a task cannot outlive the storage it
captures. A missing proof is reported with the obligation, the missing fact and
the remedy in `yashyc verify --json`, rather than as a generic failure.
