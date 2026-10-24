param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Target = 'x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist'
$name = "yashy-$Version-$Target"
$stage = Join-Path $dist $name
$zip = Join-Path $dist "$name.zip"
$binary = Join-Path $root 'target\release\yashy.exe'
$stdlib = Join-Path $root 'crates\yashy-stdlib\yashy'

if (-not (Test-Path $binary)) {
    throw "no release binary at $binary - run: cargo build --release -p yashy-driver"
}
if (-not (Test-Path $stdlib)) {
    throw "no stdlib at $stdlib"
}

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'yashy-stdlib') | Out-Null

Copy-Item $binary (Join-Path $stage 'yashy.exe')
Copy-Item -Recurse $stdlib (Join-Path $stage 'yashy-stdlib\yashy')
Copy-Item (Join-Path $PSScriptRoot 'install.ps1') (Join-Path $stage 'install.ps1')
Copy-Item (Join-Path $root 'README.md') (Join-Path $stage 'README.md')
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE')

if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
Write-Output $zip
