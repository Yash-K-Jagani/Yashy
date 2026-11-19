# Rule ids

Generated from `codes::rule_for` and the error catalog. Edit the registry, never this page; `UPDATE_GOLDEN=1` regenerates it.

Every catalog code names one rule. Rules the design names without a code yet are listed at the end.

| Rule | Code | Meaning |
|---|---|---|
| `ALIAS-1` | [E1005](errors/E1005.md) | owning pointer copied without move |
| `ALIAS-3` | [E1403](errors/E1403.md) | unproved view source |
| `ALIAS-5` | [E1404](errors/E1404.md) | assumption not reported |
| `ASYNC-1` | [E1700](errors/E1700.md) | await or spawn outside an async function |
| `ASYNC-2` | [E1701](errors/E1701.md) | task handle escapes its scope |
| `ASYNC-3` | [E1702](errors/E1702.md) | resource held across an await |
| `ASYNC-4` | [E1703](errors/E1703.md) | async method in a trait |
| `CALL-1` | [E0700](errors/E0700.md) | wrong argument count |
| `CALL-2` | [E0701](errors/E0701.md) | unknown named argument |
| `CALL-3` | [E0702](errors/E0702.md) | duplicate argument |
| `CFG-1` | [E2200](errors/E2200.md) | unsupported target |
| `CFG-2` | [E2201](errors/E2201.md) | target C toolchain missing |
| `CFG-3` | [E2202](errors/E2202.md) | unknown cfg key |
| `CFG-4` | [E2203](errors/E2203.md) | cfg in the wrong place |
| `CONST-1` | [E1520](errors/E1520.md) | constant overflow |
| `CONST-2` | [E1521](errors/E1521.md) | constant division by zero |
| `CONST-3` | [E1522](errors/E1522.md) | constant evaluation out of fuel |
| `CONST-4` | [E1523](errors/E1523.md) | constant evaluation too deep |
| `CONST-5` | [E1524](errors/E1524.md) | not a constant |
| `CONTRACT-1` | [E1100](errors/E1100.md) | undischarged contract |
| `DET-1` | [E1302](errors/E1302.md) | nondeterministic pure function |
| `DET-4` | [E1405](errors/E1405.md) | uninitialised allocation outside unsafe |
| `DOC-1` | [E2100](errors/E2100.md) | doc comment with no item |
| `EFFECT-1` | [E1300](errors/E1300.md) | effect exceeded |
| `EFFECT-2` | [E1301](errors/E1301.md) | unsafe call in a declared function |
| `FFI-1` | [E0900](errors/E0900.md) | duplicate extern symbol |
| `FFI-2` | [E0901](errors/E0901.md) | malformed extern declaration |
| `FFI-3` | [E0902](errors/E0902.md) | misplaced variadic marker |
| `FIELD-1` | [E0200](errors/E0200.md) | unknown field |
| `FIELD-2` | [E0201](errors/E0201.md) | missing field |
| `FIELD-3` | [E0202](errors/E0202.md) | duplicate field |
| `ID-1` | [E1517](errors/E1517.md) | invalid `is` comparison |
| `IN-1` | [E1516](errors/E1516.md) | membership is not defined for this container |
| `LAYOUT-1` | [E0607](errors/E0607.md) | misaligned packed field |
| `LINT-1` | [E2000](errors/E2000.md) | unused binding |
| `LINT-2` | [E2001](errors/E2001.md) | unused import |
| `LINT-3` | [E2002](errors/E2002.md) | unreachable code |
| `LINT-4` | [E2003](errors/E2003.md) | unused parameter |
| `MACRO-1` | [E1600](errors/E1600.md) | macro invocation matches no arm |
| `MACRO-2` | [E1601](errors/E1601.md) | macro expansion limit exceeded |
| `MACRO-3` | [E1602](errors/E1602.md) | macro metavariable kind mismatch |
| `MACRO-4` | [E1603](errors/E1603.md) | duplicate macro name |
| `MUT-1` | [E0400](errors/E0400.md) | const reassignment |
| `NAME-1` | [E0001](errors/E0001.md) | unknown name |
| `NAME-2` | [E0002](errors/E0002.md) | unknown type |
| `OWN-1` | [E0500](errors/E0500.md) | moved value |
| `OWN-2` | [E1000](errors/E1000.md) | leaked linear value |
| `OWN-3` | [E1002](errors/E1002.md) | spread operand used after the literal |
| `OWN-4` | [E1003](errors/E1003.md) | arena value escapes its block |
| `OWN-5` | [E1004](errors/E1004.md) | arena value freed individually |
| `PAT-1` | [E0600](errors/E0600.md) | non-exhaustive match |
| `PAT-2` | [E0601](errors/E0601.md) | unreachable switch arm |
| `PAT-3` | [E0602](errors/E0602.md) | invalid range pattern |
| `PAT-4` | [E0603](errors/E0603.md) | `if let` pattern always matches |
| `PAT-5` | [E0604](errors/E0604.md) | unknown loop label |
| `PAT-6` | [E0605](errors/E0605.md) | duplicate loop label |
| `PAT-7` | [E0606](errors/E0606.md) | invalid fixed-array pattern |
| `PKG-1` | [E1800](errors/E1800.md) | manifest syntax |
| `PKG-10` | [E1809](errors/E1809.md) | registry index mismatch |
| `PKG-2` | [E1801](errors/E1801.md) | dependency cycle |
| `PKG-3` | [E1802](errors/E1802.md) | lockfile mismatch |
| `PKG-4` | [E1803](errors/E1803.md) | package missing from the offline store |
| `PKG-5` | [E1804](errors/E1804.md) | package audit failure |
| `PKG-6` | [E1805](errors/E1805.md) | invalid package signature |
| `PKG-7` | [E1806](errors/E1806.md) | package SBOM mismatch |
| `PKG-8` | [E1807](errors/E1807.md) | package policy denial |
| `PKG-9` | [E1808](errors/E1808.md) | capability not granted |
| `RANGE-1` | [E1518](errors/E1518.md) | a range needs integer or character bounds |
| `RUNTIME-1` | [E1200](errors/E1200.md) | grapheme index out of range |
| `RUNTIME-2` | [E1201](errors/E1201.md) | provable integer overflow |
| `SANDBOX-1` | [E1900](errors/E1900.md) | sandbox capability denied |
| `SANDBOX-2` | [E1901](errors/E1901.md) | sandbox limit breached |
| `THREAD-1` | [E1400](errors/E1400.md) | double close |
| `THREAD-2` | [E1401](errors/E1401.md) | use after close |
| `THREAD-3` | [E1402](errors/E1402.md) | race |
| `TRAIT-1` | [E1500](errors/E1500.md) | unknown trait |
| `TRAIT-10` | [E1509](errors/E1509.md) | wrong number of type arguments |
| `TRAIT-11` | [E1510](errors/E1510.md) | unused type parameter |
| `TRAIT-12` | [E1511](errors/E1511.md) | generic trait outside the standard library |
| `TRAIT-13` | [E1512](errors/E1512.md) | inheritance is not in the language |
| `TRAIT-14` | [E1513](errors/E1513.md) | there is no Drop |
| `TRAIT-15` | [E1514](errors/E1514.md) | a trait has no fields |
| `TRAIT-16` | [E1515](errors/E1515.md) | operators cannot be overloaded |
| `TRAIT-2` | [E1501](errors/E1501.md) | conflicting trait impl |
| `TRAIT-3` | [E1502](errors/E1502.md) | incomplete trait impl |
| `TRAIT-4` | [E1503](errors/E1503.md) | unsatisfied bound |
| `TRAIT-5` | [E1504](errors/E1504.md) | ambiguous method |
| `TRAIT-6` | [E1505](errors/E1505.md) | instantiation budget exceeded |
| `TRAIT-7` | [E1506](errors/E1506.md) | method called through a type |
| `TRAIT-8` | [E1507](errors/E1507.md) | associated function called through a value |
| `TRAIT-9` | [E1508](errors/E1508.md) | unknown associated function |
| `TRY-1` | [E0300](errors/E0300.md) | missing `?` |
| `TRY-2` | [E0301](errors/E0301.md) | mixed `?` |
| `TYPE-1` | [E0100](errors/E0100.md) | type mismatch |
| `TYPE-10` | [E0109](errors/E0109.md) | the type is written before the name |
| `TYPE-11` | [E1519](errors/E1519.md) | recursive type alias |
| `TYPE-2` | [E0101](errors/E0101.md) | equality in a comparison chain |
| `TYPE-3` | [E0102](errors/E0102.md) | field default is not constant |
| `TYPE-4` | [E0103](errors/E0103.md) | bitwise operand is not an integer |
| `TYPE-5` | [E0104](errors/E0104.md) | bitwise operands have different types |
| `TYPE-6` | [E0105](errors/E0105.md) | shift count out of range |
| `TYPE-7` | [E0106](errors/E0106.md) | repr(C) is not allowed here |
| `TYPE-8` | [E0107](errors/E0107.md) | sizeof or alignof names no known type |
| `TYPE-9` | [E0108](errors/E0108.md) | offsetof needs a repr(C) struct |
| `VIS-1` | [E0801](errors/E0801.md) | private item used from another module |
| `VIS-2` | [E0802](errors/E0802.md) | import cycle |

## Rules without a code

- `ALIAS-2`
- `ALIAS-4`
- `DET-2`
- `DET-3`
- `DET-5`
