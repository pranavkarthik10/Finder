# Drive the app for screenshots: input.ps1 click X Y [double] | key KEY... | move X Y
# Coordinates are window-relative pixels (as in target\shot.png).
param([string]$Action, [Parameter(ValueFromRemainingArguments)][string[]]$Rest)
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class In {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(int f, int x, int y, int d, int e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte s, int f, int e);
}
"@
[In]::SetProcessDPIAware() | Out-Null
$p = Get-Process finder | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[In]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 150
$r = New-Object In+RECT
[In]::DwmGetWindowAttribute($p.MainWindowHandle, 9, [ref]$r, 16) | Out-Null
$vk = @{ down = 0x28; up = 0x26; left = 0x25; right = 0x27; enter = 0x0D; back = 0x08; ctrl = 0x11; alt = 0x12; esc = 0x1B; tab = 0x09 }
switch ($Action) {
  'move' { [In]::SetCursorPos($r.L + [int]$Rest[0], $r.T + [int]$Rest[1]) | Out-Null }
  'click' {
    # Arrive with a move first: the app hit-tests at the last move.
    [In]::SetCursorPos($r.L + [int]$Rest[0] - 3, $r.T + [int]$Rest[1]) | Out-Null
    Start-Sleep -Milliseconds 60
    [In]::SetCursorPos($r.L + [int]$Rest[0], $r.T + [int]$Rest[1]) | Out-Null
    Start-Sleep -Milliseconds 120
    $n = if ($Rest.Count -gt 2 -and $Rest[2] -eq 'double') { 2 } else { 1 }
    for ($i = 0; $i -lt $n; $i++) { [In]::mouse_event(2, 0, 0, 0, 0); [In]::mouse_event(4, 0, 0, 0, 0); Start-Sleep -Milliseconds 40 }
  }
  'key' {
    foreach ($k in $Rest) {
      $parts = $k -split '\+'
      foreach ($q in $parts) { [In]::keybd_event([byte]$vk[$q], 0, 0, 0) }
      [array]::Reverse($parts)
      foreach ($q in $parts) { [In]::keybd_event([byte]$vk[$q], 0, 2, 0) }
      Start-Sleep -Milliseconds 60
    }
  }
}
Start-Sleep -Milliseconds 250
