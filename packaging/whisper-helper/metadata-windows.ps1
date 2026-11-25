# Write the per-target metadata JSON the Solmu installer consumes, then print
# it. Nothing is uploaded; the log is the report.
# Usage: metadata-windows.ps1 -Archive <file> -ExecutableFile <file> -ExecutablePath <path-in-archive> -Target <triple>
param(
  [Parameter(Mandatory = $true)][string]$Archive,
  [Parameter(Mandatory = $true)][string]$ExecutableFile,
  [Parameter(Mandatory = $true)][string]$ExecutablePath,
  [Parameter(Mandatory = $true)][string]$Target
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$pins = . (Join-Path $here 'read-pins.ps1') -Path (Join-Path $here 'pins.env')
$metadata = [ordered]@{
  target = $Target
  filename = (Split-Path -Leaf $Archive)
  size = (Get-Item $Archive).Length
  bytes = (Get-Item $Archive).Length
  sha256 = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
  executable = $ExecutablePath
  executable_sha256 = (Get-FileHash -Algorithm SHA256 $ExecutableFile).Hash.ToLowerInvariant()
  upstream_revision = $pins['WHISPER_REVISION']
  signed = $false
  notarized = $false
}
$json = $metadata | ConvertTo-Json
[System.IO.File]::WriteAllText(($Archive + '.json'), $json)
Write-Output $json
