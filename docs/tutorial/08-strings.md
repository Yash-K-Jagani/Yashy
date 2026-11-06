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
    print(greeting.len());
    // => 13

    print(greeting.slice(0, 5));
    // => Hello

    print("${greeting} again");
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
`i` as a `char` holding that byte value, or `'\0'` when `i` is negative or past
the end. Printing it shows the character, so `'A'` prints `A`; use
`.to_int()` when the number `65` is what you want.
`s.slice` also works in bytes, so `s.slice(0, 3)` is `"hé"` while
`s.slice(0, 2)` stops in the middle of `é` and produces invalid UTF-8. That is
not an error, and the compiler cannot see it: both arguments are ordinary
`int`s.

`slice` never faults. A negative start becomes `0`, an end past the length
becomes the length, and an inverted range such as `s.slice(2, 1)` is empty.
Out-of-range `byte_at` returns `'\0'` for the same reason: a bad index has an
answer, so there is no panic path. `get` is the checked alternative: it returns
`Option[int]`, with `None` outside `0..len`.

## Byte buffers

A string is immutable, so a program that builds bytes one at a time uses a
buffer: `alloc_bytes(n)` is `n` bytes, and indexing the returned `*mut char`
reads or writes one `char` (one byte) at a time. The pointer is linear: exactly
one owner, and `.free()` returns it.

```yashy
fn main() {
    let buf = alloc_bytes(4);
    buf[0] = 'a';
    buf[1] = 'b';
    buf[2] = 'c';
    buf[3] = 'd';
    print("${buf[0]}${buf[1]}${buf[2]}${buf[3]}");
    buf.free();
    // => abcd
}
```

`read_bytes(path)` answers `Result[[]char, io.Error]` and `write_bytes(path,
body)` answers `Result[void, io.Error]`. A file that is missing is `Err`, not an
empty slice, so the two cases stay different. The slice is a real `[]char`: its
`len()` is the byte count, and `free()` releases the backing array.

```yashy
import io;

fn verify([]char bytes) {
    print(bytes.len());
    switch bytes[0] {
        case Some(first) { print(first) }
        case None { print('?') }
    }
    bytes.free();
}

fn load(string path) -> Result[[]char, io.Error] {
    const data = read_bytes(path)?;
    return Ok(data);
}

fn main() {
    let data = []char {};
    data.push('Y');
    data.push('a');
    data.push('s');
    data.push('h');
    data.push('y');
    switch write_bytes("bytes.bin", data) {
        case Ok(_) { print("saved") }
        case Err(_) { print("failed") }
    }
    data.free();
    switch load("bytes.bin") {
        case Ok(bytes) { verify(bytes) }
        case Err(_) { print("failed") }
    }
    // => saved
    // => 5
    // => Y
}
```

`write_bytes` borrows its slice, so `data.free()` after the call is the release,
not a use after a move. `read_bytes` returns a fresh slice, and the `?` in
`load` passes an `io.Error` through unchanged.

## Strings are copied, not moved

Most owning values in Yashy are linear: a pointer or an array has exactly one
owner, and passing it to a function transfers it. A `string` is the exception.
It is a small handle to a reference-counted buffer, and the language declares
it *copyable*:

```text
fn take(string s) { print(s); }

fn main() {
    const s = "shared";
    let t = s;
    take(move s);      // s is still usable: strings are copyable
    print(s);
    print(t);      // prints "sharedshared"
}
```

`let t = s;` gives `t` its own reference and bumps the count. `take(move s)`
is accepted, but `move` changes nothing for a copyable type: `s` is still
usable afterwards, which is why the final two prints both work. Use plain
`take(s)`. `move` is for pointers, where it prevents a second owner.

The buffer is immutable: `+` allocates a new string and `slice` copies the
bytes it keeps. `==` compares contents, so two separately built `"abc"` values
are equal. Ordering is not provided: `<` on strings is a type error.

## The operations

| Form | Result | Notes |
|---|---|---|
| `s.len()` | `int` | Number of bytes, not characters. |
| `s.is_empty()` | `bool` | `len() == 0`. |
| `s.byte_at(i)` | `char` | Byte as a character, or `'\0'` when out of range. |
| `s.get(i)` | `Option[int]` | Byte value, or `None` when out of range. |
| `s.slice(start, end)` | `string` | New string over `[start, end)`, clamped. |
| `s.contains(needle)` | `bool` | Whether `needle` occurs at least once. |
| `s.starts_with(prefix)` | `bool` | Byte-prefix test. |
| `s.ends_with(suffix)` | `bool` | Byte-suffix test. |
| `s.index_of(needle)` | `Option[int]` | First byte index, or `None`. |
| `s.trim()` | `string` | Removes ASCII whitespace at both ends. |
| `s.trim_start()` / `s.trim_end()` | `string` | Removes it at one end. |
| `s.to_upper()` / `s.to_lower()` | `string` | ASCII-only case mapping. |
| `s.replace(old, new)` | `string` | Replaces every non-overlapping occurrence. |
| `s.repeat(n)` | `string` | `n` copies; `n <= 0` is empty. |
| `s.split(separator)` | `[]string` | Fresh array of fresh strings. |
| `s.lines()` | `[]string` | Splits on `\n`, strips one `\r`, drops the final empty piece. |
| `s.parse_i64()` / `s.parse_f64()` | `Option[int]` / `Option[float]` | `Some` for a number, `None` for anything else. |
| `a + b` | `string` | New string; both operands are `string`. |
| `a == b` | `bool` | Compares bytes, not identity. |
| `"${expr}"` | `string` | `expr` may be `string`, `int`, `float`, or `bool`. |
| `str_from_i64(n)` and friends | `string` | The conversions interpolation uses. |

Every transform returns a fresh owned string. `split` and `lines` return a
fresh array whose `string` elements are owned by that array, so `parts.free()`
releases both the backing block and every piece.

## Methods

The string is the receiver, so the subject comes first and the method name is
the operation:

```yashy
fn show(Option[int] o) -> int {
    return switch o {
        case Some(v) { v }
        case None { 0 - 1 }
    };
}

fn main() {
    let s = "  Yashy,yashy  ";

    print(s.trim());
    // => Yashy,yashy

    print(s.len());
    // => 15

    if s.is_empty() {
        print("empty");
    } else {
        print("not empty");
    }
    // => not empty

    print(s.byte_at(2));
    // => Y

    print(show(s.get(2)));
    // => 89

    print(show(s.get(99)));
    // => -1

    print(s.slice(2, 7));
    // => Yashy

    if s.contains("yashy") {
        print("found yashy");
    }
    // => found yashy

    if s.starts_with("  Ya") {
        print("starts with spaces");
    }
    // => starts with spaces

    if s.ends_with("  ") {
        print("ends with spaces");
    }
    // => ends with spaces

    print(show(s.index_of("yashy")));
    // => 8

    let upper = s.to_upper();
    print(upper.trim());
    // => YASHY,YASHY

    let replaced = s.replace("yashy", "seal");
    print(replaced.trim());
    // => Yashy,seal

    let dash = "-";
    print(dash.repeat(3));
    // => ---

    let csv = "a,b,c";
    let parts = csv.split(",");
    print(parts.len());
    // => 3
    for part in parts {
        print(part);
        // => a
        // => b
        // => c
    }
    parts.free();

    let text = "one\r\ntwo\n";
    let lines = text.lines();
    print(lines.len());
    // => 2
    for line in lines {
        print(line);
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
let trimmed = s.trim();
print(trimmed.len());
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
    x: int,
    y: int,
}

fn main() {
    const point = Point { x: 3, y: 4 };
    print("${point:?}");
    // => Point { x: 3, y: 4 }

    print("${[1, 2, 3]:?}");
    // => [1, 2, 3]

    print("${"a\nb":?}");
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
and mean nothing -- so `"${p:?}"` on a `*mut int` is a diagnostic:

```text
error: cannot format a `*mut int` with `:?`: only numbers, `bool`, `string`,
structs, enums, tuples and arrays have a debug form
```

## Common mistakes

- **Indexing a string.** `s[0]` reports ``error: cannot index `string` ``.
  Use `s.byte_at(0)` for a byte, or `s.slice(0, 1)` for a one-byte string.
- **Adding a number to a string.** `"n = " + 1` reports
  ``error: cannot apply `+` to `string` and `int` ``. Write
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
  `let trimmed = s.trim();` and then call `trimmed.len()`.
- **Calling a method on a literal.** `"x".len()` is parsed as a zero-argument
  call to `len`, not as a method on the literal. Bind the literal first:
  `let s = "x"; print(s.len());`.

## Try it

1. Build a string from a first name, a space, and a last name, then print its
   length, its first byte with `byte_at`, and the first three bytes with
   `slice`. Hint: count bytes, and expect non-ASCII input to be longer than it
   looks.

2. Write `fn shout(string s)` that prints the string followed by `"!"`, then
   call it twice with the same binding, once with `move` and once without.
   Hint: both calls compile and both see the text, because strings are
   copyable.

## How it works

A `string` value is an `i8*` pointing at `%yashy_str = { int, int, int, i8* }`:
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
