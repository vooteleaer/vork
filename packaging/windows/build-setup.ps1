# Builds vork-setup-<version>.exe: vØrk the network for Windows in one
# installer (ADR-0029). Storage isn't part of it.
#
#     powershell -ExecutionPolicy Bypass -File packaging\windows\build-setup.ps1 [-Out DIR]
#
# Needs Rust (MSVC). It holds the node (vork.exe), the window and tray icon
# (vork-net.exe, clients\net-desktop) and wintun.dll, downloaded from
# wintun.net and checked against its known SHA-256.
param([string]$Out = "")

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not $Out) { $Out = Join-Path $Root "target\setup" }
$Stage = Join-Path $Root "target\setup-files"
# A target folder of its own: programs running from target\release (a test
# node) can't be overwritten, and aren't.
$env:CARGO_TARGET_DIR = Join-Path $Root "target\setup-build"
$Built = "$env:CARGO_TARGET_DIR\release"
$Wintun = @{ Url = "https://www.wintun.net/builds/wintun-0.14.1.zip"; Sha256 = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51" }

function Step($what) { Write-Host "== $what" }
function Invoke-Cargo { & cargo.exe @args; if ($LASTEXITCODE -ne 0) { throw "cargo $args failed" } }

# The version (ADR-0029): the node crate's, a release's when this commit is
# tagged with it, else a build towards it counted in commits.
$Version = [regex]::Match((Get-Content -Raw "$Root\Cargo.toml"), '(?ms)^\[package\].*?^version = "([^"]+)"').Groups[1].Value
if (-not $Version) { throw "no [package] version in Cargo.toml" }
$Count = (& git -C $Root rev-list --count HEAD)
# Untagged, git says so on stderr, which Windows PowerShell turns into a
# terminating error under "Stop": tags are listed instead.
$Tag = (& git -C $Root tag --points-at HEAD) | Where-Object { $_ -eq "v$Version" } | Select-Object -First 1
$Name = if ($Tag -eq "v$Version") { $Version } else { "$Version-dev.$Count" }

Step "vork $Name"
Invoke-Cargo build --release --locked --manifest-path "$Root\Cargo.toml" -p vork
Invoke-Cargo build --release --locked --manifest-path "$Root\clients\net-desktop\Cargo.toml"

Step "wintun"
$Cache = Join-Path $Root "target\wintun"
New-Item -ItemType Directory -Force $Cache | Out-Null
$Zip = Join-Path $Cache (Split-Path $Wintun.Url -Leaf)
if (-not (Test-Path $Zip)) { Invoke-WebRequest -UseBasicParsing $Wintun.Url -OutFile $Zip }
$Got = (Get-FileHash -Algorithm SHA256 $Zip).Hash.ToLower()
if ($Got -ne $Wintun.Sha256) { Remove-Item $Zip; throw "wintun: SHA-256 $Got, expected $($Wintun.Sha256)" }
Expand-Archive -Force $Zip $Cache

Step "files"
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
New-Item -ItemType Directory -Force $Stage | Out-Null
Copy-Item "$Built\vork.exe" $Stage
Copy-Item "$Built\vork-net.exe" $Stage
Copy-Item "$Cache\wintun\bin\amd64\wintun.dll" $Stage
Copy-Item "$Root\LICENSE" "$Stage\LICENSE-vork.txt"
Copy-Item "$Cache\wintun\LICENSE.txt" "$Stage\LICENSE-wintun.txt"
Get-ChildItem $Stage | ForEach-Object { "   {0,-28} {1,8:N0} KB" -f $_.Name, ($_.Length / 1KB) }

Step "installer"
$env:VORK_SETUP_FILES = $Stage
$env:VORK_SETUP_VERSION = $Name
Invoke-Cargo build --release --manifest-path "$PSScriptRoot\setup\Cargo.toml"
Remove-Item Env:VORK_SETUP_FILES, Env:VORK_SETUP_VERSION
New-Item -ItemType Directory -Force $Out | Out-Null
$Setup = Join-Path $Out "vork-setup-$Name.exe"
Copy-Item "$Built\vork-setup.exe" $Setup
"== $Setup ({0:N1} MB)" -f ((Get-Item $Setup).Length / 1MB)
