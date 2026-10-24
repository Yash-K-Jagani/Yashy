# 9. Arrays

## What you will learn

- How to build an array, append to it, walk it, and free it.
- Why indexing returns `Option[T]` instead of panicking.
- What passing an array to a function does to ownership.
- How a `[]string` owns the strings inside it.
- How a fixed array `[T; N]` differs from a growable `[]T`, and what a view
  `a[start..end]` borrows.

An array is a growable sequence. `[]int { ... }` is a literal, `xs.push(v)`
appends, `xs.len()` counts, `for` walks, and `xs.free()` releases the backing
storage. The runtime still calls the three-word value it stores a *slice
header*; the public surface is arrays and methods.

```yashy
fn main() {
    var xs = []int { 10, 20 };
    xs.push(30);
    print(xs.len());
    println();
    // => 3

    switch xs[1] {
        case Some(v) {
            print(v);
        }
        case None {
            print("missing");
        }
    }
    println();
    // => 20

    for x in xs {
        print(x);
        println();
    }
    // => 10
    // => 20
    // => 30

    switch xs[99] {
        case Some(v) {
            print(v);
        }
        case None {
            print("missing");
        }
    }
    println();
    // => missing

    xs.free();
}
```

`xs[1]` is an `Option[int]`, not an `int`. An index may be out of bounds, and
the type says so: `None` is the answer, and the compiler makes you match it.
There is no panic path for a bad index; `xs[99]` on a three-element array is
exactly the `None` printed above. A negative index is also a miss, not a
wrapped read from the end.

`xs.push(v)` appends one element. The first push allocates room for four, and
later growth doubles the capacity, so building an array one element at a time
costs amortised constant work per element. `xs.pop()` removes and returns the
last element as an `Option[T]`; it is `None` on an empty array. `pop` refuses a
`[]string` today, because the array owns the element's reference and a removed
element would be handed out without that ownership.

The query methods answer without moving the array. `xs.get(i)` is exactly
`xs[i]`. `xs.first()` and `xs.last()` are `xs.get(0)` and
`xs.get(xs.len() - 1)`, so an empty array answers `None` rather than reading
before the start. `xs.is_empty()` is `xs.len() == 0`. `xs.set(i, v)` writes one
element and returns `true`, or returns `false` and leaves the array unchanged
when `i` is out of range. Setting a `[]string` element is refused for now: the
array owns the old reference, and replacing it needs a retain of the new string
and a release of the old one.

The edit methods change the array in place. `xs.insert(i, v)` accepts
`0..=xs.len()`: inserting at `len` is an append, and anything past that returns
`false`. `xs.remove(i)` accepts `0..len` and answers `Some(value)`, or `None`
for an out-of-range index. `xs.clear()` sets the length to zero but keeps the
backing array, so a later push reuses its capacity. `xs.reverse()` swaps the
elements end to end, and `xs.sort()` orders an `[]int` ascending. `insert` and
`remove` refuse `[]string` for the same ownership reason as `pop`; `clear` on a
`[]string` is allowed because it releases every element before dropping the
length.

`xs.contains(v)` and `xs.index_of(v)` search by value. `contains` answers a
`bool`; `index_of` answers `Some(i)` for the first match and `None` when the
value is absent. Both work for `int`, `float`, `bool`, and `string` elements; a
struct or enum element has no equality and is rejected.

`xs.extend(other)` appends every element of `other`. It **consumes** `other`:
an array binding is moved, and the runtime frees its backing storage after copying.
An array literal or a call result works too and is freed the same way. This is
why `xs.extend(ys)` makes `ys` unusable afterwards, just like passing it to a
function.

`xs.slice(start, end)` copies the clamped range into a fresh array. Both bounds
are clamped to `0..=xs.len()`, and an end before the start gives an empty array.
The result is independent: freeing it does not free the source, and for a
`[]string` each copied element is retained so both arrays own a reference.

## Ownership

An array is linear: it has one owner and cannot be copied. **Passing it to a
function moves it**, because a parameter takes ownership:

```yashy
fn total(xs: []int) -> int {
    var sum = 0;
    for x in xs {
        sum += x;
    }
    xs.free();
    return sum;
}

fn main() {
    var xs = []int { 1, 2, 3 };
    print(total(xs));
    println();
    // => 6
    // xs.free(); would be a compile error here: `xs` was moved.
}
```

The callee owns the array now, so the callee frees it (or leaks it). There is no
borrow syntax for an array. Parameter passing is also where a `string` behaves
differently: a `string` parameter is borrowed, so the caller keeps its string,
while an array parameter is moved. The compiler enforces the difference; it does
not guess.

`for x in xs` reads the header on every iteration, so the loop keeps the array
alive for its body. Moving `xs`, including calling `xs.free()`, inside the
loop reports ``error: cannot move `xs` while it is being iterated``. Free the
array after the loop, never inside it.

Skipping `xs.free()` is not a compile error. The checker tracks ownership, not
reachability: an array that is never freed is a leak, and an array freed twice is
a compile error because the second call uses a moved value.

## Arrays of strings

`[]string` owns one reference per element. A literal retains each string it is
given; `push` retains a borrowed value and takes over an owned temporary; and
`xs.free()` releases every element before the array. That element-aware free is
why it is not just `dealloc`: the checker knows the element type and
emits `yashy_slice_free_strings` for `[]string` and the plain array free for
everything else. Popping a `[]string` is refused for the same reason: the
element's reference would leave the array without a matching release.

## The operations

| Form | Result | Notes |
|---|---|---|
| `[]T { a, b }` | `[]T` | An empty literal is valid and allocates nothing. |
| `xs.push(v)` | `void` | Appends; grows by doubling from capacity four. |
| `xs.len()` | `int` | Number of elements. |
| `xs[i]` / `xs.get(i)` | `Option[T]` | `None` when `i` is negative or past the end. |
| `xs.first()` / `xs.last()` | `Option[T]` | `None` on an empty array. |
| `xs.is_empty()` | `bool` | `xs.len() == 0`. |
| `xs.set(i, v)` | `bool` | Writes in range; refused for `[]string`. |
| `xs.insert(i, v)` | `bool` | Accepts `0..=len`; refused for `[]string`. |
| `xs.remove(i)` | `Option[T]` | Accepts `0..len`; refused for `[]string`. |
| `xs.clear()` | `void` | Length zero; keeps the backing array. |
| `xs.reverse()` | `void` | Reverses in place. |
| `xs.sort()` | `void` | Ascending; `[]int` only in this phase. |
| `xs.contains(v)` | `bool` | Element types with equality. |
| `xs.index_of(v)` | `Option[int]` | First match; `None` when absent. |
| `xs.extend(other)` | `void` | Consumes `other`; strings are retained. |
| `xs.slice(start, end)` | `[]T` | Fresh array over the clamped range. |
| `xs.pop()` | `Option[T]` | Removes the last element; refused for `[]string`. |
| `for x in xs { ... }` | statement | Reads each element; the array stays borrowed. |
| `xs.free()` | `void` | Releases the elements (if strings) and the array. |
| `[T; N]` | type | Fixed array in the frame; length is part of the type. |
| `[a, b, c]` | `[T; 3]` | Fixed array literal; at least one element. |
| `grid[i]` / `grid.get(i)` | `Option[T]` | Checked against the array's own length. |
| `grid.first()` / `grid.last()` | `Option[T]` | `None` only for a view of length zero. |
| `grid.len()` | `int` | The length in the type, or the view's word. |
| `a[start..end]` | `[T]` view | Clamped range borrowing `a` for the rest of the function. |

The element type is part of the array type, so `[]int` and `[]string` are
different types and an array cannot change element type. The element type is
written in the literal, and `[]int {}` is the empty array of `int`.

## Fixed arrays and views

`[T; N]` is an array whose length is part of its type. The elements live in the
frame that declares the binding, so there is no allocation, no owner, and
nothing to free: `[int; 3]` and `[int; 4]` are two different types, and a
function that returns the first cannot hand back the second.

```yashy
fn show(o: Option[int]) -> int {
    return switch o {
        case Some(v) { v }
        case None { 0 - 1 }
    };
}

fn main() {
    const grid: [int; 3] = [1, 2, 3];

    print(grid.len());
    println();
    // => 3

    print(show(grid[1]));
    println();
    // => 2

    print(show(grid[3]));
    println();
    // => -1

    const head = grid[0..2];
    print(head.len());
    println();
    // => 2

    var total = 0;
    for value in head {
        total = total + value;
    }
    print(total);
    println();
    // => 3
}
```

`grid[1]` is an `Option[int]` for the same reason `xs[1]` is: the index may be
past the end, and the type says so. There is no panic path, and the backend
guards the load with a `select` so even the machine code never reads outside the
array.

`grid[0..2]` is a **view**: a length and a pointer into `grid`, not a copy.
`0` and `2` are clamped to `grid.len()`, so `grid[0..99]` is the whole array and
`grid[2..1]` is empty rather than an error. Iterating a view walks the elements
it covers; iterating the array itself walks all of them.

A view owns nothing, so the array it borrows must not move or change while the
view exists. The checker treats a view as borrowing its source **until the
function returns**:

```yashy
fn main() {
    var xs = []int { 1, 2, 3 };
    const view = xs[0..2];
    // xs.push(4);   error: cannot change `xs` while a view of it is alive
    // move xs;      error: cannot move `xs` while a view of it is alive
    print(view.len());
    println();
    // => 2
}
```

Views are not a type you can write. `fn f(x: [int])` and `fn f() -> [int]` are
both rejected, because Yashy has no lifetime syntax to say which array a
borrowed view came from, and a view that outlived its array would read freed
storage. A view is inferred at the range and lives in the function that made
it; pass the array itself, or copy the elements into a `[]T`, to hand the data
to another function.

The borrow also means a `[]T` you take a view of **cannot be freed in that
function**. `xs.free()` under a live view is refused, and the view lives until
the function returns, so an owned array that has been viewed is never released:
a leak, not a corruption. This is the honest cost of views without lifetimes:

```yashy
fn middle_sum(xs: []int) -> int {
    const mid = xs[1..3];
    var sum = 0;
    for value in mid {
        sum = sum + value;
    }
    return sum;      // `xs` is not freed: `mid` still points into it
}

fn main() {
    // => 50
    print(middle_sum([]int { 10, 20, 30, 40 }));
    println();
}
```

A fixed array has no `free`, so the restriction costs nothing there: views are
at their best over `[T; N]`, where the storage is in the frame and the view can
never see it released early.

The two forms divide the work like this:

| Form | Storage | Length | Ownership | Copy |
|---|---|---|---|---|
| `[T; N]` | the frame | constant in the type | none | yes, by value |
| `[T]` (view) | borrowed from an array | a word it carries | none | yes, by value |
| `[]T` | the heap | a word it carries | one owner, freed with `free()` | no |

A fixed array is a value, so `var copy = grid;` is allowed and copies the
elements. A view is a value too, but copying a view copies a pointer, which is
safe only because neither copy can outlive the array. The element types are
limited to the built-in numbers, `bool`, and `usize`: copying a fixed array
copies its bytes, which is exact for those and not yet defined for a `string`
that owns a reference.

## Common mistakes

- **Treating an index as a value.** `print(xs[0])` reports
  ``error: expected `int`, found `Option[int]` ``. Match the `Option`, or use
  `pop` when the last element is the one you want.
- **Copying an array binding.** `var ys = xs;` reports
  ``error: cannot assign linear value `xs` without `move` ``. Do not follow
  that suggestion with `var ys = move xs;` today: it builds invalid IR and
  clang reports `null must be a pointer type`. Keep one binding, or pass the
  array to a function, which transfers it correctly.
- **Assigning to an element with `=`.** `xs[0] = 9;` reports ``error: cannot
  assign to an array element: an array does not expose its storage; use
  `xs.set(0, 9)` ``. The method returns a `bool` so an out-of-range index is a
  value you can handle, not a fault.
- **Freeing twice.** A second `xs.free()` reports
  ``error: value `xs` has already been moved``. The first call consumed the
  binding.
- **Freeing inside a `for` loop.** `xs.free()` in the body reports
  ``error: cannot move `xs` while it is being iterated``. Move the call after
  the loop.
- **Sorting something other than `[]int`.** `xs.sort()` on a `[]string` reports
  ``error: `sort` is only implemented for arrays of int``. Sort the array you
  have, or build a numeric key array first.
- **Using an extended array again.** `xs.extend(ys); print(ys.len());`
  reports ``error: value `ys` has already been moved``. `extend` consumed `ys`;
  use `xs` after the call, or make a copy first with `ys.slice(0, ys.len())`.
- **Changing an array a view borrows.** `const v = xs[0..2]; xs.push(3);`
  reports ``error: cannot change `xs` while a view of it is alive``. Any write
  -- `push`, `pop`, `set`, `insert`, `remove`, `clear`, `reverse`, `sort`,
  `extend`, `free`, or an assignment -- is refused until the function returns.
- **Writing a view type.** `fn total(xs: [int]) -> int` reports
  ``error: a view type cannot be written``. Yashy cannot say which array a
  borrowed parameter would point at, so views stay in the function that made
  them; take `[]int` and free it, or copy the elements first.
- **A fixed array of strings.** `[\"a\", \"b\"]` reports
  ``error: a fixed array cannot hold `string` ``. Copying the array would
  duplicate the handles without taking a reference; use a `[]string` today.
- **Assigning to a fixed array element.** `grid[0] = 9;` reports ``error:
  cannot assign to index of `[int; 3]` ``. A fixed array is built once, at its
  literal or its return; use a `[]int` when the elements have to change.

## Try it

1. Build a `[]int` of the numbers 1 through 5 with a loop and `push`, then use
   another loop to sum them and print the total. Free the array once, after
   both loops. Hint: `xs.push(i)` appends the loop index, which is already the
   value you want.

2. Build a `[]string` holding `"a"`, `"b"`, and `"c"`, print each element, and
   free the array. Then change the values to strings you built with `+` and
   confirm the output is unchanged. Hint: both literals and owned strings are
   retained by the array, so `xs.free()` handles either.

3. Write a function that returns `[int; 4]` holding the first four square
   numbers, then print `grid.len()`, `grid.last()`, and the sum of
   `grid[1..3]`. Hint: build the literal directly with `return [...]`; a fixed
   array is a value, so returning one copies the elements and nothing is freed.

4. Predict the output, then run it:

   ```yashy
   fn main() {
       const grid = [1, 2, 3];
       const whole = grid[0..99];
       const none = grid[3..3];
       print(whole.len());
       println();
       // => 3
       print(none.len());
       println();
       // => 0
   }
   ```

   Answer: `3` and `0`. Bounds are clamped, so `0..99` is the whole array and
   `3..3` is empty, not an error.

## How it works

An array value is represented by the three-word slice header
`{ i8*, int, int }`: data pointer, length, capacity. The header is a value stored
in the binding's stack slot; the backing storage is one heap block owned by that
binding. `push` takes the address
of the header because growing must update the length and capacity in place. The
runtime's `get` and `pop` return a flag and copy the element into an output
slot only on success; codegen turns that flag into the `Some`/`None` tag, which
is why a bad index is a value rather than a trap. `set` uses the same bounds
test and copies the new value only in range. `xs.free()` dispatches on the
element type: `[]string` calls `yashy_slice_free_strings`, which releases each
handle, then frees the array; every other type calls `yashy_slice_free`.

A fixed array is an LLVM aggregate `[N x T]` with no indirection: the elements
sit in the binding's `alloca`, a literal is a chain of `insertvalue`s, and
`grid[i]` takes the aggregate's address, tests `0 <= i < N`, and loads an element
through a `select`ed in-range index so the load is always defined. A view is
`{ i8*, int }` -- the address of its first element plus a length; `a[start..end]`
clamps both bounds with `select`s and computes the pointer with `mul` and
`getelementptr`, and never touches the source's header. Neither form calls the
allocator, which is why neither has a `free`.

Continue to [10-concurrency](10-concurrency.md).
