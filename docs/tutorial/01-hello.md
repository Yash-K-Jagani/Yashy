# 1. Hello, functions

A program is a list of items: functions, structs, enums, imports. `main` is
where execution starts. A function is `fn name(parameters) -> return_type`,
and the body is a block of statements ending, optionally, in `return`.

```otter
fn greeting(name: string) -> string {
    return "Hello, " + name + "!";
}

fn main() {
    // A comment the compiler ignores.
    print_str(greeting("Otter"));
    print_str("\n");
    // => Hello, Otter!
}
```

`+` concatenates strings. There is no automatic conversion from a number to a
string: `str_from_i64(n)` and `str_from_f64(x)` do that, and string
interpolation (chapter 2) is sugar over them.

A function that returns nothing writes no `->`:

```otter
fn banner() {
    print_str("===\n");
}

fn main() {
    banner();
    print_str("done\n");
    // => ===
    // => done
}
```

Statements end with `;` except a block. `return` may be bare (`return;`) in a
function with no return type, or carry a value in one with a return type.

Continue to [02-values](02-values.md).
