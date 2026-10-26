# Nusa 1.1.0: reverse-engineering notes

Target: `nusa.exe`, SHA-256 `7ef5d186dcd37352ddf6c34f1cbef725e81a1f07216f8b37913028992f97c4d5`,
1 050 624 bytes, PE32+ x86-64, GUI subsystem. Installed per user to
`%LOCALAPPDATA%\Programs\Nusa\`, using an Inno Setup 6.7 installer.

Nusa is **closed source**: the `rama2555/nusa` repository holds only the landing page,
and the binary ships through GitHub releases. These notes describe *how the features
work*, so that Notch can reimplement them independently. No Nusa code is copied or patched.

## Method

| Step | Tool | Result |
|---|---|---|
| Strings, section map, import table | `strings`, `objdump -p` | UI text, file names, APIs |
| Rust metadata | REA `inspect-rust-binary` | rustc commit `48a229ce…`, no cargo-auditable data |
| Container check | REA `inspect-artifact` | single PE, nothing embedded |
| Full analysis | REA `analyze --provider ghidra` (Ghidra 12.1.4, JDK 21) | 1868 functions, 929 strings |
| String → function map | REA `trace` | see the function table below |
| Decompilation | REA `decompile` | 8 key functions |
| Timer intervals | `objdump -d`, `SetTimer` argument registers | timer table below |

## Stack

- Rust, built against the `windows 0.58` crate. `hashbrown 0.17` is also present. There is no web engine.
- Rendering uses **D3D11, DirectComposition, Direct2D and DirectWrite**. There is a small
  wrapper around `DCompositionCreateDevice` at `0x14000e8a4`. `SetWindowRgn` is imported, but
  nothing references it.
- WinRT is used through `RoGetActivationFactory`.

## Feature → API map

| Feature | Mechanism |
|---|---|
| Music from any player | `Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager` (SMTC) |
| Synced lyrics | `Windows.Web.Http.HttpClient` → `https://lrclib.net/api/search?track_name=…&artist_name=…`, UA `Nusa/1.1.0 (+https://rama2555.github.io/nusa)` |
| Artwork | SMTC thumbnail stream → `Windows.Storage.Streams.DataReader` |
| Caps Lock HUD | `SetWindowsHookExW` (low-level keyboard) + `GetKeyState` |
| Input-language HUD | `GetKeyboardLayout` of the foreground thread |
| Volume / brightness HUD | brightness and battery via `RegisterPowerSettingNotification` |
| Battery / charger popups | power-setting notifications, "Low battery / Battery critical / Fully charged / Charger connected / unplugged" |
| Wi-Fi popup | `Windows.Networking.Connectivity.NetworkInformation`: "Connected / No internet access / Disconnected" |
| Headset popup | `Windows.Devices.Bluetooth.BluetoothDevice` + `cfgmgr32` (`CM_Get_Device_ID_ListW`), filtered by the A2DP/HFP/headset service UUIDs `0000110B`, `0000111E`, `00001108` |
| Notification badge | `Windows.UI.Notifications.Management.UserNotificationListener`. It only counts notifications, so it cannot open them |
| Hide in full screen | `SetWinEventHook` + `SHQueryUserNotificationState` + foreground-window rect vs monitor |
| Shelf | OLE `RegisterDragDrop` / `DoDragDrop`, `SHCreateDataObject`, `DragQueryFileW`. Text cards are saved as `.nusatext` |
| Start with Windows | `RegSetKeyValueW` / `RegDeleteKeyValueW` (Run key) |
| Sounds | `PlaySoundW` |
| Tray icon | `Shell_NotifyIconW`, icon built at runtime with `CreateIconIndirect` |

## Data files (`%APPDATA%\Nusa\`)

`settings.txt`, `todo.txt` / `tasks.txt`, `water.txt` (`bottle=`, `start=`, `taken=`, `fired=`, `hours=`),
`shelf.txt`. All of them are plain `key=value` or line-based files. The water goal defaults to 2000 ml.

Settings keys: `sound lyrics todo_start todo_end snooze week_monday focus short_break long_break
hud_volume hud_brightness hud_caps hud_language popup_battery popup_wifi popup_bluetooth
open_click page_icons fast_animations shelf_clear calendar_week hide_fullscreen display pet minimal`.

## Key functions (Ghidra addresses)

| Address | Size | What |
|---|---|---|
| `0x14003d238` | 18 KB | main island renderer: pages, HUD, Caps Lock |
| `0x1400427fe` | 28 KB | settings window |
| `0x14002630e` | 13.6 KB | window procedure (hover, timers) |
| `0x140003574` | 0.9 KB | Pomodoro phase change + alert |
| `0x14000391e` | 1.5 KB | To-do reminder check + alert |
| `0x14002f3bd` | 0.4 KB | water "Time to Drink" / "Time to Refill" alert |
| `0x1400306a6`, `0x1400317b9` | 3.8 / 1.3 KB | `water.txt` load / save |
| `0x1400518f1` | 3.5 KB | shelf drop UI ("Release to add") |
| `0x14004d507` | 3.7 KB | About page (LRCLIB credit) |
| `0x140020cb7` | 7.6 KB | startup: loads `settings.txt`, starts timers |

## Behaviour recovered from decompilation

**Pomodoro (`0x140003574`).** Window timer 8 ticks every 500 ms while the timer runs.
When a focus phase ends, the session counter goes up by one. While it is below 4 the next
phase is "Break Time! / 5 minute short break". The 4th completed focus gives "Long Break! /
15 min, you earned it" and resets the counter. When a break ends, the alert is "Focus Time! /
Back to work, 25 min". Alert text layouts are `125 × scale` wide.

**Pomodoro page layout (`0x14003d238`).** The "25:00 + ↻" group and the button row are both
centred on **x = 265** (island-relative). Each chip is `text width + 26 + 9` wide, with fallback
widths of 87 and 85. The play button is 54 wide and the gaps are 8. The row width is
`left + 54 + right + 16`, centred on 265.

**To-do (`0x14000391e`).** Window timer 9 runs every **15 s** and checks reminders. A task
start gives "Task Time!" with the buttons **Later / Start**; a task end gives "Time's Up!".
Buttons use DirectWrite 12 pt, weight 600. The title layout is `112 × scale` wide.

**Water (`0x14002f3bd`).** One alert shows either "Time to Drink" or "Time to Refill / Your
bottle is probably empty", depending on a flag. Text width is `100 × scale`.

## Timers (`SetTimer` id → interval)

Recovered from the immediate arguments at all 34 call sites. The ones marked "confirmed"
come from decompilation or from code around the call; the others are inferred from where
the call sits.

| Id | Interval | Use | Basis |
|---|---|---|---|
| 1 | until the next minute + 30 ms (min 100 ms) | clock. Idle wakeups are once per minute, which is why idle CPU is 0 % | confirmed (computed interval) |
| 12 | 16 ms | animation frame (60 fps), only while something moves | confirmed (interval) |
| 8 | 500 ms | Pomodoro tick | confirmed |
| 9 | 15 s, then 24 h | to-do reminder check / daily reset | confirmed |
| 10 | 530 ms | caret blink in the "Task title" input (Windows default) | inferred |
| 14 | 120 ms | hover → open delay | inferred (window procedure) |
| 5 | 160 / 250 ms | close delay after the cursor leaves | inferred (window procedure) |
| 7 | 60 ms | short follow-up in the window procedure | inferred |
| 19 / 21 / 20 | 400 / 800 / 3000 ms | window-procedure follow-ups (peek / auto-hide) | inferred |
| 6 | 1400 / 1800 ms | HUD visible time (volume, Caps, language) | inferred |
| 16 / 4 | 1500 / 3200 ms | Wi-Fi popup timings | inferred (network popup function) |
| 2 / 3 / 13 / 18 | 8 s / 1 s / 300 ms / 5 s | started at launch: background checks | inferred (startup function) |
| 11 | 15 s | water check | inferred |
| 15, 22, 23, 25, 17 | 2.5 s, 3 s, 300 ms, 2.8 s, 50 ms | popups / shelf | inferred |

## Pro features in 1.1.0

The to-do calendar, Pomodoro durations, display picker, appearance and HUD settings appear
only as "coming soon" or "Calendar is coming with Nusa Pro". None of them have an
implementation in the binary.
