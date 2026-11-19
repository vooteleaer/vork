# Signs a release with the owner's release key and puts it on the release node
# (ADR-0028); the owner's nodes take it from there by themselves.
#
#   .\packaging\release.ps1 -Linux vork-linux -Windows vork.exe [-Version 0.6.0] -Server root@release-node
#
# The release node is -Server, or $env:VORK_RELEASE_SERVER (user@host for ssh).
#
# The key stays on this machine ($env:USERPROFILE\.vork\release.key, made with
# `vork release keygen`); only the signed release goes to the server.
param(
    [string]$Linux,
    [string]$Windows,
    [string]$Version = "",
    [string]$Key = "$env:USERPROFILE\.vork\release.key",
    [string]$Server = $env:VORK_RELEASE_SERVER,
    [string]$ServerDir = "/var/lib/vork/releases"
)
$ErrorActionPreference = "Stop"
if (-not $Server) { throw "give -Server user@host (the release node), or set VORK_RELEASE_SERVER" }
# The vork that signs: the Windows binary being released, or the one on PATH.
$vork = if ($Windows) { (Resolve-Path $Windows).Path } else { "vork.exe" }
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
