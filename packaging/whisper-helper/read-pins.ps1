# Read pins.env (bash `KEY=value` syntax, values may be single-quoted) into a
# hashtable. Usage: $pins = . packaging/whisper-helper/read-pins.ps1 -Path packaging/whisper-helper/pins.env
param([Parameter(Mandatory = $true)][string]$Path)
$pins = @{}
Get-Content $Path | ForEach-Object {
  if ($_ -match '^\s*#' -or $_ -notmatch '=') { return }
  $key, $value = $_ -split '=', 2
  $value = $value.Trim() -replace "^'(.*)'$", '$1'
  $pins[$key.Trim()] = $value
}
return $pins
