param(
    [string]$Version = 'latest',
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Yashy'),
    [string]$LocalArchive = '',
    [string]$Checksums = '',
    [switch]$NoPath,
    [switch]$NoDoctor,
    [switch]$Uninstall,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Show-Usage {
@'
usage: install.ps1 [options]

  -Version <v>       install a specific release (default: latest)
  -Prefix <dir>      install prefix (default: %LOCALAPPDATA%\Yashy)
  -LocalArchive <p>  install from a local zip, for offline installs and tests
  -Checksums <p>     SHA256SUMS for -LocalArchive (default: beside the zip)
  -NoPath            do not add the prefix to the user PATH
  -NoDoctor          do not run `yashy doctor` after installing
  -Uninstall         remove the installed files and the user PATH entry
  -Help              show this help
'@ | Write-Output
}

function New-TempDirectory {
    $dir = Join-Path $env:TEMP ("yashy-install-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    return $dir
}

function Get-Sha256([string]$Path) {
    # Use the .NET hash directly rather than `Get-FileHash`: the cmdlet lives in
    # a module that a locked-down PowerShell host may not load, while the
    # installer must still be able to verify what it downloads.
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha = [System.Security.Cryptography.SHA256]::Create()
        try {
            $hash = $sha.ComputeHash($stream)
        }
        finally {
            $sha.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
    return ([System.BitConverter]::ToString($hash) -replace '-', '').ToLowerInvariant()
}

function Assert-Checksum([string]$Archive, [string]$SumsPath) {
    if (-not (Test-Path -LiteralPath $SumsPath)) {
        throw "no checksum file at $SumsPath"
    }
    $name = Split-Path -Leaf $Archive
    $expected = $null
    foreach ($line in Get-Content -LiteralPath $SumsPath) {
        $match = [regex]::Match($line.Trim(), '^([0-9a-fA-F]{64})\s+(.+)$')
        if ($match.Success -and $match.Groups[2].Value.Trim() -eq $name) {
            $expected = $match.Groups[1].Value.ToLowerInvariant()
            break
        }
    }
    if (-not $expected) {
        throw "no checksum for $name in $SumsPath"
    }
    $actual = Get-Sha256 $Archive
    if ($expected -ne $actual) {
        throw "checksum mismatch for ${name}: expected $expected, got $actual"
    }
}

function Find-PackageRoot([string]$ExtractDir) {
    if (Test-Path -LiteralPath (Join-Path $ExtractDir 'yashy.exe')) {
        return $ExtractDir
    }
    foreach ($child in Get-ChildItem -LiteralPath $ExtractDir -Directory) {
        if (Test-Path -LiteralPath (Join-Path $child.FullName 'yashy.exe')) {
            return $child.FullName
        }
    }
    throw "the package does not contain yashy.exe: $ExtractDir"
}

function Get-NormalizedPath([string]$Path) {
    $full = [System.IO.Path]::GetFullPath($Path)
    $root = [System.IO.Path]::GetPathRoot($full)
    if ($full -eq $root) {
        throw "refusing to use $full as the install prefix"
    }
    return $full.TrimEnd([char[]]@('\', '/'))
}

if ($Help) {
    Show-Usage
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Prefix)) {
    throw 'the install prefix is empty'
}
$Prefix = Get-NormalizedPath $Prefix
$marker = Join-Path $Prefix '.yashy-path'

if ($Uninstall) {
    $removed = $false
    $entry = $Prefix
    if (Test-Path -LiteralPath $marker) {
        $recorded = (Get-Content -LiteralPath $marker -Raw).Trim()
        if ($recorded) {
            $entry = $recorded
        }
    }

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($null -ne $userPath) {
        $parts = @($userPath -split ';' | Where-Object { $_ })
        $kept = @($parts | Where-Object {
            $trimmed = $_.TrimEnd([char[]]@('\', '/'))
            $trimmed -ne $entry.TrimEnd([char[]]@('\', '/')) -and
                $trimmed -ne $Prefix.TrimEnd([char[]]@('\', '/'))
        })
        if ($kept.Count -ne $parts.Count) {
            [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
            Write-Output "removed $entry from the user PATH"
            $removed = $true
        }
    }

    if (Test-Path -LiteralPath $Prefix) {
        Remove-Item -LiteralPath $Prefix -Recurse -Force
        Write-Output "removed $Prefix"
        $removed = $true
    }

    if (-not $removed) {
        Write-Output "nothing to remove at $Prefix"
    }
    Write-Output "Yashy uninstalled from $Prefix"
    exit 0
}

$stage = $null
$downloadDir = $null
$exitCode = 0

try {
    if ($LocalArchive) {
        $archive = (Resolve-Path -LiteralPath $LocalArchive).Path
        if (-not $Checksums) {
            $Checksums = Join-Path (Split-Path -Parent $archive) 'SHA256SUMS'
        }
        Assert-Checksum $archive $Checksums
        $stage = New-TempDirectory
        Expand-Archive -LiteralPath $archive -DestinationPath $stage -Force
    }
    elseif ($PSScriptRoot -and (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'yashy.exe'))) {
        # Running install.ps1 from an extracted release zip: install the files
        # beside it instead of going back to the network for "latest".
        $stage = $PSScriptRoot
    }
    else {
        if ($Version -eq 'latest') {
            $release = Invoke-RestMethod 'https://api.github.com/repos/Yash-K-Jagani/Otter/releases/latest'
            $Version = $release.tag_name.TrimStart('v')
        }
        $downloadDir = New-TempDirectory
        $archive = Join-Path $downloadDir "yashy-$Version-x86_64-pc-windows-msvc.zip"
        $sums = Join-Path $downloadDir 'SHA256SUMS'
        $base = "https://github.com/Yash-K-Jagani/Otter/releases/download/v$Version"
        Invoke-WebRequest -UseBasicParsing -Uri "$base/yashy-$Version-x86_64-pc-windows-msvc.zip" -OutFile $archive
        Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile $sums
        Assert-Checksum $archive $sums
        $stage = New-TempDirectory
        Expand-Archive -LiteralPath $archive -DestinationPath $stage -Force
    }

    $source = Find-PackageRoot $stage
    if (-not (Test-Path -LiteralPath (Join-Path $source 'yashy-stdlib\yashy\io.ya'))) {
        throw "the package does not contain yashy-stdlib\yashy\io.ya: $source"
    }

    New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
    Copy-Item -Path (Join-Path $source '*') -Destination $Prefix -Recurse -Force
    Set-Content -LiteralPath $marker -Value $Prefix -NoNewline

    if (-not $NoPath) {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $parts = @($userPath -split ';' | Where-Object { $_ })
        if ($parts -notcontains $Prefix) {
            $parts = @($Prefix) + $parts
            [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
            Write-Output "added $Prefix to the user PATH"
        }
        else {
            Write-Output "the user PATH already contains $Prefix"
        }
        $env:Path = "$Prefix;$env:Path"
    }
    else {
        Write-Output 'skipped PATH setup (-NoPath)'
    }

    $doctorStatus = 0
    if (-not $NoDoctor) {
        try {
            & (Join-Path $Prefix 'yashy.exe') doctor
            $doctorStatus = $LASTEXITCODE
        }
        catch {
            $doctorStatus = 1
            Write-Warning "could not run yashy doctor: $($_.Exception.Message)"
        }
    }

    Write-Output "Installed Yashy to $Prefix"
    if ($doctorStatus -ne 0) {
        Write-Output ''
        Write-Output 'Yashy installed, but `yashy doctor` found a problem.'
        Write-Output 'Next: winget install LLVM.LLVM'
        Write-Output 'Then run: yashy doctor'
    }
    else {
        Write-Output 'Next: yashy init my-cli'
        if ($NoPath) {
            Write-Output "      (add $Prefix to PATH, or run $Prefix\yashy.exe)"
        }
    }
}
finally {
    if ($stage -and $stage -ne $PSScriptRoot -and (Test-Path -LiteralPath $stage)) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    if ($downloadDir -and (Test-Path -LiteralPath $downloadDir)) {
        Remove-Item -LiteralPath $downloadDir -Recurse -Force
    }
}

exit $exitCode
