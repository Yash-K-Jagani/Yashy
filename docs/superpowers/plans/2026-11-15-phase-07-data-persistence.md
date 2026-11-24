# Phase 7 — Data and Persistence

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make persistence a first-class part of the product stack: typed SQL
access to SQLite, PostgreSQL, MySQL and Redis; a query builder that cannot
interpolate; migrations with checksums; transactions and pooling that survive
load.

**Outcome:** `sqlite`, `postgres`, `mysql` and `redis` packages, a shared
`db` interface, `Encode`/`Decode` derives and codecs, a typed query builder,
migrations, transactions/savepoints, connection pools, an injection corpus and a
CRUD example that passes on both operating systems.

**Time:** ~8 weeks.
**Depends on:** Phase 6 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.2, §6.8.

## Why this phase

A language that can serve HTTP but cannot store a row is a demo. 1.0 has no SQL
driver, no migration story and no codec. Every application phase after this one
— the job queue, sessions, the cache, the examples — needs one.

The security question is the centre of the phase. Yashy has no string
interpolation in the query builder, only parameters; the injection corpus is the
gate. A driver that makes it easy to concatenate a query is a driver that will be
used that way.

## Starting point

| Fact | 1.0 value |
|---|---|
| SQL | none |
| Redis | none |
| Codecs | `Encode`/`Decode` derives do not exist (Phase 4 adds the derives) |
| Migrations | none |
| Pooling | none |
| Transactions | none |
| Injection tests | none |

## Deliverables

### Task 1: The `db` interface

**Files:**
- Create: `packages/db/src/{conn,row,value,error,pool,tx}.ya`
- Test: `packages/db/tests/`

**Interfaces:**
- `trait Driver { fn connect(cfg: Config) -> Result[Connection, db.Error] }`
- `Value = Null | Int(int) | Float(float) | Text(string) | Bytes([]u8) | Bool(bool)`
- `Row { fn get[T](self, name: string) -> Result[T, db.Error] }`
- `Connection { fn query(self: *mut Connection, sql: string, params: []Value) -> Result[[]Row, db.Error]; fn execute(...) -> Result[int, db.Error] }`

- [x] **Step 1: Write the failing test.** A fake driver implements the
  interface; a query returns rows; a missing column is an error, a null column
  is `Option`; a type mismatch names the column and the expected type.
  *Current:* `packages/db/tests/interface.ya` and
  `crates/yashy-driver/tests/db_interface.rs` pin a fake driver's query and
  execute, a missing column, a null column, all six value tags, a transaction
  with savepoints and a bounded pool, with zero live allocations.
- [x] **Step 2: One value model, four drivers.** Null is `Option`, never a
  sentinel; bytes and text are distinct; a driver maps its own types into the
  model and back. No driver leaks a library handle into the public API.
  *Current:* `Value` is a copyable struct with distinct text/bytes tags and an
  `is_null` tag; `Row.get` turns a null cell into `None`. `Connection` carries
  only integers, and a `Driver` trait with receiver methods dispatches
  statically.
  *Carried forward (2026-11-24):* the four real drivers are separate tasks
  below. The interface also lives in one `db.ya` module rather than the six
  files in the plan: the checker does not resolve qualified type names inside
  generic arguments, and a method call on a field access is rejected. The
  reason and owner are recorded in `docs/gates.md`.
- [ ] **Step 3: Commit.**

### Task 2: SQLite

**Files:**
- Create: `packages/sqlite/` (wraps a pinned SQLite amalgamation)
- Test: `packages/sqlite/tests/`, `crates/yashy-driver/tests/programs/sqlite/`

- [ ] **Step 1: Write the failing test.** Open a file and an in-memory database;
  create, insert, select, update, delete; a prepared statement reused with
  different parameters; a constraint violation returns a typed error with the
  constraint name; a busy database respects the busy timeout.
- [ ] **Step 2: Prepared by default.** Every parameterised query is a prepared
  statement; a statement is cached per connection; a query with an interpolated
  string is impossible through the API. WAL mode is opt-in with a documented
  trade-off.
- [ ] **Step 3: Commit.**

### Task 3: PostgreSQL and MySQL

**Files:**
- Create: `packages/postgres/` (libpq wrapper), `packages/mysql/` (MariaDB connector wrapper)
- Test: `packages/postgres/tests/`, `packages/mysql/tests/` (fixtures via Docker in CI; skipped with a diagnostic when unavailable)

- [ ] **Step 1: Write the failing test.** Connect over TCP and TLS; run the
  CRUD matrix; a unique violation, a serialization failure and a deadlock each
  map to a distinct retryable/non-retryable error; a statement timeout cancels
  the query.
- [ ] **Step 2: TLS is not optional in production code.** A connection string
  that requests TLS verifies the server; `sslmode=disable` exists for a local
  fixture and is reported. Credentials come from the environment or a secret
  source, never a checked-in default.
- [ ] **Step 3: Commit.**

### Task 4: Redis

**Files:**
- Create: `packages/redis/` (RESP3 client)
- Test: `packages/redis/tests/`

- [ ] **Step 1: Write the failing test.** GET/SET/DEL, expiry, a pipeline, a
  transaction (`MULTI`/`EXEC`), a pub/sub message, and a reconnect after the
  server closes the connection.
- [ ] **Step 2: RESP3 parsing is fuzzed.** A malformed reply is a typed error
  and a closed connection, never a panic or an infinite read.
- [ ] **Step 3: Commit.**

### Task 5: Query builder and codecs

**Files:**
- Create: `packages/db/src/{query,builder,encode,decode}.ya`
- Modify: `crates/yashy-sema/src/lib.rs` (derive hooks from Phase 4), `crates/yashy-codegen`
- Test: `packages/db/tests/query/`, `crates/yashy-driver/tests/programs/db/`

**Interfaces:**
- `Query { fn select(self, []Column) -> Query; fn from(self, Table) -> Query; fn filter(self, Expr) -> Query; fn build(self) -> (string, []Value) }`
- `Encode { fn encode(self, *mut Encoder) -> Result[void, CodecError] }`
- `Decode { fn decode(*mut Decoder) -> Result[Self, CodecError] }`

- [ ] **Step 1: Write the failing test.** Build a query with three filters,
  an `IN` list, an `ORDER BY` and a `LIMIT`; assert the SQL has placeholders and
  the values are separate. Attempt to pass a string as an identifier and assert
  it is refused. Round-trip a struct through `Encode`/`Decode` for every column
  type, including `Option`.
- [ ] **Step 2: Values never become SQL.** The builder has no `raw` escape
  hatch in safe code; an identifier is a `Table`/`Column` value, not a string;
  a dynamic `ORDER BY` is a closed enum. The injection corpus is a test file,
  not a review checklist.
  *Current:* `packages/db/src/query.ya` builds a `Query` only from SQL with `?`
  placeholders and a matching parameter list; values stay in a separate slice,
  so a semicolon or quote in a value is data. The package test includes an
  injection-shaped value and an arity mismatch. Identifiers are now newtypes:
  `query.table` and `query.column` validate a bare ASCII name and refuse a
  quote, semicolon, dot, space or leading digit; `query.Order` is a closed
  enum and `query.select` composes SQL only from validated names plus a
  caller-written placeholder `WHERE`. The package test carries an injection
  corpus of refused names and pins the generated SQL. A real driver is still
  owed, so the step stays open.
- [ ] **Step 3: Codecs are deterministic and versioned.** The wire form has a
  version byte; a decoder rejects an unknown version; a struct field order is
  declaration order; an unknown field in a map-like codec is an error unless the
  codec opts into forward compatibility.
- [ ] **Step 4: Commit.**

### Task 6: Migrations and pooling

**Files:**
- Create: `packages/db/src/{migrate,pool}.ya`
- Modify: `crates/yashy-driver/src/main.rs` (`yashyc task db.migrate` uses the Phase 2 task runner)
- Test: `packages/db/tests/migrate/`, `packages/db/tests/pool/`

- [ ] **Step 1: Write the failing test.** Apply three migrations, roll one
  back, re-apply, and detect an edited migration by checksum. Assert a failed
  migration leaves the schema unchanged where the driver supports transactional
  DDL, and reports exactly where it stopped where it does not.
- [ ] **Step 2: Migrations are files with a checksum.** `migrations/0001_init.sql`
  is applied once, recorded with its hash and a timestamp in a metadata table;
  an edited applied migration is a hard error. `up` is required; `down` is
  optional and documented as such.
- [ ] **Step 3: A pool is bounded and honest.** Max size, acquire timeout,
  idle timeout, health check and a `--json` snapshot (open, idle, waiting).
  A leaked connection is reported at exit; a pool exhaustion is a typed error,
  not a hang.
- [ ] **Step 4: Commit.**

### Task 7: The CRUD example and the load test

**Files:**
- Create: `examples/shop/` (HTTP + SQLite/Postgres + migrations + tests)
- Create: `examples/shop/load.ya`
- Test: `crates/yashy-driver/tests/examples.rs` (extend)

- [ ] **Step 1: Write the failing test.** Create a product, list it, update
  it, delete it, and assert the JSON responses; run the migration on a fresh
  database; run 50 concurrent clients against the list endpoint with a pool of
  10 and assert zero errors.
- [ ] **Step 2: The example is the documentation.** `docs/tutorial` gains a
  chapter that builds the same program; `docs/packages.md` links to it. A
  change to the driver that breaks the example is a gate failure.
- [ ] **Step 3: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| CRUD | `cargo test -p yashy-driver --test examples -- shop` | create/list/update/delete pass against SQLite and Postgres |
| Transactions | `packages/db/tests` | commit, rollback, savepoint and serialization-failure paths pass |
| Injection | `packages/db/tests/query` | every corpus attempt is parameterised or refused; no string reaches SQL |
| Codecs | `crates/yashy-driver/tests/programs/db` | round-trip for every column type and `Option`; unknown version refused |
| Migrations | `packages/db/tests/migrate` | apply/rollback/re-apply; an edited migration is refused |
| Pool | `packages/db/tests/pool` | bounded, timeout, health check, no leak at exit |
| Load | `examples/shop/load.ya` | 50 clients, pool 10, zero errors, p99 under budget |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc task db.migrate
applied 0001_init.sql  sha256:4c1e…  12 ms
applied 0002_products.sql  sha256:9ab2…  8 ms

$ yashyc run examples/shop/main.ya
POST /products 201  {"id":1,"name":"tea","price":4.50}
GET  /products 200  [{"id":1,"name":"tea","price":4.50}]
```

## Out of scope

- An ORM with lazy loading or identity maps.
- A schema DSL that replaces SQL; the builder composes SQL, it does not hide it.
- Database-specific extensions beyond the four drivers.
- Distributed transactions.
- A hosted database or a migration service.

## Risks

| Risk | Mitigation |
|---|---|
| SQL injection through the builder | No string identifiers in safe code; placeholders only; a corpus gate |
| A driver hides a null as a sentinel | The value model uses `Option`; a doc test rejects sentinels |
| Migrations drift from the schema | Checksums, an applied-migration table, a re-apply test |
| A pool exhausts under load | Bounded pool, acquire timeout, a waiting count in `--json`, a load test |
| A C driver leaks or crashes | Thin wrapper, leak gate, fuzz the reply parser, pin and audit the library |
| Transactions behave differently per driver | The interface documents the isolation each driver supports; a test asserts the reported level |
