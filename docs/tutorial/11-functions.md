# 11. Functions as values

## What you will learn

- How to write a function type: `fn(i64) -> i64`.
- How to hand a function to another function and call it there.
- How to keep a function in a binding and call it through the binding.
- What the checker insists on, and what a function value is at runtime.

A function name is a value. `fn(i64) -> i64` is the type of a function that
takes an `i64` and returns an `i64`, and a top-level function's name has that
type, so it can be passed to another function, stored in a binding, and called
later.

```otter
fn square(x: i64) -> i64 {
    return x * x;
}

fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    return f(x);
}

fn main() {
    print_i64(apply(square, 7));
    print_str("\n");
    // => 49
}
```

`apply` does not know what `square` is. Its first parameter says "a function
of one `i64` that returns one `i64`", and `f(x)` is a call through that
value. The value that arrives is the name `square` itself: a function name
used where a value is expected *is* that function.

## A function in a binding

A binding can hold a function, and the binding can be called like one. The
binding's type is the function's type, so the checker knows what the call
returns.

```otter
fn square(x: i64) -> i64 {
    return x * x;
}

fn double(x: i64) -> i64 {
    return x + x;
}

fn main() {
    const f = square;
    print_i64(f(4));
    print_str("\n");
    // => 16

    const g = double;
    print_i64(g(4));
    print_str("\n");
    // => 8
}
```

`const f = square;` binds a function value; `f(4)` calls through it. The
binding's type is the function's type, so the checker knows what the call
returns. `const` says the binding does not change; `var f = square;` would
work just as well, because a function value is a value like any other.

## Writing a function type

The type is `fn(T, U) -> R`: the parameter types, then the return type.

- `fn(i64) -> i64` takes one `i64` and returns one `i64`.
- `fn() -> i64` takes nothing and returns an `i64`.
- `fn(i64, string) -> bool` takes an `i64` and a `string` and returns a
  `bool`.
- `fn(i64)` is a function of one `i64` that returns nothing. The arrow is
  optional, and an omitted arrow means `-> ()`.

The parameter *names* are not part of the type: a function type describes the
calling convention, not the names the callee happens to use.

## What the checker insists on

The type of a function value is checked exactly, in both directions.

- **The types must match.** Handing `apply` a function of a different shape
  reports ``error: expected `fn(i64) -> i64`, found `fn(string) -> i64` ``. A
  function that takes a `string` is not a function that takes an `i64`, even
  though both return an `i64`.
- **The arity must match.** Calling a one-argument function with two
  arguments reports ``error: expected 1 arguments, found 2 ``.
- **A value that is not a function cannot be called.** `const f = 1; f(2);`
  reports ``error: `f` is not a function (it is `i64`) ``.
- **`main` is the entry point, not a value.** `const f = main;` reports
  ``error: `main` is the entry point and cannot be used as a value ``, because
  the entry point is emitted with the operating system's own signature.

## How it works

A function value is a pair: a code pointer and an environment pointer. The
environment is where a closure's captured values would live; a top-level
function captures nothing, so its environment is null.

Using a top-level function as a value makes the compiler synthesize a small
thunk, `__fnvalue_square(i8* env, i64)`, that takes the environment and
ignores it, and calls the real function. The value is that thunk's address
paired with a null environment. A call through a value loads the pair, takes
the two halves, and calls the code pointer with the environment as its first
argument.

A call to a *named* function is still an ordinary direct call: the thunk
exists only when a name is used as a value, and one thunk is shared by every
use of that function.

One deliberate detail: a function that *receives* a function value is not
inlined into its caller. Inlining it would let the optimizer see the value it
was handed and fold the indirect call back into a direct one, quietly turning
a program that calls through a value into one that does not. WebAssembly keeps
`call_indirect` indirect for the same reason. A function value held in a local
binding is left alone: the optimizer can see that binding's value, and folding
that call is an ordinary optimization of an ordinary program.

## Try it

1. Write a function `twice(f: fn(i64) -> i64, x: i64) -> i64` that calls `f`
   twice, and use it with `square` to compute `square(square(4))`.

2. Write three functions of the same type — `negate`, `double` and
   `identity` — and a function `pick(which: fn(i64) -> i64, x: i64) -> i64`
   that calls the one it is given. Call `pick` with each of the three.

## Where this leaves you

That is the language. The [introduction](00-intro.md) restates the three
decisions, and the programs under `crates/otter-driver/tests/programs` show
the rest.
