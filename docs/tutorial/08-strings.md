# 8. Strings

A `string` is a counted sequence of bytes. `+` concatenates, `len` counts
bytes, `==` compares contents, and `str_slice(text, start, end)` returns a
substring. Bounds are clamped, so a wrong index gives an empty string rather
than a fault.

```otter
fn main() {
    const greeting = "Hello, " + "Otter!";
    print_i64(len(greeting));
    print_str("\n");
    // => 13

    print_str(str_slice(greeting, 0, 5));
    print_str("\n");
    // => Hello

    print_str("${greeting} again\n");
    // => Hello, Otter! again
}
```

`"${greeting} again"` lowers to `greeting + " again"`; interpolation is a
spelling, not a separate runtime path.

Strings are immutable: `str_slice` returns a new string rather than a view, and
`+` allocates. A string value owns its bytes, so it is linear -- it is moved or
borrowed, never silently copied. Passing a string to a function borrows it
unless the parameter is taken with `move`.

A string literal may contain `\n`, `\t`, `\\`, `\"`, and `${...}` for
interpolation. A `//` inside a literal is text, not a comment.

Continue to [09-slices](09-slices.md).
