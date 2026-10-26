# 6. Errors without exceptions

## What you will learn

- How a function reports failure in its return type.
- What `?` does to an `Ok` and to an `Err`.
- How to report an error to a user without unwinding the stack.
- Which part of the `?` rule this build enforces, and which part it does not.

A function that can fail returns `Result[T, E]`: `Ok(value)` or `Err(error)`.
There is no exception stack and no hidden unwinding. `?` propagates an `Err`
to the caller, and the function's return type is supposed to say so. This build
enforces only part of that promise; the section below shows exactly where.

```yashy
fn half(n: int) -> Result[int, string] {
    if n % 2 != 0 {
        return Err("odd");
    }
    return Ok(n / 2);
}

fn quarter(n: int) -> Result[int, string] {
    const h = half(n)?;
    return Ok(half(h)?);
}

fn report(result: Result[int, string]) {
    const message = switch result {
        case Ok(v) { "ok " + str_from_i64(v) }
        case Err(e) { "error " + e }
    };
    print(message);
    println();
}

fn main() {
    report(quarter(8));
    report(quarter(6));
    // => ok 2
    // => error odd
}
```

`quarter(6)` calls `half(6)`, which is `Ok(3)`, then `half(3)`, which is
`Err("odd")`; the `?` returns that `Err` from `quarter` immediately. No line
after a failed `?` runs.

The signature is the contract. A caller reading `Result[int, string]` knows
that the call can fail and what an error looks like. This is the trade for
losing exception unwinding: the failure is visible where the function is
declared, not only where it happens to be thrown.

## The shape of a result

| Expression | Type | What it means |
|---|---|---|
| `Ok(v)` | `Result[T, E]` | Success carrying `v`. |
| `Err(e)` | `Result[T, E]` | Failure carrying `e`. |
| `f(x)?` | `T` | `v` if `f` returned `Ok(v)`; returns `Err(e)` from the enclosing function otherwise. |
| `switch r { case Ok(v) ... case Err(e) ... }` | depends on arms | The only way to read both sides. |
| `f(x)` used as a statement | `Result[T, E]`, then discarded | Allowed today; the value is dropped. |

`?` is only legal on an expression whose type is a `Result`; applying it to an
`int` reports ``error: `?` can only be used on a `Result`, not `int` ``. On an
`Option`, the same message names `Option[int]`: there is no `?` for `Option` in
this build.

## The rule, and what is checked

The rule is: `?` belongs in a function whose return type is a `Result` with the
same error type. The `Err` it produces has to fit the caller's signature, or
the caller could not handle it.

The checker enforces both halves of that. A `?` whose operand is not a
`Result` is reported, a `?` in a function that does not return a `Result` is
reported, and a `?` whose error type does not match the function's is
reported:

```text
fn bad(n: int) -> int {
    return half(n)?;   // error: `?` needs a function returning `Result`
}
```

```text
fn bad(n: int) -> Result[int, string] {
    return half(n)?;   // error: error type mismatch: `bool` vs `string`
}
```

The second one matters most: the `Err` the caller would have to handle is a
`bool` where the signature promises a `string`, and the program would have
died on the error path with no message.

A function that returns a value also has to return one on every path. A
function whose last statement is an `if` without an `else` is reported:

```text
fn half(n: int) -> int {
    if n > 0 { return n; }   // error: missing return
}
```

For the same reason, a bare call to a `Result`-returning function is accepted
as a statement: `half(3);` compiles and the `Err` is discarded. The type makes
the possibility visible, but the compiler does not force you to look at it. If
the result matters, match it or pass it to a function that takes a `Result`.

## Common mistakes

- **Giving `Err` the wrong payload.** `return Err(1);` in a
  `Result[int, string]` function reports
  ``error[E0100]: expected `string`, found `int` ``. Construct the error the signature
  promises.
- **Applying `?` to a number.** `const x = 3?;` reports
  ``error: `?` can only be used on a `Result`, not `int` ``.
- **Using `?` in a function that does not return a `Result`.** It reports
  ``error: `?` needs a function returning `Result` ``. Change the function's
  return type to `Result[...]`, or match the result instead.
- **Mixing the error types of two `?` calls.** A `?` on `Result[int, bool]`
  inside a `Result[int, string]` function reports
  ``error: error type mismatch: `bool` vs `string` ``. Keep one error type
  through the chain.
- **Ignoring a result.** `half(3);` compiles and drops the `Err`. If the caller
  should report it, match it and print the message, as `report` does.

## Diagnostics that teach

A misspelled name is reported at the point of use, with the nearest name in
scope as a suggestion:

```text
misspell.ya:3:11
error[E0001]: cannot find value `totl` in this scope
    print(totl);
          ^^^^ did you mean `total`?
  docs: https://yashy.dev/errors/E0001
```

The suggestion is offered only when the edit distance is small enough to be a
likely typo; a name that is simply absent gets the same error without the hint.
The same rule covers functions, types, struct fields, struct-literal fields and
enum variant fields, so a typo in `Point { x: 1, yy: 2 }` names the field it
probably meant. The caret and the note are part of the diagnostic, not a
separate lint: `yashy check` and `yashy build` print the same block. The final
`docs:` line names the catalog page, and `yashy explain E0001` prints that page
offline.

## Try it

1. Write `fn parse_positive(text: string) -> Result[int, string]` that uses
   `parse_i64`, then rejects a value less than or equal to zero with `Err`.
   Call it with `"12"`, `"0"`, and `"abc"`. Hint: `parse_i64` returns `0` for
   text that is not a number, so `0` needs its own error message.

2. Rewrite `quarter` so that the two `?` calls are separate `switch`
   statements instead. Keep the same output. Hint: the `Err` arm must return
   the error from `quarter`; the `Ok` arm continues with the payload.

## How it works

`Result` is an ordinary enum with two variants. Its IR value is a tag plus
enough space for the larger payload, the same layout chapter 5 describes. `?`
tests the tag: an `Ok` extracts the payload and continues, and an `Err` returns
the whole `Result` value from the enclosing function. That return is a direct
`ret` only when the checker's function-return type has the same representation
as the operand; otherwise the generator has no value of the right shape to
return and emits `unreachable`. The checker now refuses that mismatch before
the generator ever sees it.

`&&` and `||` short-circuit: the right operand is evaluated only when the left
one does not decide the answer, so `n != 0 && 10 / n > 1` does not divide by
zero when `n` is zero. The generated code is a branch rather than an
instruction, which is what makes the difference.

Continue to [07-memory](07-memory.md).
