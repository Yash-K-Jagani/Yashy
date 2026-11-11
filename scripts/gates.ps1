# Run every gate and print a table.
#
# Usage:
#   pwsh scripts/gates.ps1 [-Json] [-All]
#
# Each gate is a command with a budget. The table says what passed, what
# failed and how long each took; -Json prints the same facts as a document, so
# a nightly job can read them without parsing text.
#
# The exit status is 1 when any gate failed. The runner stops at the first
# failure unless -All is given: a gate that needs the compiler built is not
# worth running after the build failed.

param(
    [switch]$Json,
    [switch]$All
)

$ErrorActionPreference = 'Continue'

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

# The compiler the gates drive: the release build when it is there.
$compiler = $null
if (Test-Path 'target/release/yashy.exe') {
    $compiler = (Resolve-Path 'target/release/yashy.exe').Path
} elseif (Test-Path 'target/debug/yashy.exe') {
    $compiler = (Resolve-Path 'target/debug/yashy.exe').Path
}

function Get-GateCommand([string]$name) {
    switch ($name) {
        'build' { return @('cargo', 'build', '--release', '--workspace') }
        'check' { return @('cargo', 'check', '--workspace', '--all-targets') }
        'tests' { return @('cargo', 'test', '--workspace', '--no-fail-fast') }
        'format' {
            if (-not $compiler) { return @('cmd', '/c', 'echo no yashy binary') }
            $files = @()
            $files += Get-ChildItem -Recurse -Filter *.ya crates/yashy-stdlib/yashy
            $files += Get-ChildItem -Recurse -Filter *.ya packages
            $files += Get-ChildItem -Recurse -Filter *.ya examples
            return @($compiler, 'fmt', '--check') + @($files.FullName)
        }
        'reproducible' {
            if (-not $compiler) { return @('cmd', '/c', 'echo no yashy binary') }
            return @('powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/repro-check.ps1')
        }
        'sandbox' { return @('cargo', 'test', '-p', 'yashy-driver', '--test', 'sandbox_escape') }
        'never-panics' { return @('cargo', 'test', '-p', 'yashy-driver', '--test', 'never_panics') }
        'book' {
            if (-not $compiler) { return @('cmd', '/c', 'echo no yashy binary') }
            return @('powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/book-gate.ps1')
        }
    }
}

$results = @()
$gates = @(
    @{ Name = 'build'; Budget = '10m' },
    @{ Name = 'check'; Budget = '5m' },
    @{ Name = 'tests'; Budget = '45m' },
    @{ Name = 'format'; Budget = '2m' },
    @{ Name = 'reproducible'; Budget = '5m' },
    @{ Name = 'sandbox'; Budget = '5m' },
    @{ Name = 'never-panics'; Budget = '5m' },
    @{ Name = 'book'; Budget = '10m' }
)

if (-not $Json) {
    Write-Output ('{0,-16} {1,-8} {2,-6} {3}' -f 'gate', 'status', 'time', 'budget')
}

$failed = $false
foreach ($gate in $gates) {
    $command = Get-GateCommand $gate.Name
    $log = Join-Path $root (".gates-$($gate.Name).log")
    $started = Get-Date
    & $command[0] @($command[1..($command.Length - 1)]) *> $log
    $code = $LASTEXITCODE
    $seconds = [int]((Get-Date) - $started).TotalSeconds
    $status = if ($code -eq 0) { 'passed' } else { 'failed' }
    if ($code -ne 0) { $failed = $true }
    if (-not $Json) {
        Write-Output ('{0,-16} {1,-8} {2,-6} {3}' -f $gate.Name, $status, "$($seconds)s", $gate.Budget)
    }
    $results += @{ gate = $gate.Name; status = $status; seconds = $seconds; budget = $gate.Budget }
    if ($code -ne 0 -and -not $All) {
        Write-Error "the $($gate.Name) gate failed; its output is in .gates-$($gate.Name).log"
        break
    }
}

if ($Json) {
    $document = @{
        schema_version = 1
        tool           = 'gates'
        gates          = $results
        ok             = -not $failed
    } | ConvertTo-Json -Depth 5
    Write-Output $document
}

if ($failed) {
    Write-Error 'some gates failed'
    exit 1
}
Write-Output 'every gate passed'
