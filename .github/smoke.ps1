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
    public static void Wheel(int d) { mouse_event(0x0800, 0, 0, unchecked((uint)d), UIntPtr.Zero); }
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

# the wheel over the header row flips pages: 1 Telegram, 2 Claude, 3 Pomodoro,
# 4 Water, 5 Shelf, 6 To-do
$hy = $r.T + [int](10 * $s)
for ($i = 1; $i -le 6; $i++) {
    [void][Mouse]::SetCursorPos($cx, $hy)
    [Mouse]::Wheel(-120)
    Start-Sleep -Milliseconds 900
    Shot "page$i" ([int](240 * $s))
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

# pomodoro: three pages back, press play
for ($i = 0; $i -lt 3; $i++) {
    [void][Mouse]::SetCursorPos($cx, $hy)
    [Mouse]::Wheel(120)
    Start-Sleep -Milliseconds 700
}
IClick 265 109
Start-Sleep -Milliseconds 1600
Shot "pomo-running" ([int](150 * $s))

# water: next page, drink twice -> bottle (500 ml) empty -> refill alert
[void][Mouse]::SetCursorPos($cx, $hy)
[Mouse]::Wheel(-120)
Start-Sleep -Milliseconds 700
IClick 86 109
Start-Sleep -Milliseconds 200
IClick 86 109
Start-Sleep -Milliseconds 900
Shot "water-refill" ([int](150 * $s))
IClick 320 39
Start-Sleep -Milliseconds 900

# leave, let it collapse, then Caps Lock HUD
[void][Mouse]::SetCursorPos(5, 600)
Start-Sleep -Milliseconds 1200
Shot "collapsed" ([int](60 * $s))
[Mouse]::Key(0x14)
Start-Sleep -Milliseconds 450
Shot "hud-caps" ([int](60 * $s))
[Mouse]::Key(0x14)
Start-Sleep -Milliseconds 2200

# downloads: play the browser extension through the native host
function Frame([string]$json) {
    $b = [System.Text.Encoding]::UTF8.GetBytes($json)
    return [byte[]]([BitConverter]::GetBytes([int]$b.Length) + $b)
}
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = (Resolve-Path $Exe).Path
$psi.Arguments = "chrome-extension://pbhgikiagdfeejahlihhdgcbhcaklkih/"
$psi.UseShellExecute = $false
$psi.RedirectStandardInput = $true
$host_p = [System.Diagnostics.Process]::Start($psi)
$in = $host_p.StandardInput.BaseStream
$file = Join-Path $env:RUNNER_TEMP "ubuntu-24.04-desktop-amd64.iso"
Set-Content -Path $file -Value "x"
$fj = ($file | ConvertTo-Json)
foreach ($m in @(
    '{"type":"hello","version":"1.0"}',
    ('{"type":"created","item":{"id":7,"file":' + $fj + ',"url":"https://example.com/a.iso","state":"in_progress","received":0,"total":6000000000}}'),
    ('{"type":"progress","items":[{"id":7,"file":' + $fj + ',"url":"","state":"in_progress","received":2500000000,"total":6000000000}]}')
)) { $f = Frame $m; $in.Write($f, 0, $f.Length); $in.Flush(); Start-Sleep -Milliseconds 400 }
Start-Sleep -Milliseconds 900
Shot "dl-progress" ([int](60 * $s))
$f = Frame ('{"type":"complete","item":{"id":7,"file":' + $fj + ',"url":"","state":"complete","received":6000000000,"total":6000000000}}')
$in.Write($f, 0, $f.Length); $in.Flush()
Start-Sleep -Milliseconds 1200
Shot "dl-done" ([int](100 * $s))
$in.Close()
Start-Sleep -Milliseconds 500
if (-not $host_p.HasExited) { Write-Output "::warning title=smoke::native host did not exit on EOF"; $host_p.Kill() }

if ($p.HasExited) {
    Write-Output "::error title=smoke::Notch.exe crashed while interacting (code $($p.ExitCode))"
    exit 1
}
Stop-Process -Id $p.Id -Force
Write-Output "smoke ok"
