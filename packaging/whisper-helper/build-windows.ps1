# Build the unsigned optional Whisper helper for Windows and package it as a
# .tar.gz with bin/whisper-cli.exe inside (system tar.exe, so Solmu can unpack
# it without third-party tools). The MSVC runtime is linked statically so the
# binary runs without a vcredist install.
# Usage: build-windows.ps1 -SourceDir <dir> -OutDir <dir> -Arch <x64|arm64>
param(
  [Parameter(Mandatory = $true)][string]$SourceDir,
  [Parameter(Mandatory = $true)][string]$OutDir,
  [Parameter(Mandatory = $true)][ValidateSet('x64', 'arm64')][string]$Arch
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$pins = . (Join-Path $here 'read-pins.ps1') -Path (Join-Path $here 'pins.env')
$vsArch = if ($Arch -eq 'x64') { 'x64' } else { 'ARM64' }
# Archive names follow the documented target spelling (x86_64), not CMake's x64.
$archName = if ($Arch -eq 'x64') { 'x86_64' } else { 'arm64' }
# GGML_NATIVE=OFF alone still enables AVX2/FMA/BMI2 in pinned GGML on x64.
# Use a portable baseline so the helper runs on any x64 machine.
# On ARM64, pinned GGML requires GGML_NATIVE=ON with MSVC proper and then
# forwards host feature probes (dotprod, i8mm, sve) as __ARM_FEATURE_* defines,
# so the archive would demand the build host's optional instructions. Build
# ARM64 with clang-cl instead (upstream's prescribed route) pinned to the
# plain armv8-a baseline.
$cpuFlags = @()
$toolsetArgs = @()
if ($Arch -eq 'x64') {
  $cpuFlags += @('-DGGML_NATIVE=OFF', '-DGGML_SSE42=OFF', '-DGGML_AVX=OFF',
    '-DGGML_AVX2=OFF', '-DGGML_FMA=OFF', '-DGGML_F16C=OFF', '-DGGML_BMI2=OFF')
} else {
  $cpuFlags += @('-DGGML_NATIVE=OFF', '-DGGML_CPU_ARM_ARCH=armv8-a')
  $toolsetArgs = @('-T', 'clangcl')
}
$build = Join-Path $OutDir 'build'
cmake -S $SourceDir -B $build -A $vsArch @toolsetArgs `
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DGGML_OPENMP=OFF `
  -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded `
  -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF `
  -DWHISPER_BUILD_EXAMPLES=ON @cpuFlags
if ($LASTEXITCODE -ne 0) { throw "cmake configure failed ($LASTEXITCODE)" }
cmake --build $build --config Release --target whisper-cli --parallel
if ($LASTEXITCODE -ne 0) { throw "cmake build failed ($LASTEXITCODE)" }
$stage = Join-Path $OutDir 'package'
New-Item -ItemType Directory -Force (Join-Path $stage 'bin') | Out-Null
Copy-Item (Join-Path $build 'bin\Release\whisper-cli.exe') (Join-Path $stage 'bin\whisper-cli.exe')
Copy-Item (Join-Path $SourceDir 'LICENSE') (Join-Path $stage 'whisper-LICENSE')
(Get-Content (Join-Path $here 'README-runtime.txt') -Raw) `
  -replace '@MODEL_NAME@', $pins['MODEL_NAME'] `
  -replace '@MODEL_URL@', $pins['MODEL_URL'] `
  -replace '@MODEL_BYTES@', $pins['MODEL_BYTES'] `
  -replace '@MODEL_SHA256@', $pins['MODEL_SHA256'] |
  Set-Content -NoNewline (Join-Path $stage 'README.txt')
$dist = Join-Path $OutDir 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$archive = "solmu-whisper-v$($pins['WHISPER_VERSION'])-windows-$archName.tar.gz"
tar -czf (Join-Path $dist $archive) -C $stage bin whisper-LICENSE README.txt
if ($LASTEXITCODE -ne 0) { throw "tar failed ($LASTEXITCODE)" }
Write-Output (Join-Path $dist $archive)
