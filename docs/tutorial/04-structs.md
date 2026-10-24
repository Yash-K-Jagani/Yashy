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

```yashy
struct Point {
    x: int,
    y: int,
}

struct Line {
    from: Point,
    to: Point,
}

fn manhattan(p: Point) -> int {
    return p.x + p.y;
}

fn main() {
    const p = Point { y: 4, x: 3 };
    print(p.x);
    print(" ");
    print(p.y);
    println();
    // => 3 4

    const line = Line { from: Point { x: 1, y: 2 }, to: p };
    print(line.from.x + line.to.y);
    println();
    // => 5

    print(manhattan(line.from));
    println();
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

```yashy
struct Point {
    x: int,
    y: int,
}

impl Point {
    fn sum(self) -> int {
        return self.x + self.y;
    }
}

fn main() {
    const p = Point { x: 2, y: 3 };
    print(p.sum());
    println();
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

```yashy
enum Shape {
    Circle(int),
    Square(int),
}

impl Shape {
    fn area(self) -> int {
        return switch self {
            case Circle(r) { r * r * 3 }
            case Square(s) { s * s }
        };
    }
}

fn main() {
    const circle = Shape.Circle(4);
    print(circle.area());
    println();
    // => 48
}
```

A method that does not exist is reported by name and by type:
``error: no method `area` on `Point` ``.

## Objects

An `object` is a type with its fields and its methods in one declaration. It
is a struct and an `impl` block written together.

```yashy
object Counter {
    value: int,
    fn get(self) -> int {
        return self.value;
    },
    fn add(self, n: int) -> Counter {
        return Counter { value: self.value + n };
    },
}

fn main() {
    const counter = Counter { value: 3 };
    print(counter.get());
    println();
    // => 3

    print(counter.add(4).get());
    println();
    // => 7
}
```

The object is constructed with the ordinary struct-literal syntax, and its
methods are called like any other method. A method may be called on the
result of another one, as `counter.add(4).get()` does.

## Static methods

A method with no `self` belongs to the type rather than to a value of it, and
it is called on the type.

```yashy
object Counter {
    value: int,
    fn new(start: int) -> Counter {
        return Counter { value: start };
    },
    fn get(self) -> int {
        return self.value;
    },
}

fn main() {
    print(Counter.new(3).get());
    println();
    // => 3
}
```

`Counter.new(3)` is the type's method, and `counter.get()` is the value's. A
method that needs a receiver cannot be called on the type: it says so, rather
than pretending the type is a value of itself.

## Mutating methods

A method that takes `self` receives a *copy* of the object, so it can read the
object but cannot change the caller's. To change it, the method takes the
object's address instead:

```yashy
object Counter {
    value: int,
    fn new() -> Counter {
        return Counter { value: 0 };
    },
    fn bump(self: *mut Counter) {
        self[0].value = self[0].value + 1;
    },
    fn get(self) -> int {
        return self.value;
    },
}

fn main() {
    var c = Counter.new();
    c.bump();
    c.bump();
    print(c.get());
    println();
    // => 2
}
```

Three things make this work, and each is a rule worth remembering:

- `self: *mut Counter` is a **pointer receiver**. The parameter's type is an
  address, so `self` is the caller's object rather than a copy.
- `self[0]` is the object at that address. The index is `0` because the
  receiver is exactly one object; a pointer indexes its own pointee, so
  `self[1]` would be the next object, which there is no promise of.
- `self[0].value = ...` writes through the address, so the write lands in the
  caller's binding.

Because the method writes through the address, the receiver has to be a
mutable binding. Calling `bump` on a `const` is an error: "`bump` needs a
mutable object binding". So is calling it on a temporary, such as the result
of another call -- there is no address of a value that has no home.

A pointer receiver's pointee must be the type the method belongs to. `fn bump(self:
*mut Counter)` inside `impl Other` is refused, because a method of one type
must not be able to write through another's address.

## The operations

| Operation | Syntax | Meaning |
|---|---|---|
| Declare | `struct Point { x: int, y: int }` | Names and types, in declaration order. |
| Construct | `Point { x: 3, y: 4 }` | Any field order; every field once. |
| Read a field | `p.x`, `line.from.x` | A chain follows nested structs. |
| Assign a field | `q.x = 9` | The binding must be `var`. |
| Copy | `var q = p;` | A new value; later changes are independent. |
| Pass | `manhattan(p)` | The parameter receives a copy. |
| Mutate through a method | `fn bump(self: *mut Point)` | The receiver is the object's address; call it on a `var`. |
| Hold an owner | `struct Box { name: string }` | The struct owns the reference, and so does each copy. |

Named fields are not only documentation. The declaration's order is the layout
the compiler emits, while the literal's order is the order you happened to
type. Keeping those separate means inserting a field in the declaration does
not require rewriting every construction site, and a reader can see which
value belongs to which name without counting commas.

## Owning fields

A struct may hold a `string` or an array. A `string` field is a *reference*:
the struct owns one, and so does every copy of the struct. The struct literal
takes the reference over from the value it was given, a copy retains one of
its own, and the binding's scope releases what it holds when it ends. A struct
that goes out of scope therefore leaves nothing behind.

```yashy
struct Named {
    name: string,
    count: int,
}

fn main() {
    const named = Named { name: "yas" + "hy", count: 2 };
    print(named.name);
    println();
    // => yashy
}
```

An array field is a different matter: an array owns its backing store, and
the store belongs to the array the field was built from. A struct that holds
one does not own it, so the array is freed by the binding that made it, and
the struct's field is a header that points at it. The ownership rules for
arrays are in chapter 9.

In practice: a struct of numbers and booleans is copied freely. A struct that
holds a string owns that string, and every copy of the struct owns it too. A
struct that holds an array points at an array somebody else owns.

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

1. Declare `struct Rectangle { width: int, height: int }` and write
   `fn area(r: Rectangle) -> int`. Construct one with the fields in the
   opposite order from the declaration, and confirm the area is the same.
   Hint: only the names matter at the literal.

2. Extend the chapter's `Line` with `fn midpoint(line: Line) -> Point`. Build a
   point from the two `from` and `to` coordinates. Hint: read `line.from.x` and
   `line.to.x`; there is no operator to average a whole struct for you.

3. Add `fn set_x(self: *mut Point, x: int)` and `fn shifted(self, dx: int) ->
   Point` to `Point`. Confirm that `set_x` changes the caller's point and that
   `shifted` does not. Hint: the first needs a `var` at the call site; the
   second needs nothing.

## How it works

A struct is an LLVM aggregate value, not a pointer. Assigning one loads and
stores the whole aggregate, and passing one to a function passes its fields by
value. Field assignment is a read-modify-write: the compiler loads the
aggregate, replaces one field with `insertvalue`, and stores it back. That is
also why there is no alias to worry about for a struct of numbers.

A `string` field is the one part of a struct that is tracked: the module records
each struct's field types, so construction can retain a reference for a field
that owns one, a copy can retain its own, and the binding's scope can release
what it holds.

A pointer receiver is the one path that writes without a read-modify-write. The
receiver is an address, so `self[0].value = ...` is a `getelementptr` at the
field's offset and a `store` — there is no aggregate to load and rebuild. That
is also why the call site has to pass a binding: the pointer is the local's own
storage, and a temporary has none.

Continue to [05-patterns](05-patterns.md).
