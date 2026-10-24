param(
    [string]$Version = 'latest',
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Yashy'),
    [string]$LocalArchive = ''
)

$ErrorActionPreference = 'Stop'

if ($LocalArchive) {
    $archive = $LocalArchive
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
}

$stage = Join-Path $env:TEMP "yashy-install-$([guid]::NewGuid())"
try {
    Expand-Archive -Path $archive -DestinationPath $stage -Force
    $binary = Join-Path $stage 'yashy.exe'
    if (-not (Test-Path $binary)) {
        throw "the archive does not contain yashy.exe: $archive"
    }

    New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
    Copy-Item -Recurse -Force (Join-Path $stage '*') $Prefix

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($userPath -split ';' | Where-Object { $_ })
    if ($parts -notcontains $Prefix) {
        $parts = @($Prefix) + $parts
        [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
    }
    $env:Path = "$Prefix;$env:Path"

    & (Join-Path $Prefix 'yashy.exe') doctor
    Write-Output "Installed Yashy to $Prefix"
    Write-Output 'Open a new terminal, then run: yashy doctor'
}
finally {
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
}
