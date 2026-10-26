# Smoke test on the CI runner: start Notch.exe, make sure it stays alive,
# hover it open, flip through the pages and post small screenshots as
# annotations (base64 PNG) so the layout can be checked without a Windows box.
param([string]$Exe = "dist/Notch.exe")

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Mouse {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    public static void Click(int x, int y) { SetCursorPos(x, y); mouse_event(2, 0, 0, 0, UIntPtr.Zero); mouse_event(4, 0, 0, 0, UIntPtr.Zero); }
}
"@

$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds 4
if ($p.HasExited) {
    Write-Output "::error title=smoke::Notch.exe exited during startup (code $($p.ExitCode))"
    exit 1
}
$h = [Mouse]::FindWindow("NotchWindow", $null)
if ($h -eq [IntPtr]::Zero) {
    Write-Output "::error title=smoke::NotchWindow not found"
    Stop-Process -Id $p.Id -Force
    exit 1
}
$r = New-Object Mouse+RECT
[void][Mouse]::GetWindowRect($h, [ref]$r)
$scr = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Write-Output "::notice title=smoke-info::screen $($scr.Width)x$($scr.Height) window $($r.L),$($r.T),$($r.R),$($r.B)"
$W = $r.R - $r.L
$H = $r.B - $r.T
$s = $W / 560.0

function Shot([string]$name, [int]$h) {
    $bmp = New-Object System.Drawing.Bitmap $W, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $b64 = [Convert]::ToBase64String($ms.ToArray())
    Write-Output "::notice title=shot-$name::$b64"
    $g.Dispose(); $bmp.Dispose()
}

# park the mouse away, collapsed pill
[void][Mouse]::SetCursorPos(5, 600)
Start-Sleep -Milliseconds 800
Shot "pill" ([int](60 * $s))

# hover to open
$cx = $r.L + [int](280 * $s)
[void][Mouse]::SetCursorPos($cx, $r.T + [int](12 * $s))
Start-Sleep -Milliseconds 300
[void][Mouse]::SetCursorPos($cx + 2, $r.T + [int](14 * $s))
Start-Sleep -Milliseconds 1200
Shot "page0" ([int](150 * $s))

# next-page arrow at island x 398, y 70 (island left = canvas 70)
for ($i = 1; $i -le 4; $i++) {
    [Mouse]::Click($r.L + [int]((70 + 398) * $s), $r.T + [int](70 * $s))
    Start-Sleep -Milliseconds 900
    Shot "page$i" ([int](150 * $s))
}

if ($p.HasExited) {
    Write-Output "::error title=smoke::Notch.exe crashed while interacting (code $($p.ExitCode))"
    exit 1
}
Stop-Process -Id $p.Id -Force
Write-Output "smoke ok"
