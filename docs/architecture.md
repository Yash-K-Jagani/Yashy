# Yashy Architecture

**Status:** living document. It describes the language and compiler as they are,
and marks every 2.0 target with **(2.0)**. When the two disagree, the code wins
and this page is the bug.

**Audience:** a contributor who needs to know where a fact lives; an agent that
needs the shape of the system before editing it; a user who wants to know what
the compiler promises.

**Companions:** the design record
[`superpowers/specs/2026-10-27-yashy-master-design.md`](superpowers/specs/2026-10-27-yashy-master-design.md)
(identity), [`superpowers/specs/2026-11-15-yashy-2.0-design.md`](superpowers/specs/2026-11-15-yashy-2.0-design.md)
(2.0 thesis), the [CLI reference](cli.md), and the
[error catalog](errors.md).

---

## 1. What Yashy is

Yashy is a statically typed, compiled language with explicit ownership and no
garbage collector. Its defining bet is that a program can be **proved** safe
enough to compile to ordinary native code without a borrow checker: ownership is
linear, effects and contracts are checked, and the verifier's facts are spent as
deleted checks and LLVM attributes.

Three rules organise everything else:

1. **One fact, two renderings.** Every machine-readable model (`index`,
   `context`, diagnostics, layout, trace) has a human rendering generated from
   it. No second source of truth.
2. **The verifier is the only judge.** Ownership, effects, contracts and
   typestate are decided in `yashy-verify`; no other crate re-implements a rule.
3. **A malformed program produces a diagnostic, never a panic.** The
   `never_panics` corpus enforces it.

The identity rules — no GC, no borrow checker, no exceptions, no inheritance, no
operator overloading, no procedural macros, no `dyn`, no async traits, no
pluggable runtime — are hard constraints. See §4 of the 2.0 design.

## 2. The shape of a build

```text
 source bytes
     │
     ▼
 yashy-lexer ── tokens ──► yashy-expand ──► yashy-parser ── AST
                                                │
                                                ▼
                                          yashy-sema        name resolution,
                                                │           types, exhaustiveness,
                                                │           desugaring to HIR
                                                ▼
                                          yashy-hir         typed program, layout,
                                                │           escape facts
                                                ▼
                                         yashy-verify       ownership, effects,
                                                │           contracts, typestate
                                                ▼
                                        yashy-codegen       textual LLVM IR
                                                │
                                                ▼
                                       llc + link           object + link
                                                │
                                                ▼
                                          native binary
```

`yashy-query` memoises the front end per file and owns the parallel schedule;
`yashy-cache` stores content-addressed results. The one-shot CLI is the
contract; the daemon (2.0) is a cache and an event loop in front of the same
functions, never a second compiler.

In 1.0 the last two boxes are one `clang` process: it compiles the embedded C
runtime, assembles the textual IR and links the binary. From Phase 11C they are
separate facts. The runtime is a signed per-target archive, `llc` turns IR into
an object, and a `LinkSpec` is rendered by `ld.lld`, `lld-link`, Apple `ld` or
`link.exe`, with the sysroot and CRT discovered explicitly. `clang` remains a
compatibility and sanitizer backend, not the driver. A normal build never
invokes it, and the clean-machine gate proves that on Windows and Linux.

A package's `[native]` libraries join the link as explicit inputs: `pkg-config`
or a recorded path for a system library, a hashed static archive for a prebuilt
one, or a sandboxed `build.ya` for vendored sources. The `-sys` package owns the
generated `unsafe` declarations and the layout assertions; the safe wrapper is
ordinary checked Yashy. A Yashy library exports through one list that generates
the C header, the version script and every host bridge, so the ABI is one fact
with many renderings, not a hand-written adapter per language.

| Stage | Input | Output | Refuses |
|---|---|---|---|
| `yashy-lexer` | bytes | tokens, doc lines | invalid UTF-8, bad escapes |
| `yashy-expand` | token trees | token trees | macro depth/size/fragment/duplicate (`E1600`–`E1603`) |
| `yashy-parser` | tokens | AST | syntax (`E01xx`), irrefutable-pattern violations (`E0100`) |
| `yashy-sema` | AST | HIR | names, types, exhaustiveness, arity |
| `yashy-verify` | HIR | proofs | ownership (`E1000`), effects, contracts, typestate |
| `yashy-codegen` | HIR | LLVM IR | nothing: it spends only proved facts |
| `llc` + linker | IR + runtime archive | binary | toolchain faults (`E2200`-`E2201`) |

A build is deterministic in the sense of §7: the same inputs produce the same
IR, the same diagnostics in the same order, and — with `--reproducible` — the
same bytes.

## 3. Crate map

| Crate | Job | Notes |
|---|---|---|
| `yashy-common` | diagnostics, codes, spans, JSON, source map, suggestions | schema v1 lives here |
| `yashy-lexer` | bytes to tokens | doc comments are tokens |
| `yashy-expand` | declarative macro expansion | before parsing, once |
| `yashy-parser` | tokens to AST | recovery is LSP-only (2.0) |
| `yashy-sema` | names, types, exhaustiveness, desugaring | HIR is the only typed form |
| `yashy-hir` | typed program, layout, escape, exhaustiveness | shared vocabulary |
| `yashy-verify` | ownership, effects, contracts, typestate | the only judge |
| `yashy-codegen` | HIR to textual LLVM IR | one file per module |
| `yashy-runtime` | C shim; source and signed per-target archive | source is the truth, archive is what ships |
| `yashy-cache` | content-addressed results | key covers every input |
| `yashy-query` | memoised pipeline, parallel schedule | shard per file |
| `yashy-manifest` | `yashy.toml`, `yashy.lock` | exact pins today |
| `yashy-stdlib` | Yashy-source modules | documented `pub fn` only |
| `yashy-driver` | CLI, module graph, toolchain, packages, MCP | the verbs |
| `yashy-fmt` | canonical formatter | idempotent, comment-preserving |
| `yashy-lsp` | language server | diagnostics, hover, definition, completion, tokens |
| `yashy-bench` | benchmark rows and comparators | no number, no claim |

The workspace's own packages live under `packages/` (`net`, `http`, `cli`,
`test`, `bench`, `fuzz`) and are ordinary Yashy source with manifests.

## 4. The program model

### 4.1 HIR is the typed truth

Every surface form lowers into HIR that already exists. Sugar does not add a
second semantic layer: `if let`, `while let`, `?`, spread, chained comparisons,
`for i, x`, field defaults, `arena { }`, or/range/nested patterns, patterns in
binding position, and named/default arguments all desugar. `else if` and
`else if let` chains are parser sugar for an `else` block holding one `if`, so
they add no HIR either. The two recorded exceptions are `HirStmt::IfLet` (two
blocks a `switch` arm cannot hold) and `HirBlock::arena` (one placement bit).
`crates/yashy-sema/tests/desugar.rs` is the proof.

### 4.2 Types

- Values: integers, floats, `bool`, `char`, `string` (refcount), arrays
  `[]T`, tuples, structs, enums, maps, function types.
- No implicit coercion and no implicit widening. `as` is the only conversion.
- Generics are schemes: `Pair[T]` is a scheme, `Pair[int]` is one monomorphic
  type. A generic function is instantiated per substitution, capped at 64 per
  function and 4096 per program (`E1505`).
- A const parameter (`Buffer[const N: int]`) is part of the scheme: `Buffer[16]`
  and `Buffer[64]` are distinct monomorphic types. Const evaluation is total and
  counts against the same instantiation caps (2.0, Phase 4).
- `Iterator[T]` is the compiler-known `for` protocol. User generic traits are
  `E1511` in 1.0 and become legal in 2.0 (Phase 4), still statically dispatched.
- There is no `dyn`, no vtable, no runtime type information, no `Drop`, no
  inheritance, no trait fields, and no operator overloading.

### 4.3 Ownership

A value has one owner. `move` transfers it; the source is dead. `defer` runs at
scope exit in reverse registration order. Owning values are released exactly
once, by their owner: `*mut T`, arrays, maps, channels, mutexes and built
strings use `.free()`; a `string` binding releases its own reference.

The verifier proves, for every path, that a linear value has an owner and is
released once (`E1000`). It does not track aliasing the way a borrow checker
does — see §5.

### 4.4 Effects, contracts, typestate

- **Effects** are declared on functions and checked at call sites.
- **Contracts** (`requires` / `ensures`) are checked where they can be, and
  become optimizer facts where they are proved.
- **Typestate** tracks object state transitions; an illegal transition is a
  compile error.
- **Async** is one executor: `async fn` returns `Task[T]`, `.await` suspends,
  `scope` joins or cancels children. A lock across `.await` is `E1702`; a task
  escaping its scope is `E1701`; `await` outside `async fn` is `E1700`.

### 4.5 Patterns

A pattern in `let`, `const` or `for` must be irrefutable (`E0100`). `switch`
arms may be refutable; every alternative of an or-pattern binds the same names
with the same types (`E1002`). 2.0 adds `let else`, slice/rest patterns, `@`
bindings, `matches`, and if-let chains (Phase 4).

### 4.6 Macros and `cfg`

Macros are declarative data expanded before parsing, once, with closed caps
(`E1600`–`E1603`). `cfg` is a closed key set decided before name resolution
(`E2202`, `E2203`); a disabled branch is never resolved, checked or emitted. One
target triple is the whole build's fact: `cfg` keys, data layout, platform
libraries and `clang --target` all read it.

## 5. Memory model

### 5.1 Values and owning pointers

A struct is a value, not a reference: assignment loads and stores the aggregate.
`*mut T` is Yashy's **owning** pointer: linear, typed, freed with `.free()`, and
created by `alloc(n)` (which yields `*mut int`) or `arena_alloc`. `*const T` and
bare `*T` exist at the FFI edge and do not own.

The pointee type is part of the pointer type, so a `*mut Point` cannot be
indexed as `*mut int`. Pointer arithmetic and indexing are bounds-checked where
the allocation size is known; the verifier's facts delete the check where it is
not needed.

### 5.2 Views and arenas

Views are unnamed, non-owning windows into an array; they cannot be stored in a
field or returned past their source. An arena owns many blocks and releases them
at once (`arena_new`, `arena_alloc`, `arena_free_all`). `arena { }` lowers to a
block with one placement bit.

### 5.3 Aliasing rules

Yashy has no borrow checker, so the aliasing model is **conservative by
default**:

1. **Owning pointers are unique.** Linearity means an owning pointer is moved,
   not copied. A pointer parameter passed by value is therefore the only path to
   its allocation for the duration of the call, and codegen may emit `noalias`
   and `nonnull` for it.
2. **FFI pointers may alias.** `*const T` and bare `*T` carry no uniqueness
   assumption. `noalias` on an `extern` parameter or an `unsafe fn` parameter is
   an explicit promise; safe code cannot declare one, so safe code cannot break
   one.
3. **Disjoint views are proved, not assumed.** Two views may alias unless the
   compiler proves the ranges disjoint from bounds and indexing facts.
4. **No aliasing assumption crosses a safe call.** Because safe Yashy has no
   references, there is no `&mut`-style exclusivity to miscompile.
5. **Every assumption is reported.** `--opt-report` names each `noalias`,
   `nonnull` and `dereferenceable` it emitted and the fact that licensed it.

The written rules are normative (2.0, Phase 4); the alias corpus and the
`noalias` audit are the gate. A pointer whose provenance is unknown is never
given an attribute.

### 5.4 Layout

`repr(C)` is a keyword form on a struct, enum or object, and the only thing that
licenses the C spellings `u8`, `i32` and friends. The compiler knows every
type's size, alignment and field offsets; `yashyc inspect --layout` prints them
as a table and as JSON (2.0). 2.0 adds `repr(packed)`, `repr(align(N))`,
`repr(transparent)`, explicit integer enums, and compile-time `size_of`,
`align_of` and `offset_of`.

A type without `repr(C)` has no source-level layout guarantee; the compiler may
reorder fields to reduce padding and reports the choice (2.0, Phase 11A).

### 5.5 Stack, heap and allocation

- Scalars, structs, fixed arrays and tuples are values; they live on the stack
  unless they escape.
- `alloc(n)` is a direct, visible call to the allocator. There is no hidden
  allocation in the language or the runtime.
- **(2.0)** `alloc(n)` returns zeroed memory; `alloc_uninit(n)` is the
  `unsafe` spelling for callers that will write before they read.
- **(2.0)** escape analysis promotes a non-escaping `alloc` with a constant size
  to a stack slot, and `stack_alloc(n)` is an explicit frame allocation whose
  pointer the verifier refuses to let escape.
- **(2.0)** an `Allocator` interface lets a program supply its own allocator;
  `--alloc-report` lists every allocation site, its size class and whether it
  was promoted to the stack or served by the global allocator.

### 5.6 Determinism

See §7. The memory model contributes three rules: zeroed allocation by default,
no uninitialised read in safe code, and no pointer value in observable output
(addresses are non-deterministic; printing one is refused, and `unsafe` can
observe it explicitly).

## 6. Diagnostics and machine-readable facts

A diagnostic has `code` (null when uncoded), `severity`, `message`, `file`,
`span`, optional `labels`, optional `notes`, `docs_url` when coded, and — for
the verifier — a `proof` object naming the `obligation`, the `missing_fact` and
the `remedy`. `check --json`, `verify --json` and `test --json` share schema v1;
the envelope has `schema_version`, `tool`, `yashy_version`, `success`,
`diagnostics` and `summary`.

2.0 makes the machine-readable form richer without breaking v1:

- **(2.0)** `suggestions[]` carry a byte span and replacement text, so
  `yashyc fix`, the LSP and an agent all apply the same edit.
- **(2.0)** `--sarif` renders the same model for code scanning.
- **(2.0)** every code has a page under `docs/errors/`, an entry in the
  catalog, an `explain --json` record, and at least one test that triggers it.
  `yashyc explain --all --json` is the catalog's machine form.
- **(2.0)** diagnostics are emitted in a defined order (file, byte offset,
  code), so two runs diff cleanly.

## 7. The determinism contract

2.0 makes determinism a promise, not a hope:

> Given the same program, the same inputs, the same declared capabilities and
> the same target, a Yashy program produces the same output bytes, the same
> exit code and the same event trace.

What the contract requires:

| Area | Rule |
|---|---|
| Allocation | `alloc` zeroes; uninitialised memory needs `unsafe alloc_uninit` |
| Maps and sets | iteration order is defined (insertion order); no randomised hash iteration |
| Hashing | the hash seed is fixed unless a program asks for a random one |
| Scheduling | `--deterministic` runs tasks on a deterministic scheduler; thread creation order is observable and stable |
| Time and RNG | clock, environment and entropy are capabilities; `--deterministic` supplies fixed values and a fixed seed |
| Floating point | bit-identical per target under fixed flags; `--strict-fp` for cross-target bit-identical results at a measured cost |
| Output | no addresses, no timestamps, no paths in diagnostics or `--json` |
| Panics | fixed message, no backtrace addresses unless asked |

Determinism is per-target for floating point, because the hardware differs. A
program that reads a capability the harness did not fix is not claimed
deterministic, and `yashyc trace` says which capability it read.

## 8. Tooling surface

The compiler verbs are the API; the human rendering is generated from the JSON
model. 1.0 ships `build`, `run`, `check`, `verify`, `test`, `fmt`, `explain`,
`init`, `doctor`, `expand`, `index`, `outline`, `graph`, `search`, `context`,
`mcp`, `lsp`, `doc`, `profile`, `tasks`, the package verbs (`lock`, `fetch`,
`update`, `add`, `audit`, `publish`) and `playground`.
`2.0 adds `watch`, `serve`, `new`, `fix`, `lint`, `coverage`, `task`, `repl`,
`debug`, `bench`, `fuzz`, `migrate`, `conformance`, `sbom`, `inspect`, `trace`,
`tokens`, `tree`, `why`, `info`, `outdated`, `vendor`, `remove`,
`verify-package`, `self-update`, `attest` and `verify-attestation` (Phases 2,
3, 9, 13, 14). Each new verb has a human and a `--json` form; a schema diff in
CI forbids removing a field.

The installed CLI also exposes `cache` (inspect the content-addressed store),
`help` (the one command table) and `completions` (bash, zsh, fish and
PowerShell, generated from that same table). A test checks that every verb in
the command table is named here.

`yashyc doc` is the documentation source of truth, rendered from the compiler's
own tables and byte-stable across runs and operating systems. `docs/stdlib.md`
is generated; edit the module, never the page.

## 9. Packages and reproducibility

A package is a manifest plus source. The lockfile pins exact versions; ranges
resolve once and are pinned (2.0). A package tarball is signed; the index is
signed; the SBOM lists every input hash, the toolchain, the runtime and every
native library; an in-toto/SLSA statement records the materials and is verified
by a rebuild. A package declares its capabilities (`fs`, `net`, `env`, `clock`,
`proc`, `rng`, `ffi`); the workspace policy is the ceiling and a missing
declaration grants nothing. CI uses `--locked` and never trusts a cache it did
not create.

`--reproducible` fixes the build id, removes timestamps and absolute paths,
sorts symbols and honours `SOURCE_DATE_EPOCH`. The gate builds twice in two
directories and compares bytes.

## 10. Testing and traceability

Tests are ordinary programs: a file under `tests/` with `main`; exit 0 is a
pass. The harness (2.0) links one binary, attributes stdout and exit codes to
the test that produced them, and reruns a failure alone.

**(2.0)** Every rule has an ID (`OWN-3`, `ALIAS-2`, `DET-4`). Every diagnostic
code names its rule; every conformance case names its rule; every stdlib
function names the test that covers it. `yashyc trace --json` emits the matrix,
and CI fails on an orphan. `--trace=events` emits a deterministic runtime log
(alloc, free, task, io) with source spans, so a failing run points at a line.

## 11. The performance model

Where the time goes on a 1.0 build: link ~2.5 s per test process, `clang` one
invocation, front end 22 ms, full build 259 ms. At run time the compiler may
assume only what the verifier proved: an owning pointer is unique and non-null,
an array index is in bounds where the fact exists, a contract holds where it was
checked. Every surviving check is explained by `--opt-report`.

The optimizer is a phase, not a habit: a pass is registered in the pass manager,
has its own benchmark row, and is measured interleaved on both operating
systems. **(2.0)** Phase 11A spends the memory model (layout, aliasing, stack,
heap); Phase 11B spends the code generator (inlining, vectorisation, PGO, LTO,
BOLT, incremental link).

## 12. Where 2.0 changes this document

| Phase | Change to this document |
|---|---|
| 1 | test harness, daemon, watch, incremental cache, parser recovery |
| 2 | LSP surface, `fix`/`lint`, debugger, profiler, `inspect`, error catalog, generated editor grammars, terminal surface |
| 3 | semver, workspaces, features, signing, SBOM, provenance, capabilities/policy, reproducible install |
| 4 | generic traits, associated types, patterns, `const fn`, pointers/`unsafe`, layout, aliasing rules |
| 5 | collections as types, allocation interface, deterministic iteration |
| 6–8 | network, data, application stack, `yashyc dev`, OpenAPI, OTel |
| 9 | traceability matrix, quality stack, mutation and differential harness |
| 10 | atomics, locks, deterministic scheduler, record/replay |
| 11A | layout optimisation, stack/heap reporting, aliasing audit |
| 11B | inlining, passes, vectorisation, PGO/LTO/BOLT |
| 11C | runtime archive, `llc`/`lld` pipeline, toolchain package, clean-machine install |
| 12 | WASM, SIMD, bindgen, native C libraries, C ABI library, host bridges, more triples |
| 13 | MCP v1, semantic graph, agent policy/audit/approval, review/impact, SDK, BPE alignment gate |
| 14 | language reference, conformance, editions, channels, advisories, release |

---

*If a sentence here is not true of the code, the sentence is wrong. Fix it in
the same change that makes it false.*
