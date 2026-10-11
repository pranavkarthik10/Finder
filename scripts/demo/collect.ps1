# Measures fsearch, Everything and Windows Search on every keystroke of the
# demo's queries, for render.py. Needs PowerShell 7, the fsearch daemon and
# Everything running.   pwsh -File scripts/demo/collect.ps1   (writes target/demo.json)
param(
    [string]$Out = (Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) 'target/demo.json'),
    [string]$EverythingDll = "$env:USERPROFILE\tools\everything\sdk\dll\Everything64.dll",
    [string[]]$Typed = @('chrome.exe', 'main.rs', 'readme'),
    [string[]]$Bench = @('chrome.exe', 'main.rs', 'readme', 'invoice', 'cargo.toml', 'screenshot', 'zzqqxxjj')
)
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
& (Join-Path $root 'target\release\fsearch.exe') status *> $null

# fsearch: its daemon socket.
$sock = [System.Net.Sockets.Socket]::new([System.Net.Sockets.AddressFamily]::Unix, [System.Net.Sockets.SocketType]::Stream, [System.Net.Sockets.ProtocolType]::Unspecified)
$sock.Connect([System.Net.Sockets.UnixDomainSocketEndPoint]::new((Join-Path $env:LOCALAPPDATA 'FSearch\fsearch.sock')))
$ns = [System.Net.Sockets.NetworkStream]::new($sock)
$rd = [System.IO.StreamReader]::new($ns); $wr = [System.IO.StreamWriter]::new($ns); $wr.AutoFlush = $true
function FSearch([string]$q) {
    $wr.WriteLine((@{ q = $q; limit = 50 } | ConvertTo-Json -Compress))
    @(($rd.ReadLine() | ConvertFrom-Json).hits | ForEach-Object { $_.path })
}

# Everything: its SDK (IPC to the running Everything).
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class Ev {
  const string D = @"$EverythingDll";
  [DllImport(D, CharSet = CharSet.Unicode)] public static extern void Everything_SetSearchW(string s);
  [DllImport(D)] public static extern void Everything_SetMax(uint n);
  [DllImport(D)] public static extern void Everything_SetRequestFlags(uint f);
  [DllImport(D)] public static extern void Everything_SetSort(uint s);
  [DllImport(D)] public static extern bool Everything_QueryW(bool wait);
  [DllImport(D)] public static extern uint Everything_GetNumResults();
  [DllImport(D, CharSet = CharSet.Unicode)] public static extern uint Everything_GetResultFullPathNameW(uint i, StringBuilder b, uint n);
}
"@
function Everything([string]$q) {
    [Ev]::Everything_SetSearchW($q); [Ev]::Everything_SetMax(50); [Ev]::Everything_SetRequestFlags(3); [Ev]::Everything_SetSort(1)
    [void][Ev]::Everything_QueryW($true)
    $n = [Ev]::Everything_GetNumResults()
    $sb = [System.Text.StringBuilder]::new(1024)
    @(for ($i = 0; $i -lt $n; $i++) { [void][Ev]::Everything_GetResultFullPathNameW($i, $sb, 1024); $sb.ToString() })
}

# Windows Search: word-prefix full-text match on names, as Explorer's box does.
$conn = New-Object -ComObject ADODB.Connection
$conn.Open("Provider=Search.CollatorDSO;Extended Properties='Application=Windows';")
function WinSearch([string]$q) {
    $e = $q.Replace("'", "''")
    $rs = $conn.Execute("SELECT TOP 50 System.ItemPathDisplay FROM SYSTEMINDEX WHERE SCOPE='file:' AND CONTAINS(System.FileName, '""$e*""')")
    $out = @(while (-not $rs.EOF) { $rs.Fields.Item(0).Value; $rs.MoveNext() })
    $rs.Close(); $out
}

# Median time over `runs` (after 2 warm-ups), and the last run's results.
function Time([scriptblock]$f, [string]$q, [int]$runs = 7) {
    $t = @(); $res = $null
    for ($i = 0; $i -lt $runs + 2; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew(); $res = & $f $q; $sw.Stop()
        if ($i -ge 2) { $t += $sw.Elapsed.TotalMilliseconds }
    }
    $s = $t | Sort-Object
    @{ ms = [math]::Round($s[[int]($s.Count / 2)], 3); n = $res.Count; top = [string[]]@($res | Select-Object -First 7) }
}

$engines = @{ fsearch = ${function:FSearch}; everything = ${function:Everything}; windows = ${function:WinSearch} }
$typedOut = [System.Collections.Generic.List[object]]::new()
foreach ($q in $Typed) {
    $steps = [System.Collections.Generic.List[object]]::new()
    for ($k = 1; $k -le $q.Length; $k++) {
        $p = $q.Substring(0, $k)
        $r = @{ prefix = $p }
        foreach ($e in $engines.Keys) { $r[$e] = Time $engines[$e] $p }
        $steps.Add($r)
    }
    $typedOut.Add(@{ query = $q; steps = $steps.ToArray() })
}
$benchOut = [System.Collections.Generic.List[object]]::new()
foreach ($q in $Bench) {
    $r = @{ query = $q }
    foreach ($e in $engines.Keys) { $r[$e] = Time $engines[$e] $q 21 }
    $benchOut.Add($r)
}
$entries = (& (Join-Path $root 'target/release/fsearch.exe') status | ConvertFrom-Json).entries
$json = ConvertTo-Json -Depth 10 -InputObject @{ entries = $entries; home = $env:USERPROFILE; typed = $typedOut.ToArray(); bench = $benchOut.ToArray() }
[System.IO.File]::WriteAllText($Out, $json, [System.Text.UTF8Encoding]::new($false))
$conn.Close(); $sock.Close()
