# Determinism

A program that asks for the same inputs and the same target produces the same
bytes. The compiler does not guess whether a function is pure: it checks the
effects declared on the item.

## Enforced rule

**DET-1 — a pure function cannot observe the environment.** A function
declared pure that reads time, randomness, a file or a clock is
[E1302](../errors/E1302.md). The check is at the call site of the impure
operation, so the diagnostic names the exact effect that broke the promise.

## Rules not yet enforced

These rules are design commitments that the current front end cannot express a
statically detectable violation of. They are reported as orphans by
`yashyc trace --fail-orphans` and are not claimed as checked.

- **DET-2** — a fixed seed fixes every draw from the random source.
- **DET-3** — a fixed clock fixes every time read.
- **DET-4** — a program's output does not depend on allocator addresses.
- **DET-5** — a program's output does not depend on iteration order of a map
  with unspecified order.

The runtime fixes these facts at the process edge for a deterministic build;
what remains is the compiler-side proof that no other path reads them. When
that proof exists, each rule gains a diagnostic and moves out of this list.

## Byte stability

Generated docs, completions, diagnostics and JSON reports are byte-stable
across runs and operating systems: sorted where order is not semantic, no
timestamps, no absolute paths in the body. The golden tests pin the committed
copies. A mismatch is a compiler bug, not a formatting difference.
