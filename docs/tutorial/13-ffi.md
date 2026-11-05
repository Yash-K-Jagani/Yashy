# 13. The C boundary

## What you will learn

- How to declare a C function, and when the call must say `unsafe`.
- How `link_name` separates the name a program writes from the symbol the
  linker resolves.
- How a variadic C function is declared and called.
- What `repr(C)` promises, and how `sizeof`, `alignof` and `offsetof` ask for
  the layout that promise buys.

Yashy has no C preprocessor and no header parser. An `extern` block declares the
functions a program calls, with the C types the linker and the ABI use. The
declaration is a promise about a function the Yashy compiler did not see.

```yashy
extern "C" link "m" {
    unsafe fn c_sqrt(f64 x) -> f64 link_name "sqrt";
}

fn main() {
    print(c_sqrt(2.0) == 1.4142135623730951);
    print("");
    // => true
}
```

`c_sqrt` is the name the program calls; `link_name "sqrt"` is the symbol the
linker resolves, and `link "m"` names the C library that holds it. Without the
clause the declaration's own name is the symbol.

## When a call says `unsafe`

A call to an `extern` function is checked like any other call when every
argument is a value the language can copy: `c_sqrt(2.0)` passes one `f64` and
reads one `f64`, and the compiler can see the whole transaction. The call says
`unsafe` when the boundary stops being checkable:

- the declaration is variadic and the call passes arguments beyond the fixed
  ones, because C reads them by the format string rather than by a Yashy type;
- an argument is linear — a pointer, an array, a map, a channel — because C may
  keep it or free it, and Yashy cannot see which.

An `unsafe fn` in the declaration does not by itself force `unsafe` at the call
site; the call site says it where the risk is actually taken.

## Variadic declarations

A declaration may end in `...`. Only the last parameter may be the marker, and
only an `extern` declaration may be variadic. The fixed parameters are checked
as usual; the rest are passed with their explicit C width, because there is no
default promotion:

```yashy
extern "C" {
    unsafe fn printf(*const u8 fmt, ...) -> i32;
}

fn main() {
    unsafe printf("%d\n", 42 as i32);
    // => 42
}
```

`42` alone would be an `int`, whose width C does not know; `42 as i32` is the
`int` `printf` reads for `%d`. The declaration lowers to
`declare i32 @printf(i8*, ...)`, and the call spells the full function type.

## `repr(C)`

A Yashy struct has a layout the language is free to choose. A struct that a C
function will read or write must have the layout C chose, and `repr(C)` says so:

```yashy
repr(C) struct Point {
    tag: u8,
    x: i32,
    y: i32,
}

fn main() {
    print(sizeof(Point));
    print("");
    print(alignof(Point));
    print("");
    print(offsetof(Point, tag));
    print("");
    print(offsetof(Point, x));
    print("");
    print(offsetof(Point, y));
    print("");
    // => 12
    // => 4
    // => 0
    // => 4
    // => 8
}
```

`repr(C)` is a keyword form on a struct, an enum or an object, not an attribute
system: there is no `#[...]` syntax and no other `repr`. The clause is what
licenses the C spellings `u8`, `i32` and friends inside the item; outside it,
the language's own `int`, `float`, `bool` and `char` are the names a program
uses.

`sizeof`, `alignof` and `offsetof` are compile-time queries. They fold to `int`
literals in the typed tree, so no runtime call is emitted. The C half of the
differential test prints the same five numbers:

```c
#include <stddef.h>
#include <stdio.h>

struct point {
    unsigned char tag;
    int x;
    int y;
};

int main(void) {
    printf("%zu\n", sizeof(struct point));
    printf("%zu\n", _Alignof(struct point));
    printf("%zu\n", offsetof(struct point, tag));
    printf("%zu\n", offsetof(struct point, x));
    printf("%zu\n", offsetof(struct point, y));
    return 0;
}
```

A `repr(C)` enum must be fieldless: a payload would need a tag-plus-union
layout that this build does not define. `offsetof` needs a `repr(C)` struct;
asking a plain struct for a field offset is `E0108`.

## The rules, as diagnostics

| Code | The rule |
|---|---|
| `E0900` | Two declarations share a symbol but disagree about its signature, or a declaration collides with a Yashy function of the same name. |
| `E0901` | An `extern` declaration is malformed: an empty `link_name`, or a clause on something that is not a declaration. |
| `E0902` | `...` is not the last parameter, or appears on a Yashy `fn` or a function type. |
| `E0106` | `repr(C)` is on an item that cannot have one, or a `repr(C)` enum has a payload. |
| `E0107` | `sizeof` or `alignof` names a type the checker does not know. |
| `E0108` | `offsetof` names a struct that is not `repr(C)`, or a field it does not have. |

## Common mistakes

- **Forgetting the library.** `link "m"` is part of the declaration; without it
  the linker reports an undefined symbol at build time.
- **Passing an `int` to a variadic parameter.** `printf("%d\n", 42)` reports
  that the argument needs an explicit C width. Write `42 as i32`.
- **Calling a variadic function without `unsafe`.** The extra arguments are
  unchecked by construction, so the call site must say so.
- **Putting `repr(C)` on a payload enum.** The layout of a tagged union is not
  specified here; use a fieldless enum or a struct.
- **Asking a plain struct for an offset.** `offsetof` is only defined for
  `repr(C)`; add the clause or drop the query.

## How it works

The checker records each declaration's Yashy name, its linker symbol, and its
signature. A symbol claimed twice with different signatures is `E0900`; the
same symbol with the same signature is one C function described twice, and the
module emits one `declare`. Codegen writes the `declare` with the C types and,
for a variadic call, spells the whole function type at the call site. The
layout queries read the same `repr(C)` layout the emitter uses, so the numbers a
program prints and the offsets its IR writes cannot drift apart.

Continue to the [CLI reference](../cli.md), or go back to
[12-first-cli](12-first-cli.md) for the project-shaped path.
