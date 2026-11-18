# CI templates

Two CI systems are supported: GitHub Actions (`.github/workflows/ci.yml`) and
GitLab CI (`ci/gitlab-ci.yml`). A third entry point, `ci/generic.sh`, runs the
same stages locally or in any runner that can execute a shell script.

## Stages

| Stage | Command | Gate |
|---|---|---|
| check | `cargo check --workspace --all-targets --locked` | no compiler warning may become an error in a new file |
| verify | `yashyc verify --json --inventory examples/hello/main.ya` | exit 0, empty diagnostic summary |
| build | `cargo build --release --workspace --locked` | links on both OSes |
| test | `cargo test --workspace --locked` | every suite green |
| fmt | `cargo fmt --all -- --check` | touched files only; known pre-existing drift is tracked separately |
| coverage | `yashyc coverage --json --fail-under 70 examples/hello` | the configured floor |
| benchmark gate | `benchmarks/ci.sh` | every claimed row present, no regression past its threshold |
| fuzz smoke | `yashyc fuzz --runs 1000` | no finding |

A stage that depends on a tool the machine does not have prints `SKIP <reason>`
instead of pretending it ran. A SKIP is visible in the log; an accidental pass
is not.

## Matrices

Windows and Linux are separate jobs, not one job with a conditional shell. The
cache key includes the Rust version and the target triple, so a toolchain change
starts a new cache rather than reusing stale objects. Every Cargo call uses
`--locked`: the lockfile decides what is built, not a fresh dependency
resolution.

## Artifacts and uploads

Coverage is emitted as JSON and as SARIF, uploaded from the `report` stage. The
traceability matrix from `yashyc trace --json` is an artifact of the same stage
so a reviewer can see coverage per rule id without rerunning CI.

## Testing the template itself

A fixture repository runs these templates end to end; a broken template is a
gate failure in that repository, not a quiet no-op here. Until that fixture is
in place, `ci/generic.sh` is the honest local smoke test: run it from the repo
root and read every SKIP line.
