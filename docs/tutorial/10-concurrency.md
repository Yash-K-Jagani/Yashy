# 10. Threads, mutexes, and channels

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
producer fills it.

The type annotation on the binding is required:
`var results: Channel[i64] = chan_new(1);`. `chan_new` takes only a capacity, so
there is no element type to infer from the call; the annotation is what says
what the channel carries. `chan_recv` answers `Option[T]`: it is `None` only when
the channel is closed and empty, which is how a consumer knows to stop. A
channel element must be copyable -- `i64`, `bool`, `f64`, a pointer-free struct
-- and `string` is refused, because the runtime copies elements and a string
needs a retain.

A spawned worker returns nothing (the result comes back through the channel)
and its parameters are the arguments after the function name, in order:
`thread_spawn(worker, results)` calls `worker(results)`. The handle
`thread_spawn` returns is an `i64`, and `thread_join(handle)` takes it.

A channel and a mutex are linear: each is freed exactly once. A spawned worker
*borrows* a channel argument, so the parent frees it after `thread_join`, never
while the worker may still use it.

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

The order is guaranteed: `main` holds the mutex when it spawns the worker, so
the worker's `mutex_lock` blocks until `main` unlocks. The worker prints
second without any sleep or timing assumption.

Both of these are program correctness arguments, not scheduling luck. If a
program depends on the scheduler, it is wrong; the tools give you the
synchronisation to avoid that.

That is the language. The [introduction](00-intro.md) restates the three
decisions, and the programs under `crates/otter-driver/tests/programs` show the
rest.
