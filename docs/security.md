# Security defaults in the application stack

This page names the default a caller gets without extra configuration. Each
item has a test in `crates/yashy-driver/tests/phase8_stack.rs` or in its
package's own tests.

## Transport and headers

- `packages/auth/src/security.ya` emits HSTS, a CSP with `frame-ancestors
  'none'`, `nosniff` and a strict referrer policy. There is no mode that drops
  these headers by accident; adding one is a deliberate edit.
- `packages/auth/src/ratelimit.ya` returns a bounded decision and a
  `Retry-After` value, never an unbounded wait.

## Sessions and cookies

- `packages/session/src/cookie.ya` always writes `HttpOnly; Secure;
  SameSite=Lax; Path=/`. `Max-Age` is opt-in.
- Session payloads are signed. Expiry is checked on every read by
  `packages/session/src/store.ya`, and a token with the wrong key or a changed
  byte is rejected.

## Tokens and credentials

- `packages/auth/src/jwt.ya` verifies the algorithm, signature, `exp`, `nbf`
  and `aud`; `alg: none` and a swapped key are rejected.
- `packages/auth/src/apikey.ya` stores only a digest and compares in constant
  time. The raw key is returned once at issue time.
- `packages/auth/src/oidc.ya` is a client: it adds PKCE S256, state, nonce and
  issuer checks and never runs an authorization server.
- `packages/auth/src/webauthn.ya` binds a credential to its RP id and origin
  and verifies an Ed25519 assertion over that binding plus a fresh challenge.
  Full CBOR/COSE attestation parsing is not part of this checkout; see
  `docs/gates.md`.

## Templates, logging and export

- `packages/template/src/template.ya` HTML-escapes by default. `url:`, `attr:`
  and `js:` name a context in the template; `{{!name}}` is the explicit raw
  spelling and is the only way to opt out.
- `packages/otel/src/redact.ya` redacts secrets, cookies and authorization
  headers before an exporter sees them. A span carries one trace id through a
  request, its jobs and its queries.
- `packages/otel/src/export.ya` drops the oldest event when its bounded buffer
  fills, so an exporter failure cannot block a request.

## Passwords

- `packages/auth/src/password.ya` uses a fresh CSPRNG salt per password and a
  constant-time digest comparison. Argon2id is still owed; the digest is a
  commitment, not a password KDF, and is recorded as such in `docs/gates.md`.
