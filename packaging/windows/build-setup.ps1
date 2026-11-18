# Builds vork-setup-<version>.exe: vork for Windows in one installer.
#
#     powershell -ExecutionPolicy Bypass -File packaging\windows\build-setup.ps1 [-Out DIR]
#
# Needs Rust (MSVC), Prns 0.3.8 next to vork (..\Prns-0.3.8, or $env:PRNS:
# vorkd's transport and its licenses), and vork-desktop next to it
# (..\vork-desktop, or $env:VORK_DESKTOP) for the utility in the
# notification area; without it the installer has no utility, and nothing
# starts vork at login. No prnsd: vork joins Prns's own (or another
# Reticulum shared instance), installed apart.
param([string]$Out = "")

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path "$PSScriptRoot\..\..").Path
$Prns = if ($env:PRNS) { $env:PRNS } else { Join-Path (Split-Path $Root) "Prns-0.3.8" }
$Desktop = if ($env:VORK_DESKTOP) { $env:VORK_DESKTOP } else { Join-Path (Split-Path $Root) "vork-desktop" }
# The apps, where they are next to vork with a Windows front end: Photos and Map.
$Apps = @(@{ Repo = "vork-photos"; Package = "vork-photos-win"; Exe = "vork-photos.exe" }, @{ Repo = "vork-map"; Package = "vork-map-win"; Exe = "vork-map.exe" }) |
    Where-Object { Test-Path (Join-Path (Split-Path $Root) "$($_.Repo)\windows\Cargo.toml") }
if (-not $Out) { $Out = Join-Path $Root "target\setup" }
$Stage = Join-Path $Root "target\setup-files"
# A target folder of its own: programs running from target\release (a test
# node) can't be overwritten, and aren't.
$env:CARGO_TARGET_DIR = Join-Path $Root "target\setup-build"
$Built = "$env:CARGO_TARGET_DIR\release"

function Step($what) { Write-Host "== $what" }
function Invoke-Cargo { & cargo.exe @args; if ($LASTEXITCODE -ne 0) { throw "cargo $args failed" } }

Step "vork"
Invoke-Cargo build --release --locked --manifest-path "$Root\Cargo.toml" -p vork -p vork-storage-node -p vork-cloudfiles
$HasDesktop = Test-Path "$Desktop\windows\Cargo.toml"
if ($HasDesktop) {
    Step "vork-desktop (from $Desktop)"
    Invoke-Cargo build --release --manifest-path "$Desktop\Cargo.toml" -p vork-desktop-win
} else {
    Write-Warning "no vork-desktop at ${Desktop}: the installer won't have the utility"
}

foreach ($a in $Apps) {
    Step "$($a.Repo)"
    Invoke-Cargo build --release --manifest-path (Join-Path (Split-Path $Root) "$($a.Repo)\Cargo.toml") -p $a.Package
}

Step "files"
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
New-Item -ItemType Directory -Force $Stage | Out-Null
foreach ($b in "vorkd.exe", "vork.exe", "vork-storage.exe", "vork-cloudfiles.exe") { Copy-Item "$Built\$b" $Stage }
if ($HasDesktop) { Copy-Item "$Built\vork-desktop.exe" $Stage }
foreach ($a in $Apps) { Copy-Item "$Built\$($a.Exe)" $Stage }
Copy-Item "$Root\LICENSE" "$Stage\LICENSE-vork.txt"
# The libraries a new install joins (vorkd reads it next to itself).
Copy-Item "$Root\packaging\default-libraries" "$Stage\default-libraries"
# vorkd is built with Prns's crates: their licenses come along.
Copy-Item "$Prns\LICENSE-MIT" "$Stage\LICENSE-prns-MIT.txt"
Copy-Item "$Prns\LICENSE-APACHE" "$Stage\LICENSE-prns-APACHE.txt"
Get-ChildItem $Stage | ForEach-Object { "   {0,-28} {1,8:N0} KB" -f $_.Name, ($_.Length / 1KB) }

Step "installer"
$env:VORK_SETUP_FILES = $Stage
Invoke-Cargo build --release --manifest-path "$PSScriptRoot\setup\Cargo.toml"
Remove-Item Env:VORK_SETUP_FILES
$Version = (Select-String -Path "$Root\crates\vork-storage-node\Cargo.toml" -Pattern '^version = "(.*)"').Matches[0].Groups[1].Value
# A release's name when this commit is tagged with the version, else a build
# towards it, counted in commits (docs/storage/versioning.md).
$Count = (& git -C $Root rev-list --count HEAD)
# Untagged, git says so on stderr, which Windows PowerShell turns into a
# terminating error under "Stop": tags are listed instead.
$Tag = (& git -C $Root tag --points-at HEAD) | Where-Object { $_ -eq "v$Version" } | Select-Object -First 1
$Name = if ($Tag -eq "v$Version") { $Version } else { "$Version-dev.$Count" }
New-Item -ItemType Directory -Force $Out | Out-Null
$Setup = Join-Path $Out "vork-setup-$Name.exe"
Copy-Item "$Built\vork-setup.exe" $Setup
"== $Setup ({0:N1} MB)" -f ((Get-Item $Setup).Length / 1MB)
