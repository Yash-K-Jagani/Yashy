# Capabilities

A capability is a permission a package declares in `yashy.toml`. The names are
closed, so a typo is a diagnostic rather than a permission nobody notices:

```toml
[capabilities]
fs.read = ["data/*"]
fs.write = []
net = []
env = []
clock = []
proc = []
rng = []
ffi = []
```

An empty array grants the capability with no path restriction; a pattern list
restricts it to those paths. `*` matches one segment only, so `data/*` does not
grant `data/../secret`. A package that declares nothing gets nothing: it can
compute but cannot touch the world.

## The policy is the ceiling

A workspace's `[policy]` (parsed by the audit) narrows the grant: only names it
allows survive, and a name it does not name is denied. The policy never adds a
capability a package did not declare.

```text
yashyc audit --capabilities      print the resolved union before anything runs
yashyc audit --json              the same union in the machine-readable report
yashyc add <name> --allow net    record an explicit grant when adding a dependency
```

The union is computed from the resolved graph, so it includes the root package
and every dependency: a library that asks for `net` is visible before the
program is built, not only when the linker or a sandbox refuses it.

A `[audit]` policy can narrow them further: `allow-capabilities`, `deny-capabilities`, `max-transitive`, `deny-licenses`, `deny-registries` and `require-provenance` all produce E1807 findings from `yashyc audit`.

## Enforcement at both ends

`yashyc build --print-capabilities` prints the resolved set and stops. When a
manifest declares `[capabilities]`, the build refuses a call that needs a grant
it did not declare with E1808. The same resolved set is installed once at
program entry through `yashy_capability_init`, so the runtime repeats the check
where the operation executes: a file read without `fs.read` is refused even if
the IR was hand-written around the front end, and a missing `env` grant reads as
an unset variable rather than a value. A program with no declaration skips the
call entirely and keeps the historical permissive behaviour.

The handle is process-global and off until initialized. It stores at most
sixteen names; the closed capability set is smaller than that, so a manifest
cannot overflow it. `yashyc run --sandbox` adds OS-level path, time and memory
limits on top — the capability handle decides *what* the program may touch, the
sandbox decides *where* and *for how long*.

The same names are used by the checker's call-site rule, the Phase 13 sandbox,
and the Phase 10 trace redactor. Clock, process and FFI operations are still
checked at the call site only; the handle covers the filesystem and environment
operations today.
