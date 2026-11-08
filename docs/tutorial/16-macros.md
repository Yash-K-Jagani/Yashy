# 16. Macros

## What you will learn

- What a declarative macro is: a name, and arms with metavariables.
- The seven fragment kinds an arm can declare.
- Why expansion runs before checking, so a macro cannot hide an effect.
- The caps: expansion depth, expansion size, and the diagnostics that
  protect them.

A macro is data about syntax. It is declared with a name and an order of
arms; each arm is a pattern over token trees and a replacement:

```yashy
macro say { $x:expr => { print($x); } }

fn main() {
    say("hello");
    // => hello
}
```

`macro` is the 36th keyword. Expansion is the first stage of the pipeline: a
macro name followed by `(...)` or `{...}` is an invocation, and the checker,
the verifier and the code generator only ever see the expanded tokens.
`say("hello")` is `print("hello");` before anything else runs.

## Metavariables

A `$name:kind` binds input of that kind to `name`. The kinds are exactly
`expr`, `stmt`, `pat`, `ty`, `ident`, `block`, and `item` — there is no
`tt`, deliberately: a metavariable that can be anything is one the checker
cannot re-parse deterministically.

```yashy
macro declare { $t:ty => { let $t zero = 0; } }

fn main() {
    declare(int);
    print(zero);
    // => 0
}
```

Matching is left to right, and a fragment takes the longest prefix that
parses as its kind: `$cond:expr` in `x > 0, { a(); }` takes `x > 0` whole and
stops at the comma. An identifier metavariable matches only an identifier —
`$n:ident` given `1` matches no arm at all, rather than becoming a type error
later.

An arm that matches no input is `E1600`, and the diagnostic prints both the
invocation and the patterns it tried. A metavariable given tokens of the
wrong kind is `E1602`.

## Why expansion-first is the safety property

A macro that expands to `print(..)` inside a `pure fn` is `E1300`, exactly as
if the call had been written by hand. That works because expansion happens
before checking: there is no hidden effect, only hidden syntax. The same is
true for ownership: a macro that expands to a second `.free()` is `E0500`, and
one whose expansion drops a linear value is `E1000`.

A diagnostic whose span lies inside macro output is reported at the
invocation, and its notes carry the expansion chain:

```text
main.ya:4:5
error[E0100]: expected `int`, found `string`
    bad(1);
    ^^^
note: expansion: in macro `bad`, arm at line 1
note: expansion: expanded from line 4
```

## Caps

Expansion stops at depth 64 or 1 MiB of output per invocation, and
exceeding either is `E1601` rather than a compiler that grows without bound
or a stack overflow. A name declared twice — or a macro whose name collides
with a function — is `E1603`: an invocation resolves by name, so one name can
only expand one way.

There is no recursion primitive: a macro whose output invokes itself reaches
the depth cap and is refused.

## Inspecting expansion

`yashy expand main.ya` prints the fully expanded program, formatted
canonically, and stops before checking. `yashy expand --json main.ya` lists
every invocation in source order, with its macro name, arm span, invocation
span, and output size:

```json
{
  "schema_version": 1,
  "tool": "yashy",
  "expansions": [
    {
      "macro": "say",
      "arm_span": { "start": 12, "end": 35, "line": 1, "column": 13 },
      "invocation_span": { "start": 57, "end": 60, "line": 4, "column": 5 },
      "output_tokens": 5
    }
  ]
}
```

The same expansion feeds `check`, `verify`, `build`, `run`, `test` and
`--emit-ir`, so there is one expansion per run and every command sees the
same tokens.
