# Language reference

This directory is the normative reference for the 2.0 language. Each page
states what the compiler guarantees, names the rule id for each normative
sentence and links the diagnostic that enforces it. The [error catalog](../errors.md)
is the alphabetical index of codes; this is the rule-oriented reading of the
same facts.

- [Memory model](memory-model.md) — allocation, zeroing, pointers and the
  absence of `Drop`.
- [Ownership](ownership.md) — moves, `move`, `defer`, `.free()` and arenas.
- [Aliasing](aliasing.md) — the owning/FFI split and the view-source rule.
- [Determinism](determinism.md) — what a pure program may and may not observe.
- [Layout](layout.md) — `repr` clauses, alignment and the layout table.
- [Effects and async](effects.md) — effect declarations, `unsafe` and tasks.

A rule that has no diagnostic yet is listed as unenforced on the relevant page
and reported by `yashyc trace --fail-orphans`. The reference never claims a rule
is checked when it is only written down. The [traceability matrix](../traceability.md)
is the generated index from rule to code to test.

The grammar, item and type chapters are still being written; see the
[reference task](../superpowers/plans/2026-11-15-phase-14-selfhost-spec-release.md)
for what remains.
