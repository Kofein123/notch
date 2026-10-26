//! Music page: artwork, title, lyrics, progress / volume and transport.

use crate::gfx::{self, Align, Color, Gfx, R};
use crate::media::{self, Lyrics};

const WIDE_BAR_X0: f32 = 66.0;
const WIDE_BAR_X1: f32 = 186.0;

use super::widgets::{self, Hits};
use super::{App, Hit};

const BAR_X0: f32 = 96.0;
const BAR_X1: f32 = 320.0;

impl App {
    pub(super) fn draw_music(&self, g: &Gfx, hits: &mut Hits) {
        let m = &self.m;
        if !m.has {
            g.icon('\u{EC4F}', 210.0, 54.0, 24.0, gfx::DIM);
            g.text("Nothing playing", R::new(110.0, 76.0, 200.0, 22.0), 13.0, 600, gfx::GRAY, Align::Center);
            return;
        }

        // artwork + title / artist (the header row with the clock sits above)
        let art = R::new(44.0, 16.0, 58.0, 58.0);
        match &self.art {
            Some(b) => g.image(b, art, 10.0),
            None => {
                g.rrect(art, 10.0, gfx::CHIP_HI);
                g.icon('\u{EC4F}', art.cx(), art.cy(), 22.0, gfx::GRAY);
            }
        }
        let tx = 116.0;
        let tw = 244.0;
        let title = if m.title.is_empty() { "Unknown" } else { m.title.as_str() };
        g.text(title, R::new(tx, 36.0, tw, 20.0), 14.0, 700, gfx::WHITE, Align::Left);
        g.text(&m.artist, R::new(tx, 55.0, tw, 16.0), 11.0, 400, gfx::GRAY, Align::Left);

        // progress or volume row at y = 92
        let y = 92.0;
        if self.vol_mode {
            let (v, mute) = self.audio.volume().unwrap_or((0.0, false));
            let v = if mute { 0.0 } else { v };
            g.icon(if v <= 0.001 { '\u{E74F}' } else { '\u{E767}' }, 74.0, y, 12.0, gfx::GRAY);
            widgets::bar(g, BAR_X0, y, BAR_X1 - BAR_X0, 4.0, v, gfx::WHITE);
            g.circle(BAR_X0 + (BAR_X1 - BAR_X0) * v, y, 5.0, gfx::WHITE);
            g.text(&format!("{}%", (v * 100.0).round() as i32), R::new(326.0, y - 8.0, 50.0, 16.0), 10.5, 600, gfx::GRAY, Align::Left);
            hits.add(R::new(BAR_X0 - 6.0, y - 10.0, BAR_X1 - BAR_X0 + 12.0, 20.0), Hit::Volume(R::new(BAR_X0, y - 10.0, BAR_X1 - BAR_X0, 20.0)));
        } else {
            let pos = self.seek_preview.unwrap_or_else(|| m.position());
            let frac = if m.dur > 0.0 { (pos / m.dur) as f32 } else { 0.0 };
            g.text(&media::fmt_time(pos), R::new(40.0, y - 8.0, 50.0, 16.0), 10.0, 600, gfx::GRAY, Align::Right);
            widgets::bar(g, BAR_X0, y, BAR_X1 - BAR_X0, 3.0, frac, gfx::WHITE);
            if self.seek_preview.is_some() {
                g.circle(BAR_X0 + (BAR_X1 - BAR_X0) * frac, y, 5.0, gfx::WHITE);
            }
            let left = if m.dur > 0.0 { format!("-{}", media::fmt_time(m.dur - pos)) } else { "--:--".into() };
            g.text(&left, R::new(326.0, y - 8.0, 56.0, 16.0), 10.0, 600, gfx::GRAY, Align::Left);
            if m.can_seek && m.dur > 0.0 {
                hits.add(R::new(BAR_X0 - 6.0, y - 10.0, BAR_X1 - BAR_X0 + 12.0, 20.0), Hit::Seek(R::new(BAR_X0, y - 10.0, BAR_X1 - BAR_X0, 20.0)));
            }
        }

        // controls at y = 118
        let cy = 116.0;
        let lyr_c = if self.s.lyrics_view { gfx::WHITE } else { gfx::GRAY };
        g.icon('\u{E8BD}', 74.0, cy, 14.0, lyr_c);
        hits.add(R::new(60.0, cy - 13.0, 28.0, 26.0), Hit::Lyrics);
        g.icon('\u{E892}', 162.0, cy, 16.0, gfx::WHITE);
        hits.add(R::new(146.0, cy - 13.0, 32.0, 26.0), Hit::Prev);
        g.icon(if m.playing { '\u{E769}' } else { '\u{E768}' }, 209.0, cy, 22.0, gfx::WHITE);
        hits.add(R::new(191.0, cy - 15.0, 36.0, 30.0), Hit::Toggle);
        g.icon('\u{E893}', 257.0, cy, 16.0, gfx::WHITE);
        hits.add(R::new(241.0, cy - 13.0, 32.0, 26.0), Hit::Next);
        let mute = self.audio.volume().map(|v| v.1).unwrap_or(false);
        g.icon(if mute { '\u{E74F}' } else { '\u{E767}' }, 366.0, cy, 14.0, if self.vol_mode { gfx::WHITE } else { gfx::GRAY });
        hits.add(R::new(352.0, cy - 13.0, 28.0, 26.0), Hit::VolumeBtn);
    }

    /// Wide lyrics view (520 x 140): compact player on the left, scrolling
    /// synced lyrics on the right, tinted with the artwork colour.
    pub(super) fn draw_music_wide(&self, g: &Gfx, hits: &mut Hits) {
        let m = &self.m;

        // left: compact player
        let art = R::new(24.0, 14.0, 56.0, 56.0);
        match &self.art {
            Some(b) => g.image(b, art, 10.0),
            None => {
                g.rrect(art, 10.0, gfx::CHIP_HI);
                g.icon('\u{EC4F}', art.cx(), art.cy(), 20.0, gfx::GRAY);
            }
        }
        let title = if m.title.is_empty() { "Unknown" } else { m.title.as_str() };
        g.text(title, R::new(92.0, 22.0, 152.0, 20.0), 13.5, 700, gfx::WHITE, Align::Left);
        g.text(&m.artist, R::new(92.0, 41.0, 152.0, 16.0), 10.5, 400, gfx::GRAY, Align::Left);

        let y = 90.0;
        let pos = self.seek_preview.unwrap_or_else(|| m.position());
        let frac = if m.dur > 0.0 { (pos / m.dur) as f32 } else { 0.0 };
        g.text(&media::fmt_time(pos), R::new(14.0, y - 8.0, 46.0, 16.0), 10.5, 600, gfx::WHITE, Align::Right);
        widgets::bar(g, WIDE_BAR_X0, y, WIDE_BAR_X1 - WIDE_BAR_X0, 3.0, frac, gfx::WHITE);
        if self.seek_preview.is_some() {
            g.circle(WIDE_BAR_X0 + (WIDE_BAR_X1 - WIDE_BAR_X0) * frac, y, 5.0, gfx::WHITE);
        }
        let left = if m.dur > 0.0 { format!("-{}", media::fmt_time(m.dur - pos)) } else { "--:--".into() };
        g.text(&left, R::new(192.0, y - 8.0, 56.0, 16.0), 10.5, 600, gfx::WHITE, Align::Left);
        if m.can_seek && m.dur > 0.0 {
            hits.add(
                R::new(WIDE_BAR_X0 - 6.0, y - 10.0, WIDE_BAR_X1 - WIDE_BAR_X0 + 12.0, 20.0),
                Hit::Seek(R::new(WIDE_BAR_X0, y - 10.0, WIDE_BAR_X1 - WIDE_BAR_X0, 20.0)),
            );
        }

        let cy = 117.0;
        g.icon('\u{E8BD}', 54.0, cy, 14.0, gfx::WHITE);
        hits.add(R::new(40.0, cy - 13.0, 28.0, 26.0), Hit::Lyrics);
        g.icon('\u{E892}', 92.0, cy, 15.0, gfx::WHITE);
        hits.add(R::new(76.0, cy - 13.0, 32.0, 26.0), Hit::Prev);
        g.icon(if m.playing { '\u{E769}' } else { '\u{E768}' }, 132.0, cy, 20.0, gfx::WHITE);
        hits.add(R::new(114.0, cy - 15.0, 36.0, 30.0), Hit::Toggle);
        g.icon('\u{E893}', 172.0, cy, 15.0, gfx::WHITE);
        hits.add(R::new(156.0, cy - 13.0, 32.0, 26.0), Hit::Next);

        // divider
        g.line(254.0, 20.0, 254.0, 120.0, 1.0, gfx::WHITE.a(0.18));

        // right: lyrics
        let panel = R::new(268.0, 6.0, 236.0, 128.0);
        let msg = match (m.lyrics_state, &m.lyrics) {
            (Lyrics::Ready, Some(l)) if !l.is_empty() => None,
            (Lyrics::Loading, _) | (Lyrics::None, _) => Some("Looking for lyrics…"),
            _ => Some("No synced lyrics for this track"),
        };
        if let Some(t) = msg {
            g.text_wrap(t, R::new(panel.x, 58.0, panel.w, 40.0), 13.0, 600, gfx::GRAY, Align::Left);
            return;
        }
        let Some(lines) = m.lyrics.as_ref() else { return };
        let cur = m.lyric_index();
        let n = lines.len() as i64;
        let style = |j: i64| -> (f32, Color) {
            if Some(j as usize) == cur {
                (15.0, gfx::WHITE)
            } else {
                (13.0, gfx::WHITE.a(0.38))
            }
        };
        fn line_text(lines: &[(f64, String)], j: i64) -> &str {
            let t = lines[j as usize].1.trim();
            if t.is_empty() {
                "♪"
            } else {
                t
            }
        }
        let text = |j: i64| line_text(lines, j);
        let height = |j: i64| -> f32 {
            let (size, _) = style(j);
            g.measure_wrap(text(j), panel.w, size, 700).min(size * 4.2)
        };
        const GAP: f32 = 9.0;
        let v = self.lyr.v.clamp(0.0, (n - 1).max(0) as f32);
        let k = v.floor() as i64;
        let frac = v - k as f32;
        let centre = 70.0;
        let pitch = |a: i64| -> f32 {
            if a + 1 >= n {
                height(a) / 2.0 + GAP + 10.0
            } else {
                (height(a) + height(a + 1)) / 2.0 + GAP
            }
        };
        let mut items: Vec<(i64, f32)> = Vec::new();
        let yk = centre - frac * pitch(k);
        items.push((k, yk));
        // lines below
        let (mut j, mut y) = (k, yk);
        while j + 1 < n {
            y += pitch(j);
            j += 1;
            items.push((j, y));
            if y - height(j) / 2.0 > panel.y + panel.h {
                break;
            }
        }
        // lines above
        let (mut j, mut y) = (k, yk);
        while j > 0 {
            y -= pitch(j - 1);
            j -= 1;
            items.push((j, y));
            if y + height(j) / 2.0 < panel.y {
                break;
            }
        }
        g.clip(panel);
        for (j, yc) in items {
            let (size, c) = style(j);
            let h = height(j);
            g.text_wrap(text(j), R::new(panel.x, yc - h / 2.0, panel.w, h + 6.0), size, 700, c, Align::Left);
        }
        g.unclip();
    }
}
