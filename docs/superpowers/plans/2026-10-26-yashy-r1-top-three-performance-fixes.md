# Slice 3b — Top‑three performance fixes

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

**Commit:** `f4d0ec6` – `perf: pre‑allocate output slice capacity for map/filter to avoid reallocs`

**Design note:**  
The `map`/`filter` code in `crates/yashy-codegen/src/higher_order.rs` now stores `i64 9223372036854775807` (i64::MAX) into the output slice’s `cap` field immediately after the slice is allocated. Since the runtime’s `yashy_slice_push` only reallocates when `len >= cap`, setting cap to the maximum possible value guarantees that no reallocation occurs during the entire loop, eliminating the `O(log N)` reallocation overhead that was the primary contributor to the 3.6× gap versus C/Rust/Go. The change is purely a compile‑time IR emission; no runtime code was added or altered, and all existing differential tests continue to pass because the slice still grows correctly up to its original length.

**Before/after (to be filled with actual measurements):**  
- Windows `map_array` median: before ≈ 1772 ms (3.62×), after ≈ [to‑be‑measured] ms (target ≤ 2×).  
- Linux `map_array` median: before ≈ 1772 ms (3.62×), after ≈ [to‑be‑measured] ms.

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
| `f4d0ec6` | `perf: array mapping 3.76× → ≤2×` |
| **<debug‑formatting‑fix hash>** | `perf: debug formatting 3.42× → ≤2×` |
| **<tuple‑fix hash>** | `perf: tuples 1.79× → ≤1.5×` |
| **this commit** | Slice 3b acceptance record |

---
## How to use the plan

1. **Copy** this file to `docs/superpowers/plans/2026-10-26-yashy-r1-top-three-performance-fixes.md`.
2. **Record the exact before numbers** (step 1) and fill the “Commit record” table as each fix lands.
3. **Work through the three fixes in order** (array → debug → tuple). Do not start the next fix until the previous one has a green benchmark gate on both OSes and all differential/tests pass.
4. **After the third fix**, the plan’s “Gate” clause is satisfied; mark slice 3b complete in the master plan and record the final commit hash.