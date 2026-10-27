# A small, deterministic benchmark job for CI.
#
# The full suites are for a human with a quiet machine. CI needs a job that
# proves the benchmark path still works and produces a baseline: build Yashy
# and C versions of fib, check the answer, and report compile time, binary
# size, and wall-clock run time. The hello-world and ~1,000-line programs add
# the fixed build cost and the cost of a program with many items.
#
# With -Output the gate rows are written as key=value lines; the human report
# always goes to stdout. The C rows stay in the human report only: clang's
# time is context, not a Yashy regression.
[CmdletBinding()]
param(
    [string]$Output = '',
    [int]$Runs = 3
)
$ErrorActionPreference = 'Stop'

$bench = $PSScriptRoot
$root = Split-Path -Parent $bench
$build = Join-Path $bench 'build\ci'
$yashy = Join-Path $root 'target\release\yashy.exe'
$exe = '.exe'

New-Item -ItemType Directory -Force -Path $build | Out-Null

if (-not (Test-Path $yashy)) {
    throw "no release binary at $yashy - run: cargo build --release -p yashy-driver"
}
if (-not (Get-Command clang -ErrorAction SilentlyContinue)) {
    throw 'clang is not on PATH'
}

function Invoke-Tool {
    param([string]$Exe, [string[]]$Arguments, [string]$Step)
    $output = (& $Exe @Arguments 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "$Step failed (exit $LASTEXITCODE):`n$Exe $($Arguments -join ' ')`n$output"
    }
}

# One untimed warm-up, then the median of $Samples timed runs. The median is
# what the gate compares: a single sample can catch a scheduler hiccup.
function Measure-Median {
    param([scriptblock]$Action, [int]$Samples)
    & $Action | Out-Null
    $times = New-Object System.Collections.Generic.List[double]
    for ($i = 0; $i -lt $Samples; $i++) {
        $watch = [System.Diagnostics.Stopwatch]::StartNew()
        & $Action
        $watch.Stop()
        $times.Add($watch.Elapsed.TotalMilliseconds)
    }
    $sorted = @($times | Sort-Object)
    $sorted[[int][math]::Floor($sorted.Count / 2)]
}

function Format-Ms {
    param([double]$Value)
    $Value.ToString('F1', [System.Globalization.CultureInfo]::InvariantCulture)
}

$fibYashy = Join-Path $build "fib-yashy$exe"
$fibC = Join-Path $build "fib-c$exe"
$helloYashy = Join-Path $build "hello-yashy$exe"
$largeYashy = Join-Path $build "large-yashy$exe"

$fibCompile = Measure-Median {
    Invoke-Tool $yashy @('build', "$bench\fib.ya", '-o', $fibYashy, '--release') 'yashy build fib'
} $Runs
$helloCompile = Measure-Median {
    Invoke-Tool $yashy @('build', "$bench\hello.ya", '-o', $helloYashy, '--release') 'yashy build hello'
} $Runs
$largeCompile = Measure-Median {
    Invoke-Tool $yashy @('build', "$bench\large.ya", '-o', $largeYashy, '--release') 'yashy build large'
} $Runs
$yashyOut = ''
$fibRun = Measure-Median {
    $runOut = (& $fibYashy 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "yashy fib failed (exit $LASTEXITCODE):`n$runOut"
    }
    $script:yashyOut = $runOut
} $Runs

$cOut = ''
$cCompile = 0.0
$cRun = 0.0
$cBytes = 0
$cAvailable = $true
try {
    $cCompile = Measure-Median {
        Invoke-Tool 'clang' @('-O2', "$bench\fib.c", '-o', $fibC) 'clang build fib'
    } $Runs
    $cRun = Measure-Median {
        $runOut = (& $fibC 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) {
            throw "clang fib failed (exit $LASTEXITCODE):`n$runOut"
        }
        $script:cOut = $runOut
    } $Runs
    $cBytes = (Get-Item $fibC).Length
} catch {
    $cAvailable = $false
    Write-Output "C comparison unavailable: $($_.Exception.Message.Split("`n")[0])"
}

# The answer is fixed, so a wrong program is a failed run rather than a fast
# one. The C row is context and may be unavailable on a machine without the
# MSVC CRT headers.
if ($yashyOut -ne '9227465') {
    throw "yashy fib printed '$yashyOut', expected 9227465"
}
if ($cAvailable -and $yashyOut -ne $cOut) {
    throw "fib outputs differ: yashy='$yashyOut' c='$cOut'"
}

$fibBytes = (Get-Item $fibYashy).Length
$helloBytes = (Get-Item $helloYashy).Length

$fibCompileText = Format-Ms $fibCompile
$helloCompileText = Format-Ms $helloCompile
$largeCompileText = Format-Ms $largeCompile
$fibRunText = Format-Ms $fibRun

Write-Output "fib output: $yashyOut"
Write-Output ("fib compile ms: {0} (median of {1})" -f $fibCompileText, $Runs)
Write-Output ("hello compile ms: {0}" -f $helloCompileText)
Write-Output ("large compile ms: {0}" -f $largeCompileText)
Write-Output ("fib run ms: {0}" -f $fibRunText)
Write-Output "fib binary bytes: $fibBytes"
Write-Output "hello binary bytes: $helloBytes"
if ($cAvailable) {
    $cCompileText = Format-Ms $cCompile
    $cRunText = Format-Ms $cRun
    Write-Output ("C comparison (not a gate): clang compile {0} ms, run {1} ms, {2} bytes" -f $cCompileText, $cRunText, $cBytes)
}

if ($Output) {
    $parent = Split-Path -Parent $Output
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $generatedAt = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    $commit = (& git -C $root rev-parse --short HEAD 2>$null | Out-String).Trim()
    if (-not $commit) {
        $commit = 'unknown'
    }
    $toolchain = (& clang --version | Select-Object -First 1 | Out-String).Trim()
    $content = @(
        '# generated by benchmarks/ci.ps1; update deliberately'
        'os=windows'
        "generated_at=$generatedAt"
        "commit=$commit"
        "toolchain=$toolchain"
        "fib_compile_ms=$fibCompileText"
        "fib_binary_bytes=$fibBytes"
        "fib_run_ms=$fibRunText"
        "hello_compile_ms=$helloCompileText"
        "hello_binary_bytes=$helloBytes"
        "large_compile_ms=$largeCompileText"
    ) -join "`n"
    [System.IO.File]::WriteAllText($Output, $content + "`n")
    Write-Output "wrote $Output"
}
