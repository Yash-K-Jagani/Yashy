# Error catalog

Yashy reports a problem as a diagnostic with a `path:line:column` header, a
caret under the source it is about, and a short explanation. The compiler exits
`1` for a diagnostic. R1 assigns a stable code to the common mistakes; an
uncoded diagnostic still has a span and a message.

## Reading a diagnostic

```text
main.ya:4:13
error[E0100]: expected `int`, found `string`
    const n: int = "three";
                  ^^^^^^^
  docs: https://yashy.dev/errors/E0100
```

The header is the position, the `error[CODE]` line is the rule that was broken,
and the caret is the expression the rule applied to. The code is stable:
`E0100` always means a type mismatch, and the final `docs:` line names its page.
`yashy explain E0100` prints the same page offline. A note below the caret names
the declaration or import involved when that is not obvious.

## Codes

| Code | Meaning |
|---|---|
| [E0001](errors/E0001.md) | unknown name |
| [E0002](errors/E0002.md) | unknown type |
| [E0100](errors/E0100.md) | type mismatch |
| [E0103](errors/E0103.md) | bitwise operand is not an integer |
| [E0104](errors/E0104.md) | bitwise operands have different types |
| [E0105](errors/E0105.md) | shift count out of range |
| [E0200](errors/E0200.md) | unknown field |
| [E0201](errors/E0201.md) | missing field |
| [E0300](errors/E0300.md) | missing `?` |
| [E0400](errors/E0400.md) | const reassignment |
| [E0500](errors/E0500.md) | moved value |
| [E0600](errors/E0600.md) | non-exhaustive match |
| [E0700](errors/E0700.md) | wrong argument count |
| [E1000](errors/E1000.md) | leaked linear value |
| [E1100](errors/E1100.md) | undischarged contract |
| [E1201](errors/E1201.md) | provable integer overflow |
| [E1300](errors/E1300.md) | effect exceeded |
| [E1301](errors/E1301.md) | unsafe call in a declared function |
| [E1400](errors/E1400.md) | double close |
| [E1401](errors/E1401.md) | use after close |
| [E1402](errors/E1402.md) | race |
| [E2000](errors/E2000.md) | unused binding |
| [E2001](errors/E2001.md) | unused import |
| [E2002](errors/E2002.md) | unreachable code |
| [E2003](errors/E2003.md) | unused parameter |

The message tables below quote the message; a coded diagnostic prints the code
after `error`, as in the example above.

## Parse and name errors

| Message | Meaning |
|---|---|
| `error: unexpected token` | The parser found a token that cannot continue the construct. |
| `error: expected ...` | A required token or type is missing. |
| `error: cannot find ...` | A name is not declared in the current scope or imports. |
| `error: duplicate definition of ...` | Two items share a name in one scope. |
| `error: import cycle: ...` | Modules import each other; the cycle is printed. |

## Type errors

| Message | Meaning |
|---|---|
| ``error: expected `T`, found `U` `` | An expression's type does not match its context. |
| ``error: cannot apply `+` to `string` and `int` `` | No implicit conversion; use `str_from_i64` or interpolation. |
| `error: expected N arguments, found M` | A call's arity does not match the declaration. |
| `error: type parameter ... shadows ...` | A generic parameter reuses a type name. |
| `error: cannot infer type parameter ...` | A generic call gives no argument to infer from. |

## Ownership and memory errors

| Message | Meaning |
|---|---|
| ``error: value `x` has already been moved`` | A linear value was used after `move` or `free`. |
| ``error: cannot change `x` while a view of it is alive`` | A borrowed array cannot be mutated, rebound, or freed. |
| ``error: cannot move `x` while a view of it is alive`` | A borrowed array cannot be moved. |
| ``error: cannot move `x` while it is being iterated`` | A `for` loop still borrows the array. |
| `error: a linear value cannot be captured yet: ...` | A closure would have to own an array, map, pointer, or channel. |
| ``error: `x` is not a function (it is `T`)`` | A call was written on a value that has no function type. |

## Module and project errors

| Message | Meaning |
|---|---|
| `error: cannot read <path>: ...` | The file is missing or unreadable. |
| `error: no tests at <path>` | `yashy test` was given a path that does not exist. |
| `error: <path> is not empty` | `yashy init` only creates a project in an empty directory. |
| `error: no LLVM toolchain found` | Yashy found neither `clang` nor `llc`; builds need `clang`. |

## Runtime faults

A fault stops the program, reports its position, and exits `1`. `defer` blocks
in the panicking function's open scopes run first, innermost first.

```text
main.ya:7:5: panic: assertion failed: n > 0
```

| Fault | Example |
|---|---|
| `panic: integer overflow: ...` | `9223372036854775807 + 1` |
| `panic: division by zero` | `1 / 0` |
| `panic: assertion failed: ...` | `assert(n > 0)` |
| `panic: ...` | An explicit `panic("message")` |

Indexing is not a fault: `xs[99]` evaluates to `None`, and `xs.get(99)` is the
same checked lookup. A program that wants to stop on a missing element has to
`panic` or unwrap the `Option` explicitly.

## Doctor failures

`yashy doctor` reports each failing check as a `clang:` or `stdlib:` line, then
prints one final `next:` line. `yashy doctor --json` reports the same facts as
fields (`clang`, `stdlib`, `path`, `next`) and keeps stderr empty.

## Where to look next

- [CLI reference](cli.md)
- [Quickstart](quickstart.md)
- [Tutorial: errors](tutorial/06-errors.md)
