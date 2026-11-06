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
    let i = 1;
    while i <= 5 {
        if i % 2 == 0 {
            print("even");
        } else {
            print("odd");
        }
        i += 1;
    }
    // => odd
    // => even
    // => odd
    // => even
    // => odd

    let sum = 0;
    for n in 0..5 {
        if n == 3 {
            continue;
        }
        sum += n;
    }
    print(sum);
    // => 7

    let count = 0;
    while true {
        count += 1;
        if count == 3 {
            break;
        }
    }
    print(count);
    // => 3
}
```

`for n in 0..5` walks the half-open range 0, 1, 2, 3, 4. The same form walks an
array element by element (chapter 9).

## Nested loops

A loop is a statement like any other, so a loop may be written inside another
loop's body: nested loops. The inner loop runs to completion for every pass
of the outer one.

```yashy
fn main() {
    for i in 0..3 {
        for j in 0..3 {
            print(i, j);
        }
    }
    // => 0 0
    // => 0 1
    // => 0 2
    // => 1 0
    // => 1 1
    // => 1 2
    // => 2 0
    // => 2 1
    // => 2 2
}
```

Nesting adds no new scope rule: each loop's variable lives in its own body,
and a `break` or `continue` still belongs to the innermost enclosing loop —
which is what labels (below) reach past.

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
| `break` | Leaves the innermost loop. | `break name;` leaves the loop labelled `name` instead. |
| `continue` | Starts the next pass. | `continue name;` re-tests the loop labelled `name`. |
| `switch value { ... }` | Chooses an arm by shape. | Chapter 5 covers the patterns. |

The loop variable of a `for` range is a `const` binding. `for i in 0..3 { i += 1; }`
reports ``error: cannot assign to `i`, which is immutable``. If a loop needs a
counter it can change, declare a `let` before the loop and update that.

`break` and `continue` belong to the innermost enclosing loop by default. A
loop that must stop an outer loop carries a label: write `name:` before the
loop and jump to it with `break name;` or `continue name;`. The label is
visible only inside its own loop's body, and two nested loops may not share
one — `break outer;` inside an inner `outer:` loop would be ambiguous, so it
is an error (`E0605`). A jump that names no enclosing label is `E0604`.

```yashy
fn main() {
    outer: for i in 0..3 {
        for j in 0..3 {
            if j == 1 {
                break outer;
            }
        }
        print("unreachable");
    }
    print("out");
    // => out

    let visited = 0;
    outer: for i in 0..3 {
        visited = visited + 1;
        for j in 0..2 {
            if j == 0 {
                continue outer;
            }
        }
        print("unreachable");
    }
    print(visited);
    // => 3
}
```

`continue outer;` skips the rest of the outer body and re-tests the outer
loop, exactly as `break` leaves it. A label never changes an unlabelled
`break`: it still leaves the innermost loop only. And a `defer` inside the
inner loop runs when a labelled jump fires, because the jump only ever emits
a real `break` or `continue` under the hood.

## `switch` in statement position

`switch` chooses by shape, not by equality; chapter 5 covers it. In statement
and expression position alike, an arm's body is one expression, optionally
followed by `;`; a second statement is a parse error, which is why an arm that
needs several statements calls a function. The switch must cover every shape --
or end in `default` -- in either position.

The one-expression rule keeps the grammar small. `case Some(n) { print(n); }`
works because a call is an expression. Adding `print();` after it makes
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
  ``error: cannot assign to `i`, which is immutable``. Keep a separate `let`.
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
