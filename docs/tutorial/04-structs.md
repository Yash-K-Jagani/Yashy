# 4. Structs

A struct groups named fields. It is a value: constructing it copies it,
passing it copies it, and a field is read with `.`. A literal may spell its
fields in any order; the declaration fixes the layout, not the literal.

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

A struct with a `string` field owns that string when it is constructed, and
releases it when the struct's binding is released. That is why a struct
containing a `string`, a slice, or another owning value is linear: it cannot be
copied, only moved or borrowed. A struct of numbers and booleans -- like the
two above -- is copyable.

Continue to [05-patterns](05-patterns.md).
