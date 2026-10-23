# 3. Control flow

## What you will learn

- How `if`, `while`, and `for ... in ...` choose what runs next.
- Why every condition is a `bool` and not an integer.
- How `break` and `continue` interact with nested loops.
- Why a `switch` arm holds one expression, not a block of statements.

`if` is a statement; its condition is a `bool`. `while cond { ... }` checks at
the top, `while true { ... }` is the infinite loop (there is no separate `loop`
keyword), and `break` and `continue` leave and restart it.

```yashy
fn main() {
    var i = 1;
    while i <= 5 {
        if i % 2 == 0 {
            print_str("even\n");
        } else {
            print_str("odd\n");
        }
        i += 1;
    }
    // => odd
    // => even
    // => odd
    // => even
    // => odd

    var sum = 0;
    for n in 0..5 {
        if n == 3 {
            continue;
        }
        sum += n;
    }
    print_i64(sum);
    print_str("\n");
    // => 7

    var count = 0;
    while true {
        count += 1;
        if count == 3 {
            break;
        }
    }
    print_i64(count);
    print_str("\n");
    // => 3
}
```

`for n in 0..5` walks the half-open range 0, 1, 2, 3, 4. The same form walks an
array element by element (chapter 9).

The half-open range is not an arbitrary choice. It makes `0..xs.len()` the
complete set of valid indices, and it makes the length of `a..b` equal to
`b - a` without a `+ 1` to forget. An inverted range such as `3..1` is empty;
it is not an error and it does not count down.

## The forms

| Form | What it does | Notes |
|---|---|---|
| `if cond { ... } else { ... }` | Runs one branch. | `cond` must be `bool`; `else` is optional. |
| `while cond { ... }` | Tests before each pass. | A false condition runs the body zero times. |
| `while true { ... }` | Repeats until a `break`. | There is no `loop` keyword. |
| `for n in start..end { ... }` | Walks `start` to `end - 1`. | Bounds are `int`; the binding is immutable. |
| `break` | Leaves the innermost loop. | A loop inside a loop needs its own flag or `return`. |
| `continue` | Starts the next pass. | The loop's update still happens for a `for` range. |
| `switch value { ... }` | Chooses an arm by shape. | Chapter 5 covers the patterns. |

The loop variable of a `for` range is a `const` binding. `for i in 0..3 { i += 1; }`
reports ``error: cannot assign to `i`, which is immutable``. If a loop needs a
counter it can change, declare a `var` before the loop and update that.

`break` and `continue` belong to the innermost enclosing loop. There is no
label syntax, so a loop that must stop an outer loop sets a `var` flag and
tests it, or returns from the function. This is a deliberate limit: labels are
another name to invent and another scope to get wrong.

## `switch` in statement position

`switch` chooses by shape, not by equality; chapter 5 covers it. In statement
and expression position alike, an arm's body is one expression, optionally
followed by `;`; a second statement is a parse error, which is why an arm that
needs several statements calls a function. The switch must cover every shape --
or end in `default` -- in either position.

The one-expression rule keeps the grammar small. `case Some(n) { print_i64(n); }`
works because a call is an expression. Adding `print_str("\n");` after it makes
the arm a block with two statements, and the parser stops at the second one:

```text
error: expected `}`, found an identifier
```

Put the pair in a function and call it from the arm if you need more than one
step.

## Common mistakes

- **Using a number as a condition.** `if n { ... }` reports
  ``error: expected `bool`, found `int` ``. Write `if n != 0`.
- **Assigning to the loop variable.** `for i in 0..3 { i += 1; }` reports
  ``error: cannot assign to `i`, which is immutable``. Keep a separate `var`.
- **Putting two statements in a switch arm.** The second statement is reported
  as ``error: expected `}`, found an identifier``. Call a function instead.
- **Leaving out an enum arm.** A `switch` over `Option` with only `Some` is
  reported as ``error: switch is not exhaustive: missing case `None` ``.
  Add the arm, or a `default`.
- **Using literal arms without a `default`.** `switch n { case 0 { ... } }`
  reports ``error: switch over a non-enum type is not exhaustive: add a `default` arm``.
  An `int` has too many values to list.

## Try it

1. Sum the integers from 1 to 100, skipping multiples of 3. Use `for` and
   `continue`, then write the same sum with a `while` loop. Hint: the skip test
   belongs before the addition, and `continue` goes straight to the next pass.

2. Print a countdown from 5 to 1 with `while true`, stopping with `break`.
   Then put the countdown inside a `for` loop and see which loop `break`
   leaves. Hint: it leaves the `while`, not the `for`.

## How it works

`while cond { ... }` is parsed as the same node as a `for` loop with only a
condition, so the two cannot drift apart. A range loop becomes a counter, an
end value, and a comparison at the top of each pass; an inverted range fails
that comparison immediately. A `switch` is lowered to a chain of tag or literal
tests, and an expression-position switch writes each arm's value into one
result slot that the surrounding expression reads. In statement position there
is no result slot, which is why the arm does not need to produce a value of a
particular type.

Continue to [04-structs](04-structs.md).
