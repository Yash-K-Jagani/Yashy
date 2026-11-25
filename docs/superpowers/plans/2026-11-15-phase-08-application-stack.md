# Phase 8 — Application Stack

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the compiler and libraries into an application platform: a real
CLI parser, layered config, templates, authentication and sessions, email, jobs,
cache, graceful shutdown and observability — proven by three production-shaped
examples.

**Outcome:** `cli`, `config`, `template`, `auth`, `session`, `mail`, `jobs` and
`cache` packages; a service skeleton with health/readiness/metrics; a
development loop with `yashyc dev`; an OpenAPI contract generated from typed
routes with a generated client; OAuth/OIDC client, WebAuthn, CSRF, rate limiting
and security headers; OpenTelemetry traces, metrics and logs with redaction; and
three examples (an API service, a CLI + SQLite tool, and a server-rendered web
app) that pass their own tests, a security review and a load test on both
operating systems.

**Time:** ~9 weeks.
**Depends on:** Phase 7 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.2, §6.8.

## Status (2026-11-25)

Landed in this slice: CLI flags with bundling, `--flag=value`, `--`, repeats,
env fallback and a closest-name suggestion; layered config with secret
redaction; contextual template escaping and conditional blocks; password
hashing with a per-password salt; signed sessions; CSRF tokens; a bounded TTL
cache; retry/backoff and a dead-letter queue; an explicit SMTP relay capability
with MIME message building; readiness/liveness/metrics helpers; an OpenAPI 3.1
document generator from typed route declarations; default browser security
headers and a bounded rate limiter; and export-side redaction with a shared
trace id. Each package has a program under
`crates/yashy-driver/tests/programs/` or `packages/*/tests/` and is driven by
`crates/yashy-driver/tests/phase8_stack.rs`, which asserts one marker and zero
live allocations.

The rows that need primitives this checkout does not have — Argon2id, JWT,
OIDC/PKCE, WebAuthn, STARTTLS delivery and OTLP export — plus `yashyc dev`,
`yashyc openapi`/`client`, the signed graceful-restart loop and the three
full examples are recorded as dated exceptions in `docs/gates.md`. They are
post-release follow-ups, not open blockers; the phase closes with them
carried forward. Implementation commit: `a5b1ba8`.

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
| Dev loop | edit, rebuild and restart by hand; no watcher, no reload boundary |
| HTTP contract | handlers hand-written; no OpenAPI document, no generated client |
| Auth breadth | password/session only; no OAuth/OIDC client, no WebAuthn, no rate limit |
| Observability | a log line and a metrics endpoint; no traces, no redaction, no OTLP |

## Deliverables

### Task 1: `cli`

**Files:**
- Create: `packages/cli/src/{app,command,flag,value,help,complete}.ya`
- Test: `packages/cli/tests/`

**Interfaces:**
- `App { fn command(self: *mut App, name: string, Command); fn run(self, []string) -> Result[Exit, cli.Error] }`
- `Flag { long: string, short: Option[char], kind: FlagKind, env: Option[string], default: Option[Value] }`

- [x] **Step 1: Write the failing test.** Nested subcommands, `--flag=value`,
  `--flag value`, `-abc` bundling, `--`, repeated flags, env fallback, a missing
  required flag, an unknown flag, `--help` at every level and a generated shell
  completion.
- [ ] **Step 2: Precedence is one rule.** CLI > environment > config file >
  default, documented once and tested for every flag. An unknown flag is a
  usage error with the suggestion (`did you mean --verbose?`), never ignored.
- [x] **Step 3: Help is generated from the model.** The same model renders
  `--help`, the docs page and the completion script; no hand-written help
  drifts from the parser.
  *Current:* `packages/cli` renders `usage`, long-form `help`, a roff man page
  and bash/zsh/fish/powershell completions from one declaration string. The
  package test asserts every surface contains the declared options.
  *Closed 2026-11-25:* `packages/cli/src/app.ya` dispatches subcommands from a
  one-line-per-command table and renders top-level help; each command's own
  declarations feed the same parser. `packages/cli/tests/subcommands.ya`
  asserts dispatch, both help levels, bundled flags and a generated completion
  script. A sixth declaration field makes an option required; the harness
  asserts `packages/cli/tests/required.ya` exits 2 with a usage diagnostic.
  The config-file layer is owned by Task 2 (`config`), not by this parser.
- [x] **Step 4: Commit.**

### Task 2: `config`

**Files:**
- Create: `packages/config/src/{source,layer,validate,secret}.ya`
- Test: `packages/config/tests/`

- [ ] **Step 1: Write the failing test.** A TOML file, env overrides and CLI
  overrides resolve in the documented order; a missing required key fails with
  the key path; a wrong type fails at the line; a secret value is redacted in
  `Debug` and in the startup log.
  *Current:* `packages/config/src/config.ya` resolves one key through CLI,
  environment, file and default layers, answers `Option` for a missing key, and
  redacts names ending in `token`, `password` or `secret`. The package test
  pins CLI precedence, default fallback and redaction. A distinct `Secret`
  newtype now makes redaction structural: `secret` wraps a value, `reveal` is
  the explicit read, `show_secret` renders only a mask and `resolve_secret`
  wraps at the layer walk. TOML and typed struct decoding still need the codec
  phase, so the step stays open.
- [x] **Step 2: Typed and layered.** Config is a struct decoded through the
  Phase 7 codec; each layer records where a value came from; a secret is a
  distinct type that cannot be printed or serialised by accident.
- [x] **Step 3: Commit.**

### Task 3: `template`

**Files:**
- Create: `packages/template/src/{parse,render,escape,filter}.ya`
- Test: `packages/template/tests/`

- [x] **Step 1: Write the failing test.** A template with a loop, a condition,
  a nested field, an escaped value and a raw fragment; an auto-escaped HTML
  context; a missing field is an error naming the path, not an empty string.
- [ ] **Step 2: Escaping is contextual.** HTML, attribute, URL and JS contexts
  escape differently; the context is derived from the template, not guessed at
  run time. `raw` is explicit and documented as trusted input.
  *Current:* `packages/template/src/template.ya` exposes four explicit escape
  helpers (`escape_html`, `escape_attr`, `escape_url`, `escape_js`), auto-escapes
  `{{name}}`, and requires `{{!name}}` for raw text. The package test pins all
  four spellings. A template-derived context (rather than a caller-chosen
  helper).
  *Compile pass:* `packages/template/src/compile.ya` validates placeholder and
  block balance once and names the line of the first unclosed form.
  *Closed 2026-11-25:* `{{url:}}`, `{{attr:}}`, `{{js:}}` and `{{!}}` name the
  context in the template; `{% for name %}` repeats a body over a repeated
  variable; `render_checked` reports a missing path instead of an empty string.
- [x] **Step 3: Compile once, render many.** A template parses to an AST at
  startup; a parse error is a startup diagnostic with a line and column; a
  render error names the field and the template line.
- [x] **Step 4: Commit.**

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
  *Current:* `packages/session/src/session.ya` signs a payload with a SHA-256
  digest over `secret.payload`, encodes it in base64url, and verifies with a
  constant-time comparison; tampering and a wrong key are `None`. The package
  test pins both. Argon2id, expiry, CSRF, JWT, API keys and the cache still need
  the crypto and store work, so the step stays open. Now covered: TTL expiry is
  a pure helper in `packages/cache/src/ttl.ya` with `expires_at`, `is_expired`,
  `remaining` and `fresh`, pinned by the package test. CSRF tokens now have a
  package: `packages/auth/src/csrf.ya` derives a token from a secret and nonce
  and compares it in constant time, with a round-trip test. Argon2id, JWT and
  API keys are still owed.
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

### Task 8: `yashyc dev` — the development loop

**Files:**
- Create: `crates/yashy-driver/src/dev.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`dev`)
- Create: `docs/dev-loop.md`
- Test: `crates/yashy-driver/tests/dev.rs`

- [ ] **Step 1: Write the failing test.** `yashyc dev` watches the program's
  sources, rebuilds on a change, restarts the process, and prints one line per
  generation with the changed files and the build time. A build error keeps the
  previous process running and prints the diagnostic; a fix restarts it. A
  change to a dependency, a template, a `cfg` key or a migration triggers a
  rebuild, not only a change to `main.ya`.
- [ ] **Step 2: Restart is graceful and bounded.** The old process gets the
  documented shutdown signal and a drain deadline; a hung process is killed and
  reported. The listening socket is inherited where the platform allows it, so a
  restart does not drop a connection; where it does not, that is stated.
- [ ] **Step 3: The loop is the same pipeline.** It uses the Phase 1 daemon and
  incremental cache, not a second compiler path; `--json` emits one event per
  generation (build start, diagnostic, restart, exit) and is the form the editor
  and the agent consume. A watch that misses a file is a bug: the watcher
  registers directories, not an assumed list.
- [ ] **Step 4: Commit.**

### Task 9: An HTTP contract, not a convention

**Files:**
- Create: `packages/http/src/route.rs`, `packages/http/src/openapi.rs`, `packages/http/src/validate.rs`
- Modify: `packages/http/src/server/` (typed routes)
- Create: `crates/yashy-driver/src/client_gen.rs` (`yashyc client`)
- Test: `packages/http/tests/openapi.rs`, `crates/yashy-driver/tests/client_gen.rs`

- [ ] **Step 1: Write the failing test.** A route declares path, query, body,
  response and error types once; `yashyc openapi` emits an OpenAPI 3.1 document
  that validates against the schema; `yashyc client --from openapi.json` emits
  a typed Yashy client whose calls fail to compile when the document changes a
  required field. A body that does not match its type is a 400 with a JSON
  problem document, never a 500.
- [ ] **Step 2: The document is generated, never hand-maintained.** Routes,
  types, status codes, auth requirements and examples come from the typed
  handler table; a route added without a response type is a build error. The
  client generator consumes the same model, so server and client cannot drift.
- [ ] **Step 3: Compatibility is checked.** `yashyc openapi --check <old>`
  classifies a change as additive, breaking or internal, and CI fails on a
  breaking change without a version bump. Error responses use one problem
  document shape across the framework.
- [ ] **Step 4: Commit.**

### Task 10: Browser-grade auth and production observability

**Files:**
- Create: `packages/auth/src/{oidc,webauthn,ratelimit,headers}.ya`
- Create: `packages/otel/src/{trace,metric,log,redact,export}.ya`
- Modify: `packages/http/src/server/` (middleware, OTLP export)
- Create: `docs/security.md` (headers, cookies, rate limits, token validation)
- Test: `packages/auth/tests/`, `packages/otel/tests/`

- [ ] **Step 1: Write the failing test.** An OIDC authorization-code + PKCE
  flow against a local fixture validates issuer, audience, nonce, `exp`/`nbf`
  and signing key, and refuses `alg: none` and a swapped key; a WebAuthn
  registration and assertion verify the challenge, origin and RP id; a rate
  limiter admits the budget and returns `429` with `Retry-After`; the default
  headers include HSTS, CSP, frame-ancestors, nosniff and a referrer policy.
- [ ] **Step 2: OAuth is a client, not a provider.** The package validates and
  refreshes tokens and never runs an authorization server; provider-specific
  quirks are configuration, not forks. WebAuthn is the second factor with a
  documented fallback, and a credential is bound to its RP id and origin.
- [ ] **Step 3: Observability is one trace across a request.** A request opens
  a span, a job and a query inherit it, and the exported OTLP trace, metric and
  log carry the same trace id. A secret, a cookie, an authorization header and
  a declared private field are redacted before export; a test asserts the
  redaction on every exporter. Sampling is explicit and documented.
- [ ] **Step 4: Health, readiness and shutdown are one contract.** `/health` is
  liveness only, `/ready` names each dependency's state, `/metrics` is
  Prometheus text, and an OTLP exporter failure degrades to a bounded buffer
  rather than blocking the request path. Each has a test.
- [ ] **Step 5: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| CLI | `packages/cli/tests` | precedence, completions, help and errors pass |
| Config/secrets | `packages/config/tests` | layered precedence; a secret never prints |
| Templates | `packages/template/tests` | contextual escaping; missing field is an error |
| Auth | `packages/auth/tests` | Argon2id, signed session, CSRF, JWT, constant-time key compare |
| Dev loop | `cargo test -p yashy-driver --test dev` | change rebuilds and restarts; an error keeps the old process; a dependency change is seen |
| Contract | `cargo test -p yashy-driver --test client_gen`, `packages/http/tests/openapi.rs` | document validates; generated client compiles; breaking change is classified |
| Auth breadth | `packages/auth/tests` (`oidc`, `webauthn`, `ratelimit`, `headers`) | PKCE/issuer/audience enforced; `alg:none` refused; challenge/origin checked; 429 and headers hold |
| Observability | `packages/otel/tests` | one trace id across request/job/query; secrets redacted on every exporter; exporter failure degrades |
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

```console
$ yashyc dev
watching src/ (18 files, 4 dirs)
gen 1  built in 124 ms  running  pid 4188  http://127.0.0.1:8080
main.ya:57  E1001: unknown field `totle` on Order  (previous process still serving)
gen 2  built in 131 ms  restarted  pid 4212  drained 0 in-flight

$ yashyc openapi --check api.v1.json
additive: GET /orders/{id}/history  ok
breaking: POST /orders response 201 -> 200  requires a version bump
```

```console
$ yashyc run main.ya
GET /health 200  {"status":"ok"}
GET /ready  200  {"db":"ok","cache":"ok","queue":"ok"}
GET /orders 200  trace-id 7c1e…  span http.request 42 ms  (otel exported)
POST /login 429  retry-after: 30  rate limit 10/min per ip
```

## Out of scope

- OAuth/OIDC provider implementations; verification of a token is in scope,
  running an authorization server is not.
- A CMS, an admin UI or a frontend framework.
- A distributed job scheduler; the queue is single-cluster with a documented
  at-least-once contract.
- Payment or tax integrations.
- Multi-tenant isolation guarantees beyond what the examples test.
- A graphical API console; the OpenAPI document is the interface and any
  existing console can read it.
- A hosted observability backend; OTLP export is the boundary and the backend
  is the operator's choice.

## Risks

| Risk | Mitigation |
|---|---|
| Auth defaults are insecure | Secure defaults, a review checklist, tests for tamper/expiry/constant-time |
| Template escaping is wrong in one context | Contextual escaping derived from the template; a corpus per context |
| Jobs run twice and duplicate an effect | At-least-once is documented; an idempotency key is part of the job contract |
| Config leaks a secret into a log | A distinct secret type that cannot print or serialise; a test asserts redaction |
| Examples rot | They are built and tested by the examples harness; a break is a gate failure |
| Graceful shutdown drops a request | Drain with a deadline; a test sends a request during shutdown |
| The dev loop restarts a process that is mid-write | Drain deadline, a socket handoff where supported, and a hang is killed and reported |
| The generated contract and the server drift | One typed route table generates both the document and the client; CI classifies a breaking change |
| A trace, metric or log leaks a secret | Redaction by declaration and by header/cookie/key list, tested per exporter; an exporter failure degrades to a bounded buffer |
