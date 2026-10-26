# 12. Your first CLI

## What you will learn

- How to install Yashy and confirm the install with `yashy doctor`.
- How to create a project with `yashy init`.
- How to read a command-line argument, split the work into a module, and print a
  result.
- How to run the project's tests with `yashy test`.
- How to build an optimized binary with `yashy build --release`.
- What to do when `clang`, the standard library, or `PATH` is missing.

This chapter is the path from an empty machine to a working command-line
program. The program is a small `wc`: it reads the file named on the command
line and prints the number of lines, words, and bytes. It is deliberately
ordinary -- the kind of program you would write on your first afternoon -- so
the parts you have already learned stay in the background.

The five-minute path is: install, `yashy doctor`, `yashy init`, `yashy run`.
The rest of the chapter explains the program and the tools around it.

## Install

Yashy needs `clang` and LLVM. The compiler writes LLVM IR and asks `clang` to
turn it into a native executable, so a machine without a C toolchain can run
the compiler but cannot build a program.

### Linux

From a release archive, unpack it and run the installer:

```text
tar xzf yashy-<version>-<target>.tar.gz
cd yashy-<version>-<target>
./install.sh
```

The installer copies the compiler and its standard library into a prefix
(`~/.local` by default), verifies the archive against `SHA256SUMS`, adds the
prefix to `PATH`, and runs `yashy doctor`. Use `./install.sh --prefix
/usr/local` for a system-wide install, `--no-path` if you manage `PATH`
yourself, and `--no-doctor` if you only want the files copied.

If `clang` is missing, the installer still installs Yashy and prints the
command for your distribution, for example:

```text
Next: sudo apt install clang llvm
Then run: yashy doctor
```

macOS is a 1.0 target, not an R1 target: this release does not ship a macOS
archive. Until a signed and tested archive exists, build from source on macOS
and install LLVM with `brew install llvm`.

### Windows

Unpack the release zip and run the installer from PowerShell:

```powershell
Expand-Archive yashy-<version>-<target>.zip -DestinationPath .
cd yashy-<version>-<target>
.\install.ps1
```

If PowerShell refuses to run a local script, use
`powershell -ExecutionPolicy Bypass -File .\install.ps1` for this one run.

The installer verifies the archive, copies Yashy into
`%LOCALAPPDATA%\Yashy`, adds that directory to your user `PATH`, and runs
`yashy doctor`. If LLVM is missing, install it with:

```powershell
winget install LLVM.LLVM
```

Open a new terminal after the install so the updated `PATH` is visible.

### From the repository

For a development checkout, build the compiler and point `YASHY_STDLIB` at the
standard library:

```text
cargo build --release
export YASHY_STDLIB="$PWD/crates/yashy-stdlib/yashy"
```

On Windows, use `target\release\yashy.exe` and set `YASHY_STDLIB` to the
absolute path of `crates\yashy-stdlib\yashy`.

## Check the install

```text
yashy doctor
```

A healthy install prints the version, the target, the `clang` it will use, the
standard library it found, and where the executable lives:

```text
yashy 0.1.0
target: x86_64-unknown-linux-gnu
clang: /usr/bin/clang
  clang version 18.1.8
stdlib: /home/you/.local/share/yashy/stdlib/yashy
path: /home/you/.local/bin/yashy
```

`doctor` exits non-zero if `clang` cannot be run or the standard library is
missing, and its last line tells you what to do next. The `path` line is
informational: a compiler invoked by its full path still works.

For scripts and agents, ask for the stable JSON form:

```text
yashy doctor --json
```

The top-level fields are `schema_version`, `tool`, `ok`, `yashy_version`,
`target`, `clang`, `stdlib`, `path`, and `next`. `schema_version` is `1`; a
change to the shape is a schema change, not a cosmetic one. The `next` field is
`null` when the install is healthy.

## Create the project

```text
yashy init wordcount
cd wordcount
```

`init` refuses a directory that already contains anything, because it creates a
project rather than editing one. It writes `yashy.toml`, `main.ya`,
`tests/smoke.ya`, a README, `AGENTS.md`, and a `.gitignore`. The first program
is already runnable:

```text
yashy run main.ya
```

```text
Hello, Yashy!
```

`run` compiles to a temporary executable and runs it. It is the fast loop while
you are editing. `build` writes an executable you can keep.

## The program

The finished project has two source files. Put this in `main.ya`:

```yashy
// file: main.ya
//
// Lines, words, and bytes for one file, in the shape of `wc`.
// The path is the first user argument; the executable's own path is not in
// `args()`.

import wc_words;

fn main() {
    let arguments = args();
    let path = switch arguments[0] {
        case Some(value) { value }
        case None { "" }
    };
    arguments.free();

    if path.len() == 0 {
        println("usage: wordcount <file>");
        return;
    }

    let text = read_file(path);

    let lines = text.split("\n");
    let count = lines.len();
    if count > 0 {
        let last = switch lines[count - 1] {
            case Some(value) { value }
            case None { "" }
        };
        if last.len() == 0 {
            count = count - 1;
        }
    }

    let words = 0;
    for line in lines {
        words = words + count_words(line);
    }

    print(count);
    print(" ");
    print(words);
    print(" ");
    print(text.len());
    println("");

    lines.free();
}
```

Put this in `wc_words.ya`, beside `main.ya`:

```yashy
// file: wc_words.ya
//
// A word is a run of bytes that are not a space, a tab, or a carriage return.
// The newline is already gone, so those three separators are the ones that can
// survive on a line.

pub fn count_words(line: string) -> int {
    let words = 0;
    let inside = false;
    let index = 0;
    let length = line.len();

    while index < length {
        // 32 space, 9 tab, 13 carriage return, in ASCII.
        let byte = line.byte_at(index).to_int();
        if byte == 32 || byte == 9 || byte == 13 {
            inside = false;
        } else {
            if !inside {
                words = words + 1;
                inside = true;
            }
        }
        index = index + 1;
    }

    return words;
}
```

The `import wc_words;` line looks for `wc_words.ya` beside the file that
imports it. A module's items are private unless they say `pub`, which is why
`count_words` is public and `main` is not. There is no package manager involved
yet: a module is a file, and its path is its name.

`args()` returns the program's user arguments as a fresh `[]string`; the
executable's own path is not included. The result is owned, so the program
calls `arguments.free()` on every path out. Indexing is checked and yields
`Option[string]` -- `None` when the index is out of range, rather than a trap --
and the string bound from it is retained, so the slice can be freed before the
path is used. An invocation with no arguments is an empty slice, not an error;
the program prints a usage line instead of reading a file.

`read_file(path)` returns the whole file as an owned string, or the empty string
when it cannot be read. R1 has no builtin `Result`, so a missing file and an
empty one read the same; the guide notes that rather than hiding it. The
program splits the text on `"\n"`. `split` keeps the part after the final
separator, so a file that ends with a newline has one trailing empty part; the
program drops that part before counting lines. `text.len()` is the byte count,
which is exactly the file size.

The word counter is small enough to read as a standalone program. The core is a
single pass over the line: a separator ends the current word, and a non-
separator after a separator starts a new one.

```yashy
fn count_words(line: string) -> int {
    let words = 0;
    let inside = false;
    let index = 0;

    while index < line.len() {
        let byte = line.byte_at(index).to_int();
        if byte == 32 {
            inside = false;
        } else {
            if !inside {
                words = words + 1;
                inside = true;
            }
        }
        index = index + 1;
    }

    return words;
}

fn main() {
    print(count_words("hello world"));
    println();
    // => 2
}
```

The project version also treats a tab and a carriage return as separators,
because those can appear in a line even after the newline is removed.

Create a small file and run the project with its path after `--`:

```text
printf 'hello world\nsecond line\n' > notes.txt
yashy run main.ya -- notes.txt
```

```text
2 4 24
```

Two lines, four words, and twenty-four bytes. The `--` separates Yashy's own
flags from the arguments the program receives, so `notes.txt` is the first
element of `args()` rather than a flag `run` tries to parse. The output is
three numbers on one line, with no labels, so it can be piped into another
program.

Run it with no argument and it prints the usage line instead:

```text
yashy run main.ya
```

```text
usage: wordcount <file>
```

## Add a test

`yashy test` runs every program under `tests/`. A test is an ordinary Yashy
program with a `main`; it passes when it exits with status 0 and fails when it
panics or exits non-zero. There is no test attribute and no test macro.

The generated `tests/smoke.ya` is already a check that the project still
starts:

```yashy
// file: tests/smoke.ya

fn main() {
    assert(1 + 1 == 2);
}
```

Then run:

```text
yashy test
```

```text
running 1 test
test tests/smoke.ya ... ok

test result: ok. 1 passed; 0 failed
```

A test under `tests/` is compiled on its own, so it cannot import a module from
the project root by a relative path. Keep a smoke test self-contained, or put
shared helpers in the standard library; the test runner is deliberately simple
in R1. A test that needs the real word counter can duplicate the function or
call the built executable as a subprocess.

## Build a release binary

```text
yashy build main.ya -o wordcount --release
```

`--release` compiles out `debug_assert` and defaults to `-O3`. An explicit
`-O` wins over the default:

```text
yashy build main.ya -o wordcount-fast --release -O2
```

The result is a native executable that does not need the Yashy compiler at run
time. The standard library is compiled into the program; the source files are
not.

On Windows the output is `wordcount.exe`; on Linux it is `wordcount`.

## Uninstall

The installers are the uninstallers too:

```text
./install.sh --uninstall
```

```powershell
.\install.ps1 -Uninstall
```

The Linux installer is a script, so keep the extracted archive, or re-run the
published installer as `curl ... | sh -s -- --uninstall`. The Windows install
keeps `install.ps1` in `%LOCALAPPDATA%\Yashy`, so it can uninstall after the
zip is gone.

They remove the compiler and the standard library, take the installer's `PATH`
entry out again, and leave your projects alone. Running the uninstall twice is
safe: the second run reports that there is nothing to remove. `yashy uninstall`
as a subcommand is an R2 convenience; the installer-level uninstall is the R1
contract.

## Troubleshooting

- **`clang: missing`.** Install LLVM with the command `doctor` prints. On
  Ubuntu or Debian it is `sudo apt install clang llvm`; on Fedora it is
  `sudo dnf install clang llvm`; on Arch it is `sudo pacman -S clang llvm`.
- **`stdlib: missing`.** The bundled standard library was not found. Set
  `YASHY_STDLIB` to the directory that contains `io.ya`, or reinstall Yashy.
  An install puts it under `share/yashy/stdlib/yashy`; a Windows install puts
  it under `yashy-stdlib\yashy` beside `yashy.exe`.
- **`yashy` is not recognized after installing on Windows.** Open a new
  terminal. The installer updates the user `PATH`, and a process that was
  already running keeps the old environment.
- **A build works but the program cannot find a module.** The module must be
  beside the file that imports it, and its name must match the import. There is
  no search path for project modules in R1.
- **A test fails with a module error.** Tests under `tests/` are entry files of
  their own. They cannot import a file from the project root by walking up.
  Make the test self-contained, or test the built binary as a subprocess.

## Try it

1. Change the program to print a label before each number, then update the
   test and the expected output in this chapter. Run `yashy test` and
   `yashy run` again.
2. Add a `summary` function that prints only the word count, call it from
   `main`, and update the test. A real `--quiet` mode can read a second
   argument from `args()`; remember to free the slice.
3. Break `count_words` on purpose: remove the `inside = true` assignment and
   run the standalone example. Read the wrong output, then restore the line.

## Where this leaves you

You have installed a compiler, created a project, split a program into a
module, read a command-line argument, tested the project, and built a release
binary. That is the R1 loop. Later slices add machine-readable diagnostics,
package management, and the editor and agent integration that make the loop
faster. The language itself is the one you have been writing all along.
