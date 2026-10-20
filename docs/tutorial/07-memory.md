# 7. Memory and ownership

`alloc(n)` returns a `*mut i64` pointing at `n` zeroed slots; `dealloc(ptr)`
releases it. Indexing works: `buffer[0] = 7` writes the first slot.

```otter
fn main() {
    var buffer = alloc(4);
    buffer[0] = 7;
    print_i64(buffer[0]);
    print_str("\n");
    // => 7

    var owned = move buffer;
    defer {
        dealloc(owned);
        print_str("released\n");
    }
    print_str("still inside\n");
    // => still inside
    // => released
}
```

Three rules make this safe without a garbage collector:

- **One owner.** `buffer` owns the block. `move buffer` transfers ownership to
  `owned`; after the move, using `buffer` is a compile error. A value is
  released once, by its owner.
- **`defer` runs when the scope exits**, in reverse order of registration, and
  sees the values that were live when the `defer` was reached. That is what
  makes `dealloc(owned)` run at the end of `main` even though the line appears
  before the last statement.
- **A pointer is linear.** It cannot be copied, so it cannot be freed twice;
  it must be moved into a `defer` or passed to `dealloc`.

The same ownership applies to strings, slices, maps, channels, and structs that
contain them. A type that owns memory is linear by construction, not by
convention.

Continue to [08-strings](08-strings.md).
