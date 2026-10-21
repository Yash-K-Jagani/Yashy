# 9. Slices

## What you will learn

- How to build a slice, append to it, walk it, and free it.
- Why indexing returns `Option[T]` instead of panicking.
- What passing a slice to a function does to ownership.
- How a `[]string` owns the strings inside it.

A slice is a growable array. `[]i64 { ... }` is a literal, `xs.push(v)`
appends, `xs.len()` counts, `for` walks, and `xs.free()` releases the backing
array.

```otter
fn main() {
    var xs = []i64 { 10, 20 };
    xs.push(30);
    print_i64(xs.len());
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

    xs.free();
}
```

`xs[1]` is an `Option[i64]`, not an `i64`. An index may be out of bounds, and
the type says so: `None` is the answer, and the compiler makes you match it.
There is no panic path for a bad index; `xs[99]` on a three-element slice is
exactly the `None` printed above. A negative index is also a miss, not a
wrapped read from the end.

`xs.push(v)` appends one element. The first push allocates room for four, and
later growth doubles the capacity, so building a slice one element at a time
costs amortised constant work per element. `xs.pop()` removes and returns the
last element as an `Option[T]`; it is `None` on an empty slice. `pop` refuses a
`[]string` today, because the slice owns the element's reference and a removed
element would be handed out without that ownership.

The query methods answer without moving the slice. `xs.get(i)` is exactly
`xs[i]`. `xs.first()` and `xs.last()` are `xs.get(0)` and
`xs.get(xs.len() - 1)`, so an empty slice answers `None` rather than reading
before the start. `xs.is_empty()` is `xs.len() == 0`. `xs.set(i, v)` writes one
element and returns `true`, or returns `false` and leaves the slice unchanged
when `i` is out of range. Setting a `[]string` element is refused for now: the
array owns the old reference, and replacing it needs a retain of the new string
and a release of the old one.

## Ownership

A slice is linear: it has one owner and cannot be copied. **Passing it to a
function moves it**, because a parameter takes ownership:

```otter
fn total(xs: []i64) -> i64 {
    var sum = 0;
    for x in xs {
        sum += x;
    }
    xs.free();
    return sum;
}

fn main() {
    var xs = []i64 { 1, 2, 3 };
    print_i64(total(xs));
    print_str("\n");
    // => 6
    // xs.free(); would be a compile error here: `xs` was moved.
}
```

The callee owns the slice now, so the callee frees it (or leaks it). There is no
borrow syntax for a slice. Parameter passing is also where a `string` behaves
differently: a `string` parameter is borrowed, so the caller keeps its string,
while a slice parameter is moved. The compiler enforces the difference; it does
not guess.

`for x in xs` reads the header on every iteration, so the loop keeps the slice
alive for its body. Moving `xs`, including calling `xs.free()`, inside the
loop reports ``error: cannot move `xs` while it is being iterated``. Free the
slice after the loop, never inside it.

Skipping `xs.free()` is not a compile error. The checker tracks ownership, not
reachability: a slice that is never freed is a leak, and a slice freed twice is a
compile error because the second call uses a moved value.

## Slices of strings

`[]string` owns one reference per element. A literal retains each string it is
given; `push` retains a borrowed value and takes over an owned temporary; and
`xs.free()` releases every element before the array. That element-aware free is
why it is not just `dealloc`: the checker knows the element type and
emits `otter_slice_free_strings` for `[]string` and the plain array free for
everything else. Popping a `[]string` is refused for the same reason: the
element's reference would leave the slice without a matching release.

## The operations

| Form | Result | Notes |
|---|---|---|
| `[]T { a, b }` | `[]T` | An empty literal is valid and allocates nothing. |
| `xs.push(v)` | `()` | Appends; grows by doubling from capacity four. |
| `xs.len()` | `i64` | Number of elements. |
| `xs[i]` / `xs.get(i)` | `Option[T]` | `None` when `i` is negative or past the end. |
| `xs.first()` / `xs.last()` | `Option[T]` | `None` on an empty array. |
| `xs.is_empty()` | `bool` | `xs.len() == 0`. |
| `xs.set(i, v)` | `bool` | Writes in range; refused for `[]string`. |
| `xs.pop()` | `Option[T]` | Removes the last element; refused for `[]string`. |
| `for x in xs { ... }` | statement | Reads each element; the slice stays borrowed. |
| `xs.free()` | `()` | Releases the elements (if strings) and the array. |

The element type is part of the slice type, so `[]i64` and `[]string` are
different types and a slice cannot change element type. The element type is
written in the literal, and `[]i64 {}` is the empty slice of `i64`.

## Common mistakes

- **Treating an index as a value.** `print_i64(xs[0])` reports
  ``error: expected `i64`, found `Option[i64]` ``. Match the `Option`, or use
  `pop` when the last element is the one you want.
- **Copying a slice binding.** `var ys = xs;` reports
  ``error: cannot assign linear value `xs` without `move` ``. Do not follow
  that suggestion with `var ys = move xs;` today: it builds invalid IR and
  clang reports `null must be a pointer type`. Keep one binding, or pass the
  slice to a function, which transfers it correctly.
- **Assigning to an element with `=`.** `xs[0] = 9;` reports ``error: cannot
  assign to a slice element: the slice does not expose its storage; use
  `xs.set(0, 9)` ``. The method returns a `bool` so an out-of-range index is a
  value you can handle, not a fault.
- **Freeing twice.** A second `xs.free()` reports
  ``error: value `xs` has already been moved``. The first call consumed the
  binding.
- **Freeing inside a `for` loop.** `xs.free()` in the body reports
  ``error: cannot move `xs` while it is being iterated``. Move the call after
  the loop.

## Try it

1. Build a `[]i64` of the numbers 1 through 5 with a loop and `push`, then use
   another loop to sum them and print the total. Free the slice once, after
   both loops. Hint: `xs.push(i)` appends the loop index, which is already the
   value you want.

2. Build a `[]string` holding `"a"`, `"b"`, and `"c"`, print each element, and
   free the slice. Then change the values to strings you built with `+` and
   confirm the output is unchanged. Hint: both literals and owned strings are
   retained by the slice, so `xs.free()` handles either.

## How it works

A slice value is the three-word header `{ i8*, i64, i64 }`: data pointer,
length, capacity. The header is a value stored in the binding's stack slot; the
backing array is one heap block owned by that binding. `push` takes the address
of the header because growing must update the length and capacity in place. The
runtime's `get` and `pop` return a flag and copy the element into an output
slot only on success; codegen turns that flag into the `Some`/`None` tag, which
is why a bad index is a value rather than a trap. `set` uses the same bounds
test and copies the new value only in range. `xs.free()` dispatches on the
element type: `[]string` calls `otter_slice_free_strings`, which releases each
handle, then frees the array; every other type calls `otter_slice_free`.

Continue to [10-concurrency](10-concurrency.md).
