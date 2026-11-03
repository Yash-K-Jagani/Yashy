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
#   pwsh scripts/pgo.ps1 [-Runs 5] [-Output <file>]
#
# The comparison is the plan's: the PGO binary may be up to 10% larger than the
# ThinLTO one, and the speed column says whether the profile paid for itself.
# Correctness is the gate: a corpus program that prints the wrong text, or a
# `--pgo` build that writes no profile, fails the script.
[CmdletBinding()]
param(
    [int]$Runs = 5,
    [string]$Output = ''
)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$yashy = Join-Path $root 'target\release\yashy.exe'
$corpus = Join-Path $root 'benchmarks\pgo'
$build = Join-Path $root 'benchmarks\build\pgo'
$exe = '.exe'

if (-not (Test-Path $yashy)) {
    throw "no release binary at $yashy - run: cargo build --release -p yashy-driver"
}
if (-not (Test-Path $corpus)) {
    throw "no PGO corpus at $corpus"
}
New-Item -ItemType Directory -Force -Path $build | Out-Null

# The text each corpus program prints. A profile must not change an answer, so
# every run is compared with these before it is timed.
$expected = @{
    'hello' = 'hello, world'
    'fib' = '6765'
    'wordcount' = "9000`n8"
    'maps' = '14999850000'
    'json_text' = '"a \"quoted\" line\n"' + "`n210000"
    'strings' = '900000'
}

function Invoke-Tool {
    param([string]$Exe, [string[]]$Arguments, [string]$Step)
    $output = (& $Exe @Arguments 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "$Step failed (exit $LASTEXITCODE):`n$Exe $($Arguments -join ' ')`n$output"
    }
}

function Format-Ms {
    param([double]$Value)
    $Value.ToString('F1', [System.Globalization.CultureInfo]::InvariantCulture)
}

# One untimed warm-up, then the median of $Samples timed runs. The output is
# discarded; correctness was checked before the first timing.
function Measure-Median {
    param([string]$Exe, [int]$Samples)
    & $Exe | Out-Null
    $times = New-Object System.Collections.Generic.List[double]
    for ($i = 0; $i -lt $Samples; $i++) {
        $watch = [System.Diagnostics.Stopwatch]::StartNew()
        & $Exe | Out-Null
        $watch.Stop()
        $times.Add($watch.Elapsed.TotalMilliseconds)
    }
    $sorted = @($times | Sort-Object)
    $sorted[[int][math]::Floor($sorted.Count / 2)]
}

# Run once, check the exit status and the fixed text.
function Assert-Output {
    param([string]$Exe, [string]$Expected, [string]$What)
    $text = (& $Exe 2>&1 | Out-String).Trim() -replace "`r`n", "`n"
    if ($LASTEXITCODE -ne 0) {
        throw "$What failed (exit $LASTEXITCODE):`n$text"
    }
    if ($text -ne $Expected) {
        throw "$What printed '$text', expected '$Expected'"
    }
}

$rows = New-Object System.Collections.Generic.List[string]
foreach ($file in Get-ChildItem $corpus -Filter *.ya | Sort-Object Name) {
    $stem = $file.BaseName
    if (-not $expected.ContainsKey($stem)) {
        throw "no expected output for $stem; add it to the corpus table"
    }
    $lto = Join-Path $build "$stem-lto$exe"
    $pgo = Join-Path $build "$stem-pgo$exe"

    Write-Output "== $stem"
    Invoke-Tool $yashy @('build', $file.FullName, '-o', $lto, '--release') "yashy build $stem (ThinLTO)" | Out-Null
    Invoke-Tool $yashy @('build', $file.FullName, '-o', $pgo, '--release', '--pgo') "yashy build $stem (PGO)" | Out-Null

    $profile = Join-Path $build ".yashy\pgo\$stem.profdata"
    if (-not (Test-Path $profile)) {
        throw "the PGO build of $stem wrote no profile at $profile"
    }

    Assert-Output $lto $expected[$stem] "$stem (ThinLTO)"
    Assert-Output $pgo $expected[$stem] "$stem (PGO)"

    $ltoMs = Measure-Median $lto $Runs
    $pgoMs = Measure-Median $pgo $Runs
    $ltoBytes = (Get-Item $lto).Length
    $pgoBytes = (Get-Item $pgo).Length
    $sizePct = if ($ltoBytes -eq 0) { 0.0 } else { ($pgoBytes - $ltoBytes) * 100.0 / $ltoBytes }
    # A positive "faster" means PGO ran in less time, relative to the baseline.
    $speedPct = if ($ltoMs -eq 0) { 0.0 } else { ($ltoMs - $pgoMs) * 100.0 / $ltoMs }

    $rows.Add(("{0,-12} {1,8} {2,8} {3,8} {4,9} {5,8} {6,8}" -f
            $stem,
            (Format-Ms $ltoMs),
            (Format-Ms $pgoMs),
            (Format-Ms $speedPct),
            $ltoBytes,
            $pgoBytes,
            (Format-Ms $sizePct)))
}

Write-Output ""
Write-Output ("{0,-12} {1,8} {2,8} {3,8} {4,9} {5,8} {6,8}" -f
    'program', 'lto ms', 'pgo ms', 'faster%', 'lto bytes', 'pgo bytes', 'size%')
foreach ($row in $rows) {
    Write-Output $row
}
Write-Output ""
Write-Output "profiles under $build\.yashy\pgo"

if ($Output) {
    $parent = Split-Path -Parent $Output
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $rows | Set-Content -Path $Output
    Write-Output "wrote $Output"
}
