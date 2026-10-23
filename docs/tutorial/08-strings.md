# 8. Strings

## What you will learn

- What a `string` stores, and why `len()` counts bytes rather than characters.
- Which operations exist: `+`, `==`, the string methods, and interpolation.
- Why a `string` can be copied while an array or a pointer cannot.
- Where the implementation still leaks or dangles a string reference.

A `string` is a counted sequence of bytes. `+` concatenates, `len()` counts
bytes, `==` compares contents, and `s.slice(start, end)` returns a substring.
Bounds are clamped, so a wrong index gives an empty string rather than a fault.

```yashy
fn main() {
    const greeting = "Hello, " + "Yashy!";
    print_i64(greeting.len());
    print_str("\n");
    // => 13

    print_str(greeting.slice(0, 5));
    print_str("\n");
    // => Hello

    print_str("${greeting} again\n");
    // => Hello, Yashy! again
}
```

`greeting.len()` is 13 because `"Hello, "` is seven bytes and `"Yashy!"` is
six. `greeting.slice(0, 5)` takes the half-open range `[0, 5)`, which is the
five bytes of `Hello`. `"${greeting} again"` lowers to
`greeting + " again"`; interpolation is a spelling, not a separate runtime
path.

## Bytes, not characters

A string is UTF-8 bytes with a length, not an array of characters. The length
of `"héllo"` is 6, because `é` is two bytes. `s.byte_at(i)` returns the byte at
`i` as an `i64` in `0..255`, or `0` when `i` is negative or past the end.
`s.slice` also works in bytes, so `s.slice(0, 3)` is `"hé"` while
`s.slice(0, 2)` stops in the middle of `é` and produces invalid UTF-8. That is
not an error, and the compiler cannot see it: both arguments are ordinary
`i64`s.

`slice` never faults. A negative start becomes `0`, an end past the length
becomes the length, and an inverted range such as `s.slice(2, 1)` is empty.
Out-of-range `byte_at` returns `0` for the same reason: a bad index has an
answer, so there is no panic path. `get` is the checked alternative: it returns
`Option[i64]`, with `None` outside `0..len`.

## Strings are copied, not moved

Most owning values in Yashy are linear: a pointer or an array has exactly one
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

The buffer is immutable: `+` allocates a new string and `slice` copies the
bytes it keeps. `==` compares contents, so two separately built `"abc"` values
are equal. Ordering is not provided: `<` on strings is a type error.

## The operations

| Form | Result | Notes |
|---|---|---|
| `s.len()` | `i64` | Number of bytes, not characters. |
| `s.is_empty()` | `bool` | `len() == 0`. |
| `s.byte_at(i)` | `i64` | Byte value, or `0` when out of range. |
| `s.get(i)` | `Option[i64]` | Byte value, or `None` when out of range. |
| `s.slice(start, end)` | `string` | New string over `[start, end)`, clamped. |
| `s.contains(needle)` | `bool` | Whether `needle` occurs at least once. |
| `s.starts_with(prefix)` | `bool` | Byte-prefix test. |
| `s.ends_with(suffix)` | `bool` | Byte-suffix test. |
| `s.index_of(needle)` | `Option[i64]` | First byte index, or `None`. |
| `s.trim()` | `string` | Removes ASCII whitespace at both ends. |
| `s.trim_start()` / `s.trim_end()` | `string` | Removes it at one end. |
| `s.to_upper()` / `s.to_lower()` | `string` | ASCII-only case mapping. |
| `s.replace(old, new)` | `string` | Replaces every non-overlapping occurrence. |
| `s.repeat(n)` | `string` | `n` copies; `n <= 0` is empty. |
| `s.split(separator)` | `[]string` | Fresh array of fresh strings. |
| `s.lines()` | `[]string` | Splits on `\n`, strips one `\r`, drops the final empty piece. |
| `s.parse_i64()` / `s.parse_f64()` | `i64` / `f64` | `0` when the text is not a number. |
| `a + b` | `string` | New string; both operands are `string`. |
| `a == b` | `bool` | Compares bytes, not identity. |
| `"${expr}"` | `string` | `expr` may be `string`, `i64`, `f64`, or `bool`. |
| `str_from_i64(n)` and friends | `string` | The conversions interpolation uses. |

Every transform returns a fresh owned string. `split` and `lines` return a
fresh array whose `string` elements are owned by that array, so `parts.free()`
releases both the backing block and every piece.

## Methods

The string is the receiver, so the subject comes first and the method name is
the operation:

```yashy
fn show(o: Option[i64]) -> i64 {
    return switch o {
        case Some(v) { v }
        case None { 0 - 1 }
    };
}

fn main() {
    var s = "  Yashy,yashy  ";

    print_str(s.trim());
    print_str("\n");
    // => Yashy,yashy

    print_i64(s.len());
    print_str("\n");
    // => 15

    if s.is_empty() {
        print_str("empty\n");
    } else {
        print_str("not empty\n");
    }
    // => not empty

    print_i64(s.byte_at(2));
    print_str("\n");
    // => 89

    print_i64(show(s.get(2)));
    print_str("\n");
    // => 89

    print_i64(show(s.get(99)));
    print_str("\n");
    // => -1

    print_str(s.slice(2, 7));
    print_str("\n");
    // => Yashy

    if s.contains("yashy") {
        print_str("found yashy\n");
    }
    // => found yashy

    if s.starts_with("  Ya") {
        print_str("starts with spaces\n");
    }
    // => starts with spaces

    if s.ends_with("  ") {
        print_str("ends with spaces\n");
    }
    // => ends with spaces

    print_i64(show(s.index_of("yashy")));
    print_str("\n");
    // => 8

    var upper = s.to_upper();
    print_str(upper.trim());
    print_str("\n");
    // => YASHY,YASHY

    var replaced = s.replace("yashy", "seal");
    print_str(replaced.trim());
    print_str("\n");
    // => Yashy,seal

    var dash = "-";
    print_str(dash.repeat(3));
    print_str("\n");
    // => ---

    var csv = "a,b,c";
    var parts = csv.split(",");
    print_i64(parts.len());
    print_str("\n");
    // => 3
    for part in parts {
        print_str(part);
        print_str("\n");
        // => a
        // => b
        // => c
    }
    parts.free();

    var text = "one\r\ntwo\n";
    var lines = text.lines();
    print_i64(lines.len());
    print_str("\n");
    // => 2
    for line in lines {
        print_str(line);
        print_str("\n");
        // => one
        // => two
    }
    lines.free();
}
```

`trim` removes space, tab, newline, carriage return, form feed, and vertical
tab. Case mapping is ASCII-only: `to_upper` changes `a`–`z` and leaves every
other byte alone, so non-ASCII text passes through unchanged. A split or a line
boundary is one byte. An empty separator makes `split` return the whole string
as one piece; `lines` strips one `\r` before each `\n` and does not add an empty
final piece after a trailing newline.

Method calls do not chain yet. `s.trim().len()` is not a three-part method
call; bind the intermediate result first:

```text
var trimmed = s.trim();
print_i64(trimmed.len());
```

A literal may contain `\n`, `\r`, `\t`, `\0`, `\\`, `\"`, `\'`, and `\$`.
The set is closed: a backslash followed by anything else is a diagnostic
rather than a guess. `\$` exists so a literal `$` is expressible, because
`${` begins an interpolation. A `//` inside a literal is text, not a comment.

## Debug formatting

`${value}` shows a value the way a reader wants to read it. `${value:?}` shows
the *shape* of the value instead: a struct's name and fields, the variant an
enum holds, the elements of a tuple or an array, and a string in quotes with
its control characters escaped.

```yashy
struct Point {
    x: i64,
    y: i64,
}

fn main() {
    const point = Point { x: 3, y: 4 };
    print_str("${point:?}\n");
    // => Point { x: 3, y: 4 }

    print_str("${[1, 2, 3]:?}\n");
    // => [1, 2, 3]

    print_str("${"a\nb":?}\n");
    // => "a\nb"
}
```

The string case is the point of the last line. `"a\nb"` printed plainly is two
lines, so a report that shows a value cannot say which bytes were in it. The
debug form is quoted and escaped, so the newline is visible as `\n` and the
text stays on one line.

The debug form exists for the values that have a shape to show. A number, a
`bool` and a `string` have one form each, and a struct, an enum, a tuple and
an array have theirs. A pointer has none -- printing its address would succeed
and mean nothing -- so `"${p:?}"` on a `*mut i64` is a diagnostic:

```text
error: cannot format a `*mut i64` with `:?`: only numbers, `bool`, `string`,
structs, enums, tuples and arrays have a debug form
```

## Common mistakes

- **Indexing a string.** `s[0]` reports ``error: cannot index `string` ``.
  Use `s.byte_at(0)` for a byte, or `s.slice(0, 1)` for a one-byte string.
- **Adding a number to a string.** `"n = " + 1` reports
  ``error: cannot apply `+` to `string` and `i64` ``. Write
  `"n = ${n}"`, which inserts the converted value.
- **Using an escape Yashy does not have.** `"\q"` reports
  ``error: invalid escape sequence in string literal: unknown escape `\q` ``.
- **Ordering strings.** `a < b` reports
  ``error: cannot apply `<` to `string` and `string` ``. Compare with `==`,
  or compare `a.len()` and `b.len()` when length is the question.
- **Assuming `len()` counts characters.** The length of `"héllo"` is `6`, and
  slicing at a byte offset can split a character. No diagnostic is produced;
  the program simply sees bytes.
- **Chaining methods.** `s.trim().len()` is not supported yet. Bind
  `var trimmed = s.trim();` and then call `trimmed.len()`.
- **Calling a method on a literal.** `"x".len()` is parsed as a zero-argument
  call to `len`, not as a method on the literal. Bind the literal first:
  `var s = "x"; print_i64(s.len());`.

## Try it

1. Build a string from a first name, a space, and a last name, then print its
   length, its first byte with `byte_at`, and the first three bytes with
   `slice`. Hint: count bytes, and expect non-ASCII input to be longer than it
   looks.

2. Write `fn shout(s: string)` that prints the string followed by `"!"`, then
   call it twice with the same binding, once with `move` and once without.
   Hint: both calls compile and both see the text, because strings are
   copyable.

## How it works

A `string` value is an `i8*` pointing at `%yashy_str = { i64, i64, i64, i8* }`:
refcount, byte length, capacity, and data. The runtime creates owned strings
with refcount 1; a literal is baked in with refcount `-1`, which the release
path ignores, so literals are never freed. Codegen keeps a per-function set of
owned SSA names and emits `yashy_str_release` when a scope or a consumer is
done with one. If the program contains a `thread_spawn`, generated code calls
the atomic retain/release pair instead, because two threads may drop the last
reference at once.

One hole remains. A struct field holding a `string` is not itself retained or
released. `yashy build --emit-ir` on a function that returns a struct built
from a locally concatenated string shows `yashy_str_release` before the `ret`,
and the field then points at freed bytes. Until that is fixed, put literals in
struct fields or keep the owning string alive in the caller's scope; do not
return ownership of a string inside a struct and expect the struct to keep it.

Continue to [09-arrays](09-arrays.md).
