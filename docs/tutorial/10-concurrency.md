# 10. Threads, mutexes, and channels

## What you will learn

- How to start a worker and wait for it to finish.
- Which arguments may cross a thread boundary, and how each is passed.
- How a mutex turns scheduling luck into a guaranteed order.
- How a channel blocks, closes, and reports that it is empty.

`thread_spawn(worker, args...)` starts `worker` on a new thread and returns a
handle; `thread_join(handle)` waits for it. A worker returns nothing: the
result comes back through a channel or shared state, not through the thread.

```otter
fn worker(out: Channel[i64]) {
    var i = 1;
    while i <= 3 {
        chan_send(out, i * i);
        i += 1;
    }
}

fn main() {
    var results: Channel[i64] = chan_new(1);
    var handle = thread_spawn(worker, results);

    var total = 0;
    var received = 0;
    while received < 3 {
        total = total + switch chan_recv(results) {
            case Some(v) { v }
            case None { 0 }
        };
        received += 1;
    }

    thread_join(handle);
    chan_free(results);
    print_i64(total);
    print_str("\n");
    // => 14
}
```

`chan_new(1)` makes a channel that holds one value at a time, so a sender
blocks when the slot is full and a receiver blocks when it is empty. The
argument is a capacity in elements, not bytes. With room for one value, each
send waits for the matching receive, and the loop drains the channel as the
producer fills it. `14` is `1 + 4 + 9`.

The type annotation on the binding is required:
`var results: Channel[i64] = chan_new(1);`. `chan_new` takes only a capacity, so
there is no element type to infer from the call; the annotation is what says
what the channel carries.

## Channel rules

The capacity is clamped: values below 1 act as 1, since a zero-slot ring could
never accept a value. The slot size is fixed on first use.

- `chan_send(c, v)` returns `bool`. It blocks while the channel is full, then
  stores the value and returns `true`. On a closed channel it returns `false`
  immediately, including for a sender already blocked when the close arrived.
- `chan_recv(c)` returns `Option[T]`. It blocks while the channel is empty and
  open. It returns `None` only when closed *and* drained; queued values stay
  available after `chan_close`, which wakes every blocked sender and receiver.
- `chan_free(c)` releases the channel and consumes the binding.

A channel and a mutex are linear: each is freed exactly once. A spawned worker
*borrows* a channel argument, so the parent frees it after `thread_join`, never
while the worker may still use it.

A channel element must be copyable and must not be a `string`. `i64`, `bool`,
`f64`, and a struct of them are fine; `string` is refused because the runtime
copies the slot and the two sides would share one refcount.

## Thread arguments

Arguments after the function name are passed in order:
`thread_spawn(worker, results)` calls `worker(results)`. The handle is an
`i64`, which `thread_join(handle)` takes. The runtime copies the argument bytes
into the thread, so the rules differ by type:

- `i64`, `f64`, `bool`, and copyable structs are copied by value.
- A `string` is retained for the thread. The thread releases its reference when
  the body returns, so the parent's string stays valid.
- A `Mutex` or `Channel` is borrowed, never copied. The parent keeps the
  binding and must keep it alive until `thread_join`.
- A linear value such as a slice or a pointer is refused today, because a copy
  would leave the parent and the thread both owning one allocation.

## Mutexes

A mutex serialises a section. `mutex_lock` and `mutex_unlock` borrow, and
`mutex_free` consumes:

```otter
fn worker(gate: Mutex) {
    mutex_lock(gate);
    print_str("worker\n");
    mutex_unlock(gate);
}

fn main() {
    var gate = mutex_new();
    mutex_lock(gate);
    var handle = thread_spawn(worker, gate);

    print_str("main\n");
    mutex_unlock(gate);

    thread_join(handle);
    mutex_free(gate);
    // => main
    // => worker
}
```

`main` holds the mutex while spawning, so the worker blocks until the unlock.
No sleep or timing is involved.

Both of these are correctness arguments, not scheduling luck. A program that
depends on the scheduler is wrong; the tools give you the synchronisation to
avoid one.

## The operations

| Form | Result | Notes |
|---|---|---|
| `thread_spawn(f, args...)` | `i64` handle | Starts `f`; `f` must return `()`. |
| `thread_join(h)` | `()` | Waits for the thread to finish. |
| `mutex_new()` | `Mutex` | Linear; release with `mutex_free`. |
| `mutex_lock(m)` / `mutex_unlock(m)` | `()` | Borrow `m`; block until the lock is free. |
| `mutex_free(m)` | `()` | Consumes `m`. |
| `chan_new(n)` | `Channel[T]` | `n` is elements; values below 1 act as 1. |
| `chan_send(c, v)` | `bool` | Blocks when full; `false` when closed. |
| `chan_recv(c)` | `Option[T]` | Blocks when empty; `None` when closed and drained. |
| `chan_close(c)` | `()` | Wakes blocked senders and receivers. |
| `chan_free(c)` | `()` | Consumes `c`. |

## Common mistakes

- **Letting `chan_new` infer its element type.** `var c = chan_new(1);`
  reports ``error: cannot infer the channel's element type: write
  `var c: Channel[i64] = chan_new(4)` ``. Add the annotation.
- **Putting strings in a channel.** `Channel[string]` reports ``error: a
  channel cannot carry `string` yet: a slot copy would share one refcount
  between the sender and the receiver``. Send an index or a number instead.
- **Returning a value from a worker.** `fn worker() -> i64` reports ``error: a
  thread body must return `()`; `worker` returns `i64` ``. Send the value on a
  channel and return nothing.
- **Passing a slice to a thread.** `thread_spawn(worker, xs)` where `xs` is
  `[]i64` reports ``error: a `[]i64` argument cannot cross a thread boundary
  yet: the runtime copies the value, so the body and the parent would both own
  it``. Copy the data into a channel element by element.
- **Locking a temporary.** `mutex_lock(mutex_new())` reports ``error:
  `mutex_lock` needs a mutex binding, not a temporary``. Bind the mutex first.

## Try it

1. Start two workers that each send their index squared to the same channel,
   receive four values, and print their sum. Join both handles before freeing
   the channel. Hint: the receive order is not fixed, but the sum is.

2. Hold a mutex while spawning a worker that tries to take it, print a line in
   each thread, and confirm the order on several runs. Hint: the parent must
   unlock after spawning and before joining, or the worker never runs.

## How it works

A `Channel` is a runtime ring buffer: a lock, a `not_empty` and a `not_full`
condition variable, the capacity, a slot size fixed on first use, the slot
array, a head index, a count, and a closed flag. Sending waits on `not_full`
while the ring is full, copies into the slot, and signals `not_empty`;
receiving mirrors it. Closing sets the flag and broadcasts both condition
variables. `thread_spawn` builds a struct of the arguments in the caller's
frame, retains each string, and asks the runtime to copy that struct onto the
heap. Codegen synthesizes a thread entry that unpacks the copy, calls the body,
and releases the string references the thread owned. A program with any
`thread_spawn` uses the atomic string retain/release pair, so the last
reference can be dropped safely from either thread.

That is the language. The [introduction](00-intro.md) restates the three
decisions, and the programs under `crates/otter-driver/tests/programs` show the
rest.
