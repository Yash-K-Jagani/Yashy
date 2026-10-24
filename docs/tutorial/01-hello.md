# 1. Hello, functions

## What you will learn

- How to declare a function, its parameters, and its return type.
- The difference between a function that returns a value and one that does not.
- Why a function may be called before it is written down.
- Which names are builtins, and why they need no import.

A program is a list of items: functions, structs, enums, imports. `main` is
where execution starts. A function is `fn name(parameters) -> return_type`,
and the body is a block of statements ending, optionally, in `return`. A source
file ends in `.ya`: `yashy build main.ya -o main` compiles it, and the binary is
then run directly.

```yashy
fn greeting(name: string) -> string {
    return "Hello, " + name + "!";
}

fn main() {
    // A comment the compiler ignores.
    print(greeting("Yashy"));
    println();
    // => Hello, Yashy!
}
```

`+` concatenates strings. There is no automatic conversion from a number to a
string: `str_from_i64(n)` and `str_from_f64(x)` do that, and string
interpolation (chapter 2) is sugar over them. The rule is not fussiness. A
silent conversion would make `"n = " + n` compile, and then it would be unclear
whether the program meant the number or its decimal spelling.

`print(x)` writes `x` with no newline; `println(x)` writes `x` and then a
newline, and `println()` writes only the newline. The family takes one argument
of any printable type -- `print(1)`, `print('A')` and `print("text")` all work
without a format string. A string may interpolate an expression:
`"Hello, ${name}!"` is the same program as `"Hello, " + name + "!"`.

A function that returns nothing writes no `->`:

```yashy
fn banner() {
    println("===");
}

fn main() {
    banner();
    println("done");
    // => ===
    // => done
}
```

Statements end with `;` except a block. `return` may be bare (`return;`) in a
function with no return type, or carry a value in one with a return type.

## Signature rules

Parameters are always annotated. Local bindings inside the body infer their
types from the initializer, but a signature is the boundary a caller reads, so
it is written out. Returning a value is a `return` statement, not "the value of
the last expression": a block is a sequence of statements, and a statement
does not become the function's result by being last.

| Form | Meaning | Notes |
|---|---|---|
| `fn f(a: int) -> int { ... }` | A function taking one `int` and returning one. | Every path should return a value. |
| `fn f() { ... }` | A function returning `void`. | `return;` and falling off the end are both allowed. |
| `return expr;` | Returns `expr` to the caller. | `expr` must match the return type. |
| `f(1, 2)` | A call. | Argument count and types are checked at the call. |
| `main` | The entry point. | Takes no arguments; may return `int` as the exit status. |
| `print`, `println` | Builtins. | In scope everywhere; cannot be redeclared. |

Declaration order does not matter. The checker reads every signature in the
program first, then checks every body. That is why `main` can call a function
written below it, and why two functions can call each other. It is also why a
duplicate name is reported once, at the second declaration, instead of turning
every use into "cannot find function".

## Calls and results

A call is an expression. It may be used where a value is expected, or stand
alone as a statement, in which case its result is discarded. `println` is a
call like any other; it just happens to return `void`.

`return` leaves the function as soon as it runs. A `return` inside an `if`
therefore skips everything after the `if`, which is how a guard clause works:

```text
if n < 0 {
    return 0;
}
return n * 2;
```

The checker needs a call's result type before it can type the expression around
it, and it gets that from the first pass over the signatures. That is the same
pass that makes declaration order irrelevant.

## Common mistakes

- **Calling with the wrong number of arguments.** A function declared with two
  parameters and called with one is reported at the call:
  ``error: expected 2 arguments, found 1``.
- **Returning the wrong type.** `fn f() -> int { return "x"; }` reports
  ``error: expected `int`, found `string` `` at the returned expression.
- **Declaring the same function twice.** The second declaration reports
  ``error: duplicate definition of `f` ``. Rename one, or delete it.
- **Joining a string and a number with `+`.** `"n = " + 1` reports
  ``error: cannot apply `+` to `string` and `int` ``. Convert with
  `str_from_i64` first, or use interpolation.
- **Falling off the end of a value-returning function.** A function declared
  `-> int` whose body has no `return` is accepted today, and the call yields
  `0`. Do not rely on that: write the `return`. The missing-return check is not
  yet enforced, and a function that returns a real value should say so.

## Try it

1. Write `fn square(n: int) -> int` and `fn cube(n: int) -> int`, where `cube`
   calls `square`. Print `cube(3)`. Hint: `cube` needs only one expression, but
   the `return` keyword is still required.

2. Write `fn banner(label: string)` that prints the label between two lines of
   `=`. Call it from `main`, then try calling it with no argument. Read the
   diagnostic before you fix it. Hint: the error names the expected count, not
   the missing value.

## How it works

The checker makes two passes over the program. The first declares every
`struct`, `enum` and `fn` signature in one shared root scope; the second checks
each body with a scope holding its parameters. Code generation then emits one
LLVM function per `fn`. `main` is special: the C runtime calls it and reads an
`i32`, so a Yashy `main` returning `int` has its result narrowed to that width,
the same truncation C performs for `int main`.

Continue to [02-values](02-values.md).
