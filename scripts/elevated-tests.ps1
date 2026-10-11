# Things that need admin: an MFT scan, timed.
# Run elevated; everything goes to target\elevated.log.
$root = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $root 'target\dev\release\fsearch.exe'
$log = Join-Path $root 'target\elevated.log'
Set-Content $log "elevated run $(Get-Date)"

foreach ($i in 1..2) {
    "mft scan #${i}:" | Add-Content $log
    & $exe scan 2>&1 | Add-Content $log
}

'done' | Add-Content $log
'done' | Add-Content $log
