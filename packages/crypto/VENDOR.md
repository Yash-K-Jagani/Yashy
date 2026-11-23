# Vendored native code

The Yashy crypto surface wraps **Monocypher 4.0.3**, a reviewed, dual-licensed
(BSD-2-Clause OR CC0-1.0) C library. It supplies Ed25519, X25519,
ChaCha20-Poly1305, SHA-512, HMAC-SHA512, HKDF-SHA512 and the constant-time
comparison primitive. No curve arithmetic or S-box is written by this project.

Upstream: <https://github.com/LoupVaillant/Monocypher>, tag `4.0.3`.

| File | SHA-256 of the upstream bytes |
|---|---|
| `native/monocypher.c` | `f1f838cdd483bdebe0df0ff5c5ed60535e496f769c6a2f933ac4c0b114207123` |
| `native/monocypher.h` | `fcaf6ed771358bb4f40fba016f6518ae86ec02b1b877d2cc35ad92d3a26fd7b3` |
| `native/optional/monocypher-ed25519.c` | `ce0d2f8e32ca8f66398ba5b3456cc74327c3eff14e7b950ce7d57be9025cc453` |
| `native/optional/monocypher-ed25519.h` | `3a3035181f991a158d0e1c7567258f0bae8ba0f1f23c5512b4a1db1b3c9730ce` |

One documented local edit: `optional/monocypher-ed25519.h` includes
`../monocypher.h` because the driver compiles each `link` input from its own
directory and offers no include search path for package C sources. No algorithm
code was changed. `native/yashy_crypto_shim.c` is this project's code: it only
converts between Yashy strings and the library's byte buffers.

The SBOM pins the exact bytes the driver compiles: a SHA-256 over the four
local files concatenated in the order listed above is
`a84d5c808ce023acd99c4571ce7f7f8d7726bb9f2cf94d263c8b535c24e16bf3`. It differs
from a hash of the upstream bytes by the one include edit recorded below.

Change control: update all four files together with the tag, recompute every
hash above, rerun the known-answer and tamper tests, and record the new tag in
the SBOM metadata.
