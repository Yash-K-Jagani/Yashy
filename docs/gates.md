# The all-gates runbook

One command runs every gate: `scripts/gates.sh` (Linux/Git bash) or
`scripts/gates.ps1` (PowerShell). Each gate is a command with a budget. The
table says what passed, what failed and how long each took; `--json`/`-Json`
prints the same facts as a document so a nightly job can read them without
parsing text. The exit status is 1 when any gate failed; the runner stops at
the first failure unless `--all`/`--All` is given.

This document collects every gate from every phase plan, its command, its
budget, and its owner. It is the source of truth for "what a green tree
means"; the phase plans keep the per-phase detail (cause, example, fix) and
their own run evidence.

## The runner

`scripts/gates.sh` / `scripts/gates.ps1` drive this set:

| Gate | Command | Budget | Owner |
|---|---|---|---|
| build | `cargo build --release --workspace` | 10m | compiler |
| check | `cargo check --workspace --all-targets` | 5m | compiler |
| tests | `cargo test --workspace --no-fail-fast` | 45m | compiler |
| format | `yashy fmt --check` over `crates/yashy-stdlib/yashy`, `packages`, `examples` | 2m | compiler |
| reproducible | `scripts/repro-check.sh` / `scripts/repro-check.ps1` | 5m | compiler |
| sandbox | `cargo test -p yashy-driver --test sandbox_escape` | 5m | compiler |
| never-panics | `cargo test -p yashy-driver --test never_panics` | 5m | compiler |
| book | `scripts/book-gate.ps1` / `scripts/book-reference.sh` + diff | 10m | docs |

The compiler the gates drive is the release build when it is present,
otherwise the debug build. A gate that needs the compiler is not worth
running after the build failed, so the runner stops at the first failure.

## Every phase gate

Below is the full list, one table per phase, in phase order. The "Passes
when" column is the phase plan's own criterion; the command column is what
the gate runs. Budgets are the runner defaults; a phase plan may tighten or
loosen one.

### Phase 1a — verifier core

| Gate | Command | Passes when |
|---|---|---|
| Ownership rules have one owner | `cargo test -p yashy-verify` | rule tests pass; `crates/yashy-sema/src/ownership.rs` no longer exists; `cargo tree -p yashy-sema` lists `yashy-verify` |
| Build verifies by default | `cargo test -p yashy-driver --test e2e --test examples` | every program builds and runs with no verifier diagnostic |
| Leak and defer corpus | `cargo test -p yashy-driver --test verify` | every seeded bug produces its expected code |
| No false positives on real code | `cargo test -p yashy-driver --test verify` + `--test examples` | zero verifier diagnostics on examples, programs and `file_cli.ya` |
| Contracts warn, never block | `cargo test -p yashy-verify contracts` | unprovable contract is a `Warning`; unknown identifier is `E0001` |
| JSON is additive | `cargo test -p yashy-driver --test diagnostics_json` | unchanged tests pass |
| Timing budget | `yashy check --timings`, `yashy verify --timings` | verify ≤ 30% of check, medians of 5 |
| Escape hatch is traceable | `cargo test -p yashy-driver --test unverified` | all four tests pass |
| Nothing panics | `cargo test -p yashy-driver --test never_panics` | all pass |

### Phase 1b — verifier enforcing

| Gate | Command | Passes when |
|---|---|---|
| Corpus catches every seeded bug | `cargo test -p yashy-driver --test verify` | every expected code set matches exactly |
| Zero false positives | the Task 7 loop | no `FALSE POSITIVE` line, both OSes |
| Contracts enforce | `cargo test -p yashy-driver --test verify` | unproved contract exits 1 |
| Test integration | `cargo test -p yashy-driver --test test_runner` | `compile_error` vs `failed` are distinguished |
| Inventory | `yashy verify --inventory --json` on a program with FFI | every `extern` and `unsafe` site listed |
| Warnings fire | `cargo test -p yashy-driver --test warnings` | all four codes fire on their program and stay silent on their suppression |
| Warnings are warnings | `yashy build` on a program with one unused binding | exits 0, prints one `warning:`, `--deny warnings` exits 1 |
| No panic | `cargo test -p yashy-driver --test never_panics` | passes |
| Workspace | `cargo test --workspace` | green on Windows and Linux |

### Phase 2 — effects and typestate

| Gate | Command | Passes when |
|---|---|---|
| Effect inference is sound | `cargo test -p yashy-verify effects` | all five effect tests pass; recursion terminates |
| Every `unsafe` is declared | `yashy verify --inventory` on the corpus | no `E1301`-free unsafe call site |
| Typestate | `cargo test -p yashy-verify typestate` | double-close, use-after-close, bad unlock rejected |
| Races | `cargo test -p yashy-verify race` | bare-pointer capture rejected; mutex/string accepted |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No false positives | the corpus loop | none on examples, programs, `file_cli.ya` |

### Phase 3 — speed foundation

| Gate | Command | Passes when |
|---|---|---|
| Incremental rebuild | `yashy build` twice on a 1000-line program | second run < 200 ms |
| 100 tests | `yashy test` on a 100-test project | < 2 s |
| Hello | `yashy build benchmarks/hello.ya` | < 1 s, binary < 500 KB |
| Runtime | `yashy-bench` | scalar ≤ 1.2× C, feature rows ≤ 2× |
| Compiler rebuild | touch `sema`, `cargo build -p yashy-driver` | < 60 s median |
| Cache correctness | `cargo test -p yashy-cache` + `yashy cache verify` | key changes on every input that matters |
| No check lost without a proof | `cargo test -p yashy-codegen` | unproven spans keep their checks |
| Async spike | the design doc exists and passes its own cap review | four decisions answered |

### Phase 3b — native foundation

| Gate | Command | Passes when |
|---|---|---|
| Bitwise total | `cargo test -p yashy-driver --test e2e` | `bitwise.ya` covers all six operators on all integer types, both signednesses of `>>`, and the precedence table |
| No implicit widening | `cargo test -p yashy-sema` | `u8 & i64` is `E0104`; `~float` is `E0103` |
| Shifts checked | `cargo test -p yashy-codegen -p yashy-driver --test e2e` | `1 << 64` panics at the operator; `const BAD = 1 << 64` is the same panic at compile time |
| A proof removes the check | `cargo test -p yashy-verify --test verify` | the guarded shift has no `icmp`; the unguarded one does |
| `link_name` | `cargo test -p yashy-driver --test e2e` | `sqrt` through `link_name` is bit-identical to the hardware result; `E0900`/`E0901` fire |
| Variadic | `cargo test -p yashy-driver --test e2e` | `printf("%d\n", 42 as i32)` prints `42`; `E0902` fires on the three misplacements |
| C layout matches C | `cargo test -p yashy-driver --test e2e` | `sizeof`/`alignof`/`offsetof` equal a C program's, on Windows and Linux |
| `repr(C)` rules | `cargo test -p yashy-driver --test verify` | a payload enum is `E0106`; `offsetof` on a plain struct is `E0108`; a folded query leaves no obligation |
| Bytes are bytes | `cargo test -p yashy-driver --test e2e --test leaks` | a 1 MiB round-trip is byte-identical; no leak with `.free()`, one without |
| Failure is a value | `cargo test --workspace` | no builtin returns `""`/`0` on failure; `read_line` distinguishes EOF from an empty line |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 4 — optimization

| Gate | Command | Passes when |
|---|---|---|
| Incremental analysis | `cargo test -p yashy-query --test incremental` + `--timings` | unchanged project: `query_misses == 0`; one-function edit re-analyses that function and its dependents only |
| Parallel and identical | `cargo test -p yashy-driver --test parallel_determinism` | `frontend_threads > 1`; `--json` byte-identical to `--jobs 1` |
| Attributes only where proven | `cargo test -p yashy-codegen --test attributes` | every attribute traced to a fact; no facts ⇒ no attributes |
| Escape promotion sound | `cargo test -p yashy-codegen --test escape` + ASan/UBSan corpus | no promoted value is reachable after its scope; corpus clean |
| Allocation and size | `yashy-bench` rows | `wordcount` allocations −40%; `hello` binary within the phase 3 budget |
| Runtime | `yashy-bench` on both OSes | `fib` ≤ 1.1× C; feature rows ≤ 1.8×; release ≥ 5% faster with PGO |
| Per-pass gate | `yashy-bench --per-pass` | no pass regresses its own row by more than 2% |
| Explained | `cargo test -p yashy-driver --test opt_report` | every surviving check and allocation has a reason |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 5 — syntax ergonomics

| Gate | Command | Passes when |
|---|---|---|
| Type-first, no second spelling | `cargo test -p yashy-parser --test declarations` + `yashy check` | every type shape parses before its name; `let x: int`, `fn f(a: int)` and `|x: int|` are `E0109`; no tracked source uses name-first |
| Migration complete | `cargo test --workspace` + tutorial + examples | every `.ya` file and every ` ```yashy ` block is type-first and compiles |
| Sugar adds no concept | `cargo test -p yashy-sema --test desugar` | every form's HIR equals its hand-written expansion |
| No new keywords | `cargo test -p yashy-lexer` | `KEYWORDS.len() == 34` |
| `?` is not inverted | `cargo test -p yashy-driver --test e2e` + `-p yashy-codegen --test option_try` | `option_try.ya`: `None` returns early; `Some` continues with the payload; tag 1 for `Option`, 0 for `Result`; mixing is `E0301` |
| Middle operand evaluated once | `cargo test -p yashy-driver --test e2e` | `chained_cmp.ya`: the counter in the middle is 1, operands evaluated in source order |
| Spread moves | `cargo test -p yashy-driver --test e2e` + corpus | partial move `E1002`; duplicate field `E0202` |
| Arena sound | `cargo test -p yashy-driver --test e2e arena` + leak suite | escape `E1003`, individual free `E1004`, no leak on `break` |
| Labels exit the right loop | `cargo test -p yashy-driver --test e2e` | `labeled_loops.ya`: `break outer` exits both; unlabelled `break` exits one |
| Doc comments are inert | `cargo test -p yashy-driver --test e2e` | `doc_comments.ya` output identical with/without blocks; `E2100` on misplaced |
| Formatter | `cargo test -p yashy-fmt` | every new form round-trips byte-identically |
| No false positives | the phase 1b loop | no `FALSE POSITIVE` line |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 6 — modules and unicode

| Gate | Command | Passes when |
|---|---|---|
| Large-project import test | `cargo test -p yashy-driver --test modules` | 50-module fixture resolves, cycles rejected once |
| Visibility | `cargo test -p yashy-sema` | private cross-module use rejected |
| Unicode conformance | `cargo test -p yashy-runtime --test grapheme_conformance` | every vendored case passes or is explicitly excluded |
| Bytes unchanged | `cargo test -p yashy-driver --test examples` | `wordcount` still counts bytes identically |
| Verifier coverage | the zero-false-positive loop | no new false positive on strings |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 7 — patterns and named arguments

| Gate | Command | Passes when |
|---|---|---|
| Exhaustiveness still proven | `cargo test -p yashy-sema -p yashy-verify --test exhaustive` | or/range/guard/nested cases pass; a guarded arm never counts as covering |
| Every pattern form has ownership rules | `cargo test -p yashy-verify` | partial-move corpus 100% caught |
| Named arguments are positional in the HIR | `cargo test -p yashy-sema` | two spellings produce equal HIR |
| No evaluation reordering | `cargo test -p yashy-driver --test e2e` | side-effect order is source order |
| Formatter | `cargo test -p yashy-fmt` | every new form round-trips |
| No false positives | the phase 1b loop | no `FALSE POSITIVE` line |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 8 — traits, OOP, iterators

| Gate | Command | Passes when |
|---|---|---|
| Dispatch proven safe | `cargo test -p yashy-sema` | no dynamic target exists; every trait call names one impl |
| Coherence | `cargo test -p yashy-sema` | duplicate impl `E1501`, incomplete impl `E1502`, user generic trait `E1511` |
| Bounds | `cargo test -p yashy-sema` | unsatisfied bound `E1503`, ambiguity `E1504` |
| Generic types instantiate once | `cargo test -p yashy-codegen -p yashy-driver --test e2e` | `generic_types.ya`: `Pair[i64]` written twice emits one `type`; `Pair[i64]` and `Pair[string]` emit two |
| Generic type arity and parameters | `cargo test -p yashy-driver --test verify` | wrong arity `E1509`; unused type parameter `E1510`; generic type inside a generic function monomorphises per argument |
| Associated functions | `cargo test -p yashy-driver --test e2e` | `associated_fns.ya`: `Type::name(..)` and `Type.name(..)` resolve; `E1506`/`E1507`/`E1508` fire on the three wrong forms |
| Member visibility | `cargo test -p yashy-driver --test e2e` | `visibility.ya`: private field/method across modules is `E0801`; sibling types may read it |
| Object model conformance | `cargo test -p yashy-driver --test verify --test e2e --test never_panics` | the conformance transcript matches; `E1512`/`E1513`/`E1514`/`E1515` reject the four excluded forms |
| Monomorphisation within budget | `cargo test -p yashy-bench` + `--timings` | count ≤ caps on the corpus; `E1505` fires at 65 |
| Iterators unchanged | `cargo test -p yashy-driver --test e2e` | every existing `for` produces identical output |
| Verifier covers traits | `cargo test -p yashy-verify` + corpus loop | trait effect/ownership/typestate rules hold; no false positive |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 9 — macros

| Gate | Command | Passes when |
|---|---|---|
| Verifier runs on expanded code | `cargo test -p yashy-driver --test verify` | three macro seeded bugs caught, spans in user code |
| Deterministic, inspectable expansion | `cargo test -p yashy-driver --test expand` | two runs equal; `--json` lists every expansion in source order |
| Hygiene both directions | `cargo test -p yashy-expand` | template capture and caller capture both correct |
| Caps enforced | `cargo test -p yashy-expand` | depth 65 is `E1601`, not a crash |
| No panic | `cargo test -p yashy-driver --test never_panics` | hostile corpus produces diagnostics only |
| No false positives | the phase 1b loop | no `FALSE POSITIVE` line |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 10 — async

| Gate | Command | Passes when |
|---|---|---|
| Conformance | `cargo test -p yashy-driver --test async_conformance` | 30 programs pass under both executor modes, identical output |
| No data races proven | `cargo test -p yashy-verify` | every bare linear capture is `E1402`; mutex/channel accepted |
| `async` effect enforced | `cargo test -p yashy-verify` | `E1700` outside async; async call from sync is `E1300` |
| Structured | `cargo test -p yashy-verify` | task escape `E1701`, lock across await `E1702` |
| Cancellation releases | the cancel-with-defer program | OS handle count returns to baseline |
| Frame correctness | `cargo test -p yashy-codegen` | 8 live locals survive a thread handoff |
| Debuggable | `yashy run --debug` demo | task id, function and resume index printed |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 11 — AI-native observability

| Gate | Command | Passes when |
|---|---|---|
| Context ≤ 8k tokens | `cargo test -p yashy-driver --test context` | every pack reports `tokens ≤ budget`; drops are named |
| Escape tests | `cargo test -p yashy-driver --test sandbox_escape` | all 10 attempts denied with the right code, both OSes |
| MCP is compiler-only | `cargo test -p yashy-driver --test mcp` | seven tools, traversal refused, malformed request survived |
| Schema v1 frozen | `cargo test -p yashy-driver --test schema_diff` | no removal, no type change |
| Benchmark runs | `yashy-bench agent --tasks benchmarks/agent` | 10 tasks + variants, five metrics recorded |
| Determinism | `cargo test -p yashy-driver --test e2e observability` | byte-identical stderr across runs |
| `doc` covers the surface | `cargo test -p yashy-driver --test doc` | every public item appears with its signature; output byte-identical on both OSes and across runs |
| `lsp` starts a server | `cargo test -p yashy-driver --test lsp` | `initialize` answered through `yashy lsp`; a missing sibling exits 2 with the path named |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 12 — libraries and packages

| Gate | Command | Passes when |
|---|---|---|
| Clean-checkout dependency build | fresh clone, empty cache, `yashy build` on `examples/http-service` | succeeds; lockfile unchanged after |
| Real HTTP service | `cargo test -p yashy-driver --test examples http-service` | keep-alive, chunked, caps, timeouts and 50 concurrent clients pass |
| Reproducible | two builds from the same lockfile | byte-identical binaries |
| Precompiled stdlib equivalent | `cargo test -p yashy-driver --test stdlib_precompiled` | cached and from-source images differ only in the PE `TimeDateStamp`; `ir_cache=hit` with `emit 0 ms` |
| Offline | `yashy build --offline` with a warm store | succeeds; with a cold store, `E1803` names the package |
| Audit green | `yashy audit` on the repo's own packages | `ok: true` |
| Stdlib is a product | `cargo test -p yashy-driver --test stdlib_coverage` | every module in `crates/yashy-stdlib/yashy/` has a documented `pub fn` and a test program |
| No sentinel on failure | `cargo test -p yashy-driver --test stdlib_coverage` | no `-> string`/`-> int` stdlib function documents "or the empty string"/"or zero" as its failure value |
| Stdlib is deterministic | `cargo test -p yashy-driver --test e2e` | two runs of every `tests/programs/stdlib/` program produce identical bytes |
| Registry round-trip | `cargo test -p yashy-driver --test registry` | publish → add → build passes |
| No regression | `cargo test --workspace` | green on Windows and Linux |

### Phase 13 — hardening

| Gate | Command | Passes when |
|---|---|---|
| macOS tested | `ci.yml`'s `test` and `install-smoke` on `macos-15` | the workspace suite passes; the packaged archive installs, `doctor` passes, the first project runs and chapters 00-02 pass |
| macOS supported (signed) | `release.yml`'s `macos-install`, then `install-matrix` on `macos-15` | signed, notarised, stapled; Gatekeeper accepts it; `doctor` and the first three chapters pass |
| Reproducible | `scripts/repro-check` | two independent builds, identical SHA-256, all three OSes |
| Signed releases | `release.yml` dry run | checksums verify; every artifact signed |
| Install matrix | `install-matrix.yml` | six installs from published artifacts, all green |
| Book | `book.yml` | every code block compiles; reference generated from `yashy index` |
| Playground safe | `cargo test -p yashy-driver --test playground` | phase 11 escape corpus denied through `/run` |
| Registry | `cargo test -p yashy-driver --test registry` | hosted index add builds; tampered tarball rejected |
| Self-hosting evidence | `cargo test -p yashy-driver --test selfhost_diff` | pass rate recorded; design record written |
| All gates | `scripts/gates` | every phase gate green, or a dated exception |
| No P0 | `docs/triage.md` | P0 list empty; crash corpus green |
| CI fails on unformatted source | `cargo test -p yashy-driver --test fmt_check` | `fmt --check` exits 1 on a misformatted file, 0 on the repository, and agrees with `fmt -w` |
| Cross-compilation | `yashy build --target <triple>` on each of the six triples | the binary runs on that platform; `E2200`/`E2201` name the unsupported triple and the missing toolchain |
| Platform conditionals | `cargo test -p yashy-driver --test cfg` | exactly one branch is selected; a disabled branch is never resolved; `cfg(window)` is `E2202` |
| Metrics met | `docs/1.0.md` | every number in the release note reproduced by the gate run |

**Open exceptions (2026-11-13).** Three release gates cannot be met from this
checkout: the repository has **zero Actions secrets**
(`GET /repos/Yash-K-Jagani/Yashy/actions/secrets` → `total_count: 0`), and it
has never had a Developer ID identity or a notary profile. macOS *testing* is
no longer one of them: `ci.yml` runs the workspace suite on `macos-15`, and
its install smoke packages the arm64 archive, writes `SHA256SUMS` and drives
`install.sh`, `doctor`, the first project and tutorial chapters 00-02 through
`test-install.sh --unsigned`. The runner label is `macos-15` because GitHub
retired the macOS 14 images on 2026-11-02 — a job on a retired label is
cancelled before its first step. Each open gate is recorded with the date, the
reason, the owner and the condition that closes it. **Until they close, phase
13 is not marked complete** — and macOS stays unclaimed in `README.md` and
`AGENTS.md`, as the phase's global constraint requires: an unsigned archive is
tested, not shipped.

| Gate | Reason it cannot run (2026-11-13) | Owner | Closes when | Re-run |
|---|---|---|---|---|
| macOS supported with a signed archive (`release.yml`'s `macos-install`) | No Developer ID certificate (`MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`) and no notary profile (`MACOS_NOTARY_PROFILE_BASE64`); the workflow stops at "Check the signing secrets" | release manager | The macOS secrets exist and `release.yml`'s `macos-install` job runs on `macos-15` | on the next tagged release |
| Signed releases (`release.yml` dry run) | Authenticode (`WINDOWS_CERT_PFX_BASE64`, `WINDOWS_CERT_PASSWORD`) and GPG (`RELEASE_GPG_PRIVATE_KEY`, `RELEASE_GPG_PASSPHRASE`) are both missing; the workflow stops at "Check the signing secrets", naming the missing one | release manager | The Windows, macOS and GPG secrets exist | `v1.0.0-rc2` |
| Install matrix from published artifacts | Depends on a published release; the only tagged release (`v1.0.0`) never published because its draft was cancelled after the signature check failed | release manager | A signed release is published, then `install-matrix.yml` runs from it | weekly, and on the next release |

The Linux, Windows and macOS halves of every other gate are a **delegation**,
not an exception: CI runs the same commands on `ubuntu-latest`,
`windows-latest` and `macos-15` (see below), and the phase's Windows evidence
is also observed on the development host.

## Running the runner

```sh
# Linux / Git bash
scripts/gates.sh           # stop at first failure
scripts/gates.sh --all     # run every gate, report all results
scripts/gates.sh --json    # machine-readable summary
```

```powershell
# PowerShell (pwsh, or Windows PowerShell 5.1 with powershell -File)
pwsh scripts/gates.ps1            # stop at first failure
pwsh scripts/gates.ps1 -All       # run every gate, report all results
pwsh scripts/gates.ps1 -Json      # machine-readable summary
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\gates.ps1 -All   # without pwsh
```

## What "green" means

Every gate in its phase table passes, or every open gate has a **dated
exception** recorded in this file. A gate that cannot be met is recorded as
an exception with the date, the reason, and the owner; the phase cannot be
marked complete while an exception is open.

"On Windows and Linux" is observed where the development host allows it;
CI runs the same commands on `ubuntu-latest` and `windows-latest`, and on
`macos-15` for the workspace suite and the unsigned clean-install acceptance,
which is a delegation, not an exception.
