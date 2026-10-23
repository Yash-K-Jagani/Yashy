# 0. What Yashy is

## What you will learn

- What Yashy is, and the three decisions that shape the rest of the language.
- How to build the compiler and compile a program to a native executable.
- How to read a compiler diagnostic that points at your source.
- How this tutorial is checked, so you know which claims are tested.

Yashy is a small compiled language: static types, no garbage collector, no
`null`, and no exceptions. It compiles to LLVM IR, which `clang` turns into a
native executable.

Three decisions shape everything else, so they are worth stating before the
syntax:

- **There is no `null`.** A value that might be missing is an `Option[T]`, and
  the only way to read it is to match it. There is no implicit "empty" pointer
  to forget to check. The cost is that the check is visible in the source; the
  benefit is that "I forgot" is a type error instead of a crash.
- **There is no exception unwinding.** A fallible function returns
  `Result[T, E]`, and `?` propagates the error to the caller that can handle
  it. What a function can fail with is in its signature, so a caller can see
  the failure without reading the body.
- **Memory is owned.** A value has one owner; `move` transfers it; `defer`
  releases it when the scope exits. The type checker refuses a use after a
  move, and the runtime holds no hidden bookkeeping to save a program that
  ignores one. This is how Yashy avoids a tracing collector without making
  every allocation a stack value.

## Build and run

Build the compiler from the repository root:

```text
cargo build --release
```

Then compile a program and run it. The compiler lives at
`target/release/yashy.exe` on Windows and `target/release/yashy` elsewhere; the
output is `hello.exe` on Windows and `hello` on Linux and macOS. A debug build
works the same way and compiles faster: `cargo build`, then `target/debug/yashy`.

```text
target/release/yashy build hello.ya -o hello
./hello
```

`-o` is worth using from the first program. Without it, the executable is
written next to the source file, with the source file's stem as its name. That
is convenient for a single file and confusing once you have several files with
the same stem in different directories.

The first program is the smallest complete one:

```yashy
fn main() {
    print_str("Yashy speaks.\n");
    // => Yashy speaks.
}
```

`main` takes no arguments and returns nothing, or returns an `int` that becomes
the process exit status. `print_str` is a builtin: it is in scope in every
file, imported or not, so the first example needs no `import`.

## The commands

| Command | What it does | When to use it |
|---|---|---|
| `yashy build file.ya -o name` | Compiles to a native executable. | Running the program. |
| `yashy check file.ya` | Lexes, parses and type-checks; writes no executable. | Fast feedback while editing. |
| `yashy build file.ya --emit-ir` | Prints the generated LLVM IR and stops. | Seeing what the compiler produced. |
| `yashy fmt [-w] file.ya` | Rewrites a file in the canonical style; `-w` writes it back. | Keeping a file consistent. |
| `yashy version` | Prints the compiler version. | Reporting a problem. |

`build` accepts `-O0` through `-O3` (the default is `-O2`), `--release` (which
drops `debug_assert` calls), `--fast-math` (which allows fused multiply-add and
can change float results), and `--cpu` (the default, `portable`, is the one to
use unless you know the machine that will run the binary).

## How the tutorial is checked

The tutorial is executable. Every fenced block tagged `yashy` is a whole
program with a `fn main`, and the lines beginning `// =>` inside that block are
the exact stdout the program must print, in order, one line each. The test
extracts each block, writes it to a temporary file, runs `yashy build`, runs
the result, and compares the output.

```text
cargo test -p yashy-driver --test tutorial --target-dir D:\Yashy\target\probe1
```

On another platform, pass any writable target directory. A failure names the
chapter and the line the block starts on, then shows what was printed and what
the block claimed. That is why the examples avoid output that depends on the C
library's float formatting, and why a block must be a complete program: a
fragment that cannot be compiled is a fragment that cannot be checked.

The prose around the blocks is not executed. Only the `// =>` lines are
promises; the explanations are there to make the promises make sense.

## Common mistakes

- **Running the compiler with no command.** It prints `error: no command given`
  followed by the usage text.
- **Forgetting the source file.** `yashy build` prints
  ``error: `build` needs a source file``.
- **Naming a file that is not there.** The error begins
  `error: cannot read <path>:` and includes the operating system's reason.
- **Expecting the executable in the current directory after `build` without
  `-o`.** It is written next to the source. Use `-o hello` when the location
  matters.
- **Editing an `yashy` block without updating its `// =>` lines.** This is the
  one mistake the test is designed to catch: it compiles and runs the block you
  wrote, then reports the first line where stdout and the claims disagree.

## Try it

1. Put the first program in a file named `first.ya`, build it with
   `yashy build first.ya -o first`, and run it. Change the text, rebuild, and
   confirm the change. Then delete `-o` and find where the executable landed.
   Hint: the source file's directory, not the shell's directory, is the one
   that matters.

2. Break the program on purpose: declare `const n: int = "three";` and run
   `yashy check first.ya`. Read the four lines of the diagnostic, from the
   `path:line:column` header to the caret under the expression. Hint: the
   caret points at the value that does not fit the declared type, not at the
   declaration.

## How it works

`yashy build` runs a pipeline: lexer, parser, type checker, code generator, then
the system's `clang`. The compiler writes LLVM IR as text rather than
building it through a library, so `--emit-ir` is a real view of the last stage
and a code-generation bug can be read as a diff. The type checker runs before
any IR exists, which is why most mistakes in this tutorial are reported before
a binary is written.

Continue to [01-hello](01-hello.md).
