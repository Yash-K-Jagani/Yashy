# 2. Values and variables

## What you will learn

- The six scalar spellings, their widths, and the operators each one supports.
- How `const` and `let` differ.
- Why mixed arithmetic needs an explicit `as` cast.
- How a value becomes text, and when arithmetic stops the program.

The scalar spellings are `int`, `float`, `bool`, `char`, `string` and `void`.
`const` binds a name
once and never rebinds. `let` binds a name that can be assigned again.
Assignment operators like `+=` are shorthand for the expanded form.

| Form | Reassignable | Scope | Use it for |
|---|---|---|---|
| `const` | No | Current block | A value that is fixed after it is created. |
| `let` | Yes | Current block | A value that changes in this block. |

Both are block-scoped: leaving the block makes the name unavailable, and there
is no `undefined` value to read before the declaration. This keeps every binding
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

`int` is the only integer type and `float` the only floating type. Integer
literals such as `3` are `int`; a fractional literal such as `0.5` is `float`.
`bool` has `true` and `false`; `string` is a counted sequence of bytes; `char`
is one byte; and `void` is the absence of a value.

| Spelling | Width | Literal | Operators | Notes |
|---|---|---|---|---|
| `int` | 64-bit signed | `42`, `-7` | `+ - * / %`, comparisons | Checked; a fault stops the program. |
| `float` | 64-bit | `0.5`, `-2.0` | `+ - * / %`, comparisons | No implicit `int` operand. |
| `bool` | 1 bit (`i1`; one byte in memory) | `true`, `false` | `&& \|\| !`, `== !=` | Conditions must be `bool`. |
| `char` | one byte, 0-255 | `'A'`, `'\n'` | comparisons only | No arithmetic; convert with `to_int`. |
| `string` | refcount, length, pointer | `"text"` | `+`, `== !=` | `+` concatenates; no ordering. |
| `void` | no value | — | — | Return type only. |

Both operands of an arithmetic or comparison operator must have the same type.
`total as float` and `value as int` are the two numeric casts. Casting `float` to
`int` truncates toward zero: `3.9 as int` is `3`, and `-3.9 as int` is `-3`.
There is no cast from `string` to a number; `s.parse_i64()` exists for that.

### `char`

A `char` is one byte, written `'A'`. The escapes are `'\n'`, `'\t'`, `'\r'`,
`'\0'`, `'\''`, `'\\'`, and `'\xNN'` with exactly two hex digits. A literal
outside 0-255 is a compile error.

`char` is its own type: there is no implicit conversion to or from `int`, and
there is no arithmetic on it. `'A' + 1` is an error; write `'A'.to_int() + 1`.
Two `char`s compare with `==`, `!=`, `<`, `<=`, `>`, `>=` in byte order, but a
`char` and an `int` cannot be compared.

`c.to_int()` gives the byte value, and `n.to_char()` checks its input at run
time, panicking outside 0-255 rather than truncating. `c.to_string()` gives a
one-character string, and `s.byte_at(i)` returns a `char` (with `'\0'` when the
index is out of range). `print('A')` writes `A`; inside interpolation, `"${c}"`
writes the character, not the number.

```yashy
fn main() {
    const letter = 'A';
    print(letter);
    print(letter.to_int());
    println();
    // => A65

    const code = 97;
    const lower = code.to_char();
    print(lower.to_string());
    println();
    // => a

    const word = "cat";
    print(word.byte_at(1));
    println();
    // => a
}
```

### `void`

`void` is the type of "no value". `fn f()` and `fn f() -> void` mean the same
thing, and both allow `return;` or falling off the end. `void` may be written
only as a return type: `let x: void`, `[]void`, and `Option[void]` are errors,
because no value of that type can exist. `()` is no longer a spelling; write
`void`, and `-> ()` reports ``error: `()` is spelled `void` in Yashy``.

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
- **Doing arithmetic on a `char`.** `'A' + 1` reports
  ``error: no arithmetic on `char` ``. Convert first: `'A'.to_int() + 1`.
- **Comparing a `char` with an `int`.** `'A' == 65` reports
  ``error: cannot compare `char` with `int` ``. Compare the numbers:
  `'A'.to_int() == 65`.
- **Casting instead of converting.** `c as int` reports ``error: use `to_int()` ``
  and `n as char` reports ``error: use `to_char()` ``. These are method calls,
  because `to_char` checks its input at run time.
- **Relying on `&&` to guard a fault.** As above, the right operand still
  runs. The program stops with `panic: division by zero: 10 / 0` even when the
  left operand is false.
- **Declaring a binding without a value.** `let x;` reports
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
into a call to `str_from_i64`, `str_from_f64`, `str_from_bool` or
`str_from_char` (a `string` needs no call) and joins the pieces with `+`, so the
typed tree has no interpolation node left. Checked arithmetic lowers to LLVM's overflow
intrinsics, with a branch to a cold block that reports the operands and ends the
process. `&&` and `||` lower to LLVM's `and` and `or` on `i1`, which is exactly
why both sides are evaluated. `let` lowers to the mutable declaration node;
`const` lowers to the immutable form.

Continue to [03-control-flow](03-control-flow.md).
