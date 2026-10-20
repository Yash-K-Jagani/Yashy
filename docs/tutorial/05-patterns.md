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

The language also has user-declared enums, matched the same way, but a program
in this milestone can only *match* one: constructing a variant with a payload
is not implemented yet. `Option` and `Result` are the two enums the language
provides constructors for, and they cover every "missing" and "failed" case a
program in the tutorial needs.

Continue to [06-errors](06-errors.md).
