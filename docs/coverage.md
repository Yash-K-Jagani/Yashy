# Coverage

Source-level coverage is **not implemented** in 2.0 yet. This page records why,
so the gap is not mistaken for a flag that exists.

The intended path was clang's `-fprofile-instr-generate -fcoverage-mapping`,
run over the test suite and merged into lcov and JSON with a `--fail-under`
gate. Coverage mapping needs a source mapping in the LLVM IR: the backend
would have to attach `!dbg` locations from HIR spans to instructions. The
current code generator emits no debug metadata at all, so a coverage-instrumented
build would produce profile data that cannot be attributed to a Yashy source
line.

Until that lands:

- there is no `yashyc coverage` verb; asking for it is a usage error;
- `yashyc test` reports which test programs passed, not which source lines ran;
- debug builds now emit a `!dbg` location per checked statement, so the
  metadata prerequisite is met; what remains is the instrumentation, profile
  merge and lcov/JSON rendering pipeline;
- `docs/debugging.md` tracks the parallel gap for typed locals and values.

The work order is therefore: emit `!dbg` line locations from HIR spans, then
wire coverage mapping and the source-aware debugger on top of the same fact.
Implementing one without the other would create a second, drifting notion of a
source position.
