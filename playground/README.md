# The Yashy playground

The playground compiles on the server. There is no WASM compiler: a browser
build of Yashy is a 2.0 project with its own performance story, and the
sandbox already exists.

## Running it

```text
yashy playground --port 8080
```

Then open <http://127.0.0.1:8080>.

The server is the compiler's own binary: `yashy playground` serves the static
front end in this directory and shells out to the very `yashy` that is
running it.

## What a run is

Every request is a `yashy run --sandbox` with:

| Limit | Value | Why |
|---|---|---|
| the source | 64 KB | a body that large is not a program a person meant to write |
| the wall clock | 2 s | a playground that hangs is a playground nobody uses |
| the memory | 128 MB | the same |
| the output | 64 KB | a browser tab has no room for a gigabyte |
| the queue | 8 runs | a refusal is better than a wait |
| the rate | 100 requests a minute, per caller | one caller cannot take the queue for everyone |

The program may read the directory of its own run and write nothing
anywhere: the sandbox's capability set is default-deny, and the phase 11
escape corpus is replayed through the playground in
`crates/yashy-driver/tests/playground.rs`. The playground is the sandbox's
customer, not a second sandbox.

## The API

```text
POST /run
Content-Type: application/json

{"source": "fn main() { print(\"hi\"); }", "args": []}
```

The answer is a JSON document either way:

```json
{
  "ok": true,
  "stdout": "hi\n",
  "stderr": "",
  "exit": 0,
  "diagnostics": []
}
```

A refusal carries the same fields, with `ok` false and a `status`:

| Status | Meaning |
|---|---|
| `400` | the body was not `{"source": ..., "args": [...]}` |
| `413` | the source was over the cap |
| `429` | the caller was over its rate, or the queue was full |
| `500` | the server could not make a run directory |

A refusal is never a hang and never a crash: the front end gets a document
either way.
