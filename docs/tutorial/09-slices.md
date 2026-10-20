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

    switch xs[99] {
        case Some(v) {
            print_i64(v);
        }
        case None {
            print_str("missing");
        }
    }
    print_str("\n");
    // => missing

    free_slice(xs);
}
```

`xs[1]` is an `Option[i64]`, not an `i64`. An index may be out of bounds, and
the type says so: `None` is the answer, and the compiler makes you match it.
There is no panic path for a bad index; `xs[99]` on a three-element slice is
exactly the `None` printed above.

`[]string` owns one reference per element: a literal retains each string it is
given, `push` retains a borrowed value, and `free_slice` releases every element
before the array. That element-aware free is why `free_slice` is not just
`dealloc`: the checker knows the element type and emits the right release.

A slice is linear: it has one owner and cannot be copied. **Passing it to a
function moves it**, because a parameter takes ownership:

```otter
fn total(xs: []i64) -> i64 {
    var sum = 0;
    for x in xs {
        sum += x;
    }
    free_slice(xs);
    return sum;
}

fn main() {
    var xs = []i64 { 1, 2, 3 };
    print_i64(total(xs));
    print_str("\n");
    // => 6
    // free_slice(xs); would be a compile error here: `xs` was moved.
}
```

The callee owns the slice now, so the callee frees it (or leaks it). There is no
borrow syntax for a slice. Parameter passing is also where a `string` behaves
differently: a `string` parameter is borrowed, so the caller keeps its string,
while a slice parameter is moved. The compiler enforces the difference; it does
not guess.

Skipping `free_slice` is not a compile error. The checker tracks ownership, not
reachability: a slice that is never freed is a leak, and a slice freed twice is a
compile error because the second call uses a moved value.

Continue to [10-concurrency](10-concurrency.md).
