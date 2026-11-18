# Long-term support

## Commitment

Yashy 1.0 receives security fixes and fixes for critical correctness bugs for
18 months after the 2.0 release. The clock starts at the 2.0 tag, not at the
date a feature was merged. 2.0 then becomes the LTS baseline, and the same
commitment applies to it when 3.0 ships.

A security fix is a change that closes a path by which untrusted input can read
memory it should not, write memory it should not, escape the compiler's
sandbox, or bypass a capability check. A critical correctness fix is one that
makes a verified program produce a wrong answer or crash, or that makes a
malformed program compile silently. Cosmetic and performance changes are not
LTS fixes; they go to the current release.

## Support matrix

| Version | Status | Security fixes | Critical fixes | New features |
|---|---|---|---|---|
| 1.0 | LTS until 2.0 + 18 months | yes | yes | no |
| 1.x point releases | current | yes | yes | yes, additive only |
| 2.0 | current once tagged | yes | yes | yes |
| development `main` | unsupported | no | no | yes |

Dates are recorded in the release notes for each tag. A version leaves the
matrix only by reaching the end of its window; leaving is announced in the
release notes and in `docs/deprecation.md` before it happens.

## What support means

A supported version receives a patch release with a changelog entry and a
migration note when behaviour changes. A fix that cannot be backported without
changing the language's identity is declined for the LTS line and documented
with the reason. The identity — no GC, no borrow checker, no exceptions, no
inheritance, no operator overloading, no procedural macros — never changes in
a patch release.
