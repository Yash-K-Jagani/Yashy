param(
    [string]$Version = 'latest',
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Yashy'),
    [string]$LocalArchive = ''
)

$ErrorActionPreference = 'Stop'

$stage = $null
$archive = $null
$source = $null

if ($LocalArchive) {
    $archive = $LocalArchive
    $stage = Join-Path $env:TEMP "yashy-install-$([guid]::NewGuid())"
    Expand-Archive -Path $archive -DestinationPath $stage -Force
    $source = $stage
} elseif (Test-Path (Join-Path $PSScriptRoot 'yashy.exe')) {
    # Running install.ps1 from an extracted release zip: install the files
    # beside it instead of going back to the network for "latest".
    $source = $PSScriptRoot
} else {
    if ($Version -eq 'latest') {
        $release = Invoke-RestMethod 'https://api.github.com/repos/Yash-K-Jagani/Otter/releases/latest'
        $asset = $release.assets |
            Where-Object { $_.name -like 'yashy-*-x86_64-pc-windows-msvc.zip' } |
            Select-Object -First 1
        if (-not $asset) {
            throw 'the latest release has no Windows x86-64 zip'
        }
        $url = $asset.browser_download_url
    } else {
        $url = "https://github.com/Yash-K-Jagani/Otter/releases/download/v$Version/yashy-$Version-x86_64-pc-windows-msvc.zip"
    }
    $archive = Join-Path $env:TEMP "yashy-$Version.zip"
    Invoke-WebRequest -Uri $url -OutFile $archive
    $stage = Join-Path $env:TEMP "yashy-install-$([guid]::NewGuid())"
    Expand-Archive -Path $archive -DestinationPath $stage -Force
    $source = $stage
}

try {
    $binary = Join-Path $source 'yashy.exe'
    if (-not (Test-Path $binary)) {
        throw "the package does not contain yashy.exe: $source"
    }

    New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
    Copy-Item -Path (Join-Path $source '*') -Destination $Prefix -Recurse -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($userPath -split ';' | Where-Object { $_ })
    if ($parts -notcontains $Prefix) {
        $parts = @($Prefix) + $parts
        [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
    }
    $env:Path = "$Prefix;$env:Path"

    $doctorExit = 0
    try {
        & (Join-Path $Prefix 'yashy.exe') doctor
        $doctorExit = $LASTEXITCODE
    } catch {
        $doctorExit = 1
        Write-Warning "could not run yashy doctor: $($_.Exception.Message)"
    }
    Write-Output "Installed Yashy to $Prefix"
    if ($doctorExit -ne 0) {
        Write-Warning 'Yashy installed, but doctor reported a problem; run: yashy doctor'
    } else {
        Write-Output 'Open a new terminal, then run: yashy doctor'
    }
}
finally {
    if ($stage -and (Test-Path $stage)) { Remove-Item -Recurse -Force $stage }
    if ($archive -and $LocalArchive -eq '' -and (Test-Path $archive)) {
        Remove-Item -Force $archive
    }
}
