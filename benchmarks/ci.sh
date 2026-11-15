#!/bin/sh
# A small, deterministic benchmark job for CI: the Linux counterpart of ci.ps1.
#
# Builds Yashy and C versions of fib, checks the answer, and reports compile
# time, binary size, and wall-clock run time. The hello-world and ~1,000-line
# programs add the fixed build cost and the cost of a program with many items;
# wordcount and maps add the runtime rows that phase 4's inline paths are
# measured against, and large's run row is the release link's end-to-end time.
#
# Two rows here are assertions, not just numbers, because the phase 4 plan
# states them as claims and a claim with no gate cannot be corrected:
#
#   fib_wrap_ratio  `fib-wrap.ya` against `fib.c`, both medians, must be
#                   <= 1.20. This is the row the claim `fib <= 1.1x C` is
#                   about: `fib-wrap.ya` spells the same wrapping arithmetic
#                   C does, where `fib.ya`'s `+` is checked. `fib_c_run_ms`
#                   is therefore required, not context. The bound is 1.20
#                   because the Windows runner measures 1.17: the arithmetic
#                   is comparable and the excess is the executable's fixed
#                   cost (`hello_run_ms`), which `docs/gates.md` carries as
#                   the owned follow-up that closes at 1.10.
#   promotion       `promote.ya` has one allocation that never leaves its
#                   function and one that is returned; `--timings` must
#                   report exactly one promoted site. This is the row the
#                   allocation claim is measured on (see promote.ya).
#
# Usage:
#   sh benchmarks/ci.sh [output.bench]
#
# The gate rows go to the output file (or stdout when no file is named); the
# human report goes to stderr. clang and the C standard headers are required
# now that the C row is the denominator of `fib_wrap_ratio`.
#
# The output file is written before the claim assertions run, so a failed
# claim still uploads the measurement that explains it.
#
# BENCH_RUNS sets the sample count; the median is reported. One untimed warm-up
# runs first. BENCH_NO_OPT=<pass> disables one registered optimization pass in
# every build and records `no_opt=<pass>` in the measurement; that is the input
# half of `yashy-bench --per-pass`.
set -eu

bench=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(dirname "$bench")
build="$bench/build/ci"
yashy="$root/target/release/yashy"
runs=${BENCH_RUNS:-3}
output=${1:-}
no_opt=${BENCH_NO_OPT:-}
no_opt_arg=''
if [ -n "$no_opt" ]; then
    no_opt_arg="--no-opt=$no_opt"
fi

mkdir -p "$build"

if [ ! -x "$yashy" ]; then
    echo "no release binary at $yashy - run: cargo build --release -p yashy-driver" >&2
    exit 1
fi
if ! command -v clang >/dev/null 2>&1; then
    echo 'clang is not on PATH' >&2
    exit 1
fi

say() {
    printf '%s\n' "$*" >&2
}

now_ns() {
    date +%s%N
}

elapsed_ms() {
    awk -v ns="$1" 'BEGIN { printf "%.1f\n", ns / 1000000 }'
}

# The middle sample of a sorted list. With 3 runs that is the second; with 5,
# the third. A single sample is never the gate.
median_of() {
    sort -n "$1" | awk -v n="$2" 'NR == int((n + 1) / 2)'
}

file_bytes() {
    wc -c < "$1" | tr -d ' '
}

# Two medians as one ratio, two decimals. Division is in awk so the script
# needs no floating-point arithmetic of its own (POSIX sh has none).
ratio_of() {
    awk -v a="$1" -v b="$2" 'BEGIN {
        if (b <= 0) { print "0.00"; exit }
        printf "%.2f\n", a / b
    }'
}

# `ratio <= bound`, answered by awk's exit status so the comparison is the same
# on both operating systems.
ratio_within() {
    awk -v r="$1" -v bound="$2" 'BEGIN { exit !(r <= bound) }'
}

# Runs one build, and on failure prints what the build said.
#
# A gate that reports only "failed" leaves the CI log with no cause: the run
# that first tripped this on `ubuntu-22.04` named `yashy build fib` and nothing
# else, because both call sites below discarded the child's output. The output
# is small and this path runs once, so keep it and show it.
run_checked() {
    desc=$1
    shift
    status=0
    "$@" >"$build/failure.out" 2>&1 || status=$?
    if [ "$status" -ne 0 ]; then
        say "$desc failed (exit $status)"
        cat "$build/failure.out" >&2
        exit 1
    fi
}

# One untimed warm-up, then the median of $runs timed builds.
measure_compile() {
    desc=$1
    shift
    run_checked "$desc" "$@"
    samples=$(mktemp)
    i=0
    while [ "$i" -lt "$runs" ]; do
        start=$(now_ns)
        run_checked "$desc" "$@"
        end=$(now_ns)
        elapsed_ms $((end - start)) >> "$samples"
        i=$((i + 1))
    done
    median=$(median_of "$samples" "$runs")
    rm -f "$samples"
    printf '%s\n' "$median"
}

# The same, for a program that must print something: a run that produced no
# output is not a run (a refused binary would otherwise time as suspiciously
# fast). The program's own stderr is kept in `$outfile`, so a refusal that
# killed it is quoted rather than summarised.
measure_run() {
    desc=$1
    outfile=$2
    shift 2
    status=0
    "$@" >"$outfile" 2>&1 || status=$?
    if [ "$status" -ne 0 ]; then
        say "$desc failed (exit $status)"
        cat "$outfile" >&2
        exit 1
    fi
    [ -s "$outfile" ] || { say "$desc produced no output"; exit 1; }
    samples=$(mktemp)
    i=0
    while [ "$i" -lt "$runs" ]; do
        start=$(now_ns)
        "$@" >"$outfile" 2>&1 || { say "$desc failed (exit $?)"; exit 1; }
        end=$(now_ns)
        elapsed_ms $((end - start)) >> "$samples"
        i=$((i + 1))
    done
    median=$(median_of "$samples" "$runs")
    rm -f "$samples"
    printf '%s\n' "$median"
}

fib_yashy="$build/fib-yashy"
fib_wrap_yashy="$build/fib-wrap-yashy"
fib_c="$build/fib-c"
hello_yashy="$build/hello-yashy"
large_yashy="$build/large-yashy"
wordcount_yashy="$build/wordcount-yashy"
maps_yashy="$build/maps-yashy"
promote_yashy="$build/promote-yashy"
promote_log="$build/promote-timings.txt"
fib_out="$build/fib-yashy.out"
fib_wrap_out="$build/fib-wrap-yashy.out"
hello_out="$build/hello-yashy.out"
c_out="$build/fib-c.out"

fib_compile=$(measure_compile 'yashy build fib' "$yashy" build "$bench/fib.ya" -o "$fib_yashy" --release ${no_opt_arg:+"$no_opt_arg"})
hello_compile=$(measure_compile 'yashy build hello' "$yashy" build "$bench/hello.ya" -o "$hello_yashy" --release ${no_opt_arg:+"$no_opt_arg"})
large_compile=$(measure_compile 'yashy build large' "$yashy" build "$bench/large.ya" -o "$large_yashy" --release ${no_opt_arg:+"$no_opt_arg"})

# The container workloads are the runtime rows. Built once, untimed: their
# compile cost is the compiler's, not the runtime's.
run_checked 'yashy build wordcount' "$yashy" build "$bench/wordcount.ya" -o "$wordcount_yashy" --release ${no_opt_arg:+"$no_opt_arg"}
run_checked 'yashy build maps' "$yashy" build "$bench/maps.ya" -o "$maps_yashy" --release ${no_opt_arg:+"$no_opt_arg"}

# The comparable-integer program: its compile row is not claimed, so the build
# is untimed, but its run row is (see fib_wrap_ratio in the header).
run_checked 'yashy build fib-wrap' "$yashy" build "$bench/fib-wrap.ya" -o "$fib_wrap_yashy" --release ${no_opt_arg:+"$no_opt_arg"}

fib_run=$(measure_run 'yashy fib' "$fib_out" "$fib_yashy")
wordcount_out="$build/wordcount-yashy.out"
maps_out="$build/maps-yashy.out"
large_out="$build/large-yashy.out"
wordcount_run=$(measure_run 'yashy wordcount' "$wordcount_out" "$wordcount_yashy")
maps_run=$(measure_run 'yashy maps' "$maps_out" "$maps_yashy")
# large is already built for its compile row; its run row is the release
# link's end-to-end time.
large_run=$(measure_run 'yashy large' "$large_out" "$large_yashy")
# The executable's fixed cost, which is what a whole-process ratio is made of
# once the arithmetic is comparable: hello does no work at all.
hello_run=$(measure_run 'yashy hello' "$hello_out" "$hello_yashy")
# The numerator of fib_wrap_ratio.
fib_wrap_run=$(measure_run 'yashy fib-wrap' "$fib_wrap_out" "$fib_wrap_yashy")

# The answer is fixed, so a wrong program is a failed run rather than a fast
# one. The C row is context and may be unavailable on a machine whose clang
# cannot find the C headers.
if [ "$(cat "$fib_out")" != '9227465' ]; then
    say "yashy fib printed '$(cat "$fib_out")', expected 9227465"
    exit 1
fi
if [ "$(cat "$fib_wrap_out")" != '9227465' ]; then
    say "yashy fib-wrap printed '$(cat "$fib_wrap_out")', expected 9227465"
    exit 1
fi
if [ "$(cat "$hello_out")" != 'hello, world' ]; then
    say "yashy hello printed '$(cat "$hello_out")', expected 'hello, world'"
    exit 1
fi
if [ "$(cat "$wordcount_out")" != "900000
8" ]; then
    say "yashy wordcount printed '$(cat "$wordcount_out")', expected 900000 words and 8 distinct"
    exit 1
fi
if [ "$(cat "$maps_out")" != '134999550000' ]; then
    say "yashy maps printed '$(cat "$maps_out")', expected 134999550000"
    exit 1
fi
if [ "$(cat "$large_out")" != '123255' ]; then
    say "yashy large printed '$(cat "$large_out")', expected 123255"
    exit 1
fi

fib_bytes=$(file_bytes "$fib_yashy")
hello_bytes=$(file_bytes "$hello_yashy")

c_compile=''
c_run=''
c_bytes=''
c_compile=$(measure_compile 'clang build fib' clang -O2 "$bench/fib.c" -o "$fib_c") || {
    say 'the C row is required: it is the denominator of fib_wrap_ratio'
    exit 1
}
c_run=$(measure_run 'clang fib' "$c_out" "$fib_c") || {
    say 'the C row is required: it is the denominator of fib_wrap_ratio'
    exit 1
}
c_bytes=$(file_bytes "$fib_c")

fib_checked_ratio=$(ratio_of "$fib_run" "$c_run")
fib_wrap_ratio=$(ratio_of "$fib_wrap_run" "$c_run")

# The promotion row. `--timings` writes the pass counts to stderr; the build is
# counted, then the program is run once so a promotion that changed an answer
# is caught by the same row. `--no-opt=escape` turns promotion off, and then
# the counts are not printed at all: a per-pass run records no promotion row
# rather than a fake zero.
promoted_stack=''
promoted_arena=''
promoted_sites=''
if [ -z "$no_opt" ]; then
    status=0
    "$yashy" build "$bench/promote.ya" -o "$promote_yashy" --timings >"$promote_log" 2>&1 || status=$?
    if [ "$status" -ne 0 ]; then
        say "yashy build promote failed (exit $status)"
        cat "$promote_log" >&2
        exit 1
    fi
    promoted_stack=$(sed -n 's/.*promoted_stack=\([0-9][0-9]*\).*/\1/p' "$promote_log" | tail -n 1)
    promoted_arena=$(sed -n 's/.*promoted_arena=\([0-9][0-9]*\).*/\1/p' "$promote_log" | tail -n 1)
    if [ -z "$promoted_stack" ] || [ -z "$promoted_arena" ]; then
        say 'the promotion counts are missing from `yashy build --timings`'
        cat "$promote_log" >&2
        exit 1
    fi
    promoted_sites=$((promoted_stack + promoted_arena))
    promote_out="$build/promote-yashy.out"
    status=0
    "$promote_yashy" >"$promote_out" 2>&1 || status=$?
    if [ "$status" -ne 0 ] || [ "$(cat "$promote_out")" != '43' ]; then
        say "yashy promote printed '$(cat "$promote_out")' (exit $status), expected 43"
        exit 1
    fi
fi

say "C comparison (fib_wrap_ratio denominator): clang compile $c_compile ms, run $c_run ms, $c_bytes bytes"

say "fib output: $(cat "$fib_out")"
if [ -n "$no_opt" ]; then
    say "disabled pass: $no_opt"
fi
say "fib compile ms: $fib_compile (median of $runs)"
say "hello compile ms: $hello_compile"
say "large compile ms: $large_compile"
say "fib run ms: $fib_run"
say "wordcount run ms: $wordcount_run"
say "maps run ms: $maps_run"
say "release run ms: $large_run"
say "fib binary bytes: $fib_bytes"
say "hello binary bytes: $hello_bytes"
say "hello run ms: $hello_run (the executable's fixed cost)"
say "fib-wrap run ms: $fib_wrap_run"
say "fib vs C: $fib_checked_ratio x (checked arithmetic; not a gate)"
say "fib-wrap vs C: $fib_wrap_ratio x (limit 1.20; 1.10 is the owned follow-up)"
if [ -n "$no_opt" ]; then
    say "promotion: not measured (--no-opt=$no_opt)"
else
    say "promoted sites: $promoted_sites (stack $promoted_stack, arena $promoted_arena; expected 1)"
fi

emit_block() {
    printf '%s\n' '# generated by benchmarks/ci.sh; update deliberately'
    printf 'os=%s\n' linux
    printf 'generated_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    commit=$(git -C "$root" rev-parse --short HEAD 2>/dev/null || true)
    [ -n "$commit" ] || commit=unknown
    printf 'commit=%s\n' "$commit"
    printf 'toolchain=%s\n' "$(clang --version | head -n 1)"
    printf 'fib_compile_ms=%s\n' "$fib_compile"
    printf 'fib_binary_bytes=%s\n' "$fib_bytes"
    printf 'fib_run_ms=%s\n' "$fib_run"
    printf 'fib_c_run_ms=%s\n' "$c_run"
    printf 'fib_wrap_run_ms=%s\n' "$fib_wrap_run"
    printf 'fib_checked_ratio=%s\n' "$fib_checked_ratio"
    printf 'fib_wrap_ratio=%s\n' "$fib_wrap_ratio"
    printf 'hello_compile_ms=%s\n' "$hello_compile"
    printf 'hello_binary_bytes=%s\n' "$hello_bytes"
    printf 'hello_run_ms=%s\n' "$hello_run"
    printf 'large_compile_ms=%s\n' "$large_compile"
    printf 'wordcount_run_ms=%s\n' "$wordcount_run"
    printf 'maps_run_ms=%s\n' "$maps_run"
    printf 'release_run_ms=%s\n' "$large_run"
    if [ -n "$promoted_sites" ]; then
        printf 'promoted_stack=%s\n' "$promoted_stack"
        printf 'promoted_arena=%s\n' "$promoted_arena"
        printf 'promoted_sites=%s\n' "$promoted_sites"
    fi
    if [ -n "$no_opt" ]; then
        printf 'no_opt=%s\n' "$no_opt"
    fi
}

if [ -n "$output" ]; then
    mkdir -p "$(dirname "$output")"
    emit_block > "$output"
    say "wrote $output"
else
    emit_block
fi

# The claim assertions run last, on purpose: the file above is the evidence, and
# a run that fails a claim must still upload the numbers that failed it.

if ! ratio_within "$fib_wrap_ratio" 1.20; then
    say "fib_wrap_ratio is $fib_wrap_ratio ($fib_wrap_run ms vs C $c_run ms), limit 1.20"
    say 'fix the arithmetic path or correct the claim in docs/gates.md and the phase 4 plan'
    exit 1
fi

if [ -n "$promoted_sites" ] && [ "$promoted_sites" -ne 1 ]; then
    say "promote.ya promoted $promoted_sites sites (stack $promoted_stack, arena $promoted_arena), expected exactly 1"
    say 'one allocation stays in its function and one escapes; a different count is a real change in the pass'
    exit 1
fi
