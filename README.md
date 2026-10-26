# Notch

An open-source "living notch" (dynamic island) for Windows 10/11, written in Rust.
It is modeled on the feature set of Nusa, reimplemented from scratch and documented in
[`docs/nusa-reverse-engineering.md`](docs/nusa-reverse-engineering.md).

**Status: v0.1.** Music (any SMTC player, synced lyrics, seek, volume), Pomodoro, water
reminders, a drag-and-drop shelf, a to-do list with reminders, and HUD peeks for volume,
brightness, Caps Lock, input language, battery, Wi-Fi and headsets.

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
| `src/main.rs` | window, tray icon + settings menu, message loop |
| `src/ui/` | island state, springs, layers, hit-testing; one file per page plus `hud.rs` |

## Build

Pushes to `main` build `Notch.exe` with GitHub Actions (Artifacts tab), then a smoke test
starts it on the runner, clicks through the pages and uploads screenshots
(`smoke-screenshots`). Pushing a `v*` tag publishes a release. To build locally you need Rust and the MSVC build tools:
`cargo build --release`.

Lyrics are provided by [LRCLIB](https://lrclib.net).
