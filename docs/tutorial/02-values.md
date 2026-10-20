# 2. Values and variables

The scalar types are `i64`, `f64`, `bool`, and `string`. `const` binds a name
once and never rebinds; `var` binds a name that can be assigned again.
Assignment operators like `+=` are shorthand for the expanded form.

```otter
fn main() {
    const count = 3;
    var total = count * 7;
    total += 1;

    const ratio = 0.5;
    const ready = true;

    print_str("total = ${total}\n");
    print_str("half of total = ");
    print_i64((total as f64 * ratio) as i64);
    print_str("\n");
    print_str("ready = ${ready}\n");
    // => total = 22
    // => half of total = 11
    // => ready = true
}
```

Two things to notice:

- `${...}` inside a string converts and concatenates. `"total = ${total}"` is
  the same program as `"total = " + str_from_i64(total)`; the checker lowers it
  into exactly that.
- Arithmetic between `i64` and `f64` needs an explicit cast, `as`. The language
  will not silently pick one for you. The result is cast back to `i64` before
  printing: `print_f64`'s formatting is the C library's, and the tutorial only
  claims output that does not depend on it.

There are no implicit truthiness rules: a condition is a `bool`, not an
integer.

Continue to [03-control-flow](03-control-flow.md).
