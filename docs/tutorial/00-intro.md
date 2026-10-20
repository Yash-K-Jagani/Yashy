# 0. What Otter is

Otter is a small compiled language: static types, no garbage collector, no
`null`, and no exceptions. It compiles to LLVM IR, which `clang` turns into a
native executable.

Three decisions shape everything else, so they are worth stating before the
syntax:

- **There is no `null`.** A value that might be missing is an `Option[T]`, and
  the only way to read it is to match it. There is no implicit "empty" pointer
  to forget to check.
- **There is no exception unwinding.** A fallible function returns
  `Result[T, E]`, and `?` propagates the error to the caller that can handle
  it. What a function can fail with is in its signature.
- **Memory is owned.** A value has one owner; `move` transfers it; `defer`
  releases it when the scope exits. The type checker refuses a use after a
  move, and the runtime holds no hidden bookkeeping to save a program that
  ignores one.

Build the compiler from the repository root:

```text
cargo build --release
```

Then compile a program and run it (`target/release/otter.exe` and `hello.exe` on
Windows; `./hello` on Linux and macOS):

```text
target/release/otter build hello.ot
./hello
```

The first program is the smallest complete one:

```otter
fn main() {
    print_str("Otter speaks.\n");
    // => Otter speaks.
}
```

`main` takes no arguments and returns nothing. `print_str` is a builtin: it is
in scope in every file, imported or not, so the first example needs no
`import`.

Continue to [01-hello](01-hello.md).
