# Yashy quickstart

This page is the short path from a clean machine to a running program. The
longer, project-shaped version is [Your first CLI](tutorial/12-first-cli.md).

## 1. Install LLVM

Yashy compiles through `clang`, so it needs LLVM:

| Platform | Command |
|---|---|
| Ubuntu / Debian | `sudo apt install clang llvm` |
| Fedora / RHEL | `sudo dnf install clang llvm` |
| Arch | `sudo pacman -S clang llvm` |
| Windows | `winget install LLVM.LLVM` |
| macOS | `brew install llvm` |

macOS is a 1.0 target: this release does not ship a macOS archive, so build
Yashy from source there.

## 2. Install Yashy

**Windows.** Download the release zip, extract it, and run:

```powershell
.\install.ps1
```

If PowerShell refuses to run a local script, use
`powershell -ExecutionPolicy Bypass -File .\install.ps1` for this one run.

**Linux.** Download the tarball, extract it, and run:

```sh
tar xzf yashy-<version>-x86_64-unknown-linux-gnu.tar.gz
cd yashy-<version>-x86_64-unknown-linux-gnu
./install.sh
```

Or use the published installer:

```sh
curl -fsSL https://github.com/Yash-K-Jagani/Otter/releases/latest/download/install.sh | sh
```

The installers verify `SHA256SUMS`, install into a prefix, add it to `PATH`, and
run `yashy doctor`. Use `--prefix` / `-Prefix` for a different location,
`--no-path` / `-NoPath` to manage `PATH` yourself, and `--uninstall` /
`-Uninstall` to remove the install.

## 3. Check it

```text
yashy doctor
yashy doctor --json
```

`doctor` exits non-zero if `clang` cannot run or the standard library is
missing. Its final `next:` line is the fix. The JSON form has
`schema_version: 1` and the same facts for scripts and agents.

## 4. Create and run a project

```text
yashy init wordcount
cd wordcount
yashy run main.ya
```

```text
Hello, Yashy!
```

`yashy run` compiles and runs in one step. `yashy build main.ya -o wordcount --release`
writes a native binary; `--release` defaults to `-O3`, and an explicit `-O`
wins.

## 5. Test it

```text
yashy test
```

Every `.ya` file under `tests/` is one test program. A test passes when it exits
0 and fails when it panics or exits non-zero. There are no test attributes and
no test macros; a test is an ordinary program with a `main`.

## Next

- [Your first CLI](tutorial/12-first-cli.md): a two-module stdin filter, tests,
  and a release build.
- [The language tutorial](tutorial/00-intro.md): thirteen chapters, every example
  compiled and run by the test suite.
- [`examples/`](../examples): `hello`, `wordcount`, and `json-report`, each a
  complete project.
- [`AGENTS.md`](../AGENTS.md): the repository guide for coding agents.
- [`llms.txt`](../llms.txt): canonical links for machine readers.
