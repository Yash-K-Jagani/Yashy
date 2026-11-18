# HTTP

Yashy's HTTP support is a set of ordinary packages, not a framework baked into
the compiler. The message format lives in `packages/http/src/http.ya`; a header
block parser and explicit read limits sit beside it in `headers.ya` and
`limits.ya`.

## Headers are data

A header block keeps its original `Name: value` lines so casing and order
survive a round trip. Lookups are case-insensitive, and a malformed line is an
error naming its line number rather than a silently dropped field.

```yashy
import headers;

fn show(string block) {
    switch headers.parse(block) {
        case Ok(set) {
            print(headers.content_type(set) matches Some(_));
            print(headers.to_string(set));
        }
        case Err(kind) { print(kind); }
    };
}
```

## Limits are arguments

Every cap is a value the caller chooses. `limits.strict()` is a bounded
starting point; a local fixture can widen or remove a cap explicitly. A breach
is a `LimitError` naming the limit, its cap and the observed value.

```yashy
import limits;

fn within(string block) -> bool {
    const caps = limits.strict();
    switch limits.headers(caps, block) {
        case Some(error) {
            print(limits.describe(error));
            return false;
        }
        case None { return true; }
    };
}
```

The caps cover header bytes, body bytes, redirects and connect, read, write and
total deadlines. `limits.bounded(caps)` reports whether any of them is
unbounded, which is useful when a production caller wants to refuse that.

## Messages

`http.parse_request` and `http.serialize_response` share one wire format. Chunked
bodies are decoded; a body past the cap is a value the caller can turn into a
`413`, not a panic and not a silently truncated request.

```yashy
import http;

fn reply(string raw) -> string {
    switch http.parse_request(raw) {
        case Ok(request) {
            return http.serialize_response(http.response(200, "ok"), false);
        }
        case Err(_) { return http.serialize_response(http.response(400, ""), false); }
    };
}
```

The TLS transport, connection pooling and server process control are not part
of the pure packages; those pieces arrive with the external-library work.
