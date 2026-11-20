# Builds vork-<version>.msi: vØrk the network for Windows in one installer
# (ADR-0029). Storage isn't part of it.
#
#     powershell -ExecutionPolicy Bypass -File packaging\windows\build-msi.ps1 [-Out DIR]
#
# Needs Rust (MSVC) and WiX 5 with its Util, Firewall and UI extensions:
#     dotnet tool install --global wix --version 5.0.2
#     wix extension add -g WixToolset.Util.wixext/5.0.2 WixToolset.Firewall.wixext/5.0.2 WixToolset.UI.wixext/5.0.2
# (WiX 5 is MS-RL licensed; later versions ask a fee of businesses.) It holds
# the node (vork.exe), the window and tray icon (vork-net.exe,
# clients\net-desktop) and wintun.dll, downloaded from wintun.net and checked
# against its known SHA-256.
param([string]$Out = "")

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not $Out) { $Out = Join-Path $Root "target\msi" }
$Stage = Join-Path $Root "target\msi-files"
# A target folder of its own: programs running from target\release (a test
# node) can't be overwritten, and aren't.
$env:CARGO_TARGET_DIR = Join-Path $Root "target\msi-build"
$Built = "$env:CARGO_TARGET_DIR\release"
$Wintun = @{ Url = "https://www.wintun.net/builds/wintun-0.14.1.zip"; Sha256 = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51" }

function Step($what) { Write-Host "== $what" }
function Invoke-Native { $exe, $rest = $args; & $exe @rest; if ($LASTEXITCODE -ne 0) { throw "$exe $rest failed" } }

$Wix = (Get-Command wix -ErrorAction SilentlyContinue).Source
if (-not $Wix) { $Wix = Join-Path $env:USERPROFILE ".dotnet\tools\wix.exe" }
if (-not (Test-Path $Wix)) { throw "WiX not found: dotnet tool install --global wix --version 5.0.2" }

# The version (ADR-0029): the node crate's, a release's when this commit is
# tagged with it, else a build towards it counted in commits. An MSI's own
# version is numbers only: major.minor.build, the build being the commit count.
$Version = [regex]::Match((Get-Content -Raw "$Root\Cargo.toml"), '(?ms)^\[package\].*?^version = "([^"]+)"').Groups[1].Value
if (-not $Version) { throw "no [package] version in Cargo.toml" }
$Count = [int](& git -C $Root rev-list --count HEAD)
# Untagged, git says so on stderr, which Windows PowerShell turns into a
# terminating error under "Stop": tags are listed instead.
$Tag = (& git -C $Root tag --points-at HEAD) | Where-Object { $_ -eq "v$Version" } | Select-Object -First 1
$Name = if ($Tag -eq "v$Version") { $Version } else { "$Version-dev.$Count" }
$Parts = $Version.Split("-")[0].Split(".")
$MsiVersion = "$($Parts[0]).$($Parts[1]).$Count"

Step "vork $Name (MSI $MsiVersion)"
Invoke-Native cargo build --release --locked --manifest-path "$Root\Cargo.toml" -p vork
Invoke-Native cargo build --release --locked --manifest-path "$Root\clients\net-desktop\Cargo.toml"

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

# The licence page: LICENSE as RTF (escaped, a paragraph per line).
$Rtf = Join-Path $Stage "license.rtf"
$Body = (Get-Content "$Root\LICENSE") | ForEach-Object { ($_ -replace '\\', '\\' -replace '\{', '\{' -replace '\}', '\}') + '\par' }
Set-Content -Encoding ascii $Rtf ("{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}\fs18 " + ($Body -join "`r`n") + "}")

Step "installer"
New-Item -ItemType Directory -Force $Out | Out-Null
$Msi = Join-Path $Out "vork-$Name.msi"
Invoke-Native $Wix build -arch x64 -nologo `
    -ext WixToolset.Util.wixext -ext WixToolset.Firewall.wixext -ext WixToolset.UI.wixext `
    -d "Version=$MsiVersion" -d "Files=$Stage" -d "License=$Rtf" `
    -o $Msi "$PSScriptRoot\vork.wxs"
Remove-Item (Join-Path $Out "vork-$Name.wixpdb") -ErrorAction SilentlyContinue
"== $Msi ({0:N1} MB)" -f ((Get-Item $Msi).Length / 1MB)
