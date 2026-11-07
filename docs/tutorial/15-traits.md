# 15. Traits and objects

## What you will learn

- What a trait is: a requirement set with no state and no default methods.
- How `impl Trait for Type` supplies the requirements, and why there is
  exactly one impl per (trait, type) pair.
- How a generic function is bounded by a trait, and why dispatch is
  static only.
- What the object model refuses: inheritance, `Drop`, trait fields, and
  operator overloading.
- How the iterator protocol makes `for` work over a user type.

A trait names the methods its implementors must provide. It has no fields,
no bodies, and no inheritance: a trait is a list of signatures.

```yashy
trait Show {
    fn show(self) -> string;
}

struct Point {
    x: int,
}

impl Show for Point {
    fn show(self) -> string {
        return "x=${self.x}";
    }
}

fn main() {
    let p = Point { x: 3 };
    print(p.show());
    // => x=3
}
```

The call `p.show()` resolves to the one impl for (`Show`, `Point`). There
is no dynamic dispatch: no `dyn`, no vtable, no runtime type check. The
emitted symbol is `Point.Show.show`, one concrete function, and the
verifier checks it like any other.

A second impl of the same trait for the same type is `E1501`. An impl
that misses a method is `E1502`. An impl that adds one the trait does not
declare is `E1502` as well: the impl provides exactly the requirement set.

## Bounds

A type parameter may be bounded by a trait. Inside the function, only the
bound's methods may be called on that parameter.

```yashy
trait Show { fn show(self) -> string; }

struct Point { x: int }

impl Show for Point {
    fn show(self) -> string { return "point"; }
}

fn describe[T: Show](T x) -> string {
    return x.show();
}

fn main() {
    print(describe(Point { x: 1 }));
    // => point
}
```

Passing a type that does not implement the bound is `E1503` at the call.

## Associated functions

A method with no receiver belongs to the type, not to a value. It is
called through the type: `Point::new(...)`.

```yashy
struct Point { x: int }

impl Point {
    fn new(int x) -> Point {
        return Point { x: x };
    }

    fn origin() -> Point {
        return Point { x: 0 };
    }
}

fn main() {
    print(Point::new(7).x);
    print(Point::origin().x);
    // => 7
    // => 0
}
```

Calling an associated function through a value is `E1507`; calling a
receiver method through the type is `E1506`.

## The iterator protocol

`Iterator[T]` is the one generic trait in the language, and it lives in
the standard library. A type implements it by providing
`next(self: *mut Self) -> Option[T]`. Then `for x in it` works over the
type, lowering to "call `next` until it answers `None`".

```yashy
import iter;

struct Countdown { n: int }

impl Iterator for Countdown {
    fn next(self: *mut Self) -> Option[int] {
        if self[0].n <= 0 { return None; }
        self[0].n = self[0].n - 1;
        return Some(self[0].n);
    }
}

fn main() {
    let cd = Countdown { n: 3 };
    for y in cd {
        print(y);
    }
    // => 2
    // => 1
    // => 0
}
```

Once `next` has returned `None`, the iterator is closed: calling `next`
again on that path is `E1401`. The protocol does not promise resurrection.

## What the model refuses

Four spellings are not in the language, each with its own diagnostic:

- `object Dog : Animal { .. }` is `E1512`: no inheritance. Hold the
  parent in a field and forward.
- `impl Drop for T { .. }` is `E1513`: no `Drop`. Release with `defer`
  and `.free()`.
- `trait T { x: int }` is `E1514`: a trait has no fields. Declare the
  field on a struct.
- `a + b` on a type with an `add` method is `E1515`: operators cannot be
  overloaded. Call `a.add(b)`.

## Worked example

The conformance program at `crates/yashy-driver/tests/programs/oop_conformance.ya`
exercises every rule in one transcript: an object with mixed visibility,
an inherent impl, a trait impl, a bound, an associated constructor, a
mutating receiver, a `defer` release, and an iterator consumed by `for`.
Its output is `2`, `show:2`, `2`, `3`.
