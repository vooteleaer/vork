# Builds the vØrk app for Android (net/, a debug APK): the node for each ABI
# with the NDK, into its jniLibs, then Gradle. vØrk storage, which runs on
# this app's node, is built in its own repository (vork-storage).
#
#     powershell -ExecutionPolicy Bypass -File clients\android\build-apk.ps1 [-Abis arm64-v8a,x86_64] [-Release]
#
# -Release: shrunk with R8 and signed with the debug key (for testers).
#
# Needs the Android SDK with NDK 27 ($env:ANDROID_HOME, default C:\Android\Sdk),
# JDK 17, and rustup targets aarch64-linux-android, armv7-linux-androideabi
# and x86_64-linux-android.
param([string[]]$Abis = @("arm64-v8a", "x86_64"), [switch]$Release)

$ErrorActionPreference = "Stop"
$Here = $PSScriptRoot
$Vork = (Resolve-Path "$Here\..\..").Path
$Sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { "C:\Android\Sdk" }
$Ndk = Get-ChildItem "$Sdk\ndk" | Sort-Object Name | Select-Object -Last 1
$Bin = "$($Ndk.FullName)\toolchains\llvm\prebuilt\windows-x86_64\bin"
$env:CARGO_TARGET_DIR = Join-Path $Vork "target\android"
$Api = 26

function Invoke-Cargo { & cargo.exe @args; if ($LASTEXITCODE -ne 0) { throw "cargo $args failed" } }

# Only the ABIs asked for go into the APK (an arm64-only one for phones is smaller).
Get-ChildItem "$Here\net\src\main\jniLibs" -Directory -ErrorAction SilentlyContinue |
    Where-Object { $Abis -notcontains $_.Name } | Remove-Item -Recurse -Force

foreach ($abi in $Abis) {
    $triple = @{ "arm64-v8a" = "aarch64-linux-android"; "armeabi-v7a" = "armv7-linux-androideabi"; "x86_64" = "x86_64-linux-android" }[$abi]
    # The NDK's compiler for it (32-bit ARM's is named armv7a-…).
    $clang = "$Bin\$($triple.Replace('armv7-', 'armv7a-'))$Api-clang"
    $var = $triple.ToUpper().Replace("-", "_")
    $under = $triple.Replace("-", "_")
    Set-Item "env:CARGO_TARGET_${var}_LINKER" "$clang.cmd"
    Set-Item "env:CC_$under" "$clang.cmd"
    Set-Item "env:CXX_$under" "$clang++.cmd"
    Set-Item "env:AR_$under" "$Bin\llvm-ar.exe"
    Write-Host "== $abi ($triple)"
    Invoke-Cargo build --release --target $triple --manifest-path "$Vork\Cargo.toml" -p vork
    $built = "$env:CARGO_TARGET_DIR\$triple\release"
    # A program, named as a library so Android extracts it where it may run.
    $out = "$Here\net\src\main\jniLibs\$abi"
    New-Item -ItemType Directory -Force $out | Out-Null
    Copy-Item "$built\vork" "$out\libvork.so"
    & "$Bin\llvm-strip.exe" "$out\libvork.so"
}

Write-Host "== APK"
Push-Location $Here
try {
    & .\gradlew.bat --no-daemon $(if ($Release) { "assembleRelease" } else { "assembleDebug" })
    if ($LASTEXITCODE -ne 0) { throw "gradle failed" }
} finally { Pop-Location }
$apk = if ($Release) { "$Here\net\build\outputs\apk\release\net-release.apk" } else { "$Here\net\build\outputs\apk\debug\net-debug.apk" }
"== $apk ({0:N1} MB)" -f ((Get-Item $apk).Length / 1MB)
