# Vendored Unicode Character Database

Pinned release: **Unicode 18.0.0** (the same release as the grapheme-break
tables in `crates/yashy-runtime/src/grapheme_data.h`).

These files were downloaded once at development time from
`https://www.unicode.org/Public/18.0.0/ucd/` and are checked in so builds are
offline and byte-stable. Do not edit by hand; do not re-download during a
build. The `scripts/gen_unicode_data.py` generator reads them and writes
`crates/yashy-runtime/src/unicode_data.h`.

| File | SHA-256 |
|---|---|
| `UnicodeData.txt` | `0736451de439ae7baf1425136617da495e09ee5afbe6e394374db7009ea08950` |
| `CaseFolding.txt` | `a004797658a457bec4dc11683e39f69249ea3b595b752dbea6721c4c9f587b0d` |
| `CompositionExclusions.txt` | `c759b100e9ae8960ae6b50fc5765950a7581fd9f30d4ef95f68f25a6d97e818e` |
| `EastAsianWidth.txt` | `a0cf29eacd00cfcaec4381c6b7c281685f18dbb4e7ff82b4076ccb342ca839aa` |
| `NormalizationTest.txt` | `25a50d816764b04abfb4a646d3eb2b2a803284c3873d9a06757b94fe4513dde3` |
| `ReadMe.txt` | `b0ea442f29dee90584aacd3665a6e1ce87ffc917a4d31f34e1916d2d6f7f9205` |

Unicode is a trademark of Unicode, Inc. See the license terms in each file and
at https://www.unicode.org/terms_of_use.html.
