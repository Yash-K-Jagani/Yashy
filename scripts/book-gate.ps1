# The book gate, on Windows.
#
# Usage:
#   pwsh scripts/book-gate.ps1
#
# The book's prose is executable: every Yashy block on its pages is compiled
# and run by `scripts/run-tutorial-blocks.sh`, the generated reference page is
# regenerated and diffed, and every exercise under `book/tests/` is built.
# This script is the Windows face of that work, so `scripts/gates.ps1` has a
# `book` gate to run; the CI job that owns the book runs the same steps on
# Linux.
#
# It shells out to `sh` for the block runner because that runner is the one
# implementation of "run the blocks", and a second implementation here would
# be a second set of behaviours to keep in step. The script says so plainly
# when `sh` is not on PATH rather than skipping the gate silently.

$ErrorActionPreference = "Stop"

function Fail([string]$message) {
    Write-Error $message
    exit 1
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Push-Location $root
try {
    # The block runner and the reference generator are POSIX scripts. Run them
    # through `sh` when it is on PATH, through Git for Windows' bash when it
    # is there, and only then through some other `bash`: the usual Windows
    # machine has Git and no `sh` on PATH, and a gate that gives up because of
    # a missing shell is not a gate. Git bash takes Windows paths as arguments
    # unchanged and executes them, which is what these scripts need. WSL's
    # `...\System32\bash.exe` runs Linux binaries and cannot exec a Windows
    # path, so it is never the answer.
    $wslBash = Join-Path $env:SystemRoot 'System32\bash.exe'
    $shellPath = $null
    $sh = Get-Command sh -ErrorAction SilentlyContinue
    if ($sh) { $shellPath = $sh.Source }
    if (-not $shellPath) {
        $git = Get-Command git -ErrorAction SilentlyContinue
        if ($git) {
            $gitDir = Split-Path -Parent $git.Source
            $gitRoot = Split-Path -Parent $gitDir
            foreach ($candidate in @((Join-Path $gitRoot 'bin\bash.exe'), (Join-Path $gitDir 'bash.exe'))) {
                if (Test-Path $candidate) {
                    $shellPath = (Resolve-Path $candidate).Path
                    break
                }
            }
        }
    }
    if (-not $shellPath) {
        $bash = Get-Command bash -ErrorAction SilentlyContinue
        if ($bash -and $bash.Source -ne $wslBash) { $shellPath = $bash.Source }
    }
    if (-not $shellPath) {
        Fail "no shell for the book gate's scripts; install Git for Windows (its bash runs them), or put sh on PATH"
    }

    if (Test-Path target/release/yashy.exe) {
        $yashy = (Resolve-Path target/release/yashy.exe).Path
    } elseif (Test-Path target/debug/yashy.exe) {
        $yashy = (Resolve-Path target/debug/yashy.exe).Path
    } else {
        Fail "no yashy binary; run: cargo build --release -p yashy-driver"
    }

    # The pages that carry blocks of their own. The language guide is not
    # listed: it includes the tutorial's chapters, and the tutorial harness
    # compiles those.
    $pages = @(
        "book/src/getting-started.md",
        "book/src/verification-guide.md",
        "book/src/tooling.md",
        "book/src/cookbook.md"
    )
    foreach ($page in $pages) {
        if (-not (Test-Path $page)) {
            Fail "no page at $page"
        }
    }

    Write-Output "running the book's blocks"
    & $shellPath scripts/run-tutorial-blocks.sh $yashy @pages
    if ($LASTEXITCODE -ne 0) {
        Fail "a block on the book's own pages did not compile or run"
    }

    # The reference is generated from `yashy index`, so the file must already
    # be what the generator writes. A diff means the page and the compiler
    # disagree, and the page is the one that is wrong.
    Write-Output "checking the generated reference"
    & $shellPath scripts/book-reference.sh | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Fail "scripts/book-reference.sh failed"
    }
    & git diff --exit-code book/src/reference.md
    if ($LASTEXITCODE -ne 0) {
        Fail "book/src/reference.md has drifted from what scripts/book-reference.sh writes; regenerate it and commit the result"
    }

    Write-Output "building the exercises"
    $exercises = Get-ChildItem book/tests -Filter *.ya -ErrorAction SilentlyContinue
    if (-not $exercises) {
        Fail "no exercises under book/tests"
    }
    foreach ($exercise in $exercises) {
        $out = Join-Path ([System.IO.Path]::GetTempPath()) ("yashy-book-" + [System.Guid]::NewGuid().ToString("N") + ".exe")
        & $yashy build $exercise.FullName -o $out | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Fail "$($exercise.Name) did not build, so it is not a solution"
        }
        Remove-Item $out -ErrorAction SilentlyContinue
    }

    Write-Output "ok - the book's blocks, the reference and the exercises all pass"
} finally {
    Pop-Location
}
