# SPDX-License-Identifier: GPL-3.0-or-later
# Build, test and package ONE release tag on this Windows PC, unsigned.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-windows.ps1 -Tag v0.5.3 -Checkout <dir> -Out <dir>
#
# The release workflow signs what this produces with Authenticode; nothing here touches a
# certificate. tools/release/local-release.sh runs this over SSH. The checkout is reused between
# releases so Cargo and CMake build incrementally; it is moved to the tag, never edited.
# One-time setup (Rust 1.92.0, Qt 6.11.2 msvc2022_64, Visual Studio with C++) is documented in
# tools/release/README.md.
param(
    [Parameter(Mandatory)][string]$Tag,
    [Parameter(Mandatory)][string]$Checkout,
    [Parameter(Mandatory)][string]$Out,
    [string]$QtPrefix = "$env:USERPROFILE\redrob\.toolchain\Qt\6.11.2\msvc2022_64"
)
$ErrorActionPreference = 'Stop'

function Invoke-Checked([string]$what, [scriptblock]$block) {
    & $block
    if ($LASTEXITCODE) { throw "$what failed with exit code $LASTEXITCODE" }
}

$env:PATH = "$env:USERPROFILE\.cargo\bin;$QtPrefix\bin;$env:PATH"
$env:CMAKE_PREFIX_PATH = $QtPrefix
# Korean Windows defaults Python to cp949; the build helpers read UTF-8 sources.
$env:PYTHONUTF8 = '1'
$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -property installationPath
Import-Module "$vs\Common7\Tools\Microsoft.VisualStudio.DevShell.dll"
Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64' | Out-Null

if (-not (Test-Path "$Checkout\.git")) {
    Invoke-Checked 'clone' { git clone --quiet https://github.com/redrob-labs/redrob-canvas.git $Checkout }
}
Set-Location $Checkout
# The checkout must be byte-identical to the tag, so line endings are not converted.
Invoke-Checked 'config' { git config core.autocrlf false }
Invoke-Checked 'remote' { git remote set-url origin https://github.com/redrob-labs/redrob-canvas.git }
Invoke-Checked 'fetch' { git fetch --quiet --force --tags origin '+refs/heads/*:refs/remotes/origin/*' }
Invoke-Checked 'checkout' { git checkout --quiet --force --detach $Tag }
# -Tag may also be origin/<branch>, to rehearse before tagging; the archive is then named for the
# Cargo.toml version.
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"$' | Select-Object -First 1).Matches[0].Groups[1].Value
if ($Tag -like 'v*' -and $Tag.TrimStart('v') -ne $version) { throw "$Tag but Cargo.toml says $version" }
Write-Host "== $(git rev-parse --short HEAD) $Tag"

Invoke-Checked 'cargo fetch' { cargo fetch --locked }
Invoke-Checked 'configure' { cmake -S native -B build/qt -G Ninja -DCMAKE_BUILD_TYPE=Release -DREDROB_ENABLE_GEGL=OFF -DREDROB_ENABLE_KRITA=OFF }
Invoke-Checked 'build' { cmake --build build/qt }
# The smoke tests run on the offscreen platform, so they pass over SSH with no desktop.
Invoke-Checked 'ctest' { ctest --test-dir build/qt --output-on-failure }

$work = Join-Path ([IO.Path]::GetTempPath()) ("redrob-release-" + [guid]::NewGuid())
$stage = Join-Path $work 'stage'
Invoke-Checked 'install' { cmake --install build/qt --prefix $stage }
$exe = Join-Path $stage 'bin\Redrob Canvas.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "installed executable not found at $exe" }
Invoke-Checked 'windeployqt' { windeployqt.exe $exe --qmldir "$Checkout\qml" --release }

$product = (Get-Item -LiteralPath $exe).VersionInfo.ProductName
if ($product -ne 'Redrob Canvas') { throw "version resource ProductName is '$product'" }

New-Item -ItemType Directory -Force $Out | Out-Null
$zip = Join-Path $Out "redrob-canvas-$version-windows-x86_64.unsigned.zip"
if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Remove-Item -LiteralPath $work -Recurse -Force
Get-Item -LiteralPath $zip | Select-Object Name, Length
