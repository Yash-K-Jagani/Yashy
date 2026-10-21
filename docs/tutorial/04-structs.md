# 4. Structs

## What you will learn

- How to declare a struct and construct a value with named fields.
- Why the order of fields in a literal does not matter.
- What copying a struct does, and what it does not do.
- How to give a type its own methods.
- Which struct shapes are safe in this milestone and which ones leak.

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
equal and independent. The chapter's `p` and `line.to` are two independent
`Point` values. Writing `line.to.x` would not change `p.x`, and changing a
`var` copy does not change the original:

```text
var q = p;
q.x = 9;
// p.x is still 3, q.x is 9
```

That independence is the reason a struct can be passed to a function without
ceremony. `manhattan` receives its own `Point`; the caller's value is not
borrowed or mutated.

## Methods

An `impl` block gives a type its own methods. A method is a function whose
first parameter is the value it is called on, and it is written `self`.

```otter
struct Point {
    x: i64,
    y: i64,
}

impl Point {
    fn sum(self) -> i64 {
        return self.x + self.y;
    }
}

fn main() {
    const p = Point { x: 2, y: 3 };
    print_i64(p.sum());
    print_str("\n");
    // => 5
}
```

`p.sum()` is `Point.sum` with `p` in front. The method is found by the *type* of
the value, not by the name, so two types may each have a method called `sum`.

`self` carries no type of its own: the block already says what the receiver
is. Inside the method, `Self` means the same type, and `self.x` reads a field
of the receiver.

An enum may have methods too, and a method on an enum usually switches on the
receiver:

```otter
enum Shape {
    Circle(i64),
    Square(i64),
}

impl Shape {
    fn area(self) -> i64 {
        return switch self {
            case Circle(r) { r * r * 3 }
            case Square(s) { s * s }
        };
    }
}

fn main() {
    const circle = Shape.Circle(4);
    print_i64(circle.area());
    print_str("\n");
    // => 48
}
```

A method that does not exist is reported by name and by type:
``error: no method `area` on `Point` ``.

## The operations

| Operation | Syntax | Meaning |
|---|---|---|
| Declare | `struct Point { x: i64, y: i64 }` | Names and types, in declaration order. |
| Construct | `Point { x: 3, y: 4 }` | Any field order; every field once. |
| Read a field | `p.x`, `line.from.x` | A chain follows nested structs. |
| Assign a field | `q.x = 9` | The binding must be `var`. |
| Copy | `var q = p;` | A new value; later changes are independent. |
| Pass | `manhattan(p)` | The parameter receives a copy. |
| Hold an owner | `struct Box { name: string }` | Allowed, but see the limitation below. |

Named fields are not only documentation. The declaration's order is the layout
the compiler emits, while the literal's order is the order you happened to
type. Keeping those separate means inserting a field in the declaration does
not require rewriting every construction site, and a reader can see which
value belongs to which name without counting commas.

## Owning fields are an interim limitation

A struct may also hold a `string` or an array, but the backend does not track
ownership per field. Storing a `string` stores the pointer without a retain,
and leaving the struct's scope emits no release. If the string came from a
local binding, that binding's own release can leave the field dangling; if it
came from a temporary, nothing frees it. Copying the struct copies the handle,
so the copy is not an independent owner. `Point` and `Line` above are the
supported shape; ledger L4 records owning fields as an interim limitation.

In practice: use a struct of numbers and booleans when the value is copied or
passed around. If a struct must hold a string, keep the string's own binding
alive for as long as any copy of the struct is used.

## Common mistakes

- **Leaving out a field.** `Point { x: 1 }` reports
  ``error: missing field `y` in struct `Point` ``. Every field needs a value;
  there are no defaults.
- **Naming a field that does not exist.** `Point { x: 1, y: 2, z: 3 }` reports
  ``error: unknown field `z` in struct `Point` ``. Check the declaration.
- **Assigning a field of a `const`.** `p.x = 9` on a `const p` reports
  ``error: cannot assign to a field of `p`, which is immutable``. Declare the
  binding with `var` when it must change.
- **Comparing two structs with `==`.** The checker accepts `p == q`, but the
  generated IR is rejected by LLVM with `icmp requires integer operands`. There
  is no structural equality yet; compare the fields one at a time.
- **Expecting a copy to share updates.** `var q = p; q.x = 9;` leaves `p.x`
  unchanged. A struct is a value, not a reference. Use a pointer when two
  names must see one object.

## Try it

1. Declare `struct Rectangle { width: i64, height: i64 }` and write
   `fn area(r: Rectangle) -> i64`. Construct one with the fields in the
   opposite order from the declaration, and confirm the area is the same.
   Hint: only the names matter at the literal.

2. Extend the chapter's `Line` with `fn midpoint(line: Line) -> Point`. Build a
   point from the two `from` and `to` coordinates. Hint: read `line.from.x` and
   `line.to.x`; there is no operator to average a whole struct for you.

## How it works

A struct is an LLVM aggregate value, not a pointer. Assigning one loads and
stores the whole aggregate, and passing one to a function passes its fields by
value. Field assignment is a read-modify-write: the compiler loads the
aggregate, replaces one field with `insertvalue`, and stores it back. That is
also why there is no alias to worry about for a struct of numbers. Owning fields
are the exception the backend deliberately does not track: storing a string in
a field stores the pointer without a retain, and leaving scope does not emit a
release, because the code generator has no per-field ownership information to
act on.

Continue to [05-patterns](05-patterns.md).
