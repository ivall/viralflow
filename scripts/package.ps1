# Builds a release of ViralFlow for Windows: a portable .zip with the app and
# the FFmpeg libraries it links, and (when Inno Setup is installed) an installer.
#
#   pwsh scripts/package.ps1                 # uses $env:FFMPEG_DIR or C:\Program Files\ffmpeg
#   pwsh scripts/package.ps1 -SkipBuild      # package an existing release build
param(
    [string]$FfmpegDir = $(if ($env:FFMPEG_DIR) { $env:FFMPEG_DIR } else { "C:\Program Files\ffmpeg" }),
    [string]$OutDir = "dist",
    [switch]$SkipBuild
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path "Cargo.toml" -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $SkipBuild) {
    cargo build --release -p viralflow
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
$exe = Join-Path $root "target\release\viralflow.exe"
if (-not (Test-Path $exe)) { throw "No release build at $exe" }
$dlls = Get-ChildItem (Join-Path $FfmpegDir "bin") -Filter *.dll
if (-not $dlls) { throw "No FFmpeg libraries in $FfmpegDir\bin (use a 'shared' FFmpeg 8 build)" }

$name = "ViralFlow-$version-windows-x64"
$stage = Join-Path $root "$OutDir\$name"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force (Join-Path $stage "licenses") | Out-Null

Copy-Item $exe (Join-Path $stage "ViralFlow.exe")
$dlls | ForEach-Object { Copy-Item $_.FullName $stage }
Copy-Item "LICENSE" (Join-Path $stage "LICENSE.txt")
Copy-Item "assets\licenses\*" (Join-Path $stage "licenses")
$ffmpegLicense = Join-Path $FfmpegDir "LICENSE.txt"
if (Test-Path $ffmpegLicense) { Copy-Item $ffmpegLicense (Join-Path $stage "licenses\FFmpeg-LICENSE.txt") }

$zip = Join-Path $root "$OutDir\$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Write-Host "Portable: $zip"

$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($iscc) {
    & $iscc "/DAppVersion=$version" "/DSourceDir=$stage" "/DOutputDir=$(Join-Path $root $OutDir)" "packaging\windows\viralflow.iss" | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
    Write-Host "Installer: $(Join-Path $root "$OutDir\ViralFlow-$version-setup.exe")"
} else {
    Write-Host "Inno Setup 6 not found; skipped the installer."
}
