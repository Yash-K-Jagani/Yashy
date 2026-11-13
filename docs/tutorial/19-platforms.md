# 19. Platforms

## What you will learn

- `cfg`: code that exists only on some platforms.
- `--target`: building for a platform other than the one you are sitting at.
- What the three platforms guarantee, and what cross-compiling does and does
  not give you.

A program that only works where it was written is a program with a very small
audience. Yashy gives you two tools for the other kind: `cfg`, for code that
belongs to one platform, and `--target`, for building something for a platform
that is not this one.

## `cfg`

`cfg(..)` guards an item, a statement block or a struct field. The guarded
code exists only on the platforms the condition names:

```yashy
cfg(windows) fn platform() -> string {
    return "windows";
}

cfg(unix) fn platform() -> string {
    return "unix";
}

fn main() {
    const name = platform();
    // `name` is "windows" on Windows and "unix" elsewhere. The program
    // says nothing here, because what it would say depends on the machine
    // it was built for.
}
```

The keys are `windows`, `linux`, `macos` and `unix` for the platform, and
`arch_x86_64` and `arch_aarch64` for the machine. `debug` and `release` say
which kind of build this is. They combine with `not(..)`, `any(..)` and
`all(..)`:

```yashy
cfg(any(windows, linux)) fn desktop() -> string {
    return "desktop";
}

fn main() {
    // The call is guarded too: on macOS `desktop` does not exist, so an
    // unguarded call would name a function the compiler rightly cannot find.
    cfg(any(windows, linux)) {
        const name = desktop();
        // `name` is "desktop" on Windows and Linux, and the block does not
        // exist at all on macOS.
    }
}
```

A disabled branch is **not resolved, not checked, not verified and not
emitted**. That is the whole point, and it is what distinguishes `cfg` from
`if false`: the branch may call a function that exists nowhere, because the
compiler removes the branch before it looks a single name up.

A key the compiler does not define is an error, not a condition that quietly
evaluates to false. `cfg(window)` -- one letter short -- would otherwise
delete a branch on every platform, silently, and the program would do the
wrong thing without any other sign.

To see what is true for the build you are making:

```text
$ yashy check --print-cfg
arch_aarch64
debug
macos
unix
```

## `--target`

`--target` names the platform a build is for:

```text
$ yashy build --target aarch64-apple-darwin main.ya
```

The supported triples are `x86_64-pc-windows-msvc`,
`aarch64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `x86_64-apple-darwin` and
`aarch64-apple-darwin`. Any other is refused with the list of the ones that
work.

The target is one fact, and the whole build agrees on it. The `cfg` keys
answer from it, the data layout in the emitted LLVM IR comes from it, the
platform libraries come from it, and `clang` is given `--target=<triple>`.

Cross-compiling is **not** self-contained. Yashy's runtime is C and the
program is linked by `clang`, and the compiler ships neither a linker nor a
cross runtime: a target needs its own C toolchain installed. When that
toolchain is missing, the build says so by name rather than failing later at
the link:

```text
$ yashy build --target aarch64-apple-darwin main.ya
error[E2201]: the C toolchain for `aarch64-apple-darwin` is not installed: `clang --target=aarch64-apple-darwin` could not link a trivial program
```

`yashy doctor --target <triple>` asks the same question without building.

`run` and `test` refuse a target other than the host: they run what they
build, and a binary for another platform cannot start here. `build` is the
way to make a program for another platform.

## What the platforms guarantee

| Platform | Architectures | The release ships |
|---|---|---|
| Windows | x86-64 | a zip, with the binary signed |
| Linux | x86-64 | a tarball, with a signed checksum file |
| macOS | Apple silicon and Intel | a zip, signed, notarised and stapled |

The same tree builds for all of them, and `yashy build --reproducible` makes
two builds of it produce the same bytes, so a release's checksum is a fact
about the tree and not about a moment.

## Exercises

1. Write a program that prints `windows`, `linux` or `macos`, using one `cfg`
   per platform, and a fourth branch that never runs. The fourth branch has
   to be written with the keys the compiler knows — `cfg(netbsd)` is an error,
   not a branch that never runs.

2. Add a `cfg(macos)` block that calls a function you have not written. It
   must still compile on Windows and Linux, where the block is removed
   before the compiler looks the name up.

3. Build your program for another platform with `--target` and read the
   `target triple` line at the top of the emitted IR:

   ```text
   yashy build --target aarch64-apple-darwin --emit-ir main.ya
   ```
