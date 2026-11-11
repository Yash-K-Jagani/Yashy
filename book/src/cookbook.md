# Cookbook

The programs people actually write, in the shape they actually write them.

## Read a file and count its words

`read_line()` answers `Option[string]`: `None` at the end of the input, and
`Some("")` for an empty line. A loop ends on `None`.

```yashy
fn present(Option[string] line) -> bool {
    return switch line {
        case Some(_) { true }
        case None { false }
    };
}

fn text_of(Option[string] line) -> string {
    return switch line {
        case Some(value) { value }
        case None { "" }
    };
}

fn words(string line) -> int {
    let count = 0;
    let parts = line.split(" ");
    for word in parts {
        if !word.is_empty() {
            count = count + 1;
        }
    }
    parts.free();
    return count;
}

fn main() {
    let line = read_line();
    while present(line) {
        print(words(text_of(line)));
        line = read_line();
    }
}
```

## Fail early with `?`

`?` propagates: it returns the failure to the caller that can handle it, and
the compiler proves that the failure is handled.

```yashy
fn first_number(string text) -> Option[int] {
    return text.parse_i64();
}

fn double_first(string text) -> Option[int] {
    const value = first_number(text)?;
    return Some(value * 2);
}

fn main() {
    switch double_first("42") {
        case Some(value) { print(value); }
        case None { print("not a number"); }
    }
}
```

## Own memory explicitly

There is no garbage collector, so a buffer that owns memory is released by
hand — and the verifier will not let a program forget:

```yashy
fn main() {
    const buffer = alloc(16);
    defer { buffer.free(); }
    print("owned");
}
```

## Start a thread and wait for it

```yashy
fn worker(int count) {
    print(count);
}

fn main() {
    const handle = thread_spawn(worker, 1);
    thread_join(handle);
}
```

## Write a package

A package is a directory with a manifest, a source directory and a test
directory:

```text
my-tool/
  yashy.toml
  src/
    main.ya
  tests/
    smoke.ya
```

The manifest names the package and its dependencies:

```text
[package]
name = "my-tool"
version = "0.1.0"
license = "MIT"
```

`yashy build` in that directory builds it; `yashy test` runs the tests; and
`yashy audit` checks the package against the release policy before it is
published.

## See also

- [The first-CLI guide](language-guide/12-first-cli.md) — the whole path,
  from `yashy init` to a tested command.
- [Packages](../packages.md) — the manifest format and the registry.

## Exercise

Write a program that counts the words in a line, the way the first recipe
above does, and make it print the count of the line it reads.

The solution is `book/tests/exercise-words.ya`.
