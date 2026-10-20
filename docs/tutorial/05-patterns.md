# 5. Matching

`switch` matches a value against shapes. An arm binds what it names and
evaluates one expression. In expression position the switch *is* that
expression, so it can be returned directly. Every shape must be covered: a
missing arm is a compile error, not a runtime `default`.

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
`null` and no uninitialized value. If a function can return nothing, its return
type says `Option[T]` and the caller must handle `None`. `describe(None)` type
checks because the parameter's type says which `Option` is meant.

`Result[T, E]` means "a `T` or an error": `Ok(value)` or `Err(error)`. There is
no exception unwinding; chapter 6 shows `?`, which propagates an `Err` to the
caller.

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
`case Circle(r)`. A named payload binds by name, and the pattern always names
*every* field as `field: binding`: `case Rect { w: w, h: h }`. A unit variant
is just `case Point`. The switch must cover every variant, exactly as it must
cover `Some` and `None`; `default` is accepted as the catch-all for the
remaining ones.

`Option` and `Result` are ordinary enums too, with constructors the language
supplies. The tutorial's own enums are declared the same way.

Continue to [06-errors](06-errors.md).
