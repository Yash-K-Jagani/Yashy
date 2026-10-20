# 6. Errors without exceptions

A function that can fail returns `Result[T, E]`: `Ok(value)` or `Err(error)`.
There is no exception stack and no hidden unwinding. `?` propagates an `Err`
to the caller, and the caller's signature already promises it can happen.

```otter
fn half(n: i64) -> Result[i64, string] {
    if n % 2 != 0 {
        return Err("odd");
    }
    return Ok(n / 2);
}

fn quarter(n: i64) -> Result[i64, string] {
    const h = half(n)?;
    return Ok(half(h)?);
}

fn report(result: Result[i64, string]) {
    const message = switch result {
        case Ok(v) { "ok " + str_from_i64(v) }
        case Err(e) { "error " + e }
    };
    print_str(message);
    print_str("\n");
}

fn main() {
    report(quarter(8));
    report(quarter(6));
    // => ok 2
    // => error odd
}
```

`quarter(6)` calls `half(6)`, which is `Ok(3)`, then `half(3)`, which is
`Err("odd")`; the `?` returns that `Err` from `quarter` immediately. No line
after a failed `?` runs.

`?` is only legal in a function whose return type is a `Result` with the same
error type. That is the whole mechanism: a function cannot fail silently, and a
caller cannot ignore the possibility without matching it.

Continue to [07-memory](07-memory.md).
