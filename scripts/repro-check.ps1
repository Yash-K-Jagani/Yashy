# Build the same source twice, in two directories, and compare the bytes.
#
# Usage:
#   pwsh scripts/repro-check.ps1 [-Source <file.ya>]
#
# Without a source, examples/hello/main.ya is used. The script exits 0 when
# the two binaries are identical, and 1 with the first differing byte offset
# when they are not.
#
# The offset matters: "the builds differ" is not an answer a person can act
# on, and the offset points at the byte that carries the difference.

param(
    [string]$Source = "examples/hello/main.ya"
)

$ErrorActionPreference = "Stop"

function Fail([string]$message) {
    Write-Error $message
    exit 1
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Push-Location $root
try {
    if (-not (Test-Path $Source)) {
        Fail "no source at $Source"
    }

    # The compiler: the one in target/release if it is there, the debug one
    # otherwise. A release build is what a release would ship, so it is the
    # one worth checking.
    if (Test-Path target/release/yashy.exe) {
        $yashy = (Resolve-Path target/release/yashy.exe).Path
    } elseif (Test-Path target/debug/yashy.exe) {
        $yashy = (Resolve-Path target/debug/yashy.exe).Path
    } else {
        Fail "no yashy binary; run: cargo build --release -p yashy-driver"
    }

    $work = Join-Path ([System.IO.Path]::GetTempPath()) ("yashy-repro-" + [System.Guid]::NewGuid().ToString("N"))
    try {
        foreach ($side in @("a", "b")) {
            $directory = Join-Path $work $side
            New-Item -ItemType Directory -Force -Path $directory | Out-Null
            Copy-Item $Source (Join-Path $directory "main.ya") -Force
            Push-Location $directory
            try {
                & $yashy build main.ya -o hello --reproducible | Out-Null
                if ($LASTEXITCODE -ne 0) {
                    Fail "the build in $directory failed"
                }
            } finally {
                Pop-Location
            }
        }

        # The output name is the one that was asked for, on every platform:
        # `-o hello` writes a file called `hello`.
        $first = Join-Path $work "a/hello"
        $second = Join-Path $work "b/hello"
        $left = (Get-FileHash $first -Algorithm SHA256).Hash
        $right = (Get-FileHash $second -Algorithm SHA256).Hash

        if ($left -eq $right) {
            Write-Output "ok - both builds are $left"
            exit 0
        }

        # The first byte at which they differ, so the failure names a place.
        $leftBytes = [System.IO.File]::ReadAllBytes($first)
        $rightBytes = [System.IO.File]::ReadAllBytes($second)
        $offset = -1
        $limit = [Math]::Min($leftBytes.Length, $rightBytes.Length)
        for ($index = 0; $index -lt $limit; $index++) {
            if ($leftBytes[$index] -ne $rightBytes[$index]) {
                $offset = $index
                break
            }
        }
        if ($offset -lt 0 -and $leftBytes.Length -ne $rightBytes.Length) {
            $offset = $limit
        }

        Write-Error "the two builds differ"
        Write-Error "  first:  $left"
        Write-Error "  second: $right"
        if ($offset -ge 0) {
            Write-Error "  first differing byte: $offset"
        }
        exit 1
    } finally {
        Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
    }
} finally {
    Pop-Location
}
