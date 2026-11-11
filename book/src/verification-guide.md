# What the verifier proves

Yashy has two front halves that run before a program is compiled: the type
checker and the verifier. They answer different questions, and a program has
to pass both.

- The **type checker** asks whether the program is *well-typed*: is this value
  an `int` or a `string`, is this binding still live, does this `match` cover
  every case.
- The **verifier** asks whether the program is *sound*: does every value that
  owns memory have exactly one owner, and is every contract that was written
  actually true.

## Ownership

There is no garbage collector and there are no exceptions. A value that owns
memory has exactly one owner, and the compiler can prove it:

```yashy
fn main() {
    const buffer = alloc(16);
    defer { buffer.free(); }
    print("ok");
}
```

The `defer` releases the buffer when the scope is left, on every path out of
it. Without it, the program does not compile: the verifier reports a value
that is never released.

A `move` transfers ownership:

```yashy
fn take(string text) {
    print(text);
}

fn main() {
    const owned = "hello";
    take(move owned);
    // `owned` is gone here: using it again is a compile error, not a crash.
}
```

Using a moved value, or a released one, is an error the compiler reports
with the position of the use.

## Contracts

A function can promise things about its result, and the verifier checks the
promise at every call:

```yashy
fn positive(int value) -> int requires value > 0 ensures result > 0 {
    return value;
}

fn main() {
    print(positive(3));
}
```

`requires` says what the caller must ensure; `ensures` says what the caller
may rely on. A call that cannot prove the `requires` is a compile error, and
the body has to prove the `ensures`.

## What it does not prove

The verifier is a static analyser, not a theorem prover about the whole
world. It proves:

- every linear value is released exactly once, on every path;
- every written contract holds at every call site;
- the arithmetic that can overflow is guarded, and the guard is the one the
  runtime checks.

It does not prove that your program does what you meant. A function that
returns the wrong answer, correctly typed and correctly owned, passes every
check the compiler makes.

## Seeing the proof

`yashy verify` runs the verifier on its own and prints what it could not
establish:

```text
$ yashy verify main.ya
main.ya:7:5
error[E1401]: `buffer` is used after it was released
```

`--inventory` lists what the proof rests on: every C declaration the program
calls, and every `unsafe` call it makes. It is evidence, not a verdict.

## See also

- [Memory and ownership](language-guide/07-memory.md) — the language's own
  account of ownership, `move` and `defer`.
- [Errors as values](language-guide/06-errors.md) — `Option` and `Result`.
- [The error catalog](../errors.md) — every diagnostic the compiler can
  print, with its cause and its fix.

## Exercise

Write a function that owns a buffer and hands it to a caller, and a caller
that frees it exactly once. Then write the version that frees it twice and
read what the compiler says.

The solution is `book/tests/exercise-ownership.ya`.
