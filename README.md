# Notch

An open-source "living notch" (dynamic island) for Windows 10/11, written in Rust.
It is modeled on the feature set of Nusa, reimplemented from scratch and documented in
[`docs/nusa-reverse-engineering.md`](docs/nusa-reverse-engineering.md).

**Status: v0.2.** Music (any SMTC player, synced lyrics, seek, volume), **Telegram** (message
popups, chat list, quick replies), **Claude Desktop** status (writing / answer ready),
**browser downloads** (progress in the pill, "done" popup), Pomodoro, water reminders, a
drag-and-drop shelf, a to-do list with reminders, and HUD peeks for volume, brightness,
Caps Lock, input language, battery, Wi-Fi and headsets.

### Telegram
Download `Notch-windows-x64.zip` from Releases and keep `tdjson.dll` next to `Notch.exe`.
Open the Telegram page and follow the steps: your own `api_id` / `api_hash` from
[my.telegram.org](https://my.telegram.org/apps), phone number, login code, cloud password if
you have one. The session lives in `%APPDATA%\Notch\tdlib`, encrypted with a key that only
your Windows account can unlock (DPAPI).

### Downloads
Settings → Downloads → Install: the browser opens its extensions page and the extension
folder path is copied. Turn on Developer mode, click "Load unpacked" and paste the path.
Works with Chrome, Edge, Brave, Yandex Browser, Vivaldi.

### Claude
Nothing to set up: Notch watches the Claude Desktop window and notices the "Stop response"
button, so it knows when Claude is writing and when the answer is ready.

Hover the island to open it (or click, if "Open on click" is on). Right-click the island or the
tray icon for settings: size, screen, durations, HUD toggles, start with Windows, Quit.
Data lives in `%APPDATA%\Notch\`.

| Module | What |
|---|---|
| `src/gfx.rs` | Direct2D/DirectWrite → per-pixel-alpha layered window, notch silhouette, icons, artwork |
| `src/anim.rs` | spring / fade animation |
| `src/media.rs` | SMTC music session, artwork decode, synced lyrics from LRCLIB |
| `src/sys.rs` | volume and output device, brightness, battery, Caps Lock / language hook, Wi-Fi, full screen, autostart, notification count |
| `src/store.rs` | `%APPDATA%\Notch` text files: settings, to-do, water, shelf |
| `src/shelf.rs` | OLE drag & drop in and out |
| `src/tg.rs` | Telegram via TDLib JSON (`tdjson.dll`, loaded at runtime) |
| `src/downloads.rs` | browser extension bridge (native messaging host) and download model |
| `src/claude.rs` | Claude Desktop status through UI Automation |
| `extension/` | the "Notch Downloads" browser extension (embedded in the exe) |
| `src/main.rs` | window, tray icon + settings menu, message loop |
| `src/ui/` | island state, springs, layers, hit-testing; one file per page plus `hud.rs` |

## Build

Pushes to `main` build `Notch.exe` with GitHub Actions (Artifacts tab), then a smoke test
starts it on the runner, clicks through the pages and uploads screenshots
(`smoke-screenshots`). Pushing a `v*` tag publishes a release. To build locally you need Rust and the MSVC build tools:
`cargo build --release`.

Lyrics are provided by [LRCLIB](https://lrclib.net).
