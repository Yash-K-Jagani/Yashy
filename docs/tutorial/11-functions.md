# 11. Functions as values

## What you will learn

- How to write a function type: `fn(int) -> int`.
- How to hand a function to another function and call it there.
- How to keep a function in a binding and call it through the binding.
- How to write one function that works for more than one type.
- How a `const fn` is computed by the compiler rather than at run time.
- What the checker insists on, and what a function value is at runtime.

A function name is a value. `fn(int) -> int` is the type of a function that
takes an `int` and returns an `int`, and a top-level function's name has that
type, so it can be passed to another function, stored in a binding, and called
later.

```yashy
fn square(x: int) -> int {
    return x * x;
}

fn apply(f: fn(int) -> int, x: int) -> int {
    return f(x);
}

fn main() {
    print(apply(square, 7));
    println();
    // => 49
}
```

`apply` does not know what `square` is. Its first parameter says "a function
of one `int` that returns one `int`", and `f(x)` is a call through that
value. The value that arrives is the name `square` itself: a function name
used where a value is expected *is* that function.

## A function in a binding

A binding can hold a function, and the binding can be called like one. The
binding's type is the function's type, so the checker knows what the call
returns.

```yashy
fn square(x: int) -> int {
    return x * x;
}

fn double(x: int) -> int {
    return x + x;
}

fn main() {
    const f = square;
    print(f(4));
    println();
    // => 16

    const g = double;
    print(g(4));
    println();
    // => 8
}
```

`const f = square;` binds a function value; `f(4)` calls through it. The
binding's type is the function's type, so the checker knows what the call
returns. `const` says the binding does not change; `let f = square;` would
work just as well, because a function value is a value like any other.

## A lambda

A lambda is a function written where a value is expected. It has no name; it
has parameters, an optional return type, and a body.

```yashy
fn apply(f: fn(int) -> int, x: int) -> int {
    return f(x);
}

fn main() {
    print(apply(|x: int| -> int { return x + 1; }, 41));
    println();
    // => 42

    const double = |x: int| -> int { return x * 2; };
    print(double(20));
    println();
    // => 40
}
```

The parameter types and the return type are written out, because a lambda has
no declaration for the checker to take them from. The arrow is optional: a
lambda with no arrow returns nothing, exactly as a function declaration with
no return annotation does.

### What a lambda may not do yet

A lambda may use its own parameters and any top-level function. It may not
use a variable of the function it is written in:

```text
fn main() {
    const n = 1;
    const f = |x: int| -> int { return x + n; };
}
```

```text
error: closures are not captured yet; this lambda needs `n`
```

Using `n` there would be a *capture*: the lambda would have to carry `n`
with it, wherever it goes. Yashy has no way to take a capture yet, so the
compiler says so rather than quietly copying the value or quietly reading
the wrong one.

## A closure

A closure is a function that carries the values of the scope it was written
in. It is a lambda that captures something.

```yashy
fn adder(n: int) -> fn(int) -> int {
    return |x: int| -> int { return x + n; };
}

fn main() {
    const add5 = adder(5);
    print(add5(10));
    println();
    // => 15
}
```

`adder` returns a closure. The closure captured `n`, so it carries a copy of
it, and it keeps working after `adder` has returned — the value it captured
lives in the closure, not in `adder`'s frame.

A closure may capture a number or a string. A string it captured is a
reference the closure owns: the closure holds it alive, and releases it when
the closure itself is gone.

```yashy
fn greeter(greeting: string) -> fn() -> string {
    return || -> string { return greeting; };
}

fn main() {
    const greet = greeter("hello");
    print(greet());
    println();
    // => hello
}
```

### What a closure may not capture

A closure may not capture a *linear* value — an array, a map, a pointer, a
channel. The closure would have to own a value the program still owns, and
Yashy has no way to say which of the two owners releases it:

```text
fn main() {
    const buf = alloc(16);
    const f = |x: int| -> int { return x; };
}
```

```text
error: a linear value cannot be captured yet: `buf`
```

A captured value is also a copy: assigning to it changes the copy and not the
binding it came from, so a closure's captures are read-only.

A closure cannot be put in a struct, an enum, or an array yet. An aggregate
copies the value it holds without counting the reference, and it has no way to
release it, so the copy would outlive the environment it points at.

## Generic functions

A function can work for more than one type. `[T]` after the function's name
declares a *type parameter*: `T` is a type the function does not name yet, and
each call says what it is.

```yashy
// One body, for any type.
fn identity[T](x: T) -> T {
    return x;
}

// `T` can appear inside another type, and more than once. Here the function
// value and the value it is given must agree about `T`.
fn apply_twice[T](f: fn(T) -> T, x: T) -> T {
    return f(f(x));
}

fn square(x: int) -> int {
    return x * x;
}

fn main() {
    print(identity(7));
    println();
    // => 7

    print(identity("seven"));
    println();
    // => seven

    print(apply_twice(square, 3));
    println();
    // => 81
}
```

`identity(7)` infers `T = int`; `identity("seven")` infers `T = string`. The
call says what the type is by the value it passes — there is no `identity<int>`
spelling, because there is only one thing a call can mean.

A type parameter is inferred from the arguments, and a repeated one has to
agree with itself. `apply_twice` mentions `T` three times, so the first
argument fixes it: `square` has type `fn(int) -> int`, which means `T` is
`int`, which means `3` has to be an `int` too. A call whose arguments disagree
reports the mismatch at the argument, in the same words an ordinary call does.

Every type parameter must be inferred. One that appears nowhere in the
parameters has nothing to say what it is:

```text
fn make[T]() -> int { return 0; }
fn main() { print(make()); }
```

```text
error: cannot infer type parameter `T`
```

### What happens at run time

Nothing. A generic function is not dispatched: the compiler checks the body
once for each type the program calls it with and emits an ordinary function
for each. `identity(7)` and `identity("seven")` above become two functions,
`identity_i64` and `identity_string`; the call sites name the one their
inference chose. A generic function the program never calls is never checked
either, which is why an unused generic body may mention a type it cannot
actually work with.

Recursion is ordinary recursion: a generic function that calls itself with the
same `T` is one function calling itself. A call with a *different* `T` — the
body of `swap[T, U]` calling `swap(y, x)` — is refused:

```text
error: `swap` calls itself with a different type: polymorphic recursion has no
finite instantiation
```

That call would need a new copy of the function for every type, and the next
copy would need another, so there is no finite program to emit.

### What generics cannot do yet

- A type can be generic; a *type declaration* cannot. `struct Pair[T]` is not
  in the language, so there is no way to write a container that holds a `T`.
  (The built-in `[]T`, `map[K]V`, `Option[T]` and `Result[T, E]` are generic
  because the compiler knows them by name.)
- A method cannot declare type parameters. `impl` methods take concrete types.
- A generic function's name is not a value. `apply_twice(identity, 3)` cannot
  be written, because the callee's type would have to pick which `identity`
  was meant; call it with a value instead.
- A type parameter may not share a name with a type: `fn f[T](x: T)` next to
  `struct T` reports ``type parameter `T` shadows a type of the same name``,
  because inside the body the parameter would silently replace the struct.

## The higher-order array methods

Three array methods take a function: `map` builds a new array from what the
function returns for each element, `filter` keeps the elements the function
says to keep, and `fold` carries one value through the whole array.

```yashy
fn main() {
    let xs = []int { 1, 2, 3 };

    let doubled = xs.map(|x: int| -> int { return x * 2; });
    for x in doubled {
        print(x);
        println();
    }
    // => 2
    // => 4
    // => 6
    doubled.free();

    let big = xs.filter(|x: int| -> bool { return x > 2; });
    for x in big {
        print(x);
        println();
    }
    // => 3
    big.free();

    print(xs.fold(0, |acc: int, x: int| -> int { return acc + x; }));
    println();
    // => 6

    xs.free();
}
```

`map` and `filter` build a *new* array; the source keeps its own elements, so
the new array is freed on its own. The element type may change: `map` through
`|x: int| -> string { return str_from_i64(x); }` gives a `[]string`.

`fold` starts from the value in front of the callback and ends with one value:
the callback takes the accumulator and the element, and returns the next
accumulator. The accumulator's type is the type of the value `fold` starts
from, so `xs.fold(0, ...)` folds into an `int`.

## A function the compiler runs

A `const fn` is a function the compiler runs *itself*. It has no run-time
existence: a call to one is replaced by the value the compiler computed, so
the program carries the number rather than the call that produced it.

```yashy
const fn square(n: int) -> int {
    return n * n;
}

fn main() {
    const table = square(9);
    print(table);
    println();
    // => 81
}
```

A `const fn` may call itself, and it may call another `const fn`:

```yashy
const fn fib(n: int) -> int {
    if n < 2 {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

const fn max_of(a: int, b: int) -> int {
    if a > b {
        return a;
    }
    return b;
}

const fn square(n: int) -> int {
    return n * n;
}

fn main() {
    const tenth = fib(10);
    print(tenth);
    println();
    // => 55

    const bigger = max_of(square(3), 10);
    print(bigger);
    println();
    // => 10
}
```

### What a `const fn` may use

Only what the compiler can compute without running the program:

- literals, and the parameters the call supplied;
- arithmetic and comparisons on those;
- calls to other `const fn`s;
- `if`, with a `return` in each arm.

Anything else is a diagnostic, and it names what it found:

```text
const fn read_all() -> int {
    return read_line();
}
```

```text
error: `read_line` is not allowed in a const fn
```

The rule is not a limitation of the implementation but of the idea: a value the
compiler cannot compute is a value the program has to compute at run time, and
a `const fn` has no run time to do it in.

The arguments have to be constants for the same reason. A `const fn` call
takes literals and other `const fn` calls:

```text
fn main() {
    let n = 9;
    const table = square(n);
}
```

```text
error: `n` is a run-time value: a `const fn` call can only use literals and
other `const fn` calls
```

### Where a `const fn` may be called

Only in a `const` initializer, because that is the only place a computed
value can go. A `const fn` is not a runtime function, so there is no symbol to
call:

```text
const fn square(n: int) -> int {
    return n * n;
}

fn main() {
    let t = square(9);
}
```

```text
error: `square` is a `const fn`: it can only be called in a `const` initializer
```

A `const fn` that never returns a value is a mistake too: the compiler folds
the body, and a path that fell off the end would have no value to give.

## Writing a function type

The type is `fn(T, U) -> R`: the parameter types, then the return type.

- `fn(int) -> int` takes one `int` and returns one `int`.
- `fn() -> int` takes nothing and returns an `int`.
- `fn(int, string) -> bool` takes an `int` and a `string` and returns a
  `bool`.
- `fn(int)` is a function of one `int` that returns nothing. The arrow is
  optional, and an omitted arrow means `-> void`.

The parameter *names* are not part of the type: a function type describes the
calling convention, not the names the callee happens to use.

## What the checker insists on

The type of a function value is checked exactly, in both directions.

- **The types must match.** Handing `apply` a function of a different shape
  reports ``error: expected `fn(int) -> int`, found `fn(string) -> int` ``. A
  function that takes a `string` is not a function that takes an `int`, even
  though both return an `int`.
- **The arity must match.** Calling a one-argument function with two
  arguments reports ``error: expected 1 arguments, found 2 ``.
- **A value that is not a function cannot be called.** `const f = 1; f(2);`
  reports ``error: `f` is not a function (it is `int`) ``.
- **`main` is the entry point, not a value.** `const f = main;` reports
  ``error: `main` is the entry point and cannot be used as a value ``, because
  the entry point is emitted with the operating system's own signature.

## How it works

A function value is a pair: a code pointer and an environment pointer. The
environment is where a closure's captured values would live; a top-level
function captures nothing, so its environment is null.

A `const fn` is the one kind of function with no runtime form at all. The
checker folds its body when a `const` initializer calls it, and the call is
replaced by the literal the fold produced -- so the program carries `81` where
it would otherwise carry a call. The folding is a tree walk over the body with
the arguments bound to their values, and it is bounded: a `const fn` that
recurses forever is reported rather than followed, because the compiler is
running it and would run out of stack.

Using a top-level function as a value makes the compiler synthesize a small
thunk, `__fnvalue_square(i8* env, int)`, that takes the environment and
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

1. Write a function `twice(f: fn(int) -> int, x: int) -> int` that calls `f`
   twice, and use it with `square` to compute `square(square(4))`.

2. Write three functions of the same type — `negate`, `double` and
   `identity` — and a function `pick(which: fn(int) -> int, x: int) -> int`
   that calls the one it is given. Call `pick` with each of the three.

3. Write `fn first_of[T](x: T, y: T) -> T` that returns `x`, and call it once
   with two `int`s and once with two `string`s. Then change the second call so
   the two arguments have different types and read the error.

## Where this leaves you

That is the language. The [introduction](00-intro.md) restates the three
decisions, and the programs under `crates/yashy-driver/tests/programs` show
the rest. Continue to [12-first-cli](12-first-cli.md) to install a release,
create a project, test it, and build a binary you can ship.
