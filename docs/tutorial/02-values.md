# 2. Values and variables

## What you will learn

- The four scalar types and the operators each one supports.
- How `const`, `let` and `var` differ.
- Why mixed arithmetic needs an explicit `as` cast.
- How a value becomes text, and when arithmetic stops the program.

The scalar types are `int`, `float`, `bool`, and `string`. `const` binds a name
once and never rebinds. `let` and `var` bind a name that can be assigned again.
`let` is the form to reach for in new code; `var` is kept for programs written
before `let` existed. Assignment operators like `+=` are shorthand for the
expanded form.

| Form | Reassignable | Scope | Use it for |
|---|---|---|---|
| `const` | No | Current block | A value that is fixed after it is created. |
| `let` | Yes | Current block | A value that changes in this block. |
| `var` | Yes | Current block | The older mutable form; still accepted. |

All three are block-scoped today. `let` is not JavaScript's function-scoped
`var`: leaving the block makes the name unavailable, and there is no
`undefined` value to read before the declaration. This keeps every binding
initialized and every owned string releasable.

```yashy
fn main() {
    const count = 3;
    let total = count * 7;
    total += 1;

    const ratio = 0.5;
    const ready = true;

    println("total = ${total}");
    print("half of total = ");
    print((total as float * ratio) as int);
    println();
    println("ready = ${ready}");
    // => total = 22
    // => half of total = 11
    // => ready = true
}
```

Two things to notice:

- `${...}` inside a string converts and concatenates. `"total = ${total}"` is
  the same program as `"total = " + str_from_i64(total)`; the checker lowers it
  into exactly that. The braces hold an expression, not only a name: `${p.x}`
  and `${xs.len()}` are allowed.
- Arithmetic between `int` and `float` needs an explicit cast, `as`. The language
  will not silently pick one for you. The result is cast back to `int` before
  printing: `print`'s float formatting is the C library's, and the tutorial only
  claims output that does not depend on it.

There are no implicit truthiness rules: a condition is a `bool`, not an
integer. A language that accepted `if n` would have to decide whether `0` means
false, and every reader would have to remember the answer. Requiring `n != 0`
puts the decision in the program.

## Types and operators

`int` is the only integer type. `float` is the only floating type. Integer
literals such as `3` are `int`; a fractional literal such as `0.5` is `float`.
`bool` has `true` and `false`, and `string` is a counted sequence of bytes.

| Type | Literal | Operators | Notes |
|---|---|---|---|
| `int` | `42`, `-7` | `+ - * / %`, comparisons | Checked; a fault stops the program. |
| `float` | `0.5`, `-2.0` | `+ - * / %`, comparisons | No implicit `int` operand. |
| `bool` | `true`, `false` | `&& \|\| !`, `== !=` | Conditions must be `bool`. |
| `string` | `"text"` | `+`, `== !=` | `+` concatenates; no ordering. |

Both operands of an arithmetic or comparison operator must have the same type.
`total as float` and `value as int` are the two numeric casts. Casting `float` to
`int` truncates toward zero: `3.9 as int` is `3`, and `-3.9 as int` is `-3`.
There is no cast from `string` to a number; `parse_i64` exists for that.

## How a number is written

An integer is written in decimal by default, and it may be written in another
base or with separators. All four of these are the same number:

```yashy
fn main() {
    print(1_000_000);
    print(" ");
    print(0xFF);
    print(" ");
    print(0b1010);
    print(" ");
    print(0o777);
    println();
    // => 1000000 255 10 511
}
```

- `_` separates digits and is ignored: `1_000_000` is a million. It must sit
  *between* two digits, so `1_` and `_1` are errors rather than numbers.
- `0x` is hexadecimal, `0b` is binary, and `0o` is octal: `0xFF` is 255,
  `0b1010` is 10, and `0o777` is 511. The prefix may be written in either
  case (`0X`, `0B`, `0O`).
- A prefixed number is a whole number. There is no hexadecimal fraction, and
  there are no exponent literals: `1e10` is an error, not ten million.

A float is a decimal point and more digits, and the separator works there too:
`1_000.5_0` is `1000.5`.

`&&` and `||` take two `bool`s and produce a `bool`. They are not
short-circuiting in this build: both operands are evaluated before the operator
runs. That matters when the right side has a side effect or can fault. Write
nested `if` statements when the second test is only safe after the first:

```text
if n != 0 {
    if 10 / n > 1 {
        println("large");
    }
}
```

The one-liner `if n != 0 && 10 / n > 1` divides by zero when `n` is `0`, even
though the left side is false.

## Faults, not wrapping

Integer arithmetic is checked. Overflow, division by zero, and the smallest
`int` divided by `-1` stop the program with a message and exit status 1 rather
than wrapping silently. The message names the position and the operands:

```text
main.ya:3:15: panic: integer overflow: 9223372036854775807 + 1
```

Remainder follows the same truncation as division: `-7 / 2` is `-3`, and
`-7 % 2` is `-1`. The sign of the remainder is the sign of the dividend.

If wrapping is what the program means, say so with `wrapping_add`,
`wrapping_sub` or `wrapping_mul`. `checked_*` returns an `Option[int]` instead,
and `saturating_*` clamps at the ends of the type. There is no wrapping
division: a wrapped quotient is not the quotient of anything.

## Common mistakes

- **Assigning to a `const`.** `const n = 3; n = 4;` reports
  ``error: cannot assign to `n`, which is immutable``. Use `let n = 3;`.
- **Mixing `int` and `float` in one expression.** `x + 1.5` where `x` is `int`
  reports ``error: cannot apply `+` to `int` and `float` ``. Cast one side:
  `x as float + 1.5`.
- **Interpolating a struct or enum.** `"${p}"` reports
  ``error: cannot interpolate a `Point` into a string: only `string`, `int`, `float` and `bool` have a text form``.
  Print the fields instead.
- **Relying on `&&` to guard a fault.** As above, the right operand still
  runs. The program stops with `panic: division by zero: 10 / 0` even when the
  left operand is false.
- **Declaring a binding without a value.** `let x;` (and `var x;`) reports
  ``error: expected `=`, found `;` ``. A binding is initialized when it is
  created; there is no uninitialized local.

## Try it

1. Compute the average of three `int` values as a `float`, then print it rounded
   down to an `int`. Hint: cast the sum to `float` before dividing, and cast the
   result back after.

2. Swap two `let` bindings without a third variable. Hint: `+` and `-` work,
   but only if the intermediate values stay inside `int`. Then try it with a
   temporary `const` and decide which version you would rather read.

## How it works

Interpolation is not a runtime feature. The checker rewrites each `${expr}`
into a call to `str_from_i64`, `str_from_f64` or `str_from_bool` (a `string`
needs no call) and joins the pieces with `+`, so the typed tree has no
interpolation node left. Checked arithmetic lowers to LLVM's overflow
intrinsics, with a branch to a cold block that reports the operands and ends the
process. `&&` and `||` lower to LLVM's `and` and `or` on `i1`, which is exactly
why both sides are evaluated. `let` and `var` lower to the same mutable
declaration node; they differ only in the name and in the documentation of
intent. `const` lowers to the immutable form.

Continue to [03-control-flow](03-control-flow.md).
