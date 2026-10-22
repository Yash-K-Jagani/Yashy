# 5. Matching

## What you will learn

- How `switch` reads a value by shape and binds its parts.
- Why every enum arm must be present, and when `default` is required.
- How `Option`, `Result`, and your own enums are the same mechanism.
- How to write and match the three kinds of enum variant.
- How a guard adds a condition to an arm, and why it cannot replace a
  `default`.
- How a tuple carries several values at once, and how destructuring unpacks
  one into separate bindings.

`switch` matches a value against shapes. An arm binds what it names and
evaluates one expression. In expression position the switch *is* that
expression, so it can be returned; every shape must be covered.

The two shapes every program meets are `Option[T]` and `Result[T, E]`:

```otter
fn describe(value: Option[i64]) -> string {
    return switch value {
        case Some(n) { "some " + str_from_i64(n) }
        case None { "none" }
    };
}

fn divide(a: i64, b: i64) -> Result[i64, string] {
    if b == 0 {
        return Err("division by zero");
    }
    return Ok(a / b);
}

fn main() {
    print_str(describe(Some(5)));
    print_str("\n");
    print_str(describe(None));
    print_str("\n");
    print_str(switch divide(10, 2) {
        case Ok(v) { str_from_i64(v) }
        case Err(e) { "error: " + e }
    });
    print_str("\n");
    // => some 5
    // => none
    // => 5
}
```

`Option[T]` means "a `T` or nothing": `Some(value)` or `None`. There is no
`null`. If a function can return nothing, its type says `Option[T]` and the
caller must handle `None`; `describe(None)` checks because the parameter's type
supplies the payload.

`Result[T, E]` means "a `T` or an error": `Ok(value)` or `Err(error)`. There is
no exception unwinding; chapter 6 shows `?`, which propagates an `Err`.

Exhaustiveness is the point, not a formality. If a `switch` could fall through
an unhandled `None`, the program would need a default value for a `T` it does
not have. Requiring the arm turns "I forgot the empty case" into a compile
error, the same job `null` checks do elsewhere.

A `switch` may also compare literals. The subject has infinitely many values in
that case, so a `default` arm is required to make the match exhaustive:

```otter
fn classify(n: i64) -> string {
    return switch n {
        case 0 { "zero" }
        case 1 { "one" }
        default { "many" }
    };
}

fn main() {
    print_str(classify(0) + " " + classify(1) + " " + classify(9) + "\n");
    // => zero one many
}
```

Literal arms work for `i64`, `f64`, `bool`, and `string`; a `string` compares
contents, not identity. They bind nothing and always need a `default`, and a
literal cannot match an enum subject, because the literal's type and the enum's
type differ.

## Your own enums

An enum declares a fixed set of variants. A variant may carry no payload, one
or more positional fields, or named fields:

```otter
enum Shape {
    Circle(f64),
    Rect { w: f64, h: f64 },
    Point,
}

fn area(shape: Shape) -> f64 {
    return switch shape {
        case Circle(r) { 3.0 * r * r }
        case Rect { w: w, h: h } { w * h }
        case Point { 0.0 }
    };
}

fn main() {
    const circle = Shape.Circle(2.0);
    const rect = Shape.Rect { w: 3.0, h: 4.0 };
    const point = Shape.Point;
    print_f64(area(circle));
    print_str("\n");
    print_f64(area(rect));
    print_str("\n");
    print_f64(area(point));
    print_str("\n");
    // => 12
    // => 12
    // => 0
}
```

Construction spells the enum, then the variant: `Shape.Circle(2.0)` for
positional fields, `Shape.Rect { w: 3.0, h: 4.0 }` for named ones (any order),
and `Shape.Point` for a variant with no payload. A payload's type comes from
the declaration, so `Shape.Circle(2)` is a type error: the field is `f64`.

Arm shapes follow the variant's shape. A positional payload binds by position:
`case Circle(r)`. A named payload binds by name, and the pattern names *every*
field as `field: binding`: `case Rect { w: w, h: h }`. A unit variant is
`case Point`. The switch must cover every variant, exactly as it must cover
`Some` and `None`; `default` catches the rest.

`Option` and `Result` are ordinary enums too, with supplied constructors.

## Guards

An arm may add a condition after its pattern: `case Circle(r) if r > 10.0`. The
pattern binds its names first, then the guard is checked, and only then does the
body run. A guard that is false means the arm did not match after all: the
search continues with the next arm exactly as if the pattern had failed.

```otter
enum Shape {
    Circle(f64),
    Rect { w: f64, h: f64 },
}

fn describe(shape: Shape) -> string {
    return switch shape {
        case Circle(r) if r > 10.0 { "big circle" }
        case Circle(r) { "small circle" }
        case Rect { w: w, h: h } if w == h { "square" }
        case Rect { w: w, h: h } { "rectangle" }
    };
}

fn main() {
    print_str(describe(Shape.Circle(20.0)) + "\n");
    print_str(describe(Shape.Circle(2.0)) + "\n");
    print_str(describe(Shape.Rect { w: 3.0, h: 3.0 }) + "\n");
    print_str(describe(Shape.Rect { w: 3.0, h: 4.0 }) + "\n");
    // => big circle
    // => small circle
    // => square
    // => rectangle
}
```

The order of the arms is the order of the search, so the more specific arm comes
first. `Circle(20.0)` reaches the guarded arm, passes the guard, and stops;
`Circle(2.0)` reaches the same arm, fails the guard, and falls through to the
unguarded `Circle` arm below it.

A guard may follow a pattern that matches everything, and then it still has a
place in the search order:

```otter
fn bucket(n: i64) -> i64 {
    return switch n {
        case 0 { 100 }
        case _ if n < 0 { 200 }
        default { 300 }
    };
}

fn main() {
    print_i64(bucket(0));
    print_str("\n");
    print_i64(bucket(0 - 5));
    print_str("\n");
    print_i64(bucket(5));
    print_str("\n");
    // => 100
    // => 200
    // => 300
}
```

`bucket(0 - 5)` reaches `case _ if n < 0`, the guard is true, and the arm runs.
`bucket(5)` reaches the same arm, the guard is false, and the `default` below it
runs -- a `_` arm that is guarded is *tried*, not treated as a catch-all.

A guard does not prove anything about coverage. `case A if c` leaves `A`
uncovered, because the guard can be false; the switch still needs an unguarded
arm for `A` (or a `default`). That is why the examples above pair every guarded
arm with an unguarded one. The guard is a condition on what the arm wants, not a
claim about what the value can be.

## Tuples and destructuring

A tuple is an anonymous product: `(i64, i64)` is two integers, in that order.
Position is part of the type, so `(i64, bool)` and `(bool, i64)` are different
types, and a tuple is not a struct -- its elements have no names. A function
returns both of its answers as one tuple:

```otter
fn min_max(a: i64, b: i64) -> (i64, i64) {
    if a < b {
        return (a, b);
    }
    return (b, a);
}

fn main() {
    const pair = min_max(9, 4);
    const (low, high) = pair;
    print_i64(low);
    print_str("\n");
    print_i64(high);
    print_str("\n");

    var (first, _) = min_max(2, 7);
    first = first + 1;
    print_i64(first);
    print_str("\n");
    // => 4
    // => 9
    // => 3
}
```

`(a, b)` is a tuple literal, and `(a)` is just `a` in parentheses: the comma is
what makes a tuple. The one-element spelling keeps the comma -- `(5,)` -- so
there is never a question about which one was written.

Destructuring is a statement, with the same three keywords as any binding:

```otter
fn main() {
    const pair = (1, (2, 3));
    const (x, rest) = pair;
    const (y, z) = rest;
    print_i64(x + y + z);
    print_str("\n");
    // => 6
}
```

`const` binds each name immutably, while `var` and `let` bind them mutably.
`_` skips an element: it binds nothing and allocates nothing. The number of
names must match the number of elements -- a tuple's arity is part of its type,
so a mismatch is a compile error rather than a missing value.

Tuples hold the built-in number types, `bool`, and other tuples of those. They
do not yet hold `string`, a slice, a pointer, or a struct, because a tuple is
copied by copying its bytes, and a second copy of a reference is not yet defined
for those types. There is also no field access (`pair.0`) and no tuple equality:
unpack the tuple and compare the parts.

```otter
fn main() {
    const pair = (1, 2);
    const (a, _) = pair;
    print_i64(a);
    print_str("\n");
    // => 1
}
```

## Patterns at a glance

| Pattern | Used for | Binds | Example |
|---|---|---|---|
| `case Some(n)` | Positional payload. | `n` is the payload value. | `Option[i64]` |
| `case None` | Unit variant. | Nothing. | `Option[i64]` |
| `case Ok(v)` / `case Err(e)` | `Result` payloads. | The success or error value. | `Result[i64, string]` |
| `case Circle(r)` | A user variant with one positional field. | `r`. | `Shape` |
| `case Rect { w: w, h: h }` | Named fields; every field is written. | `w` and `h`. | `Shape` |
| `case Point` | A variant with no payload. | Nothing. | `Shape` |
| `case 0` / `default` | Literal arms. | Nothing; `default` is required. | `i64`, `f64`, `bool`, `string` |
| `case Circle(r) if r > 10.0` | An arm with a guard. | `r`, then the guard is checked. | Any pattern |
| `const (a, b) = pair` | Destructuring statement. | Each name, by position; `_` skips. | A tuple value |

The `field: binding` spelling is not optional shorthand. `case Rect { w, h }`
is a parse error at the comma, because the parser is looking for `:` after the
first name. Writing both halves shows which field binds to which local name.

## Common mistakes

- **Leaving out `None`.** A switch over `Option` with only `Some` reports
  ``error: switch is not exhaustive: missing case `None` ``. Add the arm.
- **Using literal arms without `default`.** `switch n { case 0 { ... } }`
  reports ``error: switch over a non-enum type is not exhaustive: add a `default` arm``.
  A number has too many possible values to list.
- **Writing `case Rect { w, h }`.** The parser reports
  ``error: expected `}`, found `,` ``. Write `case Rect { w: w, h: h }`.
- **Passing the wrong payload type.** `Shape.Circle(2)` reports
  ``error: expected `f64`, found `i64` ``. Write `2.0`.
- **Writing `const x = None;` with no context.** The checker reports
  ``error: cannot infer the payload type of `None` ``. Annotate the binding or
  pass `None` where the expected type is known.
- **Using a guard as coverage.** `switch e { case A if c { ... } case B { ... } }`
  reports ``error: switch is not exhaustive: missing case `A` ``. The guard can
  be false, so add an unguarded `A` arm or a `default`.
- **A guard that is not a `bool`.** `case A(v) if v` reports
  ``error: a guard must be a `bool`, found `i64` ``. Write the comparison:
  `if v > 0`.
- **Putting a `string` in a tuple.** `(1, "s")` reports
  ``error: a tuple cannot hold `string` ``. Keep the string in its own binding;
  tuples are for the values that are copied by their bytes.
- **Destructuring the wrong arity.** `const (a, b) = (1, 2, 3)` reports
  ``error: this pattern binds 2 names, but the tuple has 3 elements``.

## Try it

1. Add `Square(f64)` to `Shape` and update `area`, then call it with each
   variant and print the four results. Hint: the compiler names the missing arm.

2. Write `fn describe_result(r: Result[i64, string]) -> string` that returns a
   sentence for both `Ok` and `Err`. Call it with a successful division and
   with a division by zero. Hint: both arms must be `string`; use
   `str_from_i64` on the success value.

3. Write `fn min_max(a: i64, b: i64) -> (i64, i64)` and a `switch` over the
   smaller answer: `case 0` prints "tie", `case n if n < 10` prints "close",
   `default` prints "far". Call it with `(4, 4)`, `(3, 9)`, and `(1, 40)`.
   Hint: destructure the tuple first, then switch on one element.

## How it works

An enum is a tag plus the payload space of its largest variant. The compiler
emits one LLVM struct per enum: an `i8` tag in field 0, followed by every
variant's fields flattened in declaration order. The tag is the variant's
index; a payload field's position is 1 plus the number of payload fields
declared before it. Matching
compares the tag, branches to the arm, and extracts the fields the pattern
names. Literal arms lower to an integer, float, or string comparison.
`Option` and `Result` use the same layout and matching code.

A guard is a branch of its own. The arm binds its pattern, evaluates the guard,
and branches to the body when it is true. When it is false the branch goes to
the *next test in the arm chain* -- the same block the tag test would have gone
to on a mismatch -- so a false guard and a failed pattern are the same edge. A
pattern that matches everything gets an unconditional entry in that chain when
it has a guard, which is what keeps `case _ if c` ahead of the arms below it.

A tuple is an LLVM literal struct whose fields are the element types in order,
so `(i64, f64)` is `{ i64, double }` and a nested tuple is a nested struct. The
value is built with one `insertvalue` per element onto `undef`; there is no
allocation and no owner. Destructuring is the inverse: one `extractvalue` per
name into that binding's own stack slot, and `_` extracts nothing. Because the
representation is a struct of plain bytes, passing and returning a tuple passes
and returns the aggregate itself, with no hidden pointer.

Continue to [06-errors](06-errors.md).
