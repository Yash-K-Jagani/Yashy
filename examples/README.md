# Examples

Three application-shaped examples accompany the Phase 8 packages:

| Example | Shows |
|---|---|
| `api-service` | typed routes, an OpenAPI document from the route table, readiness, and a bounded drain |
| `cli-tool` | one declaration table for flags, layered config, TOML input and JSON/table output |
| `web-app` | escaped templates, signed sessions and a CSRF token in a rendered form page |

Each is a project with `yashy.toml`, `main.ya` and `tests/smoke.ya`, so
`yashyc test` inside the directory passes on its own. `phase8_stack.rs` also
builds and runs each `main.ya` and requires zero live allocations.

## Security review (2026-11-25)

| Check | Result |
|---|---|
| SQL | No example builds SQL by string concatenation; `api-service` uses typed route data only. |
| Template escaping | `web-app` uses `{{name}}` (HTML-escaped), never `{{!name}}` for user data. |
| Sessions | `web-app` signs its session payload and rejects a wrong key or a tampered token. |
| CSRF | `web-app` mints a token from a secret and a nonce and verifies it before trusting a form. |
| Secrets | `cli-tool` keeps config secrets behind the `Secret` wrapper; nothing prints them. |
| Error responses | `api-service` emits one problem shape for errors and liveness-only `/health`. |
| Load test | Not run here; recorded in `docs/gates.md` as a dated follow-up for both CI OSes. |
