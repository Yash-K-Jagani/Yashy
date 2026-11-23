# Security review record — Phase 5 crypto package

**Status: self-review complete; independent review still owed.**

## Scope

The Yashy surface is `crates/yashy-stdlib/yashy/crypto.ya`. The native code it
links is:

- Monocypher 4.0.3 and its Ed25519 extension — curve arithmetic, SHA-512,
  HMAC/HKDF, ChaCha20-Poly1305 and constant-time comparison;
- tiny_sha3 at commit `dcbb319…` — SHA3-256;
- miniz 1.14 — zlib-compatible deflate/inflate;
- Zstandard 1.5.7 — zstd compression/decompression.

Exact hashes are in `VENDOR.md` and `NATIVE.lock`; the SBOM carries the same
bytes. The only project-written C is `yashy_crypto_shim.c`, which converts
between Yashy strings and byte buffers and frames gzip around miniz's raw
deflate.

## Findings from the self-review

1. **No homegrown primitives.** No S-box, field arithmetic, Keccak round or
   tag comparison is written in this repository. The gzip framing (header,
   CRC32, ISIZE) is format plumbing around miniz, not a cipher.
2. **Failures are values.** Every fallible wrapper answers `Result`; a null C
   return, malformed hex, an oversized requested length and an AEAD tag failure
   are typed errors. No function uses `""` or `0` as a failure value.
3. **Bounded decompression.** zlib and gzip check an explicit `max_bytes`
   before returning data; zstd checks the frame's declared content size before
   allocating. A caller cannot make the shim allocate without a cap.
4. **Constant-time comparison.** AEAD verification uses the library's
   `crypto_aead_unlock`; the exposed 16-byte compare uses `crypto_verify16`.
   The timing smoke test fails on a gross leak, with the documented caveat
   that it is not a proof.
5. **Entropy stays a capability.** The libraries take caller-supplied keys,
   salts and nonces. There is no call to `rand()` or any OS entropy source in
   the vendored tree reachable from this surface.

## Open items

- An independent review of the pinned library versions and the shim's buffer
  handling is not yet done. This is the gate Step 2 names and it is **not**
  discharged by this document.
- The shim uses `malloc`/`free` for C buffers, tracked separately from Yashy's
  allocator; a fuzz campaign over malformed hex, gzip and zstd frames should
  be run before the package is used with untrusted input.
- AES-GCM is not provided by the vendored library and is still unimplemented
  on this surface.
