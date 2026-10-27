param(
    [string]$LocalArchive = '',
    [string]$Binary = '',
    [string]$Stdlib = ''
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    throw "FAIL: $Message"
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        Fail $Message
    }
}

function New-TestDirectory {
    $dir = Join-Path $env:TEMP ("yashy-test-install-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    return $dir
}

function Invoke-Installer {
    param(
        [string]$Installer,
        [string[]]$Arguments
    )
    # Redirect both streams to files. A native command's stderr is an error
    # record in Windows PowerShell, and $ErrorActionPreference = 'Stop' would
    # turn the expected checksum failure into a terminating error here instead
    # of letting the test assert on it.
    $id = [guid]::NewGuid().ToString('N')
    $stdout = Join-Path $env:TEMP "yashy-installer-$id.out"
    $stderr = Join-Path $env:TEMP "yashy-installer-$id.err"
    $all = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $Installer) + $Arguments
    try {
        $process = Start-Process -FilePath 'powershell.exe' -ArgumentList $all -Wait -PassThru -NoNewWindow `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        $outText = if (Test-Path -LiteralPath $stdout) { Get-Content -LiteralPath $stdout -Raw } else { '' }
        $errText = if (Test-Path -LiteralPath $stderr) { Get-Content -LiteralPath $stderr -Raw } else { '' }
        return @{ Output = (([string]$outText) + ([string]$errText)); Code = $process.ExitCode }
    }
    finally {
        Remove-Item -LiteralPath $stdout -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stderr -Force -ErrorAction SilentlyContinue
    }
}

function Get-UserPath {
    return [Environment]::GetEnvironmentVariable('Path', 'User')
}

function Set-UserPath([string]$Value) {
    [Environment]::SetEnvironmentVariable('Path', $Value, 'User')
}

function Get-CanonicalPath([string]$Value) {
    if ([string]::IsNullOrWhiteSpace($Value)) {
        return ''
    }
    $trimmed = $Value.TrimEnd([char[]]@('\', '/'))
    try {
        # Get-Item expands an 8.3 name such as RUNNER~1 to the long form. The
        # installer writes the long path, while $env:TEMP can be the short one.
        return (Get-Item -LiteralPath $trimmed -ErrorAction Stop).FullName.TrimEnd([char[]]@('\', '/'))
    }
    catch {
        return $trimmed
    }
}

function Test-PathContains([string]$PathValue, [string]$Entry) {
    if ($null -eq $PathValue) {
        return $false
    }
    $wanted = Get-CanonicalPath $Entry
    foreach ($part in ($PathValue -split ';')) {
        if ((Get-CanonicalPath $part) -eq $wanted) {
            return $true
        }
    }
    return $false
}

$root = Split-Path -Parent $PSScriptRoot
$installer = Join-Path $PSScriptRoot 'install.ps1'
$tmp = New-TestDirectory
$originalUserPath = Get-UserPath

try {
    if ($LocalArchive) {
        $archive = Join-Path $tmp (Split-Path -Leaf $LocalArchive)
        Copy-Item -LiteralPath $LocalArchive -Destination $archive -Force
    }
    else {
        if (-not $Binary) {
            $Binary = Join-Path $root 'target\release\yashy.exe'
        }
        if (-not $Stdlib) {
            $Stdlib = Join-Path $root 'crates\yashy-stdlib\yashy'
        }
        Assert-True (Test-Path -LiteralPath $Binary) "no compiler binary at $Binary; run cargo build --release -p yashy-driver or pass -Binary"
        Assert-True (Test-Path -LiteralPath $Stdlib) "no stdlib at $Stdlib"

        $name = 'yashy-0.0.0-test-x86_64-pc-windows-msvc'
        $stage = Join-Path $tmp $name
        New-Item -ItemType Directory -Force -Path (Join-Path $stage 'yashy-stdlib') | Out-Null
        Copy-Item -LiteralPath $Binary -Destination (Join-Path $stage 'yashy.exe')
        Copy-Item -LiteralPath $Stdlib -Destination (Join-Path $stage 'yashy-stdlib\yashy') -Recurse
        Copy-Item -LiteralPath $installer -Destination (Join-Path $stage 'install.ps1')
        Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination (Join-Path $stage 'README.md')
        Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $stage 'LICENSE')
        $archive = Join-Path $tmp "$name.zip"
        Compress-Archive -Path $stage -DestinationPath $archive
    }

    $archiveName = Split-Path -Leaf $archive
    $sums = Join-Path $tmp 'SHA256SUMS'
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    "$hash  $archiveName" | Set-Content -LiteralPath $sums -Encoding ASCII

    # 1. A fresh prefix installs, doctor passes, and the first project runs.
    $prefix = Join-Path $tmp 'prefix-fresh'
    $result = Invoke-Installer -Installer $installer -Arguments @(
        '-LocalArchive', $archive,
        '-Checksums', $sums,
        '-Prefix', $prefix,
        '-NoPath'
    )
    Assert-True ($result.Code -eq 0) "install failed:`n$($result.Output)"
    Assert-True ($result.Output -match 'Installed Yashy') "install did not report success:`n$($result.Output)"
    Assert-True ($result.Output -match 'Next: yashy init my-cli') "install did not print the next command:`n$($result.Output)"
    Assert-True (Test-Path -LiteralPath (Join-Path $prefix 'yashy.exe')) 'the installer did not create yashy.exe'
    Assert-True (Test-Path -LiteralPath (Join-Path $prefix 'yashy-stdlib\yashy\io.ya')) 'the installer did not copy the stdlib'
    Assert-True ((Get-UserPath) -eq $originalUserPath) '-NoPath changed the user PATH'

    $yashy = Join-Path $prefix 'yashy.exe'
    & $yashy doctor
    Assert-True ($LASTEXITCODE -eq 0) 'doctor failed after a clean install'
    $json = (& $yashy doctor --json | Out-String)
    Assert-True ($LASTEXITCODE -eq 0) 'doctor --json failed after a clean install'
    Assert-True ($json -match '"schema_version": 1') 'doctor --json has no schema_version'
    Assert-True ($json -match '"tool": "doctor"') 'doctor --json has no tool'
    Assert-True ($json -match '"ok": true') 'doctor --json did not report ok'
    Assert-True ($json -match '"next": null') 'doctor --json has a next step on success'

    $demo = Join-Path $tmp 'demo'
    & $yashy init $demo
    Assert-True ($LASTEXITCODE -eq 0) 'init failed'
    Push-Location $demo
    try {
        & $yashy run main.ya
        Assert-True ($LASTEXITCODE -eq 0) 'run failed'
        & $yashy test
        Assert-True ($LASTEXITCODE -eq 0) 'test failed'
    }
    finally {
        Pop-Location
    }
    Write-Output 'ok - fresh install, doctor, init, run, and test'

    # 2. A bad checksum is a hard failure and leaves no partial install.
    $badDir = Join-Path $tmp 'bad'
    New-Item -ItemType Directory -Force -Path $badDir | Out-Null
    $badArchive = Join-Path $badDir $archiveName
    Copy-Item -LiteralPath $archive -Destination $badArchive
    $badSums = Join-Path $badDir 'SHA256SUMS'
    ("0" * 64) + "  $archiveName" | Set-Content -LiteralPath $badSums -Encoding ASCII
    $badPrefix = Join-Path $tmp 'prefix-bad'
    $bad = Invoke-Installer -Installer $installer -Arguments @(
        '-LocalArchive', $badArchive,
        '-Checksums', $badSums,
        '-Prefix', $badPrefix,
        '-NoPath',
        '-NoDoctor'
    )
    Assert-True ($bad.Code -ne 0) 'a bad checksum must fail the install'
    Assert-True ($bad.Output -match 'checksum mismatch') "the bad-checksum error is not clear:`n$($bad.Output)"
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $badPrefix 'yashy.exe'))) 'a bad checksum left a partial install'
    Write-Output 'ok - bad checksum fails before writing the prefix'

    # 3. A missing checksum file is also a hard failure.
    $missingPrefix = Join-Path $tmp 'prefix-missing-sums'
    $missing = Invoke-Installer -Installer $installer -Arguments @(
        '-LocalArchive', $archive,
        '-Checksums', (Join-Path $tmp 'does-not-exist'),
        '-Prefix', $missingPrefix,
        '-NoPath',
        '-NoDoctor'
    )
    Assert-True ($missing.Code -ne 0) 'a missing checksum file must fail the install'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $missingPrefix 'yashy.exe'))) 'a missing checksum left a partial install'
    Write-Output 'ok - missing checksum file fails before writing the prefix'

    # 4. Uninstall removes the prefix and is idempotent.
    $uninstall = Invoke-Installer -Installer $installer -Arguments @(
        '-Uninstall',
        '-Prefix', $prefix,
        '-NoPath'
    )
    Assert-True ($uninstall.Code -eq 0) "uninstall failed:`n$($uninstall.Output)"
    Assert-True (-not (Test-Path -LiteralPath $prefix)) 'uninstall left the prefix behind'
    $again = Invoke-Installer -Installer $installer -Arguments @(
        '-Uninstall',
        '-Prefix', $prefix,
        '-NoPath'
    )
    Assert-True ($again.Code -eq 0) "a second uninstall failed:`n$($again.Output)"
    Assert-True ($again.Output -match 'nothing to remove') "a second uninstall is not a no-op:`n$($again.Output)"
    Write-Output 'ok - uninstall removes the prefix and is idempotent'

    # 5. PATH add is idempotent and uninstall removes the entry.
    $pathPrefix = Join-Path $tmp 'prefix-path'
    $pathInstall = Invoke-Installer -Installer $installer -Arguments @(
        '-LocalArchive', $archive,
        '-Checksums', $sums,
        '-Prefix', $pathPrefix,
        '-NoDoctor'
    )
    Assert-True ($pathInstall.Code -eq 0) "PATH install failed:`n$($pathInstall.Output)"
    $pathAfterInstall = Get-UserPath
    Assert-True (Test-PathContains $pathAfterInstall $pathPrefix) "the installer did not add the prefix to the user PATH`ninstaller output:`n$($pathInstall.Output)`nuser PATH:`n$pathAfterInstall"
    $pathInstall2 = Invoke-Installer -Installer $installer -Arguments @(
        '-LocalArchive', $archive,
        '-Checksums', $sums,
        '-Prefix', $pathPrefix,
        '-NoDoctor'
    )
    Assert-True ($pathInstall2.Code -eq 0) "PATH reinstall failed:`n$($pathInstall2.Output)"
    $canonicalPrefix = Get-CanonicalPath $pathPrefix
    $occurrences = @((Get-UserPath) -split ';' | Where-Object { (Get-CanonicalPath $_) -eq $canonicalPrefix }).Count
    Assert-True ($occurrences -eq 1) "the user PATH has $occurrences copies of the prefix"
    $pathUninstall = Invoke-Installer -Installer $installer -Arguments @(
        '-Uninstall',
        '-Prefix', $pathPrefix,
        '-NoPath'
    )
    Assert-True ($pathUninstall.Code -eq 0) "PATH uninstall failed:`n$($pathUninstall.Output)"
    Assert-True (-not (Test-PathContains (Get-UserPath) $pathPrefix)) 'uninstall left the prefix in the user PATH'
    Write-Output 'ok - PATH add is idempotent and uninstall removes it'

    # 6. The install.ps1 inside an extracted zip installs without the network.
    $extract = Join-Path $tmp 'extract'
    Expand-Archive -LiteralPath $archive -DestinationPath $extract -Force
    $stage = Get-ChildItem -LiteralPath $extract -Directory | Select-Object -First 1
    Assert-True ($null -ne $stage) 'the zip has no top-level package directory'
    $packagedInstaller = Join-Path $stage.FullName 'install.ps1'
    Assert-True (Test-Path -LiteralPath $packagedInstaller) 'the zip has no install.ps1'
    $extractPrefix = Join-Path $tmp 'prefix-extract'
    $extracted = Invoke-Installer -Installer $packagedInstaller -Arguments @(
        '-Prefix', $extractPrefix,
        '-NoPath',
        '-NoDoctor'
    )
    Assert-True ($extracted.Code -eq 0) "the extracted installer failed:`n$($extracted.Output)"
    Assert-True (Test-Path -LiteralPath (Join-Path $extractPrefix 'yashy.exe')) 'the extracted installer did not install'
    Write-Output 'ok - extracted zip installs without the network'

    # 7. A missing clang keeps the install and prints the platform command.
    $fakeBin = Join-Path $tmp 'fakebin'
    New-Item -ItemType Directory -Force -Path $fakeBin | Out-Null
    # where.exe exits 1 for `--version`, so doctor's version probe fails.
    Copy-Item -LiteralPath (Join-Path $env:SystemRoot 'System32\where.exe') -Destination (Join-Path $fakeBin 'clang.exe')
    $noClangPrefix = Join-Path $tmp 'prefix-noclang'
    $savedPath = $env:Path
    try {
        $env:Path = "$fakeBin;$savedPath"
        $noClang = Invoke-Installer -Installer $installer -Arguments @(
            '-LocalArchive', $archive,
            '-Checksums', $sums,
            '-Prefix', $noClangPrefix,
            '-NoPath'
        )
    }
    finally {
        $env:Path = $savedPath
    }
    Assert-True ($noClang.Code -eq 0) "a broken clang must not fail the install:`n$($noClang.Output)"
    Assert-True ($noClang.Output -match 'Yashy installed, but') "the missing-clang message is absent:`n$($noClang.Output)"
    Assert-True ($noClang.Output -match 'Next: winget install LLVM.LLVM') "the missing-clang next step is not actionable:`n$($noClang.Output)"
    Assert-True (Test-Path -LiteralPath (Join-Path $noClangPrefix 'yashy.exe')) 'a broken clang must not roll back the install'
    Write-Output 'ok - missing clang keeps the install and prints an actionable next step'

    Write-Output ''
    Write-Output 'all Windows installer tests passed'
}
finally {
    Set-UserPath $originalUserPath
    if (Test-Path -LiteralPath $tmp) {
        Remove-Item -LiteralPath $tmp -Recurse -Force
    }
}
