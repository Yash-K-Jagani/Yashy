# Slice — Strict Type / Verification System (separate plan)

**Goal:** Provide the strictest possible type‑ and memory‑safety verification while keeping the language’s design space as small as Go’s — i.e. *few core constructs, explicit everything, no hidden magic*.

**Start condition:** Slice 3 acceptance is green (commit e015b77). This plan can be started immediately.

**Design‑space constraint:** The language may only have the following *core* forms (nothing else is allowed without an explicit exception recorded in the plan):
1. **Literals** – integers, booleans, strings, chars, arrays, maps, tuples.
2. **Variables** – `let`, `mut` with explicit `move`/`defer`.
3. **Functions** – `fn` with a single return type, `move` semantics for parameters, `defer` for cleanup.
4. **Control flow** – `if`, `else`, `loop`, `break`, `continue`.
5. **Sum types** – `enum` with named variants (each variant may hold data).
6. **Product types** – `struct` with named fields, `tuple` (positional) as a lightweight product.
7. **Ownership** – every value has a single owner; `move` transfers ownership; `defer` releases at scope exit.
8. **Error handling** – `Result[T, E]` (explicit); no exceptions.
9. **Memory** – stack‑allocated or arena‑allocated; explicit `.free()` for heap; no garbage collector.
10. **Procedural macros** – *not* allowed (violates the “no power‑user macros” rule).

Any future addition must be justified in a design note, recorded as an exception, and accompanied by a full test suite that proves *no* semantics change.

---
## Deliverables

| Item | Description |
|---|---|
| **Formal type‑checking rules** | Small‑step semantics or macro‑free rules covering variables, functions, enums, structs, tuples, moves, and `defer`. |
| **Verified core library** | `yashy‑stdlib` modules (`io`, `string`, `math`) compiled and tested under `-Zformat‑check` (or the project’s equivalent) with 100 % path coverage. |
| **Static verification harness** | A new `crates/yashy‑verify` crate that can (a) parse a Yashy program, (b) build a data‑flow graph, (c) report *unreachable* code, *use‑after‑free*, *double‑free*, *ownership violations*, and *missing `defer`* for every `move`. |
| **Test‑suite expansion** | At least one `tests/**/*.ya` program per core construct that exercises the verifier and asserts *no* warnings/errors when the program is correct, and asserts the appropriate error when it is not. |
| **Documentation** | A `docs/strict-type.md` page that (a) lists the 10 core forms, (b) explains the ownership/borrow model, (c) gives the verifier’s output format, and (d) records any recorded exceptions. |
| **Commit record** | Each landing commit (e.g. `perf: strict‑type‑verifier v1`) recorded in the plan’s “Commit record” table. |

---
## Review focus (likely trouble spots)

| Area | Why it matters | Typical violations |
|---|---|---|
| **Ownership & move** | The single‑owner model is the heart of the “no GC” guarantee. | Forgetting `move` on a value passed to a function, using a moved value, missing `defer` on a heap allocation. |
| **Enum / sum‑type exhaustiveness** | Guarantees the compiler knows all cases. | Non‑exhaustive `match`/`switch`; the verifier should warn. |
| **`defer` / cleanup** | Mirrors Go’s `defer` but must be tracked statically. | Omitted `defer` on a file handle, double `defer` on the same resource. |
| **Result/T error propagation** | Explicit error values, no exceptions. | Ignoring `Result`/`Err`, using `?` in a context that panics. |
| **Tuple vs. struct** | Minimal design space – tuples are just positional products. | Mixing tuple and struct semantics without a clear rule, causing ambiguous type inference. |
| **Integer overflow / wrapping** | Go defines wrapping; Yashy must decide. | Unchecked overflow leading to undefined behaviour. |

---
## TDD / measurement steps

1. **Add the verifier crate** (`cargo add yashy‑verify`) and a single test that a trivial `let x = 1; move x;` passes, while `let x = 1; use x;` (after move) fails.  
2. **Run the full test suite** (`cargo test --workspace --no-fail-fast`) – all must stay green.  
3. **Extend the test‑suite** with one program per core construct (literals, functions, enums, structs, tuples, move, defer, Result). Each must compile and type‑check without warnings.  
4. **Check for regressions** in the three benchmark rows (map_array, debug_format, tuple) – they must not worsen beyond the thresholds already recorded.  
5. **Record a design note** – one paragraph per added core form, explaining why it was kept (or removed) and how the verifier handles it.  
6. **Commit** – `git commit -m "perf: strict‑type‑verifier v1"`; record the hash in the plan’s “Commit record” table.  
7. **Push & CI** – `git push origin slice-1-install-first-cli`; the two benchmark jobs must remain green.

---
## Commit record (to be filled after each landing)

| Commit | Contents |
|---|---|
| `b5fcb11` | Task 10: stable error codes and catalog |
| `cde33d8` | Task 11: schema v1 JSON diagnostics and exit codes |
| `c8ee623` | Task 11 plan record |
| `58b8522` | Task 12: `explain`, docs links, top‑level usage exits |
| `2dffa8c` | Task 12 plan record |
| `4644ec7` | Task 13: AGENTS/llms diagnostics contract |
| `d87d8ff` | Task 13 plan record |
| `984702f` | Windows benchmark compare under `pwsh` |
| `e015b77` | Runner Windows baseline and canonical install paths |
| **<strict‑type‑verifier hash>** | `perf: strict type + verification system (Go‑like design space)` |
| **this commit** | Slice 3b‑plus‑verification acceptance record |

---
## How to use the plan

1. **Copy** this file to `docs/superpowers/plans/2026-10-26-yashy-r1-strict-type-system.md`.  
2. **Add the verifier crate** and implement the 10 core‑form rules as described.  
3. **Run the TDD steps** above; after each step verify that the benchmark gate stays green and that no other row exceeds 2× without an recorded exception.  
4. **When all core forms are verified**, fill the “Commit record” table and mark the plan complete in the master plan.  

This separate plan keeps the design space as minimal as Go’s (only the 10 core forms above) while giving Yashy the *strictest* static verification possible – ownership, move, exhaustive sum types, explicit error handling, and a mechanically checked `defer`/cleanup model – all without a garbage collector or hidden runtime magic.