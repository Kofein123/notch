# Notch

An open-source "living notch" (dynamic island) for Windows 10/11, written in Rust.
It is modeled on the feature set of Nusa, reimplemented from scratch and documented in
[`docs/nusa-reverse-engineering.md`](docs/nusa-reverse-engineering.md).

**Status: work in progress.** The core modules are in place. The island UI and the window
entry point (`src/main.rs`) are next, so CI skips the build until `main.rs` lands.

| Module | What |
|---|---|
| `src/gfx.rs` | Direct2D/DirectWrite → per-pixel-alpha layered window, notch silhouette, icons, artwork |
| `src/anim.rs` | spring / fade animation |
| `src/media.rs` | SMTC music session, artwork decode, synced lyrics from LRCLIB |
| `src/sys.rs` | volume and output device, brightness, battery, Caps Lock / language hook, Wi-Fi, full screen, autostart, notification count |
| `src/store.rs` | `%APPDATA%\Notch` text files: settings, to-do, water, shelf |
| `src/shelf.rs` | OLE drag & drop in and out |

## Build

Pushes to `main` build `Notch.exe` with GitHub Actions (Artifacts tab). Pushing a `v*` tag
publishes a release. To build locally you need Rust and the MSVC build tools:
`cargo build --release`.

Lyrics are provided by [LRCLIB](https://lrclib.net).
