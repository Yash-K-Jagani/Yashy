# Deprecation

Yashy removes nothing silently. Every removal follows the same four steps.

1. **Name it.** The item or spelling is marked deprecated in the compiler with
   a warning that carries a diagnostic code and a page in `docs/errors.md`. The
   warning names the replacement and the release in which removal is planned.
2. **Keep it working.** For at least two releases the old spelling compiles and
   behaves exactly as before. A deprecation warning never changes generated
   code, exit codes or verification results.
3. **Document the move.** The release note shows the old form and the new form
   side by side, and says what happens to programs that mix them. An edition
   change needs an RFC and a row in the migration table below.
4. **Remove it at an announced version.** Removal happens only after the
   announced version is tagged, and the old code becomes an error with the same
   diagnostic code the warning used.

A warning can be promoted to an error locally with `--deny warnings`; that is a
choice the project makes for its own CI, not a change to the language.

## Migration table

| Old form | Replacement | Deprecated in | Removed in |
|---|---|---|---|
| name-first binding `let x: int = 5` (`E0109`) | type-first `let int x = 5` | 1.0 | not scheduled |

The table grows only through an accepted RFC. A row with "not scheduled" is a
warning about style, not a promise of removal.
