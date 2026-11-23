# Phase 6 — Network and Web

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a Yashy program speak to the network safely: sockets, DNS, TLS,
HTTP, WebSocket, SSE, and a server framework with limits that hold under load.

**Outcome:** TCP/UDP/Unix sockets and DNS with deadlines, TLS 1.2/1.3 with
certificate verification, an HTTP/1.1 and HTTP/2 client, an HTTP/1.1 server with
a router/middleware/extractor framework, WebSocket, SSE, explicit limits, and a
fuzz-clean, leak-free HTTPS service that survives 50 concurrent clients.

**Time:** ~10 weeks.
**Depends on:** Phase 5 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.2, §6.6, §6.8.

## Why this phase

A language without TLS and HTTP is not a product language. 1.0's `net` and
`http` packages are small enough to demonstrate a socket and too small to serve
a request from a stranger: no TLS, no HTTP/2, no router, no body limits, no
timeouts. Every later phase — SQL services, the application stack, the agent
sandbox's local API — speaks HTTP.

The phase is also the first place where "no hidden failure" matters at a
boundary: a remote peer can send anything. Every parser is fuzzed, every limit is
explicit, and every operation has a deadline.

## Starting point

| Module | 1.0 value |
|---|---|
| `net` | TCP connect/listen, small; no UDP, no DNS API, no deadlines |
| `http` | minimal request/response; no client pool, no TLS, no HTTP/2 |
| TLS | none |
| Server framework | none |
| WebSocket/SSE | none |
| Limits | not specified |
| Fuzzing | no network corpus |

## Deliverables

### Task 1: Sockets, DNS and deadlines

**Files:**
- Modify: `packages/net/src/*.ya`, `crates/yashy-runtime/` (platform socket shim)
- Create: `packages/net/src/{tcp,udp,unix,dns,addr}.ya`
- Test: `packages/net/tests/`, `crates/yashy-driver/tests/programs/net/`

**Interfaces:**
- `TcpStream { fn read(self: *mut TcpStream, []u8) -> Result[int, net.Error]; fn write(self: *mut TcpStream, []u8) -> Result[int, net.Error]; fn set_deadline(self: *mut TcpStream, Instant) }`
- `TcpListener { fn accept(self: *mut TcpListener) -> Result[TcpStream, net.Error] }`
- `resolve(host: string, port: int) -> Result[[]Addr, net.Error]`

- [ ] **Step 1: Write the failing test.** A loopback echo server and client;
  a read that times out returns `Err(Timeout)`, not `0`; a UDP datagram
  round-trip; a DNS lookup of `localhost` and of a name with several A/AAAA
  records; a Unix socket on Linux/macOS and a named pipe on Windows behind the
  same `cfg`-selected API.
  *Current:* `packages/net/tests/sockets.ya` pins a loopback TCP echo, a UDP
  datagram round-trip, a DNS lookup of `localhost`, and an already-expired
  absolute deadline answered as `Err(Timeout)`. The typed `NetError` now names
  refused/reset/timeout/cancelled/DNS/address-in-use causes from a thread-local
  runtime status. A `cfg(unix)` Unix-domain socket API now has a loopback
  round-trip in the same program. Still owed in this step: a multi-record
  A/AAAA fixture and the Windows named-pipe half of the `cfg`-selected API.
- [x] **Step 2: A deadline on every blocking call.** Timeouts are absolute
  instants, not per-read resets, so a slow peer cannot extend a request
  forever. Cancellation closes the socket and reports `Cancelled`, distinct
  from `Timeout` and from `Closed`.
  *Current:* `net_set_deadline(fd, unix_ms)` stores an absolute instant in a
  bounded runtime table and refuses a read or write begun after it with the
  `Timeout` cause. The older per-call `SO_*TIMEO` spelling remains for callers
  that want a reset-per-operation timeout. `net_cancel` closes the socket and
  answers `Err(Cancelled)`; a later use of a closed descriptor is named as
  `Closed` by the runtime's per-descriptor state.
- [x] **Step 3: Errors are values with a cause.** `net.Error` distinguishes
  refused, reset, timeout, cancelled, DNS failure and address-in-use; each maps
  to one message and one line in the docs. No errno number leaks as the only
  explanation.
  *Current:* `NetError` has all six causes plus `Closed` and a legacy
  `Failure` variant. The runtime maps OS errors to one of seven stable codes,
  `net_describe` renders each as a sentence, and the package test pins
  `Timeout` from an expired deadline and `Cancelled` from `net_cancel`. A
  Windows-specific errno mapping check is still owed on a Windows runner.
- [ ] **Step 4: Commit.**

### Task 2: TLS

**Files:**
- Create: `packages/tls/` (Yashy package wrapping the Phase 5 C library)
- Modify: `packages/net/src/tcp.ya` (upgrade path), `crates/yashy-runtime/`
- Create: `docs/tls.md`
- Test: `packages/tls/tests/`, `crates/yashy-driver/tests/programs/tls/`

**Interfaces:**
- `TlsConfig { roots: TrustStore, alpn: []string, min_version: TlsVersion, client_auth: Option[ClientAuth] }`
- `TlsStream { fn read(...); fn write(...); fn peer_certificate(self) -> Option[Certificate] }`
- `connect(host: string, port: int, cfg: TlsConfig) -> Result[TlsStream, tls.Error]`

- [ ] **Step 1: Write the failing test.** Connect to a local TLS fixture with a
  test CA; assert a valid chain succeeds, an expired certificate fails, a wrong
  hostname fails, a revoked certificate fails when a CRL is supplied, and ALPN
  selects `h2` when offered.
- [ ] **Step 2: Wrap the vetted library.** TLS 1.2/1.3, SNI, ALPN, session
  resumption and certificate verification come from the pinned library; Yashy
  supplies configuration and I/O callbacks. No homegrown record layer, no
  homegrown X.509 parsing.
- [ ] **Step 3: Default to verify.** A trust store is required for a client
  connection; `--insecure` exists for tests and is printed as a warning, never
  silently enabled. A server requires a certificate and key; a missing file is
  a startup diagnostic, not a panic mid-request.
- [ ] **Step 4: Commit.**

### Task 3: The HTTP client

**Files:**
- Modify: `packages/http/src/client.ya`, `request.ya`, `response.ya`
- Create: `packages/http/src/{pool,redirect,body,limits}.ya`
- Test: `packages/http/tests/client/`, `crates/yashy-driver/tests/programs/http/client/`

- [ ] **Step 1: Write the failing test.** GET/POST against a loopback fixture;
  a redirect chain with a loop; a chunked response; a gzip response; a body
  larger than the limit; a request that exceeds its deadline; two requests that
  reuse one connection.
- [ ] **Step 2: HTTP/1.1 and HTTP/2 share one API.** ALPN selects the version;
  the request/response types do not change. A response body is a stream or a
  buffered value, chosen explicitly; a stream that is dropped closes the
  connection.
- [ ] **Step 3: Limits are arguments.** Max header bytes, max body bytes, max
  redirects, connect/read/write deadlines and a total deadline. A limit breach
  is a typed error naming the limit and its value; there is no unbounded read.
  *Current:* `packages/http/src/headers.ya` parses a header block into its
  original lines, gives case-insensitive lookup with a `Result` for malformed
  lines, and round-trips CRLF. `packages/http/src/limits.ya` now makes every
  cap an argument in a `Limits` value: header and body bytes, redirects and
  connect/read/write/total deadlines. `check` returns a `LimitError` naming
  the limit, its cap and the observed value; `describe` renders it and
  `bounded` reports unbounded (non-positive) caps. The package test pins a
  header breach and a body breach. Applying these caps inside the client and
  server read loops still needs the pipeline work, so the step stays open.
- [ ] **Step 4: Commit.**

### Task 4: The HTTP server and framework

**Files:**
- Create: `packages/http/src/server/{listen,conn,router,middleware,extract,respond}.ya`
- Create: `packages/http/src/server/http2.ya`
- Test: `packages/http/tests/server/`, `crates/yashy-driver/tests/programs/http/server/`

**Interfaces:**
- `Router { fn get(self: *mut Router, pattern: string, Handler); fn post(...); fn nest(self: *mut Router, prefix: string, Router) }`
- `Request`, `Response`, `Next`, `Handler = fn(Request) -> Response`
- `Extractor[T] { fn from(req: *mut Request) -> Result[T, Response] }`

- [ ] **Step 1: Write the failing test.** A router with a path parameter, a
  query extractor, a JSON body extractor, a middleware that adds a header and
  one that rejects an unauthenticated request; a 404, a 405, a 413 and a 500 from
  a handler panic (caught at the connection boundary, connection stays alive).
- [ ] **Step 2: One request pipeline.** Parse → route → extract → handle →
  respond → log. Middleware wraps the pipeline, not the socket. A handler
  returns a `Response`; a panic in a handler is a 500 with a stable body, and
  the process stays up.
- [ ] **Step 3: Limits and slowloris.** Header read deadline, body deadline,
  max connections, max per-connection requests, header count and size caps.
  A client that trickles bytes is closed at the deadline. These are defaults
  with names, overridable in config.
- [ ] **Step 4: Graceful shutdown.** `serve` stops accepting, drains in-flight
  requests up to a deadline, then closes. A second signal exits immediately.
  `--json` reports accepted, completed, failed and drained counts.
- [ ] **Step 5: Commit.**

### Task 5: WebSocket and SSE

**Files:**
- Create: `packages/http/src/ws.ya`, `packages/http/src/sse.ya`
- Test: `packages/http/tests/ws/`, `packages/http/tests/sse/`

- [ ] **Step 1: Write the failing test.** A WebSocket handshake, a text frame,
  a binary frame, a ping/pong, a close handshake, and a fragmented message; an
  SSE stream that reconnects with `Last-Event-ID`.
- [ ] **Step 2: Frames are parsed with limits.** Max frame size, max message
  size, UTF-8 validation for text frames, and a close code that is echoed
  correctly. A malformed frame closes with the protocol error code, never
  panics.
- [ ] **Step 3: Commit.**

### Task 6: Fuzz, load and leak gates

**Files:**
- Create: `packages/http/tests/fuzz/`, `packages/http/tests/load/`
- Modify: `crates/yashy-driver/tests/fuzz.rs` (corpus), `crates/yashy-driver/tests/leaks.rs`
- Create: `docs/http.md`

- [ ] **Step 1: Write the failing load test.** 50 concurrent clients against
  the HTTPS fixture, 1,000 requests, keep-alive, mixed sizes; assert zero
  failed requests, p99 under a recorded budget, and zero leaked allocations.
- [ ] **Step 2: Fuzz the parsers.** Request line, headers, chunked encoding,
  HTTP/2 frames, WebSocket frames, TLS is the library's problem but its
  configuration is ours. A crash or a hang is a P0; the corpus is committed.
- [ ] **Step 3: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| HTTPS service | `cargo test -p yashy-driver --test programs -- http` | a real TLS request/response succeeds against a test CA |
| 50 clients | `packages/http/tests/load` | 50 concurrent clients, 1,000 requests, zero failures, p99 under budget |
| Fuzz | `cargo test -p yashy-driver --test fuzz -- http` | no crash, hang or unbounded allocation |
| Leaks | `cargo test -p yashy-driver --test leaks` | zero |
| Limits | `packages/http/tests/server` | every limit has a test; a trickle client is closed at the deadline |
| TLS negative | `packages/tls/tests` | expired, wrong-host, revoked and untrusted each fail with a distinct code |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc run examples/https_server/main.ya
listening on https://127.0.0.1:8443
GET /health 200 (1.2 ms)
GET /users/7 200 (2.8 ms)

$ yashyc run examples/http_client/main.ya
GET https://example.test/ 200  text/html  12,481 bytes  h2
```

## Out of scope

- HTTP/3 and QUIC.
- A reverse proxy or a load balancer.
- Server push; HTTP/2 without push.
- WebSocket extensions (permessage-deflate) until a later phase.
- A hosted TLS certificate authority.

## Risks

| Risk | Mitigation |
|---|---|
| A hand-rolled TLS is wrong | Wrap the vetted library; no record layer or X.509 parser of our own |
| A server is trivially slowloris-able | Absolute deadlines, connection caps and a trickle test |
| A handler panic kills the server | Panic is caught at the connection boundary; a 500 is returned; the process stays up |
| Limits are hidden constants | Limits are named arguments with defaults, documented and tested |
| Fuzzing finds a hang late | Bounded fuzz in CI, longer nightly; a hang is a P0 |
| A connection pool leaks sockets | Pool size is capped; a dropped stream closes; the load test checks the handle count |
