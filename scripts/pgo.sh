#!/bin/sh
# Build every program in the PGO corpus twice: once with the plain ThinLTO
# release link, once with `--pgo`, and report what the profile changed.
#
# A profile is produced by running a program, so it is per program: each corpus
# member trains on itself. The corpus exists so the two-pass build is exercised
# across the language's runtime-heavy features rather than on one toy. The
# profile a build writes lives under `<output-dir>/.yashy/pgo/`, which this
# script checks before it believes the build.
#
# Usage:
#   sh scripts/pgo.sh [output file]
#
# BENCH_RUNS sets the sample count; the median is reported. Correctness is the
# gate: a corpus program that prints the wrong text, or a `--pgo` build that
# writes no profile, fails the script. The size and speed columns are the
# plan's report card, not a timer gate.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
yashy="$root/target/release/yashy"
corpus="$root/benchmarks/pgo"
build="$root/benchmarks/build/pgo"
runs=${BENCH_RUNS:-5}
output=${1:-}

if [ ! -x "$yashy" ]; then
    echo "no release binary at $yashy - run: cargo build --release -p yashy-driver" >&2
    exit 1
fi
if [ ! -d "$corpus" ]; then
    echo "no PGO corpus at $corpus" >&2
    exit 1
fi
mkdir -p "$build"

say() {
    printf '%s\n' "$*"
}

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

now_ns() {
    date +%s%N
}

elapsed_ms() {
    awk -v ns="$1" 'BEGIN { printf "%.1f\n", ns / 1000000 }'
}

median_of() {
    sort -n "$1" | awk -v n="$2" 'NR == int((n + 1) / 2)'
}

percent() {
    awk -v before="$1" -v after="$2" 'BEGIN {
        if (before == 0) { printf "0.0\n" } else { printf "%.1f\n", (after - before) * 100 / before }
    }'
}

speedup() {
    awk -v baseline="$1" -v candidate="$2" 'BEGIN {
        if (baseline == 0) { printf "0.0\n" } else { printf "%.1f\n", (baseline - candidate) * 100 / baseline }
    }'
}

# The text each corpus program prints. A profile must not change an answer, so
# every run is compared with these before it is timed.
expected_for() {
    case "$1" in
        hello) printf 'hello, world' ;;
        fib) printf '6765' ;;
        wordcount) printf '9000\n8' ;;
        maps) printf '14999850000' ;;
        json_text) printf '"a \\"quoted\\" line\\n"\n210000' ;;
        strings) printf '900000' ;;
        *) return 1 ;;
    esac
}

# Run once, check the exit status and the fixed text.
assert_output() {
    exe=$1
    want=$2
    what=$3
    "$exe" >"$build/out.txt" 2>&1 || fail "$what failed (exit $?)"
    got=$(cat "$build/out.txt")
    if [ "$got" != "$want" ]; then
        fail "$what printed '$got', expected '$want'"
    fi
}

# One untimed warm-up, then the median of $runs timed runs.
measure_median() {
    exe=$1
    "$exe" >/dev/null 2>&1
    samples=$(mktemp)
    i=0
    while [ "$i" -lt "$runs" ]; do
        start=$(now_ns)
        "$exe" >/dev/null 2>&1
        end=$(now_ns)
        elapsed_ms $((end - start)) >> "$samples"
        i=$((i + 1))
    done
    median=$(median_of "$samples" "$runs")
    rm -f "$samples"
    printf '%s\n' "$median"
}

rows=$(mktemp)
printf '%-12s %8s %8s %8s %9s %9s %8s\n' \
    'program' 'lto ms' 'pgo ms' 'faster%' 'lto bytes' 'pgo bytes' 'size%' > "$rows"

for file in "$corpus"/*.ya; do
    stem=$(basename "$file" .ya)
    want=$(expected_for "$stem") || fail "no expected output for $stem; add it to the corpus table"
    lto="$build/$stem-lto"
    pgo="$build/$stem-pgo"

    say "== $stem"
    "$yashy" build "$file" -o "$lto" --release >/dev/null 2>&1 \
        || fail "yashy build $stem (ThinLTO) failed"
    "$yashy" build "$file" -o "$pgo" --release --pgo >/dev/null 2>&1 \
        || fail "yashy build $stem (PGO) failed"

    profile="$build/.yashy/pgo/$stem.profdata"
    [ -f "$profile" ] || fail "the PGO build of $stem wrote no profile at $profile"

    assert_output "$lto" "$want" "$stem (ThinLTO)"
    assert_output "$pgo" "$want" "$stem (PGO)"

    lto_ms=$(measure_median "$lto")
    pgo_ms=$(measure_median "$pgo")
    lto_bytes=$(wc -c < "$lto" | tr -d ' ')
    pgo_bytes=$(wc -c < "$pgo" | tr -d ' ')
    # A positive "faster" means PGO ran in less time, relative to the baseline.
    faster=$(speedup "$lto_ms" "$pgo_ms")
    size=$(percent "$lto_bytes" "$pgo_bytes")
    printf '%-12s %8s %8s %8s %9s %9s %8s\n' \
        "$stem" "$lto_ms" "$pgo_ms" "$faster" "$lto_bytes" "$pgo_bytes" "$size" >> "$rows"
done

say ""
cat "$rows"
say ""
say "profiles under $build/.yashy/pgo"

if [ -n "$output" ]; then
    mkdir -p "$(dirname "$output")"
    cat "$rows" > "$output"
    say "wrote $output"
else
    rm -f "$rows"
fi
