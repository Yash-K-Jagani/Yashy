# 17. Async

## What you will learn

- What an `async fn` is: a function whose call returns a `Task[T]`.
- How `.await` drives a task and keeps locals alive across the suspension.
- `spawn` and `scope`: structured concurrency, where children are joined
  or cancelled before the scope is left.
- Why a lock held across `.await` is a compile error, not a style warning.
- `yashy run --debug` and `yashy tasks`: the live-task table a hung
  program can be debugged with.

An `async fn` does not run when it is called. The call allocates a frame,
captures the arguments, and returns a task handle; nothing in the body has
executed yet. `.await` is what runs it: it drives the task until it either
finishes, in which case `.await` yields its result, or suspends, in which
case the driver runs whatever is queued next.

```yashy
async fn inner(int x) -> int {
    return x + 1;
}

async fn main_task() -> int {
    let int child = inner(1).await;
    return 41 + child;
}

fn main() {
    print(block_on(main_task()));
    // => 43
}
```

`block_on` is the top-level driver: it polls the task it is given until the
task is ready. `main` is a synchronous function, so it cannot `.await`; this
is why the program ends in `block_on` rather than calling the async function
directly.

Locals that are live across an `.await` live in the task's frame, not on the
stack. The frame is heap memory owned by the task, and it is freed when the
task completes, is dropped or is cancelled.

## `spawn` and `scope`

A task that nobody drives makes no progress. `spawn` starts one on the
executor and returns its handle, and `.await` on that handle drives it:

```yashy
async fn inner(int x) -> int {
    return x + 1;
}

async fn worker() -> int {
    let int doubled = inner(20).await;
    return doubled * 2;
}

async fn main_task() -> int {
    let int before = 41;
    let int child = inner(1).await;
    let int after = before + child;
    let int from_worker = 0;
    scope {
        let task = spawn(worker);
        let int done = task.await;
        from_worker = done;
    }
    return after + from_worker;
}

fn main() {
    let int result = block_on(main_task());
    print(result);
    // => 85
}
```

`scope` is the structured part: every task spawned inside a scope is
registered on it, and leaving the scope joins or cancels them, in reverse
spawn order, before execution continues. A task handle may not escape its
scope — returning one, storing one in a field, or pushing one into a
collection that outlives the scope is `E1701`. The child started in the
example completes inside the scope because it is awaited there; a child that
is never awaited is driven home by the scope itself.

## Holding a lock across `.await`

A mutex held while a task suspends is a deadlock waiting for its timing: the
lock is `Locked` in the verifier's typestate, and the next statement
suspends, so nobody else can take the lock in the meantime. That is `E1702`:

```text
main.ya:4:5
error[E1702]: mutex is locked across an await
    lock(m);
    g().await;
    ^^^^^^
  docs: https://yashy.dev/errors/E1702
```

The fix is to release before suspending, or to register the release with
`defer` so the cancellation path also releases it. A cancelled task runs its
`defer` bodies, which is why this is safe: the defer is the release the
verifier needs to see on every path out of the scope.

## The cap

One executor. A single-threaded mode that polls inline on the caller of
`block_on`, and a thread-pool mode; the same binary runs both, selected by
`YASHY_EXECUTOR_THREADS`. There is no pluggable runtime, no `async` method
in a trait (`E1703` — traits are requirement sets, and a task handle is
linear state), no `Future` trait in user code, and no async closures: `spawn`
takes a function and its arguments, and the capture rule that governs threads
governs tasks too (`E1402` for a bare linear capture).

## Debugging a hung program

`yashy run --debug` runs the program with `YASHY_DEBUG=1`: on a panic or an
arithmetic fault, the executor prints its live tasks before the process ends.
`yashy tasks` goes further — it prints the table while the program runs:

```text
tasks: 2
task 1 main_task resume=0 state=running
task 2 blocked_on_channel resume=0 state=pending
```

`resume` is the task's resume index, the next suspension point its poll
dispatches on; `state` is `running` while the task is inside its poll and
`pending` while it is queued or suspended. The table is the difference
between a program that hangs silently and one that tells you which task is
stuck and where it suspended.
