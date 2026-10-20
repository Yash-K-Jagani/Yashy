# 9. Slices

A slice is a growable array. `[]i64 { ... }` is a literal, `push` appends,
`len` counts, `for` walks, and `free_slice` releases the backing array.

```otter
fn main() {
    var xs = []i64 { 10, 20 };
    push(xs, 30);
    print_i64(len(xs));
    print_str("\n");
    // => 3

    switch xs[1] {
        case Some(v) {
            print_i64(v);
        }
        case None {
            print_str("missing");
        }
    }
    print_str("\n");
    // => 20

    for x in xs {
        print_i64(x);
        print_str("\n");
    }
    // => 10
    // => 20
    // => 30

    free_slice(xs);
}
```

`xs[1]` is an `Option[i64]`, not an `i64`. An index may be out of bounds, and
the type says so: `None` is the answer, and the compiler makes you match it.
There is no panic path for a bad index.

`[]string` owns one reference per element: a literal retains each string it is
given, `push` retains a borrowed value, and `free_slice` releases every element
before the array. That element-aware free is why `free_slice` is not just
`dealloc`: the checker knows the element type and emits the right release.

A slice is linear. It is moved into a `defer`, freed explicitly, or returned;
it cannot be copied.

Continue to [10-concurrency](10-concurrency.md).
