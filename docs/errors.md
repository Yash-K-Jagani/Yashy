# Error catalog

Yashy reports a problem as a diagnostic with a `path:line:column` header, a
caret under the source it is about, and a short explanation. The compiler exits
`1` for a diagnostic. R1 assigns a stable code to the common mistakes; an
uncoded diagnostic still has a span and a message.

## Reading a diagnostic

```text
main.ya:4:19
error[E0100]: expected `int`, found `string`
    const int n = "three";
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
| [E0101](errors/E0101.md) | equality in a comparison chain |
| [E0102](errors/E0102.md) | field default is not constant |
| [E0103](errors/E0103.md) | bitwise operand is not an integer |
| [E0104](errors/E0104.md) | bitwise operands have different types |
| [E0105](errors/E0105.md) | shift count out of range |
| [E0106](errors/E0106.md) | repr(C) is not allowed here |
| [E0107](errors/E0107.md) | sizeof or alignof names no known type |
| [E0108](errors/E0108.md) | offsetof needs a repr(C) struct |
| [E0109](errors/E0109.md) | the type is written before the name |
| [E0200](errors/E0200.md) | unknown field |
| [E0201](errors/E0201.md) | missing field |
| [E0202](errors/E0202.md) | duplicate field |
| [E0300](errors/E0300.md) | missing `?` |
| [E0301](errors/E0301.md) | mixed `?` |
| [E0400](errors/E0400.md) | const reassignment |
| [E0500](errors/E0500.md) | moved value |
| [E0600](errors/E0600.md) | non-exhaustive match |
| [E0601](errors/E0601.md) | unreachable switch arm |
| [E0602](errors/E0602.md) | invalid range pattern |
| [E0603](errors/E0603.md) | `if let` pattern always matches |
| [E0604](errors/E0604.md) | unknown loop label |
| [E0605](errors/E0605.md) | duplicate loop label |
| [E0700](errors/E0700.md) | wrong argument count |
| [E0701](errors/E0701.md) | unknown named argument |
| [E0702](errors/E0702.md) | duplicate argument |
| [E0801](errors/E0801.md) | private item used from another module |
| [E0802](errors/E0802.md) | import cycle |
| [E1200](errors/E1200.md) | grapheme index out of range |
| [E0900](errors/E0900.md) | duplicate extern symbol |
| [E0901](errors/E0901.md) | malformed extern declaration |
| [E0902](errors/E0902.md) | misplaced variadic marker |
| [E1000](errors/E1000.md) | leaked linear value |
| [E1002](errors/E1002.md) | spread operand used after the literal |
| [E1003](errors/E1003.md) | arena value escapes its block |
| [E1004](errors/E1004.md) | arena value freed individually |
| [E1100](errors/E1100.md) | undischarged contract |
| [E1201](errors/E1201.md) | provable integer overflow |
| [E1300](errors/E1300.md) | effect exceeded |
| [E1301](errors/E1301.md) | unsafe call in a declared function |
| [E1400](errors/E1400.md) | double close |
| [E1401](errors/E1401.md) | use after close |
| [E1402](errors/E1402.md) | race |
| [E1500](errors/E1500.md) | unknown trait |
| [E1501](errors/E1501.md) | conflicting trait impl |
| [E1502](errors/E1502.md) | incomplete trait impl |
| [E1503](errors/E1503.md) | unsatisfied bound |
| [E1504](errors/E1504.md) | ambiguous method |
| [E1505](errors/E1505.md) | instantiation budget exceeded |
| [E1506](errors/E1506.md) | method called through a type |
| [E1507](errors/E1507.md) | associated function called through a value |
| [E1508](errors/E1508.md) | unknown associated function |
| [E1509](errors/E1509.md) | wrong number of type arguments |
| [E1510](errors/E1510.md) | unused type parameter |
| [E1511](errors/E1511.md) | generic trait outside the standard library |
| [E1512](errors/E1512.md) | inheritance is not in the language |
| [E1513](errors/E1513.md) | there is no Drop |
| [E1514](errors/E1514.md) | a trait has no fields |
| [E1515](errors/E1515.md) | operators cannot be overloaded |
| [E1600](errors/E1600.md) | macro invocation matches no arm |
| [E1601](errors/E1601.md) | macro expansion limit exceeded |
| [E1602](errors/E1602.md) | macro metavariable kind mismatch |
| [E1603](errors/E1603.md) | duplicate macro name |
| [E1700](errors/E1700.md) | await or spawn outside an async function |
| [E1800](errors/E1800.md) | manifest syntax |
| [E1802](errors/E1802.md) | lockfile mismatch |
| [E1900](errors/E1900.md) | sandbox capability denied |
| [E1901](errors/E1901.md) | sandbox limit breached |
| [E1701](errors/E1701.md) | task handle escapes its scope |
| [E1702](errors/E1702.md) | resource held across an await |
| [E1703](errors/E1703.md) | async method in a trait |
| [E2000](errors/E2000.md) | unused binding |
| [E2001](errors/E2001.md) | unused import |
| [E2002](errors/E2002.md) | unreachable code |
| [E2003](errors/E2003.md) | unused parameter |
| [E2100](errors/E2100.md) | doc comment with no item |

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
