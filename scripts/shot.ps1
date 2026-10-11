# Screenshot one window (by process name) to a PNG, without bringing it
# forward: PrintWindow renders it even when covered.
#   shot.ps1 [process] [out.png]
param([string]$Name = 'finder', [string]$Out = "$PSScriptRoot\..\target\shot.png")
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
"@
[W]::SetProcessDPIAware() | Out-Null
$p = Get-Process $Name -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$r = New-Object W+RECT
[W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
$w = $r.R - $r.L; $h = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$dc = $g.GetHdc()
[W]::PrintWindow($p.MainWindowHandle, $dc, 2) | Out-Null  # PW_RENDERFULLCONTENT
$g.ReleaseHdc($dc)
# Popups the window owns (Quick Look's hosted previews), drawn where they sit.
$owned = New-Object System.Collections.ArrayList
[W]::EnumWindows({ param($hw, $l) if ([W]::GetWindow($hw, 4) -eq $p.MainWindowHandle -and [W]::IsWindowVisible($hw)) { [void]$owned.Add($hw) }; $true }, [IntPtr]::Zero) | Out-Null
foreach ($hw in $owned) {
  $o = New-Object W+RECT
  [W]::GetWindowRect($hw, [ref]$o) | Out-Null
  $ow = $o.R - $o.L; $oh = $o.B - $o.T
  if ($ow -le 0 -or $oh -le 0) { continue }
  $ob = New-Object System.Drawing.Bitmap $ow, $oh
  $og = [System.Drawing.Graphics]::FromImage($ob)
  $odc = $og.GetHdc(); [W]::PrintWindow($hw, $odc, 2) | Out-Null; $og.ReleaseHdc($odc)
  $g.DrawImage($ob, $o.L - $r.L, $o.T - $r.T)
}
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
"$w x $h -> $Out"
