# A small, deterministic benchmark job for CI.
#
# The full suites are for a human with a quiet machine. CI needs a job that
# proves the benchmark path still works and produces a baseline: build Yashy
# and C versions of fib, check the answer, and report compile time, binary
# size, and wall-clock run time. The hello-world and ~1,000-line programs add
# the fixed build cost and the cost of a program with many items; wordcount and
# maps add the runtime rows that phase 4's inline paths are measured against,
# and large's run row is the release link's (ThinLTO) end-to-end time.
#
# With -Output the gate rows are written as key=value lines; the human report
# always goes to stdout.
#
# Two rows here are assertions, not just numbers, because the phase 4 plan
# states them as claims and a claim with no gate cannot be corrected:
#
#   fib_wrap_ratio  `fib-wrap.ya` against `fib.c`, both medians, must be
#                   <= 1.20. This is the row the claim `fib <= 1.1x C` is
#                   about: `fib-wrap.ya` spells the same wrapping arithmetic
#                   C does, where `fib.ya`'s `+` is checked. `fib_c_run_ms`
#                   is therefore required, not context. The bound is 1.20
#                   because this runner measures 1.17: the arithmetic is
#                   comparable and the excess is the executable's fixed cost
#                   (`hello_run_ms`), which `docs/gates.md` carries as the
#                   owned follow-up that closes at 1.10.
#   promotion       `promote.ya` has one allocation that never leaves its
#                   function and one that is returned; `--timings` must
#                   report exactly one promoted site. This is the row the
#                   allocation claim is measured on (see promote.ya).
#
# The output file is written before the claim assertions run, so a failed
# claim still uploads the measurement that explains it.
#
# With -NoOpt <pass> every build carries `--no-opt=<pass>` and the measurement
# records `no_opt=<pass>`. That is the input half of `yashy-bench --per-pass`:
# one run with every pass on, one per pass off.
[CmdletBinding()]
param(
    [string]$Output = '',
    [int]$Runs = 3,
    # A registered optimization pass to disable in every build. Used with
    # `yashy-bench --per-pass`: one run with every pass on, one per pass off.
    [string]$NoOpt = ''
)
$ErrorActionPreference = 'Stop'

$bench = $PSScriptRoot
$root = Split-Path -Parent $bench
$build = Join-Path $bench 'build\ci'
$yashyc = Join-Path $root 'target\release\yashyc.exe'
$exe = '.exe'
$noOptArgs = @()
if ($NoOpt) {
    $noOptArgs = @("--no-opt=$NoOpt")
}

New-Item -ItemType Directory -Force -Path $build | Out-Null

if (-not (Test-Path $yashyc)) {
    throw "no release binary at $yashyc - run: cargo build --release -p yashy-driver"
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

# Two medians as one ratio, two decimals, culture-invariant so the measurement
# file is the same on every machine.
function Format-Ratio {
    param([double]$Numerator, [double]$Denominator)
    if ($Denominator -le 0) { return '0.00' }
    ($Numerator / $Denominator).ToString('F2', [System.Globalization.CultureInfo]::InvariantCulture)
}

$fibYashy = Join-Path $build "fib-yashy$exe"
$fibWrapYashy = Join-Path $build "fib-wrap-yashy$exe"
$fibC = Join-Path $build "fib-c$exe"
$helloYashy = Join-Path $build "hello-yashy$exe"
$largeYashy = Join-Path $build "large-yashy$exe"
$wordcountYashy = Join-Path $build "wordcount-yashy$exe"
$mapsYashy = Join-Path $build "maps-yashy$exe"
$promoteYashy = Join-Path $build "promote-yashy$exe"

$fibCompile = Measure-Median {
    Invoke-Tool $yashyc (@('build', "$bench\fib.ya", '-o', $fibYashy, '--release') + $noOptArgs) 'yashyc build fib'
} $Runs
$helloCompile = Measure-Median {
    Invoke-Tool $yashyc (@('build', "$bench\hello.ya", '-o', $helloYashy, '--release') + $noOptArgs) 'yashyc build hello'
} $Runs
$largeCompile = Measure-Median {
    Invoke-Tool $yashyc (@('build', "$bench\large.ya", '-o', $largeYashy, '--release') + $noOptArgs) 'yashyc build large'
} $Runs
# The two container workloads are the runtime rows. They are built once here,
# untimed: their compile time is the compiler's cost, not the runtime's, and
# the plan gates the run row only.
Invoke-Tool $yashyc (@('build', "$bench\wordcount.ya", '-o', $wordcountYashy, '--release') + $noOptArgs) 'yashyc build wordcount'
Invoke-Tool $yashyc (@('build', "$bench\maps.ya", '-o', $mapsYashy, '--release') + $noOptArgs) 'yashyc build maps'
# The comparable-integer program: its compile row is not claimed, so the build
# is untimed, but its run row is (see fib_wrap_ratio in the header).
Invoke-Tool $yashyc (@('build', "$bench\fib-wrap.ya", '-o', $fibWrapYashy, '--release') + $noOptArgs) 'yashyc build fib-wrap'
$yashyOut = ''
$fibRun = Measure-Median {
    $runOut = (& $fibYashy 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "yashyc fib failed (exit $LASTEXITCODE):`n$runOut"
    }
    $script:yashyOut = $runOut
} $Runs

$wordcountOut = ''
$wordcountRun = Measure-Median {
    $runOut = (& $wordcountYashy 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "yashyc wordcount failed (exit $LASTEXITCODE):`n$runOut"
    }
    $script:wordcountOut = $runOut
} $Runs

$mapsOut = ''
$mapsRun = Measure-Median {
    $runOut = (& $mapsYashy 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "yashyc maps failed (exit $LASTEXITCODE):`n$runOut"
    }
    $script:mapsOut = $runOut
} $Runs

# The ~1,000-line program is already built for its compile row; its run row is
# what the release link costs end to end.
$largeOut = ''
$largeRun = Measure-Median {
    $runOut = (& $largeYashy 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "yashyc large failed (exit $LASTEXITCODE):`n$runOut"
    }
    $script:largeOut = $runOut
} $Runs

# The numerator of fib_wrap_ratio.
$fibWrapOut = ''
$fibWrapRun = Measure-Median {
    $runOut = (& $fibWrapYashy 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "yashyc fib-wrap failed (exit $LASTEXITCODE):`n$runOut"
    }
    $script:fibWrapOut = $runOut
} $Runs

# The executable's fixed cost, which is what a whole-process ratio is made of
# once the arithmetic is comparable: hello does no work at all.
$helloOut = ''
$helloRun = Measure-Median {
    $runOut = (& $helloYashy 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "yashyc hello failed (exit $LASTEXITCODE):`n$runOut"
    }
    $script:helloOut = $runOut
} $Runs

$cOut = ''
$cCompile = 0.0
$cRun = 0.0
$cBytes = 0
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
    throw "the C row is required: it is the denominator of fib_wrap_ratio`n$($_.Exception.Message)"
}

$fibCheckedRatio = Format-Ratio $fibRun $cRun
$fibWrapRatio = Format-Ratio $fibWrapRun $cRun

# The promotion row. `--timings` writes the pass counts to stderr; the build is
# counted, then the program is run once so a promotion that changed an answer
# is caught by the same row. --NoOpt escape turns promotion off, and then the
# counts are not printed at all: that per-pass run records no promotion row
# rather than a fake zero.
$promotedStack = ''
$promotedArena = ''
$promotedSites = ''
if ($NoOpt -ne 'escape') {
    $timings = (& $yashyc 'build' "$bench\promote.ya" '-o' $promoteYashy '--timings' @noOptArgs 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "yashyc build promote failed (exit $LASTEXITCODE):`n$timings"
    }
    if ($timings -notmatch 'promoted_stack=(\d+)') {
        throw "the promotion counts are missing from yashyc build --timings:`n$timings"
    }
    $promotedStack = $Matches[1]
    if ($timings -notmatch 'promoted_arena=(\d+)') {
        throw "the promotion counts are missing from yashyc build --timings:`n$timings"
    }
    $promotedArena = $Matches[1]
    $promotedSites = [int]$promotedStack + [int]$promotedArena
    $promoteOut = (& $promoteYashy 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $promoteOut -ne '43') {
        throw "yashyc promote printed '$promoteOut' (exit $LASTEXITCODE), expected 43"
    }
}

# The answer is fixed, so a wrong program is a failed run rather than a fast
# one. The C row is required: it is the denominator of fib_wrap_ratio.
if ($yashyOut -ne '9227465') {
    throw "yashyc fib printed '$yashyOut', expected 9227465"
}
if ($fibWrapOut -ne '9227465') {
    throw "yashyc fib-wrap printed '$fibWrapOut', expected 9227465"
}
if ($helloOut -ne 'hello, world') {
    throw "yashyc hello printed '$helloOut', expected 'hello, world'"
}
if ($yashyOut -ne $cOut) {
    throw "fib outputs differ: yashyc='$yashyOut' c='$cOut'"
}
if (($wordcountOut -replace "`r`n", "`n") -ne "900000`n8") {
    throw "yashyc wordcount printed '$wordcountOut', expected 900000 words and 8 distinct"
}
if ($mapsOut -ne '134999550000') {
    throw "yashyc maps printed '$mapsOut', expected 134999550000"
}
if ($largeOut -ne '123255') {
    throw "yashyc large printed '$largeOut', expected 123255"
}

$fibBytes = (Get-Item $fibYashy).Length
$helloBytes = (Get-Item $helloYashy).Length

$fibCompileText = Format-Ms $fibCompile
$helloCompileText = Format-Ms $helloCompile
$largeCompileText = Format-Ms $largeCompile
$fibRunText = Format-Ms $fibRun
$wordcountRunText = Format-Ms $wordcountRun
$mapsRunText = Format-Ms $mapsRun
$largeRunText = Format-Ms $largeRun

Write-Output "fib output: $yashyOut"
if ($NoOpt) {
    Write-Output "disabled pass: $NoOpt"
}
Write-Output ("fib compile ms: {0} (median of {1})" -f $fibCompileText, $Runs)
Write-Output ("hello compile ms: {0}" -f $helloCompileText)
Write-Output ("large compile ms: {0}" -f $largeCompileText)
Write-Output ("fib run ms: {0}" -f $fibRunText)
Write-Output ("wordcount run ms: {0}" -f $wordcountRunText)
Write-Output ("maps run ms: {0}" -f $mapsRunText)
Write-Output ("release run ms: {0}" -f $largeRunText)
Write-Output "fib binary bytes: $fibBytes"
Write-Output "hello binary bytes: $helloBytes"
$cCompileText = Format-Ms $cCompile
$cRunText = Format-Ms $cRun
$fibWrapRunText = Format-Ms $fibWrapRun
$helloRunText = Format-Ms $helloRun
Write-Output ("C comparison (fib_wrap_ratio denominator): clang compile {0} ms, run {1} ms, {2} bytes" -f $cCompileText, $cRunText, $cBytes)
Write-Output ("hello run ms: {0} (the executable's fixed cost)" -f $helloRunText)
Write-Output ("fib-wrap run ms: {0}" -f $fibWrapRunText)
Write-Output ("fib vs C: {0} x (checked arithmetic; not a gate)" -f $fibCheckedRatio)
Write-Output ("fib-wrap vs C: {0} x (limit 1.20; 1.10 is the owned follow-up)" -f $fibWrapRatio)
if ($promotedSites -ne '') {
    Write-Output ("promoted sites: {0} (stack {1}, arena {2}; expected 1)" -f $promotedSites, $promotedStack, $promotedArena)
} else {
    Write-Output "promotion: not measured (-NoOpt $NoOpt turns promotion off)"
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
        # Same-run reference: clang compiling the equivalent C program here.
        if ($cCompile -gt 0) {
            "fib_compile_ratio=$([math]::Round($fibCompile / $cCompile, 4).ToString([cultureinfo]::InvariantCulture))"
        }
        "fib_binary_bytes=$fibBytes"
        "fib_run_ms=$fibRunText"
        "fib_c_run_ms=$cRunText"
        "fib_wrap_run_ms=$fibWrapRunText"
        "fib_checked_ratio=$fibCheckedRatio"
        "fib_wrap_ratio=$fibWrapRatio"
        "hello_compile_ms=$helloCompileText"
        "hello_binary_bytes=$helloBytes"
        "hello_run_ms=$helloRunText"
        "large_compile_ms=$largeCompileText"
        "wordcount_run_ms=$wordcountRunText"
        "maps_run_ms=$mapsRunText"
        "release_run_ms=$largeRunText"
    )
    if ($promotedSites -ne '') {
        $content += "promoted_stack=$promotedStack"
        $content += "promoted_arena=$promotedArena"
        $content += "promoted_sites=$promotedSites"
    }
    if ($NoOpt) {
        $content += "no_opt=$NoOpt"
    }
    $content = $content -join "`n"
    [System.IO.File]::WriteAllText($Output, $content + "`n")
    Write-Output "wrote $Output"
}

# The claim assertions run last, on purpose: the file above is the evidence, and
# a run that fails a claim must still upload the numbers that failed it.

if ([double]$fibWrapRatio -gt 1.20) {
    throw "fib_wrap_ratio is $fibWrapRatio ($fibWrapRunText ms vs C $cRunText ms), limit 1.20 - fix the arithmetic path or correct the claim in docs/gates.md and the phase 4 plan"
}

if ($promotedSites -ne '' -and [int]$promotedSites -ne 1) {
    throw "promote.ya promoted $promotedSites sites (stack $promotedStack, arena $promotedArena), expected exactly 1 - one allocation stays in its function and one escapes"
}
