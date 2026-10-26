//! Music page: artwork, title, lyrics, progress / volume and transport.

use crate::gfx::{self, Align, Gfx, R};
use crate::media::{self, Lyrics};

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

        // artwork
        let art = R::new(44.0, 12.0, 62.0, 62.0);
        match &self.art {
            Some(b) => g.image(b, art, 10.0),
            None => {
                g.rrect(art, 10.0, gfx::CHIP_HI);
                g.icon('\u{EC4F}', art.cx(), art.cy(), 22.0, gfx::GRAY);
            }
        }

        // title / artist / lyrics
        let tx = 122.0;
        let tw = 236.0;
        let title = if m.title.is_empty() { "Unknown" } else { m.title.as_str() };
        g.text(title, R::new(tx, 13.0, tw, 20.0), 14.0, 700, gfx::WHITE, Align::Left);
        g.text(&m.artist, R::new(tx, 32.0, tw, 16.0), 11.0, 400, gfx::GRAY, Align::Left);
        if self.s.lyrics {
            match (m.lyrics_state, m.lyric_line()) {
                (Lyrics::Ready, Some((cur, next))) => {
                    let cur = if cur.trim().is_empty() { "♪".to_string() } else { cur };
                    g.text(&cur, R::new(tx, 50.0, 270.0, 16.0), 11.5, 700, gfx::WHITE, Align::Left);
                    if let Some(n) = next {
                        g.text(&n, R::new(tx, 65.0, 270.0, 14.0), 10.5, 400, gfx::DIM, Align::Left);
                    }
                }
                (Lyrics::Ready, None) => {
                    g.text("♪", R::new(tx, 50.0, 40.0, 16.0), 11.5, 700, gfx::DIM, Align::Left);
                }
                (Lyrics::Loading, _) => {
                    g.text("Looking for lyrics…", R::new(tx, 52.0, 200.0, 14.0), 10.5, 400, gfx::DIM, Align::Left);
                }
                (Lyrics::Missing, _) => {
                    g.text("No synced lyrics", R::new(tx, 52.0, 200.0, 14.0), 10.5, 400, gfx::DIM, Align::Left);
                }
                _ => {}
            }
        }
        widgets::equalizer(g, 372.0, 22.0, m.playing, self.t(), gfx::WHITE);

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
        let lyr_c = if self.s.lyrics { gfx::WHITE } else { gfx::DIM };
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
}
