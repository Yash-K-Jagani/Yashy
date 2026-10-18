<#
.SYNOPSIS
  Build and time the Otter benchmark suite against C, Rust, Go, Node and Python.

.DESCRIPTION
  Two questions are measured, and they are different questions:

    1. How fast is the code Otter produces?   -> the runtime table
    2. How fast does Otter compile it?        -> the compile-time table

  For (1) every language runs the same algorithm with the same input, compiled
  with its own native optimizer at -O2 (Rust -O, which is the same thing). Node
  and Python run the same source at the same input and are reported on their own
  rows, because an interpreting runtime is a different category and pretending
  otherwise would flatter the native numbers.

  Otter is additionally built at -O0, so the table shows what enabling
  optimization actually buys rather than just asserting that it helps.

  Four rows carry a machine flag, in two pairs: `--native` / `-march=native`,
  which build for the CPU of the machine running the benchmark, and
  `--cpu=x86-64-v3` / `-march=x86-64-v3`, which build for an AVX2-era CPU
  instead of the architecture's baseline. Each pair is always listed together,
  one Otter and one clang, because the gain comes from the instruction set
  rather than from the compiler, and none of them is comparable with the
  portable rows above it.

  Timing is the minimum of N runs: for a short-lived process the minimum is the
  run that was least disturbed by the rest of the machine, and it is far more
  stable than the mean when other processes compete for the CPU. The median is
  printed beside it as a check on how noisy the machine was.

.PARAMETER Runs
  Timed runs per native binary, after one untimed warm-up run. The minimum and
  median are reported. Default 5.

.PARAMETER InterpRuns
  Timed runs per interpreted program (Node, CPython). Default 3: these rows cost
  seconds each and their run-to-run variance is negligible next to the gap
  between an interpreter and native code.

.PARAMETER SkipBuild
  Reuse the binaries already in benchmarks/build instead of rebuilding.

.EXAMPLE
  pwsh benchmarks/run.ps1
  pwsh benchmarks/run.ps1 -Runs 3
#>
param(
    [int]$Runs = 5,
    [int]$InterpRuns = 3,
    [switch]$SkipBuild
)

# Not 'Stop': in Windows PowerShell a native command writing to stderr becomes a
# terminating error record under 'Stop', and several of the tools here write
# progress and warnings to stderr. Failures are detected from the exit code and
# thrown explicitly instead, with the tool's own output attached.
$ErrorActionPreference = 'Continue'

$bench = $PSScriptRoot
$root = Split-Path -Parent $bench
$build = Join-Path $bench 'build'
$otter = Join-Path $root 'target\release\otter.exe'
$exe = '.exe'

# Input sizes, and the work each one implies. `fib` reports calls into the
# recursive tree, not the argument: fib(35) is ~30 million calls, and calls are
# what the benchmark actually measures.
$fibCalls = 2 * 14930352 - 1   # 2 * F(36) - 1, the node count of fib(35)
$work = @{ fib = $fibCalls; loop = 200000000; float = 100000000 }
$benchmarks = @('fib', 'loop', 'float')

New-Item -ItemType Directory -Force -Path $build | Out-Null

# Compile the C benchmarks for the same target the compiler targets.
#
# This is not cosmetic. On a Windows machine with no MSVC CRT, bare `clang`
# defaults to the MSVC triple and fails with "'stdio.h' file not found". Otter
# passes the host triple to clang on every build, so the C rows are compiled the
# same way; the triple is read back out of Otter's own IR so the two cannot
# drift apart. On a host whose triple needs no help (Linux), no `--target` is
# passed and clang uses its default.
$clangTarget = @()
if (Test-Path $otter) {
    $triple = & $otter build "$bench\fib.ot" --emit-ir 2>$null |
        Select-String -Pattern '^target triple = ' |
        ForEach-Object { ($_.Line -split '"')[1] }
    if ($triple -like '*windows*') {
        $clangTarget = @("--target=$triple")
    }
}

function Invoke-Tool {
    param([string]$Exe, [string[]]$Arguments, [string]$Step)
    $output = (& $Exe @Arguments 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "$Step failed (exit $LASTEXITCODE):`n$Exe $($Arguments -join ' ')`n$output"
    }
}

function Build-All {
    if (-not (Test-Path $otter)) {
        throw "no release binary at $otter - run: cargo build --release --workspace"
    }

    foreach ($name in $benchmarks) {
        # Otter at the old default (-O0) and the new one (-O2), so the runtime
        # table shows the effect of the change instead of asserting it.
        Invoke-Tool $otter @('build', "$bench\$name.ot", '-o', "$build\$name-otter-O0$exe", '-O0') "otter build $name -O0"
        Invoke-Tool $otter @('build', "$bench\$name.ot", '-o', "$build\$name-otter-O2$exe") "otter build $name -O2"

        # `--native` targets the machine that builds, so its C counterpart is
        # compiled with `-march=native` too. The pair answers "what is the flag
        # worth" without either side quietly receiving a flag the other did not.
        Invoke-Tool $otter @('build', "$bench\$name.ot", '-o', "$build\$name-otter-native$exe", '--native') "otter build $name --native"
        Invoke-Tool 'clang' ($clangTarget + @('-O2', '-march=native', "$bench\$name.c", '-o', "$build\$name-clang-native$exe")) "clang $name -march=native"

        # `x86-64-v3` is the same idea with the portability spent more carefully:
        # an AVX2-era CPU rather than this exact one, so the binary is the same
        # on every machine of that generation. It is the tier to compare when
        # `--native`'s numbers look unrepeatable across two machines.
        Invoke-Tool $otter @('build', "$bench\$name.ot", '-o', "$build\$name-otter-v3$exe", '--cpu=x86-64-v3') "otter build $name --cpu=x86-64-v3"
        Invoke-Tool 'clang' ($clangTarget + @('-O2', '-march=x86-64-v3', "$bench\$name.c", '-o', "$build\$name-clang-v3$exe")) "clang $name -march=x86-64-v3"

        # The `v3` pair above is *not* a like-for-like comparison on its own.
        # Clang contracts floating-point by default (C's `-ffp-contract=on`), so
        # its `v3` row already has a fused multiply-add, while Otter has to be
        # asked for one. This row is Otter asking, and it is the one to read
        # beside `C (clang -O2, -march=x86-64-v3)`.
        Invoke-Tool $otter @('build', "$bench\$name.ot", '-o', "$build\$name-otter-v3-fast$exe", '--cpu=x86-64-v3', '--fast-math') "otter build $name --cpu=x86-64-v3 --fast-math"

        # Two C compilers, so the Otter numbers are not graded against one
        # backend's quirks. clang gets the same target Otter passes.
        Invoke-Tool 'clang' ($clangTarget + @('-O2', "$bench\$name.c", '-o', "$build\$name-clang$exe")) "clang $name"
        Invoke-Tool 'gcc' @('-O2', "$bench\$name.c", '-o', "$build\$name-gcc$exe") "gcc $name"

        Invoke-Tool 'rustc' @('-O', "$bench\$name.rs", '-o', "$build\$name-rust$exe") "rustc $name"
        Invoke-Tool 'go' @('build', '-o', "$build\$name-go$exe", "$bench\$name.go") "go build $name"
    }
}

function Measure-Runs {
    param([string]$Exe, [string[]]$Arguments, [int]$Count)
    # One untimed warm-up. The first execution of a file that was written
    # seconds ago pays for it not being in the OS cache, and that cost has
    # nothing to do with the language.
    $warm = (& $Exe @Arguments 2>$null) -join ''
    # A binary the machine's Application Control policy refuses produces no
    # output at all, and a refusal is fast. Timing it would record a blocked
    # binary as the quickest row in the table, so a run with no output is
    # reported as a failure instead of a measurement.
    if ([string]::IsNullOrWhiteSpace($warm)) {
        return [pscustomobject]@{ Min = $null; Median = $null; Output = ''; Blocked = $true }
    }
    $times = New-Object System.Collections.Generic.List[double]
    $last = ''
    for ($i = 0; $i -lt $Count; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $last = (& $Exe @Arguments 2>$null) -join ''
        $sw.Stop()
        $times.Add($sw.Elapsed.TotalMilliseconds)
    }
    $sorted = @($times | Sort-Object)
    [pscustomobject]@{
        Min     = [math]::Round($sorted[0], 1)
        Median  = [math]::Round($sorted[[int][math]::Floor($sorted.Count / 2)], 1)
        Output  = $last.Trim()
        Blocked = $false
    }
}

function Invoke-Runtime {
    $rows = @()
    foreach ($name in $benchmarks) {
        $programs = @(
            @{ Lang = 'Otter (clang -O0)'; Path = "$build\$name-otter-O0$exe" },
            @{ Lang = 'Otter (clang -O2)'; Path = "$build\$name-otter-O2$exe" },
            @{ Lang = 'Otter (-O2, --native)'; Path = "$build\$name-otter-native$exe" },
            @{ Lang = 'Otter (-O2, --cpu=x86-64-v3)'; Path = "$build\$name-otter-v3$exe" },
            @{ Lang = 'C (clang -O2)';     Path = "$build\$name-clang$exe" },
            @{ Lang = 'C (clang -O2, -march=native)'; Path = "$build\$name-clang-native$exe" },
            @{ Lang = 'Otter (-O2, --cpu=x86-64-v3 --fast-math)'; Path = "$build\$name-otter-v3-fast$exe" },
            @{ Lang = 'C (clang -O2, -march=x86-64-v3)'; Path = "$build\$name-clang-v3$exe" },
            @{ Lang = 'C (gcc -O2)';       Path = "$build\$name-gcc$exe" },
            @{ Lang = 'Rust (rustc -O)';   Path = "$build\$name-rust$exe" },
            @{ Lang = 'Go (go build)';     Path = "$build\$name-go$exe" },
            @{ Lang = 'Node 24';           Path = 'node';   Args = @("$bench\$name.mjs") },
            @{ Lang = 'CPython 3.14';      Path = 'python'; Args = @("$bench\$name.py") }
        )
        foreach ($program in $programs) {
            $extra = @()
            if ($program.ContainsKey('Args')) { $extra = [string[]]$program.Args }
            $count = if ($program.ContainsKey('Args')) { $InterpRuns } else { $Runs }
            $result = Measure-Runs $program.Path $extra $count
            # A blocked row sorts to the bottom and says so, rather than being
            # reported as a fast row that silently did no work.
            $min = if ($result.Blocked) { [double]::PositiveInfinity } else { $result.Min }
            $label = if ($result.Blocked) { "$($program.Lang) -- BLOCKED" } else { $program.Lang }
            $rows += [pscustomobject]@{
                Benchmark = $name
                Language  = $label
                MinMs     = $min
                MedianMs  = $result.Median
                NsPerOp   = if ($result.Blocked) { $null } else { [math]::Round(($min * 1e6) / $work[$name], 3) }
                Output    = $result.Output
            }
        }
    }
    $rows
}

function Invoke-CompileTime {
    $cases = @(
        @{ Lang = 'Otter (front end only)'; Exe = $otter;  Args = @('build', "$bench\fib.ot", '--emit-ir') },
        @{ Lang = 'Otter (full, -O2)';      Exe = $otter;  Args = @('build', "$bench\fib.ot", '-o', "$build\ct-otter$exe") },
        @{ Lang = 'C (clang -O2)';          Exe = 'clang'; Args = ($clangTarget + @('-O2', "$bench\fib.c", '-o', "$build\ct-clang$exe")) },
        @{ Lang = 'Rust (rustc -O)';        Exe = 'rustc'; Args = @('-O', "$bench\fib.rs", '-o', "$build\ct-rust$exe") },
        @{ Lang = 'Go (go build)';          Exe = 'go';    Args = @('build', '-o', "$build\ct-go$exe", "$bench\fib.go") }
    )
    $rows = @()
    foreach ($case in $cases) {
        $program = $case.Exe
        $arguments = [string[]]$case.Args
        $times = New-Object System.Collections.Generic.List[double]
        for ($i = 0; $i -lt $Runs; $i++) {
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            & $program @arguments 2>$null | Out-Null
            $sw.Stop()
            $times.Add($sw.Elapsed.TotalMilliseconds)
        }
        $sorted = @($times | Sort-Object)
        $rows += [pscustomobject]@{
            Toolchain = $case.Lang
            MinMs     = [math]::Round($sorted[0], 1)
            MedianMs  = [math]::Round($sorted[[int][math]::Floor($sorted.Count / 2)], 1)
        }
    }
    $rows
}

if (-not $SkipBuild) {
    Write-Host 'building benchmarks...'
    Build-All
}

Write-Host ''
Write-Host "== runtime (best of $Runs runs) =="
Invoke-Runtime | Sort-Object Benchmark, NsPerOp |
    Format-Table Benchmark, Language, MinMs, MedianMs, NsPerOp, Output -AutoSize

Write-Host "== compile time (fib, best of $Runs runs) =="
Invoke-CompileTime | Sort-Object MinMs | Format-Table Toolchain, MinMs, MedianMs -AutoSize

Write-Host 'done.'
