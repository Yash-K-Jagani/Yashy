# Editions

An edition is a compatibility promise, not a version flag. It says which
spellings and which semantics a program gets, and it changes only when a
language change cannot be made without breaking a program that compiled before.

## The promise

Within one edition, no program that compiled stops compiling. A change that
would break such a program is a new edition with a migration, never an update to
an existing one. The compiler keeps reading every edition it has shipped: a
project pinned to an older edition does not need new source just because a newer
compiler is installed.

The supported set is closed. A manifest naming an unknown edition is a
configuration error that lists the valid values. A manifest with no `edition`
key builds under the default, so a 1.0 project is unchanged by the upgrade.

| Edition | Status | Default |
|---|---|---|
| `2021` | the 1.0 language, still read by the compiler | yes, when the key is absent |
| `2027` | the 2.0 language | no |

## What an edition can change

An edition may change the meaning of a spelling or remove a form that was
already deprecated under the previous edition. It may not change the data
model, the verifier's soundness contract or the determinism contract: those are
promises the compiler makes to every edition at once.

An edition change is an RFC. The RFC names the rule ids and diagnostic codes it
touches, the conformance cases that pin the new behaviour and the migration that
rewrites old source. The [RFC process](rfcs/README.md) requires those sections
before an RFC can be accepted.

## Removing a form

Nothing is removed silently. The [deprecation policy](deprecation.md) is the
path: warn with a coded diagnostic, keep the old spelling working for at least
two releases, document the move, then remove it at an announced version. Removal
happens in an edition, and the migration notes list every removal in that
edition. A deprecation warning never changes generated code, exit codes or
verification results.

## Tooling status

The compiler currently reads the default edition and reports an unknown edition
as a configuration error. The `edition` manifest key, `yashyc migrate` with
`--dry-run`, and `yashyc fix` are still owed; until they land the table above is
the normative promise and the [migration chapter](../book/src/migration.md) is the
human guide. The [support matrix](lts.md) says how long each edition receives
fixes.
