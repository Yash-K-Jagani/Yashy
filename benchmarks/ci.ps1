# A small, deterministic benchmark job for CI.
#
# The full suites are for a human with a quiet machine. CI needs a job that
# proves the benchmark path still works and produces a baseline: build Yashy
# and C versions of fib, check the answer, and report compile time, binary
# size, and wall-clock run time.
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

function Invoke-Tool {
    param([string]$Exe, [string[]]$Arguments, [string]$Step)
    $output = (& $Exe @Arguments 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "$Step failed (exit $LASTEXITCODE):`n$Exe $($Arguments -join ' ')`n$output"
    }
}

$yashyExe = Join-Path $build "fib-yashy$exe"
$cExe = Join-Path $build "fib-c$exe"

$yashyCompile = Measure-Command {
    Invoke-Tool $yashy @('build', "$bench\fib.ya", '-o', $yashyExe, '--release') 'yashy build fib'
}
$cCompile = Measure-Command {
    Invoke-Tool 'clang' @('-O2', "$bench\fib.c", '-o', $cExe) 'clang build fib'
}

$yashyRun = Measure-Command {
    $yashyOut = (& $yashyExe 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) {
        throw "yashy fib failed (exit $LASTEXITCODE):`n$yashyOut"
    }
}
$cRun = Measure-Command {
    $cOut = (& $cExe 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) {
        throw "clang fib failed (exit $LASTEXITCODE):`n$cOut"
    }
}

if ($yashyOut.Trim() -ne $cOut.Trim()) {
    throw "fib outputs differ: yashy='$($yashyOut.Trim())' c='$($cOut.Trim())'"
}

$yashySize = (Get-Item $yashyExe).Length
$cSize = (Get-Item $cExe).Length

Write-Output "fib output: $($yashyOut.Trim())"
Write-Output ("yashy compile ms: {0:N1}" -f $yashyCompile.TotalMilliseconds)
Write-Output ("clang compile ms: {0:N1}" -f $cCompile.TotalMilliseconds)
Write-Output ("yashy run ms: {0:N1}" -f $yashyRun.TotalMilliseconds)
Write-Output ("clang run ms: {0:N1}" -f $cRun.TotalMilliseconds)
Write-Output "yashy binary bytes: $yashySize"
Write-Output "clang binary bytes: $cSize"
