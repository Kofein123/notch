//! Small drawing helpers shared by the pages.

use crate::gfx::{self, Align, Color, Gfx, R};
use crate::sys::Battery;

use super::Hit;

/// Hit regions collected while painting. Pages draw in island-local
/// coordinates; `ox` converts them back to canvas coordinates.
pub struct Hits {
    pub ox: f32,
    pub on: bool,
    pub v: Vec<(R, Hit)>,
}

impl Hits {
    pub fn add(&mut self, r: R, h: Hit) {
        if self.on {
            self.v.push((R::new(r.x + self.ox, r.y, r.w, r.h), h));
        }
    }
}

pub fn chip_width(g: &Gfx, label: &str, size: f32) -> f32 {
    g.measure(label, size, 600) + 26.0
}

/// Rounded chip with centred label and optional leading icon.
pub fn chip(g: &Gfx, r: R, label: &str, icon: Option<char>, bg: Color, fg: Color, size: f32) {
    g.rrect(r, r.h / 2.0, bg);
    match icon {
        Some(ic) if !label.is_empty() => {
            let tw = g.measure(label, size, 600);
            let total = size + 5.0 + tw;
            let x = r.cx() - total / 2.0;
            g.icon(ic, x + size / 2.0, r.cy(), size, fg);
            g.text(label, R::new(x + size + 5.0, r.y, tw + 2.0, r.h), size, 600, fg, Align::Left);
        }
        Some(ic) => g.icon(ic, r.cx(), r.cy(), size + 2.0, fg),
        None => g.text(label, r, size, 600, fg, Align::Center),
    }
}

/// iOS-like switch, 28 x 16.
pub fn switch(g: &Gfx, x: f32, y: f32, on: bool, accent: Color) {
    let r = R::new(x, y, 28.0, 16.0);
    g.rrect(r, 8.0, if on { accent } else { gfx::CHIP_HI });
    let kx = if on { x + 20.0 } else { x + 8.0 };
    g.circle(kx, y + 8.0, 6.0, gfx::WHITE);
}

/// Thin progress bar with a track.
pub fn bar(g: &Gfx, x: f32, y: f32, w: f32, thick: f32, frac: f32, fg: Color) {
    let frac = frac.clamp(0.0, 1.0);
    g.rrect(R::new(x, y - thick / 2.0, w, thick), thick / 2.0, gfx::WHITE.a(0.18));
    if frac > 0.0 {
        g.rrect(R::new(x, y - thick / 2.0, (w * frac).max(thick), thick), thick / 2.0, fg);
    }
}

/// Four animated equalizer bars, `x` is the left edge.
pub fn equalizer(g: &Gfx, x: f32, cy: f32, playing: bool, t: f32, c: Color) {
    let phases = [0.0f32, 1.7, 3.1, 4.6];
    let speeds = [7.0f32, 9.5, 6.2, 8.4];
    for i in 0..4 {
        let h = if playing {
            let s = ((t * speeds[i] + phases[i]).sin() * 0.5 + 0.5) * 0.75 + 0.25;
            3.0 + 11.0 * s
        } else {
            3.0
        };
        let bx = x + i as f32 * 4.5;
        g.rrect(R::new(bx, cy - h / 2.0, 2.6, h), 1.3, c);
    }
}

pub const EQ_W: f32 = 16.1;

/// Battery ring with a bolt while charging.
pub fn battery_ring(g: &Gfx, cx: f32, cy: f32, rad: f32, b: &Battery) {
    let width = (rad * 0.32).max(2.0);
    if !b.has {
        g.icon('\u{E83E}', cx, cy, rad * 1.6, gfx::WHITE);
        return;
    }
    let frac = b.percent as f32 / 100.0;
    let c = if b.charging || b.ac {
        gfx::GREEN
    } else if b.percent <= 20 {
        gfx::RED
    } else {
        gfx::WHITE
    };
    g.ring(cx, cy, rad, width, c.a(0.22));
    g.arc(cx, cy, rad, width, frac, c);
    if b.charging || b.ac {
        g.icon('\u{E945}', cx, cy, rad * 1.05, c);
    }
}

/// The idle mascot: two blinking eyes.
pub fn mascot(g: &Gfx, cx: f32, cy: f32, blink: bool) {
    let h = if blink { 1.6 } else { 7.0 };
    for dx in [-4.0f32, 4.0] {
        g.rrect(R::new(cx + dx - 1.6, cy - h / 2.0, 3.2, h), 1.6, gfx::WHITE);
    }
}

/// Bell with a small count badge.
pub fn bell(g: &Gfx, cx: f32, cy: f32, n: u32) {
    g.icon('\u{EA8F}', cx, cy, 12.0, gfx::WHITE);
    g.circle(cx + 5.0, cy - 5.0, 3.6, gfx::RED);
    let _ = n;
}

/// Glyph for a file shown on the shelf.
pub fn file_glyph(path: &str) -> char {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        return '\u{E8B7}';
    }
    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "heic" | "svg" => '\u{EB9F}',
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" => '\u{E8D6}',
        "mp4" | "mkv" | "mov" | "avi" | "webm" => '\u{E714}',
        "zip" | "rar" | "7z" | "tar" | "gz" => '\u{F012}',
        "exe" | "msi" => '\u{E756}',
        _ => '\u{E8A5}',
    }
}
