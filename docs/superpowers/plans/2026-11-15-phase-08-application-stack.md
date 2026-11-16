# Phase 8 — Application Stack

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the compiler and libraries into an application platform: a real
CLI parser, layered config, templates, authentication and sessions, email, jobs,
cache, graceful shutdown and observability — proven by three production-shaped
examples.

**Outcome:** `cli`, `config`, `template`, `auth`, `session`, `mail`, `jobs` and
`cache` packages; a service skeleton with health/readiness/metrics; and three
examples (an API service, a CLI + SQLite tool, and a server-rendered web app)
that pass their own tests, a security review and a load test on both operating
systems.

**Time:** ~8 weeks.
**Depends on:** Phase 7 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.2, §6.8.

## Why this phase

After Phase 7 a program can store data; it still cannot parse `--flag=value`,
load a config file with precedence, render an HTML page, hash a password, send
an email, run a retrying job, or shut down without dropping a request. Those are
the ordinary parts of an application, and every one of them is a place where a
security mistake is easy and a good default is worth a package.

The three examples are the phase's contract: if they are not pleasant to build,
the stack is not done, no matter how many modules exist.

## Starting point

| Fact | 1.0 value |
|---|---|
| `cli` | small argument helper; no subcommands, completions or precedence |
| Config | environment and files read ad hoc |
| Templates | none |
| Auth | none |
| Sessions | none |
| Email/jobs/cache | none |
| Service skeleton | none |

## Deliverables

### Task 1: `cli`

**Files:**
- Create: `packages/cli/src/{app,command,flag,value,help,complete}.ya`
- Test: `packages/cli/tests/`

**Interfaces:**
- `App { fn command(self: *mut App, name: string, Command); fn run(self, []string) -> Result[Exit, cli.Error] }`
- `Flag { long: string, short: Option[char], kind: FlagKind, env: Option[string], default: Option[Value] }`

- [ ] **Step 1: Write the failing test.** Nested subcommands, `--flag=value`,
  `--flag value`, `-abc` bundling, `--`, repeated flags, env fallback, a missing
  required flag, an unknown flag, `--help` at every level and a generated shell
  completion.
- [ ] **Step 2: Precedence is one rule.** CLI > environment > config file >
  default, documented once and tested for every flag. An unknown flag is a
  usage error with the suggestion (`did you mean --verbose?`), never ignored.
- [ ] **Step 3: Help is generated from the model.** The same model renders
  `--help`, the docs page and the completion script; no hand-written help
  drifts from the parser.
- [ ] **Step 4: Commit.**

### Task 2: `config`

**Files:**
- Create: `packages/config/src/{source,layer,validate,secret}.ya`
- Test: `packages/config/tests/`

- [ ] **Step 1: Write the failing test.** A TOML file, env overrides and CLI
  overrides resolve in the documented order; a missing required key fails with
  the key path; a wrong type fails at the line; a secret value is redacted in
  `Debug` and in the startup log.
- [ ] **Step 2: Typed and layered.** Config is a struct decoded through the
  Phase 7 codec; each layer records where a value came from; a secret is a
  distinct type that cannot be printed or serialised by accident.
- [ ] **Step 3: Commit.**

### Task 3: `template`

**Files:**
- Create: `packages/template/src/{parse,render,escape,filter}.ya`
- Test: `packages/template/tests/`

- [ ] **Step 1: Write the failing test.** A template with a loop, a condition,
  a nested field, an escaped value and a raw fragment; an auto-escaped HTML
  context; a missing field is an error naming the path, not an empty string.
- [ ] **Step 2: Escaping is contextual.** HTML, attribute, URL and JS contexts
  escape differently; the context is derived from the template, not guessed at
  run time. `raw` is explicit and documented as trusted input.
- [ ] **Step 3: Compile once, render many.** A template parses to an AST at
  startup; a parse error is a startup diagnostic with a line and column; a
  render error names the field and the template line.
- [ ] **Step 4: Commit.**

### Task 4: `auth`, `session` and `cache`

**Files:**
- Create: `packages/auth/src/{password,session,csrf,jwt,apikey}.ya`
- Create: `packages/session/src/{store,cookie}.ya`
- Create: `packages/cache/src/{memory,redis,ttl}.ya`
- Test: `packages/auth/tests/`, `packages/session/tests/`, `packages/cache/tests/`

- [ ] **Step 1: Write the failing test.** Hash and verify a password
  (Argon2id with a per-password salt); a session cookie is signed, expires, and
  is rejected after tampering; CSRF token round-trip; a JWT with a valid and an
  invalid signature; an API key compared in constant time; a cache hit, miss,
  expiry and eviction.
- [ ] **Step 2: The secure choice is the default.** Argon2id parameters are
  the current recommendation; a cookie is `HttpOnly`, `Secure` and
  `SameSite=Lax` unless explicitly changed; a session id is CSPRNG-generated;
  a comparison is constant-time; a JWT verifies `alg`, `exp`, `nbf` and `aud`.
- [ ] **Step 3: No ambient state.** A session store is passed in; the default
  is in-memory with a documented "single process only" warning; Redis is the
  production store and is a separate constructor.
- [ ] **Step 4: Commit.**

### Task 5: `mail` and `jobs`

**Files:**
- Create: `packages/mail/src/{smtp,message,template}.ya`
- Create: `packages/jobs/src/{queue,worker,retry,schedule}.ya`
- Test: `packages/mail/tests/` (local SMTP fixture), `packages/jobs/tests/`

- [ ] **Step 1: Write the failing test.** Send a multipart message to a local
  SMTP fixture over STARTTLS; a template renders subject and body; a job fails
  twice and succeeds on the third attempt with backoff; a job that exhausts its
  retries lands in a dead-letter queue with its error; a scheduled job fires
  once.
- [ ] **Step 2: At-least-once, idempotent by contract.** A job carries an id
  and an idempotency key; the worker documents that a job may run twice, and the
  queue records attempts. A dead letter is inspectable and replayable.
- [ ] **Step 3: Mail is a capability.** A test build uses the fixture and never
  a real relay; a production build requires an explicit relay configuration;
  no default `localhost:25` send.
- [ ] **Step 4: Commit.**

### Task 6: The service skeleton

**Files:**
- Create: `templates/service/` (Phase 3 created it; this phase fills it in)
- Create: `packages/http/src/server/health.ya`, `metrics.ya`
- Test: `crates/yashy-driver/tests/examples.rs` (extend)

- [ ] **Step 1: Write the failing test.** `/health` (liveness), `/ready`
  (checks the pool and the cache), `/metrics` (Prometheus text and JSON),
  graceful shutdown on SIGINT/SIGTERM with in-flight drain, and a startup
  failure that exits non-zero with one diagnostic.
- [ ] **Step 2: The skeleton is the documentation.** Config, logging,
  routing, a database pool, a migration task, a health endpoint and a shutdown
  path — each wired once, each replaceable, none hidden in a macro.
- [ ] **Step 3: Commit.**

### Task 7: Three production-shaped examples

**Files:**
- Create: `examples/api-service/`, `examples/cli-tool/`, `examples/web-app/`
- Modify: `docs/tutorial/` (new chapters), `examples/README.md` (create if absent)
- Test: `crates/yashy-driver/tests/examples.rs`

- [ ] **Step 1: API service.** HTTP + Postgres/SQLite + migrations + auth +
  jobs + metrics; its own test suite and a load script.
- [ ] **Step 2: CLI tool.** `cli` + SQLite + config + output formats
  (`--json`, table); a shell-completion test.
- [ ] **Step 3: Web app.** Server-rendered templates + sessions + CSRF + a
  form POST + static files; an end-to-end browser-free test using the HTTP
  client.
- [ ] **Step 4: Security review.** Each example's auth, session, CSRF, SQL,
  template escaping, secret handling and error responses are reviewed against a
  checklist; findings are fixed or recorded as exceptions with a date.
- [ ] **Step 5: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| CLI | `packages/cli/tests` | precedence, completions, help and errors pass |
| Config/secrets | `packages/config/tests` | layered precedence; a secret never prints |
| Templates | `packages/template/tests` | contextual escaping; missing field is an error |
| Auth | `packages/auth/tests` | Argon2id, signed session, CSRF, JWT, constant-time key compare |
| Jobs/mail | `packages/jobs/tests`, `packages/mail/tests` | retry/backoff/dead-letter; SMTP fixture over STARTTLS |
| Examples | `cargo test -p yashy-driver --test examples` | all three pass on Windows and Linux |
| Load | `examples/api-service/load.ya` | 50 clients, p99 under budget, zero errors |
| Security review | `docs/security-review-2027.md` | checklist complete; no open high finding |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc new shop --template service && cd shop
$ yashyc task db.migrate && yashyc run main.ya
listening on http://127.0.0.1:8080
GET /health 200  {"status":"ok"}
GET /ready  200  {"db":"ok","cache":"ok"}
POST /login 303  set-cookie: session=…; HttpOnly; Secure; SameSite=Lax
GET /orders 200  text/html  4,812 bytes
```

## Out of scope

- OAuth/OIDC provider implementations; verification of a token is in scope,
  running an authorization server is not.
- A CMS, an admin UI or a frontend framework.
- A distributed job scheduler; the queue is single-cluster with a documented
  at-least-once contract.
- Payment or tax integrations.
- Multi-tenant isolation guarantees beyond what the examples test.

## Risks

| Risk | Mitigation |
|---|---|
| Auth defaults are insecure | Secure defaults, a review checklist, tests for tamper/expiry/constant-time |
| Template escaping is wrong in one context | Contextual escaping derived from the template; a corpus per context |
| Jobs run twice and duplicate an effect | At-least-once is documented; an idempotency key is part of the job contract |
| Config leaks a secret into a log | A distinct secret type that cannot print or serialise; a test asserts redaction |
| Examples rot | They are built and tested by the examples harness; a break is a gate failure |
| Graceful shutdown drops a request | Drain with a deadline; a test sends a request during shutdown |
