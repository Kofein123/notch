# Handoff: finish Notch v0.1

You have **push access to this GitHub repo (`Kofein123/notch`)**. Work on `main`
(or open a PR if you prefer); GitHub Actions builds `Notch.exe` on every push to `main`.

## Goal

A working `Notch.exe`: a dynamic island for Windows 10/11 that replaces Nusa for the owner.
Read [`nusa-reverse-engineering.md`](nusa-reverse-engineering.md) first. It has every API,
string, data format, timer interval and layout number recovered from Nusa with REA/Ghidra.

Definition of done:
1. CI (`.github/workflows/build.yml`) is green and uploads the `Notch-windows-x64` artifact.
2. The exe starts with no console window and shows the island at the top centre of the
   primary monitor, with a tray icon whose menu has Quit.
3. All five pages and the HUDs below work.
4. Tag `v0.1.0` → GitHub release with `Notch.exe`.

## What already exists (not yet compiled, expect type errors)

| File | Contents |
|---|---|
| `src/gfx.rs` | Direct2D DC render target → 32bpp DIB → `UpdateLayeredWindow`. Gives the notch silhouette with concave "ears", rounded rects, rings/arcs, Segoe Fluent/MDL2 icon glyphs, text with ellipsis + measuring, bitmaps for artwork, and a procedural tray icon. Coordinates are logical px; DPI is applied via `SetDpi`. |
| `src/anim.rs` | `Spring` (with bounce) and `Fade`. |
| `src/media.rs` | Background MTA thread polling SMTC: title/artist/position/duration/playing, artwork decoded to 96×96 BGRA, synced lyrics from LRCLIB (`parse_lrc`), commands `Toggle/Next/Prev/Seek`. Posts a window message on change. |
| `src/sys.rs` | Volume + default output device (Bluetooth detection), power notifications (AC, battery %, brightness), low-level keyboard hook for Caps Lock / volume keys, input language, Wi-Fi status + change events, notification-count poller, full-screen detection, monitor list + DPI, autostart (HKCU Run), chimes, localized date/time strings, open file, copy text. |
| `src/store.rs` | `%APPDATA%\Notch\` text files: `Settings` (all toggles), to-do tasks, water state, shelf items; date helpers. |
| `src/shelf.rs` | `IDropTarget` (files + text into an inbox, posts a message) and `drag_out()` via `SHCreateDataObject` + `DoDragDrop`. |
| `Cargo.toml` | `windows = 0.58` with the needed features; release profile tuned for size. |

## What to write

1. **`src/main.rs`**
   - `#![windows_subsystem = "windows"]`, single-instance mutex, `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`, `OleInitialize`.
   - Window: class `NotchWindow`, `WS_POPUP`, ex-style `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, sized to the layered canvas (about 560×190 logical).
   - Tray icon via `gfx::make_tray_icon()` + `Shell_NotifyIconW`. Right-click opens a menu with toggles that mirror `Settings`, plus Size, Screen, Pomodoro durations, water goal, HUD toggles, Open data folder and Quit.
   - Wire up `media::spawn`, `sys::install_keyboard_hook`, `sys::register_power`, `sys::watch_network`, `sys::watch_notifications` and `shelf::register` with `WM_APP + n` messages.
   - Hold app state in a `thread_local! RefCell`. **Use `try_borrow_mut` in the window procedure** (`SetWindowPos`, `DoDragDrop` and `TrackPopupMenu` re-enter it). Run drag-out and the menu *after* releasing the borrow.
2. **UI layer.** Split it into files, e.g. `src/ui/mod.rs` (state, springs, layers, hit-testing) plus one file per page (`media.rs`, `pomodoro.rs`, `water.rs`, `shelf.rs`, `todo.rs`) and `hud.rs` (peek/alerts). It renders through `gfx`.

## Behaviour spec (numbers from the RE notes)

- **Silhouette.**
  - Collapsed pill: about 172×30 (minimal mode: 128×26), bottom radius 13.
  - Expanded: 420×140, radius 30. Ears: 9.
  - Alert: 380×78. HUD peek: about 250×32.
  - Springs animate width, height and radius with a slight bounce, and the layer content cross-fades.
- **Interaction.**
  - Open on hover after **120 ms** (or on click if `open_click`). Close **250 ms** after the cursor leaves.
  - Pages switch with arrows (`E76B`/`E76C`) at x = 22 and 398, y = 70, or with the page dots at y ≈ 133.
  - Page order: Music, Pomodoro, Water, Shelf, To-do.
  - Fully transparent pixels must stay click-through.
- **Timers.**
  - Animation runs at 16 ms only while something moves.
  - The clock timer fires at the next minute boundary + 30 ms, so idle CPU stays at about 0.
  - Pomodoro tick: 500 ms. To-do check: 15 s. Water check: 15 s.
  - HUD visible for 1.4–1.8 s. Caret blink: 530 ms.
- **Header** (expanded):
  - Left: page title ("Pomodoro", "Water **Reminder**" in blue, "Shelf", "Todo**List**" in green). Music has no title.
  - Centre: battery ring (green with a bolt while charging) + `sys::date_string()` + bold `sys::time_string()`.
  - Right: equalizer bars, plus a bell badge if notifications exist.
- **Music page.**
  - Artwork 62×62 at (44, 12), radius 10.
  - Title 14/700 and artist 11 gray at x = 122. When lyrics are on, show the current line in bold and the next line dimmed.
  - Progress row at y = 92: elapsed text, a 3 px bar from x = 96 to 320, and remaining time as `-mm:ss`. Click or drag on the bar to seek.
  - Controls at y = 118: lyrics toggle (74), previous (162), play/pause (209), next (257), volume (366). Volume swaps the progress row for a volume slider; the mouse wheel adjusts it.
  - Empty state: "Nothing playing".
- **Pomodoro page.**
  - Red ring with a stopwatch icon at (81, 70), r ≈ 34, showing a progress arc.
  - "25:00" at 29/700 and a reset button, centred on **x = 265**. Below it, "Focus time" in gray.
  - Chip row centred on 265: `[Short Break] [▶ 54 px] [Focus Time]`. Chip width = text + 35, gap 8.
  - Phase logic: after 4 focus phases a long break, then the counter resets. Alert texts: "Break Time! / 5 minute short break", "Long Break! / 15 min, you earned it", "Focus Time! / Back to work, 25 min". Durations come from `Settings`.
- **Water page.**
  - Text "0.0 / 1.5 L", a reminders toggle, the line "Today · N% · Next HH:MM" and a progress bar.
  - Buttons: Drink (blue, adds `water_glass` ml) and Settings (goal 1.5/2/2.5/3 L, bottle size, active hours).
  - Gauge ring on the right.
  - Reminders are spread evenly over the active hours. Alerts: "Time to Drink" with Drank / Snooze, and "Time to Refill / Your bottle is probably empty" with Refilled.
- **Shelf page.**
  - Empty state: dashed box with "Drop files here" / "Drag out anytime · click to open or copy". While dragging over: "Release to add".
  - Tiles show an icon and a name.
  - Click opens a file or copies a text card (show "Copied"). Right-click removes. Drag a tile out with `shelf::drag_out`. "Clear all" empties it.
  - A drag entering the collapsed pill expands it to the Shelf page.
- **To-do page.**
  - Big "done/total" count, a calendar button that cycles the day (today/tomorrow/…), "Task Done Today".
  - Buttons: Add Task and Edit Task. The right card lists tasks with a checkbox, title and time, or shows "No tasks.".
  - Add mode: a text input (call `SetForegroundWindow` for keyboard focus and drop `WS_EX_NOACTIVATE` while editing), a start-time chip and a duration chip, then Cancel / Save. Enter saves; Esc cancels.
  - Alerts: "Task Time!" (Later / Start) at the start time, "Time's Up!" at the end. Later snoozes by `snooze` minutes.
- **HUD peeks** (collapsed only):
  - volume / brightness: icon + bar + %;
  - Caps Lock: ON / OFF;
  - language: e.g. RU / EN;
  - battery: low ≤ 20 %, critical ≤ 10 %, full, charger connected / unplugged;
  - Wi-Fi: Connected / No internet access / Disconnected;
  - headset connected (output device changed to a Bluetooth endpoint).
  - Respect the `hud_*` and `popup_*` toggles.
- **Pill content.**
  - Left: battery ring, or the artwork thumbnail while playing.
  - Centre: the clock, or the current lyric line while playing (pill grows to about 330).
  - Right: equalizer while playing, or the blinking-eyes mascot (`mascot`) when idle.
  - Red Pomodoro countdown while running; bell badge.
- **Hide in full screen** (`sys::fullscreen_on`, checked about once per second). Re-assert topmost every few seconds.

## How to work

- This environment can't compile for Windows, and crates.io may be blocked here. **Iterate through CI:**
  push, read the failing job log (`gh run view --log-failed`), fix, and push again.
- Check every `windows 0.58` signature against the crate source rather than memory.
  You can clone `microsoft/windows-rs` at tag `0.58.0` and grep
  `crates/libs/windows/src/Windows/...`. Points to note in 0.58:
  - `Param<T>` accepts `None` for optional handles and interfaces;
  - COM implementations use `#[implement(IFoo)]` + `impl IFoo_Impl for Foo_Impl`;
  - WinRT async ops have a blocking `.get()` (only on MTA threads, never on the UI thread).
- `.github/workflows/build.yml` has a job-level `if: hashFiles('src/main.rs')`. Remove it once
  `main.rs` exists, because `hashFiles` isn't reliable at job level.
- Keep dependencies to just the `windows` crate unless there is a strong reason.
- Commit in small steps with clear messages.
