# Vendored SQLite

This package pins the SQLite amalgamation by exact bytes. The engine is not
reimplemented in Yashy; only a thin C shim moves bytes across the string ABI.

| Source | Version | License | SHA-256 |
|---|---|---|---|
| `native/sqlite3.c` | 3.54.1 (amalgamation 3540100) | Public Domain | `0c657666d819f261cc4007526ad57e1669de9949df7a8ea847aa284a2bf0ee52` |
| `native/sqlite3.h` | 3.54.1 (amalgamation 3540100) | Public Domain | `d6c84557261893439b341d2e6c35187ae3ea24fbfd843ad94616f72d89242b8c` |

Downloaded from <https://sqlite.org/2026/sqlite-amalgamation-3540100.zip>
(SHA3-256 `92281c86e2957f9b9305890f9ac04204caa7b677cc868dc326d942689df28e30`).

`native/yashy_sqlite_shim.c` is first-party: it adapts the C API to Yashy
strings and integer handles and owns no SQL logic.

The amalgamation compiles as one `link` input beside the shim. No system SQLite
development package is required, so the same build works on Windows and Linux.
