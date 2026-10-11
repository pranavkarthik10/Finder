# Times File Explorer searching a whole drive, through UI Automation (no
# keyboard or mouse): opens the search, then watches the window until its
# "Busy" progress bar goes away. Prints time to first result, time to done,
# and how many items it found.
#   pwsh -File scripts/bench-explorer-search.ps1 [query] [location]
param([string]$Query = 'chrome.exe', [string]$Location = 'C:\', [int]$TimeoutMin = 20)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$root = $A::RootElement
$any = [System.Windows.Automation.Condition]::TrueCondition

function Search-Windows { @($root.FindAll('Children', $any) | Where-Object { $_.Current.Name -like '*Search Results*' }) }
function Close-Window($w) {
    try { $w.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern).Close() } catch {}
}
# A clean start: no earlier search windows.
Search-Windows | ForEach-Object { Close-Window $_ }
Start-Sleep -Milliseconds 500

$uri = 'search-ms:query={0}&crumb=location:{1}' -f [uri]::EscapeDataString($Query), [uri]::EscapeDataString($Location)
$sw = [System.Diagnostics.Stopwatch]::StartNew()
Start-Process $uri
$win = $null
while (-not $win -and $sw.Elapsed.TotalSeconds -lt 30) { Start-Sleep -Milliseconds 100; $win = Search-Windows | Select-Object -First 1 }
if (-not $win) { throw 'no search window' }

$progress = New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::ProgressBar)
$statusId = New-Object System.Windows.Automation.PropertyCondition($A::AutomationIdProperty, 'PropertyValue')
function Busy { $null -ne ($win.FindAll('Descendants', $progress) | Where-Object { $_.Current.Name -like 'Busy*' } | Select-Object -First 1) }
function Count {
    $t = $win.FindAll('Descendants', $statusId) | ForEach-Object { $_.Current.Name } | Where-Object { $_ -match '^\d[\d,]* items?' } | Select-Object -First 1
    if ($t -match '^([\d,]+)') { [int]($Matches[1] -replace ',', '') } else { 0 }
}

$first = $null; $seenBusy = $false; $idle = 0
while ($sw.Elapsed.TotalMinutes -lt $TimeoutMin) {
    Start-Sleep -Milliseconds 250
    $b = Busy
    if ($b) { $seenBusy = $true; $idle = 0 } elseif ($seenBusy) { $idle++ }
    if (-not $first -and (Count) -gt 0) { $first = $sw.Elapsed.TotalSeconds }
    # Done once the busy bar has been gone for a second.
    if ($idle -ge 4) { break }
}
$done = $sw.Elapsed.TotalSeconds - 1.0
$n = Count
Close-Window $win
[ordered]@{
    query          = $Query
    location       = $Location
    first_result_s = if ($first) { [math]::Round($first, 1) } else { $null }
    done_s         = [math]::Round($done, 1)
    items          = $n
    timed_out      = $idle -lt 4
} | ConvertTo-Json -Compress
