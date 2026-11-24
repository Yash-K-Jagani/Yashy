# The HTTP package

The `http` package is the Yashy-facing web surface. It contains no hidden
runtime: every module is Yashy source under `packages/http/src/` and every
fallible function answers `Result` or `Option`.

## Modules

| Module | Surface |
|---|---|
| `http.ya` | `Request`, `Response`, `HttpError`, `response`, `header_value`, `header_end`, `content_length`, `is_chunked`, `decode_chunked`, `request_line`, `parse_request`, `status_text`, `serialize_response` |
| `headers.ya` | `Headers`, `parse`, `get`, `content_type`, `to_string` — case-insensitive lookup, original CRLF preserved |
| `limits.ya` | `Limits`, `LimitKind`, `LimitError`, `strict`, `bounded`, `check`, `value`, `name`, `describe`, `headers`, `body` |
| `client.ya` | `get`, `post`, `get_limited` — one request per call, followed once to a redirect, with typed `HttpError` results |
| `server.ya` | `serve` — one thread per connection, keep-alive, a 10 s header deadline, a 30 s idle timeout and a 10 MiB body cap |
| `router.ya` | `path_of`, `query_of`, `path_matches`, `param` — `:name` path parameters, no mutable route table |
| `extract.ya` | `query`, `json_string`, `json_int` — request parameters and JSON body fields |
| `middleware.ya` | `with_header`, `require_header`, `normalize_status` — combinators over `fn(Request) -> Response` |
| `ws.ya` | `Frame`, `WsError`, `encode`, `decode`, `decode_limited`, `pong`, `close_code`, `close_frame`, `valid_text`, `join` |
| `sse.ya` | `Event`, `parse`, `resume_header` — one event at a time with a `rest` cursor |

## Design rules

- **Errors are values.** A missing header is `Option[string]`; a malformed
  request, header block, frame or limit breach is a typed error. `""` and `0`
  are never failure sentinels.
- **Limits are arguments.** `limits.Limits` names every cap; `get_limited`
  accepts one. The server's read-loop constants are named defaults and are
  not yet per-server arguments.
- **Dispatch is static.** A struct field cannot hold a function type, so a
  router is a dispatch function using `router.ya` matchers. Middleware
  composes function values without changing the handler signature.
- **Ownership is explicit.** A `[]string` passed by value belongs to the
  callee; `client.post` frees its header array. Binding a function call's
  result before reading a field avoids a leaked temporary in today's backend.

## Verification

`cargo test -p yashy-driver --test http_client --test http_framework
--test http_streams --test fuzz --test leaks` covers the loopback client, the
framework, the stream parsers, a deterministic hostile-input corpus and the
runtime leak report. Each program asserts a zero live-allocation count.

## Carried forward

TLS, HTTP/2/ALPN, hard pre-buffering caps, configurable slowloris limits,
signal-driven shutdown, the WebSocket handshake, raw high-bit byte
construction and live SSE reconnection are **dated, owned exceptions** in
`docs/gates.md`. This page does not claim them.
