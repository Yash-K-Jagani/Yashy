# 5. Matching

## What you will learn

- How `switch` reads a value by shape and binds its parts.
- Why every enum arm must be present, and when `default` is required.
- How `Option`, `Result`, and your own enums are the same mechanism.
- How to write and match the three kinds of enum variant.

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

## Try it

1. Add `Square(f64)` to `Shape` and update `area`, then call it with each
   variant and print the four results. Hint: the compiler names the missing arm.

2. Write `fn describe_result(r: Result[i64, string]) -> string` that returns a
   sentence for both `Ok` and `Err`. Call it with a successful division and
   with a division by zero. Hint: both arms must be `string`; use
   `str_from_i64` on the success value.

## How it works

An enum is a tag plus the payload space of its largest variant. The compiler
emits one LLVM struct per enum: an `i8` tag in field 0, followed by every
variant's fields flattened in declaration order. The tag is the variant's
index; a payload field's position is 1 plus the number of payload fields
declared before it. Matching
compares the tag, branches to the arm, and extracts the fields the pattern
names. Literal arms lower to an integer, float, or string comparison.
`Option` and `Result` use the same layout and matching code.

Continue to [06-errors](06-errors.md).
