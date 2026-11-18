# Migrating from 1.0

Yashy 2.0 keeps the language's identity: no garbage collector, no borrow
checker, no exceptions, no inheritance, no operator overloading, no procedural
macros, no `dyn` and no pluggable async runtime. A program that stayed inside
that identity keeps working. This chapter names the places where source has to
change and the order in which to change them.

## The short list

| 1.0 spelling | 2.0 spelling | Why |
|---|---|---|
| `let x: int = 5;` | `let int x = 5;` | The type comes first everywhere (`E0109`). |
| `arena { ... }` | `arena { ... }` with `defer { a.free(); }` | Release is still explicit; 2.0 adds `defer expr;` as sugar for one release. |
| `1.0..2.0` as a range value | `1..2` as a `Range[T]` value | The half-open spelling has a total length; float bounds are refused. |
| `switch e { case A if c {...} }` | add an unguarded arm or `default` | A guard can be false, so it covers nothing (`E0600`). |
| `alloc(n)` | `alloc(n)` (unchanged) | Safe allocation is zeroed from Phase 4 on; no sentinel answers were added. |

A name-first binding still parses far enough to produce `E0109`, which points
at the exact fix. Nothing is removed silently: see
[deprecation](https://yashy.dev/deprecation) for the four-step policy and the
current migration table.

## How to migrate a module

1. Run `yashyc check --json <file.ya>` and read `diagnostics[]`. Each coded
   diagnostic has a `docs:` link and a machine-readable `code`.
2. Run `yashyc explain <code>` for the long-form page: cause, minimal example
   and fix. The same page ships offline, so migration works without a network.
3. Fix one diagnostic class at a time, rerunning `check` after each. Type-first
   bindings are mechanical; guard and ownership changes need a look at the
   surrounding control flow.
4. Run `yashyc verify --json <file.ya>` before claiming a build. `build` and
   `run` refuse an unsound program; `proof` names what is missing.
5. Run `yashyc test --json` and inspect `tests[]` and `summary`. The event
   stream has one `start` and one `end` per test, so a failing test's own
   output is attributable.

## Support window

1.0 receives security and critical correctness fixes for 18 months after the
2.0 release; 2.0 then becomes the LTS baseline. The support matrix and the
definition of a security fix live in `docs/lts.md` in the repository. An
edition change, if one is ever needed, is an RFC with an explicit migration
note — never a silent change in meaning.

## What did not change

Errors are still values (`Option[T]`, `Result[T, E]`) with `?` propagation.
Ownership is still explicit: `move`, `defer`, `.free()`. Dispatch is still
static: traits are requirement sets, `impl` is inherent or `impl Trait for T`,
and there is no vtable. Async is still one executor: `async fn` returns
`Task[T]`, `scope` joins or cancels its children, and a lock held across
`.await` is `E1702`.
