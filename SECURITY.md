# Security policy

## Reporting a vulnerability

Report privately to the maintainers through the repository's security contact
or a private message to a maintainer. Do not open a public issue for an
unfixed vulnerability. Include the version, target OS, a minimal reproducing
program, the observed behaviour and the behaviour you expected. You should get
an acknowledgement within seven days and a status update every two weeks until
the issue is closed.

## What is in scope

- A program that passes `yashyc verify` reading or writing memory outside its
  allocations.
- A sandbox escape: a capability, file path or network target outside the set
  the program was granted.
- A signature, lockfile or installer check that accepts tampered bytes.
- A compiler crash or hang on malformed source where a diagnostic is required.
- A diagnostic that can be tricked into approving an unsound program.

A behaviour that merely surprises a user, a missing feature, or a performance
regression is not a security issue; use the normal issue tracker.

## Supported versions

See `docs/lts.md`. Only versions in the support matrix receive security fixes.

## Disclosure

Fixes land in a patch release with a changelog entry. An advisory follows the
format described in Phase 14 Task 7 of the 2.0 plan and names the affected
versions, the attack, the fix and the upgrade path. Reporters are credited in
the advisory unless they ask not to be.
