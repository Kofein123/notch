# Smoke test on the CI runner: start Notch.exe, make sure it stays alive,
# hover it open, flip through the pages and save small screenshots as
# the smoke-screenshots artifact so the layout can be checked without a Windows box.
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
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, UIntPtr e);
    public static void Key(byte vk) { keybd_event(vk, 0, 0, UIntPtr.Zero); keybd_event(vk, 0, 2, UIntPtr.Zero); }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    public static void Click(int x, int y) { SetCursorPos(x, y); mouse_event(2, 0, 0, 0, UIntPtr.Zero); mouse_event(4, 0, 0, 0, UIntPtr.Zero); }
}
"@

# isolated data folder: water reminders off so no alert covers the pages
$env:APPDATA = Join-Path $env:RUNNER_TEMP "notch-appdata"
New-Item -ItemType Directory -Force (Join-Path $env:APPDATA "Notch") | Out-Null
Set-Content -Path (Join-Path $env:APPDATA "Notch/settings.txt") -Value "water_on=false`nautostart=false"
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds 4
if ($p.HasExited) {
    Write-Output "::error title=smoke::Notch.exe exited during startup (code $($p.ExitCode))"
    exit 1
}
$h = [Mouse]::FindWindow("NotchWindow", "Notch")
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

New-Item -ItemType Directory -Force shots | Out-Null
function Shot([string]$name, [int]$h) {
    $bmp = New-Object System.Drawing.Bitmap $W, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
    $bmp.Save((Join-Path (Get-Location) "shots/$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
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

function IClick([double]$x, [double]$y) {
    # island-local point (island left = canvas 70) of the expanded layout
    [Mouse]::Click($r.L + [int]((70 + $x) * $s), $r.T + [int]($y * $s))
}

# to-do: Add Task, type a title, give it a time, save with Enter
IClick 81 109
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait("Buy milk")
IClick 256 81
Start-Sleep -Milliseconds 400
Shot "todo-add" ([int](150 * $s))
[System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
Start-Sleep -Milliseconds 500
Shot "todo-saved" ([int](150 * $s))

# pomodoro: go to page 1 and press play
IClick 32 70
Start-Sleep -Milliseconds 300
IClick 32 70
Start-Sleep -Milliseconds 300
IClick 32 70
Start-Sleep -Milliseconds 700
IClick 265 109
Start-Sleep -Milliseconds 1600
Shot "pomo-running" ([int](150 * $s))

# water: drink twice -> bottle (500 ml) empty -> refill alert
IClick 398 70
Start-Sleep -Milliseconds 700
IClick 86 109
Start-Sleep -Milliseconds 200
IClick 86 109
Start-Sleep -Milliseconds 900
Shot "water-refill" ([int](150 * $s))
IClick 320 39
Start-Sleep -Milliseconds 900

# settings panel from the gear in the header
IClick 400 19
Start-Sleep -Milliseconds 900
Shot "settings" ([int](335 * $s))
IClick 50 104
Start-Sleep -Milliseconds 500
Shot "settings-appearance" ([int](335 * $s))
IClick 442 25
Start-Sleep -Milliseconds 700

# leave, let it collapse, then Caps Lock HUD
[void][Mouse]::SetCursorPos(5, 600)
Start-Sleep -Milliseconds 1200
Shot "collapsed" ([int](60 * $s))
[Mouse]::Key(0x14)
Start-Sleep -Milliseconds 450
Shot "hud-caps" ([int](60 * $s))
[Mouse]::Key(0x14)
Start-Sleep -Milliseconds 2200

if ($p.HasExited) {
    Write-Output "::error title=smoke::Notch.exe crashed while interacting (code $($p.ExitCode))"
    exit 1
}
Stop-Process -Id $p.Id -Force
Write-Output "smoke ok"
