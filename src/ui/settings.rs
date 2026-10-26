//! In-island settings panel (opened from the gear in the header or the tray).

use crate::gfx::{self, Align, Gfx, R};
use crate::store::{self, Settings};
use crate::sys;

use super::widgets::{self, Hits};
use super::{App, Deferred, Hit};

// ------------------------------------------------------------------ boolean settings

macro_rules! toggles {
    ($($field:ident => $label:expr),* $(,)?) => {
        pub const TOGGLE_LABELS: &[&str] = &[$($label),*];
        pub fn toggle_field(s: &mut Settings, i: usize) -> Option<&mut bool> {
            let mut k = 0usize;
            $(
                if i == k {
                    return Some(&mut s.$field);
                }
                k += 1;
            )*
            let _ = k;
            None
        }
    };
}

toggles!(
    autostart => "Start with Windows",          // 0
    minimal => "Minimal mode",                  // 1
    mascot => "Mascot",                         // 2
    sounds => "Sounds",                         // 3
    open_click => "Open on click",              // 4
    fast_animations => "Fast animations",       // 5
    hide_fullscreen => "Auto-hide over apps",   // 6
    lyrics => "Synced lyrics",                  // 7
    lyrics_in_pill => "Lyrics in the pill",     // 8
    water_on => "Reminders",                    // 9
    todo_start => "Remind at start",            // 10
    todo_end => "Remind at end",                // 11
    hud_volume => "Volume",                     // 12
    hud_brightness => "Brightness",             // 13
    hud_caps => "Caps Lock",                    // 14
    hud_language => "Input language",           // 15
    popup_battery => "Battery popups",          // 16
    popup_wifi => "Wi-Fi popups",               // 17
    popup_bluetooth => "Headset popups",        // 18
    notif_badge => "Notification badge",        // 19
    shelf_clear => "Remove after drag out",     // 20
    lyrics_view => "Lyrics view",               // 21
);

pub fn toggle_get(s: &Settings, i: usize) -> bool {
    let mut c = s.clone();
    toggle_field(&mut c, i).map(|b| *b).unwrap_or(false)
}

/// Whether changing this toggle needs the window re-laid out.
pub fn toggle_needs_layout(_i: usize) -> bool {
    false
}

// ------------------------------------------------------------------ numeric settings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Size,
    Display,
    Focus,
    Short,
    Long,
    Goal,
    Glass,
    Bottle,
    Snooze,
    From,
    Until,
}

impl Field {
    pub fn get(self, s: &Settings) -> i32 {
        match self {
            Field::Size => s.size as i32,
            Field::Display => s.display as i32,
            Field::Focus => s.focus as i32,
            Field::Short => s.short_break as i32,
            Field::Long => s.long_break as i32,
            Field::Goal => s.water_goal as i32,
            Field::Glass => s.water_glass as i32,
            Field::Bottle => s.water_bottle as i32,
            Field::Snooze => s.snooze as i32,
            Field::From => s.water_from,
            Field::Until => s.water_until,
        }
    }

    pub fn set(self, s: &mut Settings, v: i32) {
        let u = v.max(0) as u32;
        match self {
            Field::Size => s.size = u,
            Field::Display => s.display = u as usize,
            Field::Focus => s.focus = u,
            Field::Short => s.short_break = u,
            Field::Long => s.long_break = u,
            Field::Goal => s.water_goal = u,
            Field::Glass => s.water_glass = u,
            Field::Bottle => s.water_bottle = u,
            Field::Snooze => s.snooze = u,
            Field::From => s.water_from = v,
            Field::Until => s.water_until = v,
        }
    }

    fn bounds(self) -> (i32, i32) {
        match self {
            Field::From => (4, 14),
            Field::Until => (16, 24),
            _ => (0, 100_000),
        }
    }

    pub fn needs_layout(self) -> bool {
        matches!(self, Field::Size | Field::Display)
    }
}

// ------------------------------------------------------------------ layout

pub const CATS: [(&str, char); 8] = [
    ("General", '\u{E713}'),
    ("Appearance", '\u{E790}'),
    ("Music", '\u{EC4F}'),
    ("Pomodoro", '\u{E916}'),
    ("Water", '\u{0}'),
    ("To-do", '\u{E73E}'),
    ("HUD", '\u{E7F4}'),
    ("Shelf", '\u{E8B7}'),
];

enum Kind {
    Toggle(usize),
    Choice(Field, Vec<(String, i32)>),
    Hours(Field),
    Button(&'static str, Hit),
}

struct Row {
    label: &'static str,
    kind: Kind,
}

fn row(label: &'static str, kind: Kind) -> Row {
    Row { label, kind }
}

fn tog(i: usize) -> Row {
    Row { label: TOGGLE_LABELS[i], kind: Kind::Toggle(i) }
}

fn mins(v: &[i32]) -> Vec<(String, i32)> {
    v.iter().map(|m| (format!("{m}"), *m)).collect()
}

fn liters(v: &[i32]) -> Vec<(String, i32)> {
    v.iter()
        .map(|ml| {
            let l = *ml as f32 / 1000.0;
            let t = if (l * 10.0).fract().abs() < 0.01 { format!("{l:.1}") } else { format!("{l:.2}") };
            (t, *ml)
        })
        .collect()
}

fn rows(cat: usize, monitors: usize) -> Vec<Row> {
    match cat {
        0 => vec![
            tog(0),
            tog(6),
            tog(4),
            tog(3),
            tog(5),
            row("Data folder", Kind::Button("Open", Hit::SetFolder)),
        ],
        1 => vec![
            row(
                "Size",
                Kind::Choice(Field::Size, vec![("Auto".into(), 0), ("90%".into(), 90), ("100%".into(), 100), ("110%".into(), 110), ("125%".into(), 125)]),
            ),
            row("Screen", Kind::Choice(Field::Display, (0..monitors.max(1)).map(|i| (format!("{}", i + 1), i as i32)).collect())),
            tog(1),
            tog(2),
            tog(19),
        ],
        2 => vec![tog(7), tog(21), tog(8)],
        3 => vec![
            row("Focus, min", Kind::Choice(Field::Focus, mins(&[15, 20, 25, 30, 45, 50]))),
            row("Short break, min", Kind::Choice(Field::Short, mins(&[3, 5, 10]))),
            row("Long break, min", Kind::Choice(Field::Long, mins(&[10, 15, 20, 30]))),
        ],
        4 => vec![
            tog(9),
            row("Daily goal, L", Kind::Choice(Field::Goal, liters(&[1500, 2000, 2500, 3000]))),
            row("Glass, ml", Kind::Choice(Field::Glass, mins(&[150, 200, 250, 300]))),
            row("Bottle, L", Kind::Choice(Field::Bottle, liters(&[500, 750, 1000, 1500]))),
            row("Reminders from", Kind::Hours(Field::From)),
            row("Reminders until", Kind::Hours(Field::Until)),
        ],
        5 => vec![tog(10), tog(11), row("“Later” snoozes, min", Kind::Choice(Field::Snooze, mins(&[5, 10, 15, 30])))],
        6 => vec![tog(12), tog(13), tog(14), tog(15), tog(16), tog(17), tog(18)],
        _ => vec![tog(20), row("Clear the shelf", Kind::Button("Clear", Hit::ShelfClear))],
    }
}

pub const W: f32 = 520.0;
pub const H: f32 = 330.0;
const RX: f32 = 500.0; // right edge of the controls
const ROW_H: f32 = 34.0;

impl App {
    pub(super) fn draw_settings(&self, g: &Gfx, hits: &mut Hits) {
        g.text("Settings", R::new(28.0, 12.0, 200.0, 26.0), 15.0, 700, gfx::WHITE, Align::Left);
        g.circle(492.0, 25.0, 11.0, gfx::CHIP_HI);
        g.icon('\u{E711}', 492.0, 25.0, 9.0, gfx::WHITE);
        hits.add(R::new(478.0, 11.0, 28.0, 28.0), Hit::SetClose);

        // sidebar
        for (i, (name, glyph)) in CATS.iter().enumerate() {
            let r = R::new(18.0, 48.0 + i as f32 * 28.0, 130.0, 26.0);
            let sel = i == self.set_cat;
            if sel {
                g.rrect(r, 9.0, gfx::CHIP_HI);
            }
            let c = if sel { gfx::WHITE } else { gfx::GRAY };
            if *glyph == '\u{0}' {
                g.drop_icon(r.x + 16.0, r.cy(), 12.0, c);
            } else {
                g.icon(*glyph, r.x + 16.0, r.cy(), 11.0, c);
            }
            g.text(name, R::new(r.x + 32.0, r.y, r.w - 34.0, r.h), 12.0, 600, c, Align::Left);
            hits.add(r, Hit::SetCat(i as u8));
        }
        let q = R::new(18.0, 292.0, 130.0, 24.0);
        widgets::chip(g, q, "Quit Notch", Some('\u{E7E8}'), gfx::CHIP, gfx::RED, 11.0);
        hits.add(q, Hit::SetQuit);
        g.line(160.0, 50.0, 160.0, 312.0, 1.0, gfx::WHITE.a(0.08));

        // rows
        let mons = sys::monitors().len();
        for (i, rw) in rows(self.set_cat, mons).iter().enumerate() {
            let y = 46.0 + i as f32 * ROW_H;
            g.text(rw.label, R::new(176.0, y, 170.0, ROW_H), 12.0, 600, gfx::WHITE, Align::Left);
            if i > 0 {
                g.line(176.0, y, RX, y, 1.0, gfx::WHITE.a(0.06));
            }
            match &rw.kind {
                Kind::Toggle(t) => {
                    let on = toggle_get(&self.s, *t);
                    widgets::switch(g, RX - 28.0, y + 9.0, on, gfx::GREEN);
                    hits.add(R::new(RX - 40.0, y, 40.0, ROW_H), Hit::SetToggle(*t as u8));
                }
                Kind::Choice(f, opts) => {
                    let cur = f.get(&self.s);
                    let widths: Vec<f32> = opts.iter().map(|(l, _)| g.measure(l, 11.0, 600) + 18.0).collect();
                    let total: f32 = widths.iter().sum::<f32>() + 5.0 * (widths.len().saturating_sub(1)) as f32;
                    let mut x = RX - total;
                    for ((l, v), w) in opts.iter().zip(widths) {
                        let r = R::new(x, y + 6.0, w, 22.0);
                        let on = *v == cur;
                        widgets::chip(g, r, l, None, if on { gfx::BLUE } else { gfx::CHIP }, if on { gfx::WHITE } else { gfx::GRAY }, 11.0);
                        hits.add(r, Hit::SetChoice(*f, *v));
                        x += w + 5.0;
                    }
                }
                Kind::Hours(f) => {
                    let v = f.get(&self.s);
                    let plus = R::new(RX - 26.0, y + 6.0, 26.0, 22.0);
                    let minus = R::new(RX - 26.0 - 64.0 - 26.0, y + 6.0, 26.0, 22.0);
                    widgets::chip(g, minus, "", Some('\u{E738}'), gfx::CHIP, gfx::WHITE, 9.0);
                    widgets::chip(g, plus, "", Some('\u{E710}'), gfx::CHIP, gfx::WHITE, 9.0);
                    g.text(&format!("{:02}:00", v), R::new(minus.x + 26.0, y, 64.0, ROW_H), 12.0, 600, gfx::WHITE, Align::Center);
                    hits.add(minus, Hit::SetStep(*f, -1));
                    hits.add(plus, Hit::SetStep(*f, 1));
                }
                Kind::Button(label, h) => {
                    let w = g.measure(label, 11.0, 600) + 24.0;
                    let r = R::new(RX - w, y + 6.0, w, 22.0);
                    widgets::chip(g, r, label, None, gfx::CHIP, gfx::WHITE, 11.0);
                    hits.add(r, *h);
                }
            }
        }
    }

    pub fn open_settings(&mut self) {
        self.settings_open = true;
        self.hud = None;
        self.kick();
    }

    pub(super) fn close_settings(&mut self) {
        self.settings_open = false;
        self.expanded = false;
        self.kick();
        // re-open the island if the pointer is still on it
        if self.cursor_in_island() {
            self.expand();
        }
    }

    pub(super) fn settings_click(&mut self, h: Hit) -> Option<Deferred> {
        match h {
            Hit::SetCat(i) => {
                self.set_cat = (i as usize).min(CATS.len() - 1);
                self.draw();
            }
            Hit::SetClose => self.close_settings(),
            Hit::SetQuit => return Some(Deferred::Quit),
            Hit::SetFolder => sys::open_path(&store::data_dir().to_string_lossy()),
            Hit::SetToggle(i) => {
                if let Some(b) = toggle_field(&mut self.s, i as usize) {
                    *b = !*b;
                }
                self.settings_changed(toggle_needs_layout(i as usize));
            }
            Hit::SetChoice(f, v) => {
                f.set(&mut self.s, v);
                self.settings_changed(f.needs_layout());
            }
            Hit::SetStep(f, d) => {
                let (lo, hi) = f.bounds();
                let v = (f.get(&self.s) + d as i32).clamp(lo, hi);
                f.set(&mut self.s, v);
                self.settings_changed(false);
            }
            _ => {}
        }
        None
    }
}
