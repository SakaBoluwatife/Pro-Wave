# Builds Ovrtone and makes both the update patch and the full installer, offline.
# No internet and no Claude needed. From the repo root, in PowerShell:
#
#   powershell -ExecutionPolicy Bypass -File dist\windows\build-release.ps1
#
# The version comes from crates\ovrtone-app\Cargo.toml (keep installer.iss's AppVersion the same).
# Both installers land in dist\windows\output.

$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

# The version, as written in Cargo.toml (for example 1.1.1).
$line = Select-String -Path "crates\ovrtone-app\Cargo.toml" -Pattern '^version = "([0-9.]+)"' | Select-Object -First 1
if (-not $line) { throw "No version in crates\ovrtone-app\Cargo.toml" }
$version = $line.Matches[0].Groups[1].Value
$fileVersion = "$version.0"
Write-Host "Ovrtone $version" -ForegroundColor Cyan

# Inno Setup's compiler: installed per user here, or in Program Files.
$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found" }

Write-Host "1/3 Building the program (release, offline)..." -ForegroundColor Cyan
cargo build --release -p ovrtone-app --features asio --offline
if ($LASTEXITCODE -ne 0) { throw "The build failed" }

Set-Location (Join-Path $root "dist\windows")

Write-Host "2/3 Update patch..." -ForegroundColor Cyan
& $iscc /Q "/DPatchVersion=$version" "/DPatchFileVersion=$fileVersion" patch.iss
if ($LASTEXITCODE -ne 0) { throw "The update patch failed" }

Write-Host "3/3 Full installer..." -ForegroundColor Cyan
& $iscc /Q installer.iss
if ($LASTEXITCODE -ne 0) { throw "The full installer failed" }

Write-Host "Done:" -ForegroundColor Green
Get-ChildItem output | Sort-Object LastWriteTime -Descending | Select-Object -First 2 Name, @{n="MB";e={[math]::Round($_.Length/1MB,1)}}
