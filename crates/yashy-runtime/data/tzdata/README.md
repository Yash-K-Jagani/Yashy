# Vendored time-zone data

- tzdata release: 2026c
- source: Python `zoneinfo` from the pinned system package
- generated header: `crates/yashy-runtime/src/zone_data.h`
- SHA-256 of the header: `33ee8057c85e50eefc234b199846e6c71aa86577cd8f7666c6672c018d08d94f`
- zones: UTC, America/New_York, Europe/London, Asia/Tokyo, Australia/Sydney
- range: 1970-01-01 through 2100-01-01, second-resolution transitions

Regenerate with `TZDATA_VERSION=<release> python3 scripts/gen_zone_data.py`.
