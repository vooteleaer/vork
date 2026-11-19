# Signs a release with the owner's release key and puts it on the release node
# (ADR-0028); the owner's nodes take it from there by themselves.
#
#   .\packaging\release.ps1 -Linux vork-linux -Windows vork.exe [-Version 0.6.0]
#
# The key stays on this machine ($env:USERPROFILE\.vork\release.key, made with
# `vork release keygen`); only the signed release goes to the server.
param(
    [string]$Linux,
    [string]$Windows,
    [string]$Version = "",
    [string]$Key = "$env:USERPROFILE\.vork\release.key",
    [string]$Server = "root@10.10.30.2",
    [string]$ServerDir = "/var/lib/vork/releases"
)
$ErrorActionPreference = "Stop"
$vork = if ($Windows) { $Windows } else { "vork.exe" }
$dir = Join-Path $env:TEMP "vork-release"
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
$files = @()
if ($Linux) { $files += "linux-x86_64=$((Resolve-Path $Linux).Path)" }
if ($Windows) { $files += "windows-x86_64=$((Resolve-Path $Windows).Path)" }
if (-not $files) { throw "give -Linux and/or -Windows" }
& $vork release make --key $Key --dir $dir --version $Version @files
if ($LASTEXITCODE) { throw "release make failed" }
scp -o BatchMode=yes -r "$dir/*" "${Server}:$ServerDir/"
if ($LASTEXITCODE) { throw "copying to $Server failed" }
# The release node takes it at once (the others within hours, or `vork update`).
ssh -o BatchMode=yes $Server "vork update --config /etc/vork/vork.toml"
"release sent to ${Server}:$ServerDir"
