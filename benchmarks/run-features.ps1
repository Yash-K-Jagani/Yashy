<#
.SYNOPSIS
  Build and time the language-feature benchmarks: FFI, arrays, maps, enums,
  channels, mutexes and function values.

.DESCRIPTION
  The scalar suite (run.ps1) answers "how fast is the arithmetic Yashy emits?".
  This suite answers a different question: "what does each language feature
  cost, and how does it compare with the same idea written in another
  language?". Every row runs the same algorithm with the same input and the
  same output; where a feature has no comparable standard-library equivalent
  (a C hash map, a user-level shared pointer across threads) the row is
  omitted and the README says why.

  Programs are built at -O2. Yashy's binary is benchmarks/build/features/*;
  the C rows use the same target triple Yashy passes to clang, and gcc, rustc
  and go build themselves as usual.

  Timing is the minimum of N runs after one untimed warm-up, and each table is
  measured in two passes; the better of the two minima is reported. This is the
  same methodology as run.ps1, and it exists for the same reason: the machine's
  clock varies between windows. A blocked binary produces no output and is
  marked BLOCKED rather than being timed as a suspiciously fast row.

.PARAMETER Runs
  Timed runs per binary, after one untimed warm-up. Default 5.

.PARAMETER Passes
  Full passes over the suite. The best minimum per row wins. Default 2.

.PARAMETER SkipBuild
  Reuse the binaries already in benchmarks/build/features.

.EXAMPLE
  pwsh benchmarks/run-features.ps1
  pwsh benchmarks/run-features.ps1 -Runs 3 -Passes 1
#>
param(
    [int]$Runs = 5,
    [int]$Passes = 2,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Continue'

$bench = $PSScriptRoot
$root = Split-Path -Parent $bench
$features = Join-Path $bench 'features'
$build = Join-Path $bench 'build\features'
$yashy = Join-Path $root 'target\release\yashy.exe'
$exe = '.exe'

# One workload per feature. `Work` is the number of operations the printed
# number is divided by; for the map it counts both the insert and the lookup,
# and for the mutex it counts lock/unlock pairs. `Expected` is the output every
# language must print -- a wrong hash, a lost update, or an array growth bug
# shows up as a mismatch instead of a speed number.
$workloads = @(
    @{ Name = 'ffi';     Work = 100000000; Unit = 'call';       Expected = '100000000'; Source = 'ffi.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'ffi.ya'; Out = 'ffi-yashy' },
           @{ Lang = 'C (clang -O2)';   Kind = 'clang'; Src = 'ffi.c';  Out = 'ffi-clang' },
           @{ Lang = 'C (gcc -O2)';     Kind = 'gcc';   Src = 'ffi.c';  Out = 'ffi-gcc' }
       ) },
    @{ Name = 'array';   Work = 10000000;  Unit = 'element';    Expected = '49999995000000';   Source = 'array.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'array.ya'; Out = 'array-yashy' },
           @{ Lang = 'C (clang -O2)';   Kind = 'clang'; Src = 'array.c'; Out = 'array-clang' },
           @{ Lang = 'C (gcc -O2)';     Kind = 'gcc';   Src = 'array.c'; Out = 'array-gcc' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'array.go'; Out = 'array-go' },
           @{ Lang = 'Rust (rustc -O)'; Kind = 'rust';  Src = 'array.rs'; Out = 'array-rust' }
       ) },
    @{ Name = 'map';     Work = 2000000;    Unit = 'operation'; Expected = '1499998500000';    Source = 'map.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'map.ya'; Out = 'map-yashy' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'map.go'; Out = 'map-go' },
           @{ Lang = 'Rust (rustc -O)'; Kind = 'rust';  Src = 'map.rs'; Out = 'map-rust' }
       ) },
    @{ Name = 'enum';    Work = 50000000;  Unit = 'iteration';  Expected = '1250000008333333'; Source = 'enum.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'enum.ya'; Out = 'enum-yashy' },
           @{ Lang = 'C (clang -O2)';   Kind = 'clang'; Src = 'enum.c'; Out = 'enum-clang' },
           @{ Lang = 'C (gcc -O2)';     Kind = 'gcc';   Src = 'enum.c'; Out = 'enum-gcc' }
       ) },
    @{ Name = 'threads'; Work = 200000;    Unit = 'round trip'; Expected = '20000100000';      Source = 'threads.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'threads.ya'; Out = 'threads-yashy' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'threads.go'; Out = 'threads-go' }
       ) },
    @{ Name = 'mutex';   Work = 1000000;   Unit = 'lock pair';  Expected = '1000000';          Source = 'mutex.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'mutex.ya'; Out = 'mutex-yashy' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'mutex.go'; Out = 'mutex-go' }
       ) },
    # The function-value workload has two rows per language: one that calls
    # through a function value and one that calls the same function directly.
    # The gap between them is the price of the value. Every row prints the same
    # number, so a row that computed the wrong thing shows up as a mismatch
    # instead of as a speed number.
    @{ Name = 'function'; Work = 100000000; Unit = 'call';       Expected = '4999999950099999999'; Source = 'function.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2 (value)';         Kind = 'yashy'; Src = 'function.ya';         Out = 'function-yashy' },
           @{ Lang = 'Yashy -O2 (direct)';        Kind = 'yashy'; Src = 'function_direct.ya'; Out = 'function-direct-yashy' },
           @{ Lang = 'Yashy -O2 (lambda)';        Kind = 'yashy'; Src = 'function_lambda.ya'; Out = 'function-lambda-yashy' },
           @{ Lang = 'Yashy -O2 (closure)';       Kind = 'yashy'; Src = 'function_closure.ya'; Out = 'function-closure-yashy' },
           @{ Lang = 'C (clang -O2, value)';      Kind = 'clang'; Src = 'function.c';         Out = 'function-clang' },
           @{ Lang = 'C (clang -O2, direct)';     Kind = 'clang'; Src = 'function_direct.c'; Out = 'function-direct-clang' },
           @{ Lang = 'C (gcc -O2, value)';        Kind = 'gcc';   Src = 'function.c';         Out = 'function-gcc' },
           @{ Lang = 'C (gcc -O2, direct)';       Kind = 'gcc';   Src = 'function_direct.c'; Out = 'function-direct-gcc' },
           @{ Lang = 'Go (go build, value)';      Kind = 'go';    Src = 'function.go';        Out = 'function-go' },
           @{ Lang = 'Go (go build, direct)';     Kind = 'go';    Src = 'function_direct.go'; Out = 'function-direct-go' },
           @{ Lang = 'Rust (rustc -O, value)';    Kind = 'rust';  Src = 'function.rs';        Out = 'function-rust' },
           @{ Lang = 'Rust (rustc -O, direct)';   Kind = 'rust';  Src = 'function_direct.rs'; Out = 'function-direct-rust' }
       ) },
    # A mutating method: one call per iteration, each handing the object's
    # address to a method that increments one field. The same loop in every
    # language, so the number is the price of a pointer receiver rather than
    # of an algorithm.
    @{ Name = 'method';  Work = 10000000;  Unit = 'call';       Expected = '10000000';        Source = 'method.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'method.ya'; Out = 'method-yashy' },
           @{ Lang = 'C (clang -O2)';   Kind = 'clang'; Src = 'method.c';  Out = 'method-clang' },
           @{ Lang = 'C (gcc -O2)';     Kind = 'gcc';   Src = 'method.c';  Out = 'method-gcc' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'method.go'; Out = 'method-go' },
           @{ Lang = 'Rust (rustc -O)'; Kind = 'rust';  Src = 'method.rs'; Out = 'method-rust' }
       ) },
    # A generic function: one body, called with one type. The direct row is
    # the same loop with the type written out, so the gap between the two rows
    # is the cost of the type parameter -- which monomorphization should make
    # nothing.
    @{ Name = 'generics'; Work = 100000000; Unit = 'call';       Expected = '4999999950099999999'; Source = 'generics.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2 (generic)';     Kind = 'yashy'; Src = 'generics.ya';        Out = 'generics-yashy' },
           @{ Lang = 'Yashy -O2 (direct)';      Kind = 'yashy'; Src = 'generics_direct.ya'; Out = 'generics-direct-yashy' },
           @{ Lang = 'C (clang -O2)';           Kind = 'clang'; Src = 'generics.c';          Out = 'generics-clang' },
           @{ Lang = 'C (gcc -O2)';             Kind = 'gcc';   Src = 'generics.c';          Out = 'generics-gcc' },
           @{ Lang = 'Go (go build)';           Kind = 'go';    Src = 'generics.go';         Out = 'generics-go' },
           @{ Lang = 'Rust (rustc -O)';         Kind = 'rust';  Src = 'generics.rs';         Out = 'generics-rust' }
       ) },
    # A fixed array: eight elements in the frame, summed by an inner `for`.
    # There is no allocation and no capacity in the loop, so the gap to the C
    # row is the cost of iterating an aggregate the compiler knows the length
    # of rather than of a growable array.
    @{ Name = 'fixed_array'; Work = 400000000; Unit = 'element'; Expected = '1800000000'; Source = 'fixed_array.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2 (checked +)'; Kind = 'yashy'; Src = 'fixed_array.ya';          Out = 'fixed_array-yashy' },
           @{ Lang = 'Yashy -O2 (wrapping)';  Kind = 'yashy'; Src = 'fixed_array_wrapping.ya'; Out = 'fixed_array-wrapping-yashy' },
           @{ Lang = 'C (clang -O2)';   Kind = 'clang'; Src = 'fixed_array.c';  Out = 'fixed_array-clang' },
           @{ Lang = 'C (gcc -O2)';     Kind = 'gcc';   Src = 'fixed_array.c';  Out = 'fixed_array-gcc' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'fixed_array.go'; Out = 'fixed_array-go' },
           @{ Lang = 'Rust (rustc -O)'; Kind = 'rust';  Src = 'fixed_array.rs'; Out = 'fixed_array-rust' }
       ) },
    # A tuple: `min_max` returns two i64s as one value, the caller destructures
    # it, and both halves feed the accumulator. The C row returns the same pair
    # as a struct by value, Go returns two values, and Rust returns a tuple.
    # There is no allocation anywhere, so the row measures the aggregate return
    # and the extraction, not a data structure.
    @{ Name = 'tuple'; Work = 50000000; Unit = 'iteration'; Expected = '3750000000000000'; Source = 'tuple.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'tuple.ya'; Out = 'tuple-yashy' },
           @{ Lang = 'C (clang -O2)';   Kind = 'clang'; Src = 'tuple.c';  Out = 'tuple-clang' },
           @{ Lang = 'C (gcc -O2)';     Kind = 'gcc';   Src = 'tuple.c';  Out = 'tuple-gcc' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'tuple.go'; Out = 'tuple-go' },
           @{ Lang = 'Rust (rustc -O)'; Kind = 'rust';  Src = 'tuple.rs'; Out = 'tuple-rust' }
       ) },
    # The array mapping: the same loop in every language, building a second
    # array the size of the first.
    @{ Name = 'map_array'; Work = 10000000; Unit = 'element'; Expected = '99999990000000'; Source = 'map_array.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'map_array.ya'; Out = 'map_array-yashy' },
           @{ Lang = 'C (clang -O2)';   Kind = 'clang'; Src = 'map_array.c';  Out = 'map_array-clang' },
           @{ Lang = 'C (gcc -O2)';     Kind = 'gcc';   Src = 'map_array.c';  Out = 'map_array-gcc' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'map_array.go'; Out = 'map_array-go' },
           @{ Lang = 'Rust (rustc -O)'; Kind = 'rust';  Src = 'map_array.rs'; Out = 'map_array-rust' }
       ) },
    # Walking a map: the table's keys and values copied into two arrays, ten
    # times over, and a probe of a key that is there and one that is not.
    # Yashy's `keys()`/`values()` copy the whole table; Go's row collects into
    # two slices and Rust's into two vectors, which is the same work.
    @{ Name = 'map_scan'; Work = 20000000; Unit = 'element'; Expected = '19999981000000'; Source = 'map_scan.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'map_scan.ya'; Out = 'map_scan-yashy' },
           @{ Lang = 'Go (go build)';   Kind = 'go';    Src = 'map_scan.go'; Out = 'map_scan-go' },
           @{ Lang = 'Rust (rustc -O)'; Kind = 'rust';  Src = 'map_scan.rs'; Out = 'map_scan-rust' }
       ) },
    # A debug-formatted value: `${value:?}` builds the text a programmer reads.
    # Rust's derived `Debug` prints the same shape, so the two rows are the
    # same work; there is no C row because C has no equivalent.
    @{ Name = 'debug_format'; Work = 1000000; Unit = 'format'; Expected = '30333335'; Source = 'debug_format.ya';
       Programs = @(
           @{ Lang = 'Yashy -O2';       Kind = 'yashy'; Src = 'debug_format.ya'; Out = 'debug_format-yashy' },
           @{ Lang = 'Rust (rustc -O)'; Kind = 'rust';  Src = 'debug_format.rs'; Out = 'debug_format-rust' }
       ) }
)

New-Item -ItemType Directory -Force -Path $build | Out-Null

# The C rows must target the same triple Yashy does; on a machine with no MSVC
# CRT a bare clang picks the MSVC triple and cannot find stdio.h. Read the
# triple out of Yashy's own IR so the two cannot drift.
$clangTarget = @()
if (Test-Path $yashy) {
    $triple = & $yashy build (Join-Path $features 'ffi.ya') --emit-ir 2>$null |
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
    if (-not (Test-Path $yashy)) {
        throw "no release binary at $yashy - run: cargo build --release --workspace"
    }
    foreach ($workload in $workloads) {
        foreach ($program in $workload.Programs) {
            $src = Join-Path $features $program.Src
            $out = Join-Path $build "$($program.Out)$exe"
            switch ($program.Kind) {
                'yashy' { Invoke-Tool $yashy @('build', $src, '-o', $out) "yashy build $($program.Src)" }
                'clang' { Invoke-Tool 'clang' ($clangTarget + @('-O2', $src, '-o', $out)) "clang $($program.Src)" }
                'gcc'   { Invoke-Tool 'gcc' @('-O2', $src, '-o', $out) "gcc $($program.Src)" }
                'rust'  { Invoke-Tool 'rustc' @('-O', $src, '-o', $out) "rustc $($program.Src)" }
                'go'    { Invoke-Tool 'go' @('build', '-o', $out, $src) "go build $($program.Src)" }
                default { throw "unknown build kind: $($program.Kind)" }
            }
        }
    }
}

function Measure-Runs {
    param([string]$Path, [string[]]$Arguments, [int]$Count)
    $warm = (& $Path @Arguments 2>$null) -join ''
    if ([string]::IsNullOrWhiteSpace($warm)) {
        return [pscustomobject]@{ Min = $null; Median = $null; Output = ''; Blocked = $true }
    }
    $times = New-Object System.Collections.Generic.List[double]
    $last = ''
    for ($i = 0; $i -lt $Count; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $last = (& $Path @Arguments 2>$null) -join ''
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

if (-not $SkipBuild) {
    Write-Host 'building feature benchmarks...'
    Build-All
}

$rows = @{}
for ($pass = 1; $pass -le $Passes; $pass++) {
    Write-Host "pass $pass of $Passes..."
    foreach ($workload in $workloads) {
        foreach ($program in $workload.Programs) {
            $path = Join-Path $build "$($program.Out)$exe"
            if (-not (Test-Path $path)) {
                throw "missing binary $path - run without -SkipBuild"
            }
            $result = Measure-Runs $path @() $Runs
            $key = "$($workload.Name)|$($program.Lang)"
            if ($result.Blocked) {
                Write-Warning "$($workload.Name) / $($program.Lang): BLOCKED (no output)"
            } elseif ($result.Output -ne $workload.Expected) {
                throw "$($workload.Name) / $($program.Lang) printed '$($result.Output)', expected '$($workload.Expected)'"
            }
            $ns = if ($result.Blocked) { $null } else {
                [math]::Round(($result.Min * 1e6) / $workload.Work, 3)
            }
            $row = [pscustomobject]@{
                Benchmark = $workload.Name
                Language  = $program.Lang
                MinMs     = $result.Min
                MedianMs  = $result.Median
                NsPerOp   = $ns
                Unit      = $workload.Unit
                Output    = $result.Output
                Blocked   = $result.Blocked
            }
            if (-not $rows.ContainsKey($key) -or ($result.Blocked -eq $false -and
                    ($rows[$key].Blocked -or $result.Min -lt $rows[$key].MinMs))) {
                $rows[$key] = $row
            }
        }
    }
}

Write-Host ''
Write-Host "== feature runtime (best of $Runs runs x $Passes passes) =="
$rows.Values | Sort-Object Benchmark, NsPerOp |
    Format-Table Benchmark, Language, MinMs, MedianMs, NsPerOp, Unit, Output -AutoSize

if ($rows.Values | Where-Object { $_.Blocked }) {
    Write-Host 'Some rows were blocked by Application Control (Device Guard); rebuild them under a new path.'
}
Write-Host 'done.'
