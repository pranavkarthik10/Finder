# fsearch vs Windows Search (the indexer behind Explorer's search box):
# the same queries, top 50 with paths, warm medians of 20 runs.
# Needs PowerShell 7 (Unix domain sockets) and the fsearch daemon.
#   pwsh -File scripts/bench-vs-windows-search.ps1 [query...]
param([Parameter(ValueFromRemainingArguments)][string[]]$Queries)
if (-not $Queries) { $Queries = 'main.rs', 'chrome.exe', 'readme', 'invoice', 'cargo.toml', 'screenshot', 'zzqqxxjj' }

$root = Split-Path $PSScriptRoot -Parent
& (Join-Path $root 'target\release\fsearch.exe') status *> $null   # starts the daemon if needed

# fsearch: one connection, JSON lines.
$sockPath = Join-Path $env:LOCALAPPDATA 'FSearch\fsearch.sock'
$sock = [System.Net.Sockets.Socket]::new([System.Net.Sockets.AddressFamily]::Unix, [System.Net.Sockets.SocketType]::Stream, [System.Net.Sockets.ProtocolType]::Unspecified)
$sock.Connect([System.Net.Sockets.UnixDomainSocketEndPoint]::new($sockPath))
$stream = [System.Net.Sockets.NetworkStream]::new($sock)
$reader = [System.IO.StreamReader]::new($stream)
$writer = [System.IO.StreamWriter]::new($stream); $writer.AutoFlush = $true
function FSearch([string]$q) {
    $writer.WriteLine((@{ q = $q; limit = 50 } | ConvertTo-Json -Compress))
    ($reader.ReadLine() | ConvertFrom-Json).hits.Count
}

# Windows Search: its OLE DB provider, as Explorer and apps query it.
$conn = New-Object -ComObject ADODB.Connection
$conn.Open("Provider=Search.CollatorDSO;Extended Properties='Application=Windows';")
function WSearch([string]$where) {
    $rs = $conn.Execute("SELECT TOP 50 System.ItemPathDisplay FROM SYSTEMINDEX WHERE SCOPE='file:' AND $where")
    $n = 0
    while (-not $rs.EOF) { $null = $rs.Fields.Item(0).Value; $n++; $rs.MoveNext() }
    $rs.Close()
    $n
}

function Median([scriptblock]$f) {
    $times = @(); $n = 0
    for ($i = 0; $i -lt 23; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $n = & $f
        $sw.Stop()
        if ($i -ge 3) { $times += $sw.Elapsed.TotalMilliseconds }
    }
    $sorted = $times | Sort-Object
    [pscustomobject]@{ ms = $sorted[[int]($sorted.Count / 2)]; n = $n }
}

$rows = foreach ($q in $Queries) {
    $esc = $q.Replace("'", "''")
    $a = Median { FSearch $q }
    $b = Median { FSearch "$q in:C:/Users" }
    $c = Median { WSearch "System.FileName LIKE '%$esc%'" }
    $d = Median { WSearch "CONTAINS(System.FileName, '""$esc*""')" }
    [pscustomobject]@{
        Query                    = $q
        'fsearch (whole disk)'   = '{0,7:N2} ms [{1}]' -f $a.ms, $a.n
        'fsearch (C:\Users)'     = '{0,7:N2} ms [{1}]' -f $b.ms, $b.n
        'WinSearch LIKE'         = '{0,7:N2} ms [{1}]' -f $c.ms, $c.n
        'WinSearch CONTAINS'     = '{0,7:N2} ms [{1}]' -f $d.ms, $d.n
    }
}
$rows | Format-Table -AutoSize | Out-String -Width 200
$conn.Close(); $sock.Close()
