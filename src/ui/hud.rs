//! HUD peeks (volume, brightness, Caps Lock, language, battery, Wi-Fi,
//! headset) and alerts (Pomodoro, water, to-do).

use crate::gfx::{self, Align, Gfx, R};

use super::widgets::{self, Hits};
use super::{Alert, AlertIcon, Hit, Hud};

/// 250 x 32 peek.
pub fn draw_hud(g: &Gfx, h: &Hud) {
    let cy = 16.0;
    g.icon(h.icon, 24.0, cy, 13.0, h.color);
    match h.value {
        Some(v) => {
            widgets::bar(g, 44.0, cy, 150.0, 4.0, v, gfx::WHITE);
            g.text(&h.right, R::new(198.0, 0.0, 40.0, 32.0), 11.0, 600, h.right_color, Align::Right);
        }
        None => {
            let tw = g.measure(&h.title, 12.0, 600);
            g.text(&h.title, R::new(40.0, 0.0, tw.min(160.0) + 2.0, 32.0), 12.0, 600, gfx::WHITE, Align::Left);
            let rx = 40.0 + tw.min(160.0) + 8.0;
            g.text(&h.right, R::new(rx, 0.0, (238.0 - rx).max(0.0), 32.0), 11.0, 600, h.right_color, Align::Right);
        }
    }
}

/// 380 x 78 alert with optional buttons; the last button is the primary one.
pub fn draw_alert(g: &Gfx, a: &Alert, hits: &mut Hits) {
    hits.add(R::new(0.0, 0.0, 380.0, 78.0), Hit::AlertBody);
    let (cx, cy) = (40.0, 38.0);
    g.circle(cx, cy, 19.0, a.color.a(0.18));
    match a.icon {
        AlertIcon::Glyph(c) => g.icon(c, cx, cy, 17.0, a.color),
        AlertIcon::Drop => g.drop_icon(cx, cy, 18.0, a.color),
    }

    // buttons, right-aligned
    let size = 12.0;
    let widths: Vec<f32> = a.buttons.iter().map(|(l, _)| g.measure(l, size, 600) + 28.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 8.0 * (widths.len().saturating_sub(1)) as f32;
    let mut x = 356.0 - total;
    let n = a.buttons.len();
    for (i, ((label, _), w)) in a.buttons.iter().zip(widths.iter()).enumerate() {
        let r = R::new(x, 25.0, *w, 28.0);
        let primary = i + 1 == n;
        widgets::chip(
            g,
            r,
            label,
            None,
            if primary { a.color } else { gfx::CHIP_HI },
            if primary { gfx::BLACK } else { gfx::WHITE },
            size,
        );
        hits.add(r, Hit::AlertBtn(i));
        x += w + 8.0;
    }

    let text_w = (356.0 - total - 12.0 - 70.0).max(80.0);
    g.text(&a.title, R::new(70.0, 18.0, text_w, 22.0), 14.0, 700, gfx::WHITE, Align::Left);
    g.text(&a.sub, R::new(70.0, 39.0, text_w, 18.0), 11.0, 400, gfx::GRAY, Align::Left);
}
