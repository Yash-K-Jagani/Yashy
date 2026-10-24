# 7. Memory and ownership

## What you will learn

- How to allocate and release a block of heap memory.
- What `move` transfers, and what the compiler refuses after it.
- When a `defer` body runs, in what order, and what it captures.
- Why an unfreed allocation is a leak rather than an error.

`alloc(n)` returns a `*mut int` pointing at `n` slots; `p.free()` releases
it. Indexing works: `buffer[0] = 7` writes the first slot.

```yashy
fn main() {
    let buffer = alloc(4);
    buffer[0] = 7;
    print(buffer[0]);
    println();
    // => 7

    let owned = move buffer;
    defer {
        owned.free();
        println("released");
    }
    println("still inside");
    // => still inside
    // => released
}
```

The slots are **not** zeroed. `alloc` asks the C allocator for raw memory, and
the bytes are whatever was there before. Write a slot before you read it;
`buffer[0] = 7` above is what makes the following read meaningful. A read of
untouched memory is not a diagnostic, and no value is promised.

Three rules make this safe without a garbage collector:

- **One owner.** `buffer` owns the block. `move buffer` transfers ownership to
  `owned`; after the move, using `buffer` is a compile error. A value is
  released once, by its owner.
- **`defer` runs when the scope exits**, in reverse order of registration, and
  sees the values that were live when the `defer` was reached. That is what
  makes `owned.free()` run at the end of `main` even though the line appears
  before the last statement.
- **A pointer is linear.** It cannot be copied, so it cannot be freed twice.
  Using it after it is moved is a compile error. Not freeing it is not: the
  checker tracks ownership, not reachability, so a forgotten `.free()` is a
  leak rather than a diagnostic.

`defer` takes a block: `defer { owned.free(); }`. The spec's shorter
`defer expression;` spelling is not accepted by this parser; writing
`defer owned.free();` reports ``error: expected `{`, found an identifier``.

## The operations

| Operation | Type or form | Ownership effect |
|---|---|---|
| `alloc(n)` | `*mut int` | Creates one owner; memory is uninitialised. |
| `p.free()` | `void` | Releases the block; the binding is moved. |
| `move x` | same type as `x` | Transfers the value; `x` is dead afterwards. |
| `let y = move x;` | pointer | Gives the pointer a second name; the source is dead. |
| `defer { ... }` | statement | Runs at scope exit, last registered first. |
| `arena_new()` | arena handle | A bump allocator; one release for many blocks. |
| `arena_alloc(a, n)` | `*mut int` | The arena owns the block, not the binding. |
| `arena_free_all(a)` | `void` | Releases every block from that arena. |

### One release spelling

`.free()` is the release for every owning value:

| Value | `.free()` | Notes |
|---|---|---|
| `*mut T` | yes | Releases the block; consumes the binding. |
| `[]T` | yes | Releases the elements that own a reference, then the array. |
| `map[K]V` | yes | Releases keys and values, then the table. |
| `string` | no | The binding releases its own reference; there is nothing to free. |

A `string` binding releases its reference when its scope ends, so a `string`
never leaks. Calling `s.free()` reports
``error: a `string` releases itself: there is nothing to free``.

A `defer` captures a linear value when the `defer` is reached, not when it
runs. That is why a move after the registration is refused: the deferred body
already holds the pointer, and a second owner would free it twice. The error is
the ordinary moved-value message:

```text
let b = alloc(1);
defer { b.free(); }
let c = move b;          // error: value `b` has already been moved
```

Copy-typed values are different. A `defer` that reads an `int` sees the value
at scope exit, because there is nothing to capture and no free to get wrong.
This distinction is deliberate: capturing every local would cost a slot per
binding per defer, while capturing only the values whose ownership matters is
enough for safety.

## Arenas

An arena is a second way to get heap memory. `arena_new()` returns a handle,
`arena_alloc(arena, n)` hands out `n` slots from it, and `arena_free_all(arena)`
releases everything at once. There is no per-block free. A block from an arena
is owned by the arena, so moving its pointer transfers only the handle; the
arena keeps the storage alive until it is freed. This is useful when a group of
allocations shares one lifetime and freeing each one individually would be
noise.

## Common mistakes

- **Freeing twice.** `xs.free()` called twice, or `b.free()` after
  `b.free()`, reports ``error: value `xs` has already been moved`` (or `b`,
  for the pointer). The first call consumed the binding.
- **Passing a pointer to a function and then freeing it.** A parameter takes
  ownership, so `take(b); b.free();` reports ``error: value `b` has already
  been moved``. The callee owns the block now; it frees it or leaks it.
- **Writing `defer b.free();`.** The parser expects a block and reports
  ``error: expected `{`, found an identifier``. Write `defer { b.free(); }`.
- **Forgetting to free.** No diagnostic. The checker follows ownership, not
  reachability, so the block stays allocated until the process exits. That is
  a leak, which is visible with the runtime's leak report but not at compile
  time.
- **Reading an unwritten slot.** No diagnostic. The value is whatever the
  allocator returned; write before you read.

## Try it

1. Allocate four slots, fill them with the squares of 1 through 4 in a loop,
   sum them, and release the block with a `defer`. Print the sum. Hint: keep
   the loop index and the slot index the same, and do not read a slot the loop
   has not written.

2. Deliberately free a pointer twice and read the second diagnostic. Then fix
   it by moving the pointer into a second binding and freeing only the new
   owner. Hint: the error points at the second use, not the first.

## How it works

`alloc` becomes one call to the runtime's allocator, and `.free()` one call to
its release. `move` produces the value as an ordinary read and then overwrites the
source binding's stack slot with null, so even if the checker were bypassed the
second free would be a free of null rather than a double free. A `defer`
registers its body in the current scope; a normal exit, `return`, `break`, or
`continue` runs that scope's bodies in reverse order. A panic runs the bodies of
the function that panicked, innermost scope first; a `defer` in a caller does
not run yet, because that needs unwinding across frames (ledger L4). None of
this requires a tracing collector: the program says who owns each block, and
the checker refuses the two mistakes that would free it twice or use it after
it is gone.

Continue to [08-strings](08-strings.md).
