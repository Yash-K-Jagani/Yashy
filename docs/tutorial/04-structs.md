# 4. Structs

A struct groups named fields. It is a value: assigning it or passing it copies
it, and a field is read with `.`. A literal may spell its fields in any order;
the declaration fixes the layout, not the literal.

```otter
struct Point {
    x: i64,
    y: i64,
}

struct Line {
    from: Point,
    to: Point,
}

fn manhattan(p: Point) -> i64 {
    return p.x + p.y;
}

fn main() {
    const p = Point { y: 4, x: 3 };
    print_i64(p.x);
    print_str(" ");
    print_i64(p.y);
    print_str("\n");
    // => 3 4

    const line = Line { from: Point { x: 1, y: 2 }, to: p };
    print_i64(line.from.x + line.to.y);
    print_str("\n");
    // => 5

    print_i64(manhattan(line.from));
    print_str("\n");
    // => 3
}
```

A struct whose fields are numbers or booleans is copyable: both copies are
equal and independent.

A struct may also hold a `string` or a slice, but this milestone does not yet
release an owning field when the struct's binding goes out of scope, and it does
not count the field when deciding whether the struct may be copied. Two
consequences a reader should know before writing one: the owning field is
**leaked** rather than freed, and a copy of the struct copies the field's handle
without retaining it, so the copy must not outlive the original's allocation.
`Point` and `Line` above are the supported shape; owning fields in structs are
recorded in ledger L4 as an interim limitation (M9's escape analysis is where it
is scheduled to be replaced).

Continue to [05-patterns](05-patterns.md).
