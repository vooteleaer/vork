# Builds vork-storage-<version>.msi: vØrk storage for Windows, for the user
# installing it (no administrator), apart from vØrk the network, which it
# needs (vork-<version>.msi).
#
#     powershell -ExecutionPolicy Bypass -File packaging\windows\build-storage-msi.ps1 [-Out DIR]
#
# Needs Rust (MSVC) and WiX 5 with its Util and UI extensions (see build-msi.ps1).
param([string]$Out = "")

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not $Out) { $Out = Join-Path $Root "target\msi" }
$Stage = Join-Path $Root "target\msi-storage-files"
# A target folder of its own, as for vØrk's MSI.
$env:CARGO_TARGET_DIR = Join-Path $Root "target\msi-build"
$Built = "$env:CARGO_TARGET_DIR\release"

function Step($what) { Write-Host "== $what" }
function Invoke-Native { $exe, $rest = $args; & $exe @rest; if ($LASTEXITCODE -ne 0) { throw "$exe $rest failed" } }

$Wix = (Get-Command wix -ErrorAction SilentlyContinue).Source
if (-not $Wix) { $Wix = Join-Path $env:USERPROFILE ".dotnet\tools\wix.exe" }
if (-not (Test-Path $Wix)) { throw "WiX not found: dotnet tool install --global wix --version 5.0.2" }

# The version as vØrk's (ADR-0029): storage is released with it.
$Version = [regex]::Match((Get-Content -Raw "$Root\Cargo.toml"), '(?ms)^\[package\].*?^version = "([^"]+)"').Groups[1].Value
if (-not $Version) { throw "no [package] version in Cargo.toml" }
$Count = [int](& git -C $Root rev-list --count HEAD)
$Tag = (& git -C $Root tag --points-at HEAD) | Where-Object { $_ -eq "v$Version" } | Select-Object -First 1
$Name = if ($Tag -eq "v$Version") { $Version } else { "$Version-dev.$Count" }
$Parts = $Version.Split("-")[0].Split(".")
$MsiVersion = "$($Parts[0]).$($Parts[1]).$Count"

Step "vork-storage $Name (MSI $MsiVersion)"
Invoke-Native cargo build --release --locked --manifest-path "$Root\Cargo.toml" -p vork-storage-node -p vork-web -p vork-cloudfiles -p vork-extract

Step "files"
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
New-Item -ItemType Directory -Force $Stage | Out-Null
foreach ($f in "vorkd", "vork-storage", "vork-storage-run", "vork-web", "vork-cloudfiles", "vork-extractd") { Copy-Item "$Built\$f.exe" $Stage }
Copy-Item "$Root\LICENSE" "$Stage\LICENSE.txt"
Get-ChildItem $Stage | ForEach-Object { "   {0,-28} {1,8:N0} KB" -f $_.Name, ($_.Length / 1KB) }

$Rtf = Join-Path $Stage "license.rtf"
$Body = (Get-Content "$Root\LICENSE") | ForEach-Object { ($_ -replace '\\', '\\' -replace '\{', '\{' -replace '\}', '\}') + '\par' }
Set-Content -Encoding ascii $Rtf ("{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}\fs18 " + ($Body -join "`r`n") + "}")

Step "installer"
New-Item -ItemType Directory -Force $Out | Out-Null
$Msi = Join-Path $Out "vork-storage-$Name.msi"
Invoke-Native $Wix build -arch x64 -nologo `
    -ext WixToolset.Util.wixext -ext WixToolset.UI.wixext `
    -d "Version=$MsiVersion" -d "Files=$Stage" -d "License=$Rtf" `
    -o $Msi "$PSScriptRoot\vork-storage.wxs"
Remove-Item (Join-Path $Out "vork-storage-$Name.wixpdb") -ErrorAction SilentlyContinue
Step "$Msi ($([math]::Round((Get-Item $Msi).Length / 1MB, 1)) MB)"
