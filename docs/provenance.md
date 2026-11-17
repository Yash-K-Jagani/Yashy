# Build provenance

`yashyc attest <artifact> --key-id <id> [--material <file>]...` writes an
in-toto Statement v1 with an SLSA v1 predicate next to the artifact:

```text
demo.tar.gz.attestation.json
demo.tar.gz.attestation.sig
```

The statement names the artifact's SHA-256 as its subject, the compiler version,
target and a build type in the predicate, and every `--material` as a named
digest. The signing seed comes from `YASHY_SIGNING_SEED` (64 hex digits), the
same source `publish --key-id` uses.

```text
YASHY_TRUSTED_KEY=release:<hex> yashyc verify-attestation demo.tar.gz \
  --attestation demo.tar.gz.attestation.json \
  --signature demo.tar.gz.attestation.sig
```

Verification checks the signature against the explicit trust set and the
subject digest against the artifact bytes. A signed claim is not the same as a
rebuild: `verify-attestation` does not yet reconstruct the artifact from the
named materials. The reproducible build gate is the stronger check.
