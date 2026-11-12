# Getting started

Yashy is a small compiled language: static types, no garbage collector, no
`null`, and no exceptions. It compiles to LLVM IR, which `clang` turns into a
native executable.

## Install

The release ships an archive for each platform: a zip for Windows and for
macOS, a tarball for Linux. Unpack it and run the installer it contains, then
check that the toolchain is complete:

```text
$ yashy doctor
yashy 1.0.0
target: x86_64-unknown-linux-gnu
clang: /usr/bin/clang
  clang version 18.1.8
stdlib: /home/you/.local/share/yashy/stdlib/yashy
path: /home/you/.local/bin/yashy
```

`doctor` names what is missing and what to do about it. The macOS install is
described in [the macOS guide](../install/macos.md); the release process
itself is in [the release guide](../release.md).

## Your first program

```yashy
fn main() {
    print("Hello, Yashy!");
}
```

Save it as `main.ya` and run it:

```text
$ yashy run main.ya
Hello, Yashy!
```

## A program with a type

Every value has a type, and the type is written before the name:

```yashy
fn add(int a, int b) -> int {
    return a + b;
}

fn main() {
    print(add(2, 3));
}
```

The compiler checks it before it runs, and the verifier proves that every
value that owns memory is released. A program that does not compile does not
run; a program that would leak does not link.

## Where to go next

- [The language guide](language-guide/00-intro.md) starts with what Yashy
  is and why it is shaped this way.
- [The verification guide](verification-guide.md) explains what the verifier
  proves, and what it does not.
- [The cookbook](cookbook.md) has the programs people actually write.
- [The reference](reference.md) lists every function the standard library
  has, generated from the compiler's own tables.

## Exercise

Write a program that adds two numbers and prints the sum. The numbers come
from the command line, so the program has to read its arguments:

```text
$ yashy run main.ya 2 3
5
```

The solution is `book/tests/exercise-sum.ya`.
