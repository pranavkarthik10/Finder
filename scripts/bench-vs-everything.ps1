# fsearch vs Everything, same queries, top 50 with paths, warm medians.
# Starts the fsearch daemon and Everything (elevated: a UAC prompt) if needed.
param([string]$Everything = "$env:USERPROFILE\tools\everything")

$root = Split-Path $PSScriptRoot -Parent
$fs = Join-Path $root 'target\release\fsearch.exe'
$bench = Join-Path $root 'target\release\examples\vs_everything.exe'

& $fs status *> $null   # spawns the daemon if it isn't running
if (-not (Get-Process everything -ErrorAction SilentlyContinue)) {
    Start-Process (Join-Path $Everything 'everything.exe') -ArgumentList '-startup' -Verb RunAs
}

Write-Host 'waiting for both indexes...'
do {
    Start-Sleep -Seconds 1
    $s = & $fs status 2>$null | ConvertFrom-Json
} until ($s.ok)
Write-Host ("fsearch: {0:N0} entries" -f $s.entries)

& $bench (Join-Path $Everything 'sdk\dll\Everything64.dll') @args
while ($LASTEXITCODE -ne 0) {
    Start-Sleep -Seconds 1
    & $bench (Join-Path $Everything 'sdk\dll\Everything64.dll') @args 2>$null
}
