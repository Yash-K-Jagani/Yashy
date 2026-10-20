# 3. Control flow

`if` is a statement; its condition is a `bool`. `while cond { ... }` checks at
the top, `while true { ... }` is the infinite loop (there is no separate `loop`
keyword), and `break` and `continue` leave and restart it.

```otter
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

`for n in 0..5` walks the half-open range 0, 1, 2, 3, 4. The same form walks a
slice element by element (chapter 9).

`switch` chooses by shape, not by equality; chapter 5 covers it. In statement
and expression position alike, an arm's body is one expression, optionally
followed by `;`; a second statement is a parse error, which is why an arm that
needs several statements calls a function. The switch must cover every shape --
or end in `default` -- in either position.

Continue to [04-structs](04-structs.md).
