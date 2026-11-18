# Inference smoke test for a built helper binary: fetch the pinned CI-only
# smoke model, transcribe the pinned source's JFK sample and require the
# expected phrase.
# Usage: smoke-test.ps1 -Binary <whisper-cli.exe> -SourceDir <dir> -WorkDir <dir>
param(
  [Parameter(Mandatory = $true)][string]$Binary,
  [Parameter(Mandatory = $true)][string]$SourceDir,
  [Parameter(Mandatory = $true)][string]$WorkDir
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$pins = . (Join-Path $here 'read-pins.ps1') -Path (Join-Path $here 'pins.env')
New-Item -ItemType Directory -Force $WorkDir | Out-Null
$model = Join-Path $WorkDir $pins['SMOKE_MODEL_NAME']
curl.exe --fail --location --retry 2 --silent --show-error $pins['SMOKE_MODEL_URL'] -o $model
if ($LASTEXITCODE -ne 0) { throw "model download failed ($LASTEXITCODE)" }
$actualBytes = (Get-Item $model).Length
if ($actualBytes -ne [uint64]$pins['SMOKE_MODEL_BYTES']) { throw "Smoke model size mismatch: $actualBytes" }
$actualHash = (Get-FileHash -Algorithm SHA256 $model).Hash.ToLowerInvariant()
if ($actualHash -ne $pins['SMOKE_MODEL_SHA256']) { throw "Smoke model hash mismatch: $actualHash" }
$outBase = Join-Path $WorkDir 'jfk'
& $Binary -m $model -f (Join-Path $SourceDir 'samples\jfk.wav') -nt -otxt -of $outBase | Out-Null
if ($LASTEXITCODE -ne 0) { throw "whisper-cli failed ($LASTEXITCODE)" }
$transcript = Get-Content ($outBase + '.txt') -Raw
if ($transcript -notmatch [regex]::Escape($pins['SMOKE_EXPECTED'])) {
  throw "Smoke test transcript missing the expected phrase: $transcript"
}
Write-Output "Smoke test passed: $Binary transcribed jfk.wav with $($pins['SMOKE_MODEL_NAME'])"
