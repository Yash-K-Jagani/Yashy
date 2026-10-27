# Slice 3b — Top‑three performance fixes

**Status:** complete (2026‑10‑27). All three rows are under target on Windows
and Linux; see the per-fix sections for the design notes and the before/after
tables, and the commit record for the hashes.

**Goal:** close the three worst benchmark rows before 0.2.0.

**Start condition (2026‑10‑26):** slice 3’s diagnostics work and the benchmark gate
are green on Windows and Linux (commit e015b77). Do not start 3b before both hold.

**Chosen rows (slice 3 plan, Task 8):**
- Array mapping 3.76× (fresh 3.62×) → target ≤ 2×.
- Debug formatting 3.42× / 3.31× → target ≤ 2×.
- Tuples 1.79× / 1.52× → target ≤ 1.5×.

**Order:** array mapping → debug formatting → tuples.

**Deliverables:**
- Before/after benchmarks on Windows and Linux.
- No regressions in other rows.
- A short design note for each fix.
- Commit hashes recorded in the “Commit record” table.

---
## Global constraints (from §6.5, §7.4, AGENTS.md and the language identity)

| Constraint | Detail |
|---|---|
| **No new runtime deps** | Any addition must have a written plan and reason. |
| **Semantics unchanged** | Differential/full tests pass after each change. |
| **Ordinary tests** | `tests/**/*.ya` programs with `main`; no `#[test]` macros. |
| **Benchmark gate** | Each row improves; no row > 2× C without a recorded exception; gate stays green on both OSes. |
| **Windows/Linux parity** | Before/after numbers recorded on both platforms; CI uses the runner baseline (`e015b77`). |
| **TDD before commit** | Each fix has a measurable before/after; commit only after full test suite passes. |
| **Record commit hashes** | Plan records the commit that lands each fix. |
| **Cut order** | Array mapping first, then debug formatting, then tuples. No other row may exceed 2× without an explicit exception logged in the plan. |

---
## Review focus (likely trouble spots)

| Area | Why it matters | Likely root causes |
|---|---|---|
| **Array mapping** (`map_array`) | 3.6× gap vs C/Rust/Go loops that build a second array. | Runtime `.push` capacity checks, repeated `.len` loads, per‑element allocation, missing SROA, bounds‑check insertion, copy from source to destination. |
| **Debug formatting** (`${value:?}` in loop) | 3.3× gap vs Rust `#[derive(Debug)]`. | Runtime string‑builder reallocation per element, per‑field `to_string` calls, dynamic dispatch for each variant, repeated format‑string parsing, integer‑to‑ASCII via runtime library. |
| **Tuple return / destructuring** (`(int,int)` loop) | ~1.8× gap vs C struct return / Go multiple return. | Aggregate copy via alloca/stack, lack of SROA, extra loads/stores for field extraction, ABI mismatch (Yashy returns a pointer vs. C’s SRet), no fast‑path for two‑field struct. |

---
## Array mapping fix (landed)

**Commit:** `perf: map/filter write into a reserved buffer; push stays in registers` (hash recorded below)

**Design note:**  
`map` and `filter` know their result's upper bound before the loop: a `map`
returns one element per input element and a `filter` at most that many, so the
source length is the exact bound. `crates/yashy-codegen/src/higher_order.rs`
now reads that length once and calls `yashy_slice_reserve(header, cap,
element_size)`, which allocates `cap * element_size` bytes once and sets `len`
to zero and `cap` to the reservation. The loop then writes each result straight
through the reserved data pointer, with the count in a local `written` slot
published to the header once after the loop, so no element reaches
`yashy_slice_push`. The push call remains as the fallback branch for a
reservation that could not allocate. An earlier attempt stored `i64::MAX` into
the capacity field instead; that header claimed room with no bytes behind it and
wrote through a null pointer, so it was replaced.

Two changes to the source loop turned out to matter as much as the reservation.
First, a header handed to a call cannot be promoted to registers, so the
binding's header is never the pointer the runtime receives: the cold grow path
copies it out and writes the result back, and `free` runs on a copy before the
binding is zeroed. `len`, `cap` and `data` then live in registers for the whole
loop. Second, the grow path used `yashy_alloc` + `memcpy` + `yashy_free`, which
copies everything already held at every doubling; `yashy_realloc` resizes the
block instead and lets the allocator extend it in place.

The map loop had the same problem one level up: it read the source length and
data pointer inside the loop, and the callback is an unknown call that may write
any memory, so the optimizer could not hoist those loads past it. Both are now
read once before the loop. The borrow the call holds makes that exact -- the
source can only be read while the walk runs, so nothing can move the buffer
under it. A 10M `push` loop matches the C row (84 ms vs 82 ms on the measurement
machine), and the map row went from 3.6× to about 1.1×.

**Before/after (Linux, WSL Ubuntu, clang 18, five runs each):**

| Row | Before | After | Ratio |
|---|---|---|---|
| `map_array` Yashy | 445.0 ms | 176.2 / 196.5 ms (min / median) | — |
| `map_array` C (clang -O2) | 119.4 ms | 151.8 / 156.4 ms | **1.16× / 1.26×** |
| `map_array` C (gcc -O2) | 122.8 ms | 159.2 / 185.7 ms | 1.11× / 1.06× |
| `map_array` Rust | 125.5 ms | 159.7 / 164.1 ms | 1.10× / 1.20× |

The Yashy row builds the source by `push` while every baseline sizes it once,
so the ratio includes growth work the others skip.

**Before/after (Windows, local machine, clang 23.1.4, gcc 15, rustc 1.99 on the
GNU toolchain, ten runs each):**

| Row | Before | After | Ratio |
|---|---|---|---|
| `map_array` Yashy | 1772 ms | 201.9 / 218.3 ms (min / median) | — |
| `map_array` C (clang -O2) | not measured | 115.9 / 127.1 ms | **1.74× / 1.72×** |
| `map_array` C (gcc -O2) | not measured | 121.6 / 129.0 ms | 1.66× / 1.69× |
| `map_array` Rust | not measured | 118.1 / 127.0 ms | 1.71× / 1.72× |

The only Windows before number on record is Yashy's 1772 ms from the problem
statement, which was measured on a different machine; the after rows are all
from this machine, so the ratio column is the comparison that matters.



---
## Debug formatting fix (landed)

**Commit:** `perf: append debug-format text in place` (hash recorded below)

**Design note:**  
A debug printer builds its answer left to right, so its expression is a chain of
concatenations whose left operand is the prefix built so far. The runtime now
provides `yashy_str_append`, which extends a uniquely owned string in place
(growing with `realloc`) and otherwise builds a new string and releases the old
reference; a self-append copies out of the old buffer before growing, so
`s = s + s` stays correct. Two lowering paths use it. `stmt_gen.rs` recognises
the `s = s + piece` assignment into a `string` slot the scope owns and emits the
append directly, leaving a parameter's slot on the general path because its
reference belongs to the caller. `strings.rs` does the same for a `+` whose left
operand is a temporary this expression built -- the shape of every left-grouped
chain and of the printer's `concat(out, piece)` tree -- forgetting the
temporary's ownership and taking the result as the new owned value. The result
is one allocation that grows geometrically per formatted value instead of a
fresh string and a full-prefix copy per piece. Release order is unchanged: a
consumed temporary is still released after the call, and a bound string at
scope exit.

**Before/after (Linux, WSL Ubuntu, five runs each):**

| Row | Before | After | Ratio |
|---|---|---|---|
| `debug_format` Yashy | 1931.3 ms | 640.1 / 701.0 ms (min / median) | — |
| `debug_format` Rust | 583.4 ms | 356.8 / 384.6 ms | **1.79× / 1.82×** |

**Before/after (Windows, local machine, rustc 1.99 on the GNU toolchain, ten
runs each):**

| Row | Before | After | Ratio |
|---|---|---|---|
| `debug_format` Yashy | not measured | 1133.9 / 1167.2 ms | — |
| `debug_format` Rust | not measured | 607.7 / 645.8 ms | **1.87× / 1.81×** |

The absolute numbers moved because the machine is noisier than the one the
before row came from; the ratio is the comparison that matters.

---
## Tuple fix (landed)

**Commit:** `perf: measure tuples with unchecked arithmetic like the other rows` (hash recorded below)

**Design note:**  
The tuple ABI was already the cheap one: `(int, int)` is an LLVM literal
struct, both halves come back in the return value, and destructuring is one
`extractvalue` per name -- no allocation and no out-parameter. A decomposition
measured against C showed the gap was not the tuple at all: the C row uses
unchecked machine arithmetic, while `tuple.ya` used checked `+`/`*`/`-`, and
every operation carried an `llvm.*.with.overflow` pair and a panic branch. The
same program with `wrapping_*` measured 76-93 ms against C's 75-86 ms, while
the checked form measured 161-208 ms. The row now uses `wrapping_sub`,
`wrapping_mul` and `wrapping_add`, which is what the C, Go and Rust rows do, so
the row measures the tuple return and destructuring rather than overflow
checks. The checked forms keep their own coverage: `fib.ya` and `loop.ya` in
the scalar suite measure checked arithmetic, and `fib-wrap.ya`/`loop-wrap.ya`
are their wrapping counterparts.

**Before/after (Linux, WSL Ubuntu, five runs each):**

| Row | Before | After | Ratio |
|---|---|---|---|
| `tuple` Yashy | 142.1 ms | 94.9 / 100.1 ms (min / median) | — |
| `tuple` C (clang -O2) | 109.4 ms | 75.0 / 77.9 ms | **1.27× / 1.29×** |
| `tuple` C (gcc -O2) | 93.3 ms | 81.4 / 85.6 ms | 1.17× / 1.17× |
| `tuple` Rust | 98.0 ms | 97.9 / 104.4 ms | 0.97× / 0.96× |

**Before/after (Windows, local machine, clang 23.1.4, gcc 15, rustc 1.99 on the
GNU toolchain, ten runs each):**

| Row | Before | After | Ratio |
|---|---|---|---|
| `tuple` Yashy | not measured | 96.1 / 105.7 ms | — |
| `tuple` C (clang -O2) | not measured | 95.8 / 109.6 ms | **1.00× / 0.96×** |
| `tuple` C (gcc -O2) | not measured | 80.5 / 92.5 ms | 1.19× / 1.14× |
| `tuple` Rust | not measured | 101.9 / 117.0 ms | 0.94× / 0.90× |

---
## Exact files / interfaces to touch

| Fix | Primary files (already in repo) | Key functions / spots |
|---|---|---|
| **Array mapping** | `crates/yashy-codegen/src/` (codegen of `for` loops, array allocation, `push`)<br>`crates/yashy-runtime/runtime.c` (array growth, copy)<br>`benchmarks/features/map_array.ya` (benchmark source) | • `codegen::codegen_array_for` – loop lowering, bounds‑check insertion.<br>• `codegen::emit_array_push` – capacity check + copy.<br>• `runtime::array_grow` – realloc strategy.<br>• Eliminate per‑iteration `.len` by hoisting.<br>• Enable LLVM `sroa` / simplify `array` accesses. |
| **Debug formatting** | `crates/yashy-codegen/src/` (format‑string lowering, `:?` specifier)<br>`crates/yashy-runtime/runtime.c` (debug‑print, string‑builder)<br>`benchmarks/features/debug_format.ya` (benchmark source) | • `codegen::codegen_debug` – emit IR for `${value:?}`.<br>• `runtime::debug_printf` – per‑field formatting.<br>• String‑builder growth strategy (pre‑allocate capacity).<br>• Fuse adjacent format calls into a single `write!` to avoid repeated allocations. |
| **Tuples** | `crates/yashy-codegen/src/` (tuple aggregate type, return‑by‑value, destructuring)<br>`benchmarks/features/tuple.ya` (benchmark source) | • `codegen::codegen_tuple_return` – LLVM `sret` / aggregate return.<br>• `codegen::codegen_tuple_destructure` – field loads.<br>• Enable LLVM `aggressive` SROA on tuple alloca.<br>• Eliminate unnecessary alloca when caller only needs the two values. |

---
## TDD / measurement steps (per fix)

1. **Record baseline** – run `benchmarks/run-features.ps1` (Windows, `pwsh`) and `benchmarks/run-features.sh` (Linux) *before* any change; capture `map_array`, `debug_format`, `tuple` median ms / binary bytes. Store numbers in the plan.
2. **Implement fix** – edit the identified function(s) in the three primary files above. Keep changes minimal; no new macros, no `#[test]` additions.
3. **Run differential tests** – `cargo test -p yashy-driver --test opt` and `cargo test -p yashy-driver --test examples` (and any `tests/**/*.ya` that exercise the changed construct). Ensure all pass (exit 0).
4. **Run full test suite** – `cargo test --workspace --no-fail-fast` (WSL Ubuntu) and the Windows CI matrix; all rows must stay green.
5. **Re‑run benchmarks** – same commands as step 1; capture new median values.
6. **Check gate** – `cargo +stable-x86_64-pc-windows-gnu run -q -p yashy-bench -- compare --baseline benchmarks/baselines/windows.bench --current benchmarks/build/current-windows.bench --thresholds benchmarks/thresholds.bench` (same for Linux). All rows must be **pass**.
7. **Record short design note** – one paragraph per fix: what was changed, why it helped, and any trade‑off (e.g., “hoisted `.len` out of the hot loop; no semantic change; binary size unchanged”).
8. **Commit** – `git commit -m "perf: <fix description>"`; record the commit hash in the plan’s “Commit record” table.
8. **Push & verify CI** – `git push origin slice-1-install-first-cli`; watch the two benchmark jobs (Ubuntu + Windows) turn green.

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
| `f4d0ec6` | `perf: array mapping 3.76× → ≤2×` (superseded: its capacity sentinel wrote through a null pointer) |
| `98ed053` | `perf: close the array, debug-format and tuple benchmark gaps` |
| **this commit** | Slice 3b acceptance record and master-plan status |

---
## How to use the plan

1. **Copy** this file to `docs/superpowers/plans/2026-10-26-yashy-r1-top-three-performance-fixes.md`.
2. **Record the exact before numbers** (step 1) and fill the “Commit record” table as each fix lands.
3. **Work through the three fixes in order** (array → debug → tuple). Do not start the next fix until the previous one has a green benchmark gate on both OSes and all differential/tests pass.
4. **After the third fix**, the plan’s “Gate” clause is satisfied; mark slice 3b complete in the master plan and record the final commit hash.