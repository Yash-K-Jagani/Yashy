# 8. Strings

## What you will learn

- What a `string` stores, and why `len` counts bytes rather than characters.
- Which operations exist: `+`, `==`, `str_slice`, `byte_at`, and interpolation.
- Why a `string` can be copied while a slice or a pointer cannot.
- Where the implementation still leaks or dangles a string reference.

A `string` is a counted sequence of bytes. `+` concatenates, `len` counts
bytes, `==` compares contents, and `str_slice(text, start, end)` returns a
substring. Bounds are clamped, so a wrong index gives an empty string rather
than a fault.

```otter
fn main() {
    const greeting = "Hello, " + "Otter!";
    print_i64(len(greeting));
    print_str("\n");
    // => 13

    print_str(str_slice(greeting, 0, 5));
    print_str("\n");
    // => Hello

    print_str("${greeting} again\n");
    // => Hello, Otter! again
}
```

`len(greeting)` is 13 because `"Hello, "` is seven bytes and `"Otter!"` is
six. `str_slice(greeting, 0, 5)` takes the half-open range `[0, 5)`, which is
the five bytes of `Hello`. `"${greeting} again"` lowers to
`greeting + " again"`; interpolation is a spelling, not a separate runtime
path.

## Bytes, not characters

A string is UTF-8 bytes with a length, not an array of characters. `len("héllo")`
is 6, because `é` is two bytes. `byte_at(s, i)` returns the byte at `i` as an
`i64` in `0..255`, or `0` when `i` is negative or past the end. `str_slice`
also works in bytes, so `str_slice("héllo", 0, 3)` is `"hé"` while
`str_slice("héllo", 0, 2)` stops in the middle of `é` and produces invalid
UTF-8. That is not an error, and the compiler cannot see it: both arguments are
ordinary `i64`s.

`str_slice` never faults. A negative start becomes `0`, an end past the length
becomes the length, and an inverted range such as `str_slice(s, 2, 1)` is
empty. Out-of-range `byte_at` returns `0` for the same reason: a bad index has
an answer, so there is no panic path and no `Option` to match.

## Strings are copied, not moved

Most owning values in Otter are linear: a pointer or a slice has exactly one
owner, and passing it to a function transfers it. A `string` is the exception.
It is a small handle to a reference-counted buffer, and the language declares
it *copyable*:

```text
fn take(s: string) { print_str(s); }

fn main() {
    const s = "shared";
    var t = s;
    take(move s);      // s is still usable: strings are copyable
    print_str(s);
    print_str(t);      // prints "sharedshared"
}
```

`var t = s;` gives `t` its own reference and bumps the count. `take(move s)`
is accepted, but `move` changes nothing for a copyable type: `s` is still
usable afterwards, which is why the final two prints both work. Use plain
`take(s)`. `move` is for pointers, where it prevents a second owner.

The buffer is immutable: `+` allocates a new string and `str_slice` copies the
bytes it keeps. `==` compares contents, so two separately built `"abc"` values
are equal. Ordering is not provided: `<` on strings is a type error.

## The operations

| Form | Result | Notes |
|---|---|---|
| `len(s)` | `i64` | Number of bytes, not characters. |
| `byte_at(s, i)` | `i64` | Byte value, or `0` when out of range. |
| `str_slice(s, start, end)` | `string` | New string over `[start, end)`, clamped. |
| `a + b` | `string` | New string; both operands are `string`. |
| `a == b` | `bool` | Compares bytes, not identity. |
| `"${expr}"` | `string` | `expr` may be `string`, `i64`, `f64`, or `bool`. |
| `parse_i64(s)` / `parse_f64(s)` | `i64` / `f64` | `0` when the text is not a number. |
| `str_from_i64(n)` and friends | `string` | The conversions interpolation uses. |

A literal may contain `\n`, `\r`, `\t`, `\0`, `\\`, `\"`, `\'`, and `\$`.
The set is closed: a backslash followed by anything else is a diagnostic
rather than a guess. `\$` exists so a literal `$` is expressible, because
`${` begins an interpolation. A `//` inside a literal is text, not a comment.

## Common mistakes

- **Indexing a string.** `s[0]` reports ``error: cannot index `string` ``.
  Use `byte_at(s, 0)` for a byte, or `str_slice(s, 0, 1)` for a one-byte
  string.
- **Adding a number to a string.** `"n = " + 1` reports
  ``error: cannot apply `+` to `string` and `i64` ``. Write
  `"n = ${n}"`, which inserts the converted value.
- **Using an escape Otter does not have.** `"\q"` reports
  ``error: invalid escape sequence in string literal: unknown escape `\q` ``.
- **Ordering strings.** `a < b` reports
  ``error: cannot apply `<` to `string` and `string` ``. Compare with `==`,
  or compare `len(a)` and `len(b)` when length is the question.
- **Assuming `len` counts characters.** `len("héllo")` is `6`, and slicing
  at a byte offset can split a character. No diagnostic is produced; the
  program simply sees bytes.

## Try it

1. Build a string from a first name, a space, and a last name, then print its
   length, its first byte with `byte_at`, and the first three bytes with
   `str_slice`. Hint: count bytes, and expect non-ASCII input to be longer
   than it looks.

2. Write `fn shout(s: string)` that prints the string followed by `"!"`, then
   call it twice with the same binding, once with `move` and once without.
   Hint: both calls compile and both see the text, because strings are
   copyable.

## How it works

A `string` value is an `i8*` pointing at `%otter_str = { i64, i64, i64, i8* }`:
refcount, byte length, capacity, and data. The runtime creates owned strings
with refcount 1; a literal is baked in with refcount `-1`, which the release
path ignores, so literals are never freed. Codegen keeps a per-function set of
owned SSA names and emits `otter_str_release` when a scope or a consumer is
done with one. If the program contains a `thread_spawn`, generated code calls
the atomic retain/release pair instead, because two threads may drop the last
reference at once.

One hole remains. A struct field holding a `string` is not itself retained or
released. `otter build --emit-ir` on a function that returns a struct built
from a locally concatenated string shows `otter_str_release` before the `ret`,
and the field then points at freed bytes. Until that is fixed, put literals in
struct fields or keep the owning string alive in the caller's scope; do not
return ownership of a string inside a struct and expect the struct to keep it.

Continue to [09-slices](09-slices.md).
