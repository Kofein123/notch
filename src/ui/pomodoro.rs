//! Pomodoro page: focus / short break / long break with a 500 ms tick.

use std::time::{Duration, Instant};

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::gfx::{self, Align, Gfx, R};
use crate::store::Settings;

use super::widgets::{self, Hits};
use super::{AlertIcon, App, Hit, T_POMO};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    #[default]
    Focus,
    Short,
    Long,
}

#[derive(Default)]
pub struct Pomo {
    pub phase: Phase,
    pub running: bool,
    end: Option<Instant>,
    left: f64,
    pub sessions: u32,
}

impl Pomo {
    pub fn total(&self, s: &Settings) -> f64 {
        let min = match self.phase {
            Phase::Focus => s.focus,
            Phase::Short => s.short_break,
            Phase::Long => s.long_break,
        };
        min.max(1) as f64 * 60.0
    }

    pub fn remaining(&self) -> f64 {
        if self.running {
            self.end.map(|e| e.saturating_duration_since(Instant::now()).as_secs_f64()).unwrap_or(0.0)
        } else {
            self.left
        }
    }

    pub fn reset(&mut self, s: &Settings) {
        self.running = false;
        self.end = None;
        self.left = self.total(s);
    }

    pub fn text(&self, _s: &Settings) -> String {
        let r = self.remaining().ceil() as i64;
        format!("{:02}:{:02}", r / 60, r % 60)
    }

    fn start(&mut self) {
        self.running = true;
        self.end = Some(Instant::now() + Duration::from_secs_f64(self.left.max(0.0)));
    }

    fn pause(&mut self) {
        self.left = self.remaining();
        self.running = false;
        self.end = None;
    }
}

fn phase_label(p: Phase) -> &'static str {
    match p {
        Phase::Focus => "Focus time",
        Phase::Short => "Short break",
        Phase::Long => "Long break",
    }
}

impl App {
    pub(super) fn draw_pomo(&self, g: &Gfx, hits: &mut Hits) {
        let total = self.pomo.total(&self.s);
        let rem = self.pomo.remaining();
        let prog = if total > 0.0 { (1.0 - rem / total) as f32 } else { 0.0 };

        // ring
        g.ring(81.0, 70.0, 34.0, 5.0, gfx::RED.a(0.22));
        g.arc(81.0, 70.0, 34.0, 5.0, prog, gfx::RED);
        g.icon('\u{E916}', 81.0, 70.0, 22.0, gfx::RED);

        // time + reset, centred on x = 265
        let txt = self.pomo.text(&self.s);
        let tw = g.measure(&txt, 29.0, 700);
        let group = tw + 8.0 + 22.0;
        let x0 = 265.0 - group / 2.0;
        g.text(&txt, R::new(x0, 38.0, tw + 2.0, 40.0), 29.0, 700, gfx::WHITE, Align::Left);
        let rx = x0 + tw + 8.0 + 11.0;
        g.circle(rx, 58.0, 11.0, gfx::CHIP);
        g.icon('\u{E72C}', rx, 58.0, 11.0, gfx::GRAY);
        hits.add(R::new(rx - 13.0, 45.0, 26.0, 26.0), Hit::PomoReset);
        g.text(phase_label(self.pomo.phase), R::new(165.0, 74.0, 200.0, 16.0), 11.0, 400, gfx::GRAY, Align::Center);

        // chip row: [Short Break] [play 54] [Focus Time], centred on 265
        let l = "Short Break";
        let r = "Focus Time";
        let lw = widgets::chip_width(g, l, 11.5) + 9.0;
        let rw = widgets::chip_width(g, r, 11.5) + 9.0;
        let row = lw + 54.0 + rw + 16.0;
        let mut x = 265.0 - row / 2.0;
        let y = 96.0;
        let h = 26.0;
        let short_on = self.pomo.phase != Phase::Focus;
        widgets::chip(
            g,
            R::new(x, y, lw, h),
            l,
            None,
            if short_on { gfx::CHIP_HI } else { gfx::CHIP },
            if short_on { gfx::WHITE } else { gfx::GRAY },
            11.5,
        );
        hits.add(R::new(x, y, lw, h), Hit::PomoShort);
        x += lw + 8.0;
        let glyph = if self.pomo.running { '\u{E769}' } else { '\u{E768}' };
        widgets::chip(g, R::new(x, y, 54.0, h), "", Some(glyph), gfx::RED, gfx::WHITE, 12.0);
        hits.add(R::new(x, y, 54.0, h), Hit::PomoPlay);
        x += 54.0 + 8.0;
        widgets::chip(
            g,
            R::new(x, y, rw, h),
            r,
            None,
            if !short_on { gfx::CHIP_HI } else { gfx::CHIP },
            if !short_on { gfx::WHITE } else { gfx::GRAY },
            11.5,
        );
        hits.add(R::new(x, y, rw, h), Hit::PomoFocus);
    }

    pub(super) fn pomo_toggle(&mut self) {
        if self.pomo.running {
            self.pomo.pause();
            unsafe {
                let _ = KillTimer(self.hwnd, T_POMO);
            }
        } else {
            if self.pomo.left <= 0.0 {
                self.pomo.reset(&self.s);
            }
            self.pomo.start();
            unsafe {
                SetTimer(self.hwnd, T_POMO, 500, None);
            }
        }
        self.kick();
    }

    pub(super) fn pomo_reset(&mut self) {
        self.pomo.reset(&self.s);
        unsafe {
            let _ = KillTimer(self.hwnd, T_POMO);
        }
        self.kick();
    }

    pub(super) fn pomo_select(&mut self, p: Phase) {
        let p = if p == Phase::Short && self.pomo.phase == Phase::Short { Phase::Long } else { p };
        self.pomo.phase = p;
        self.pomo_reset();
    }

    pub(super) fn pomo_tick(&mut self) {
        if !self.pomo.running {
            unsafe {
                let _ = KillTimer(self.hwnd, T_POMO);
            }
            return;
        }
        if self.pomo.remaining() > 0.0 {
            if !self.hidden {
                self.draw();
            }
            return;
        }
        // phase finished: Focus -> short break (long after 4), break -> focus
        let (title, sub) = match self.pomo.phase {
            Phase::Focus => {
                self.pomo.sessions += 1;
                if self.pomo.sessions >= 4 {
                    self.pomo.sessions = 0;
                    self.pomo.phase = Phase::Long;
                    ("Long Break!".to_string(), format!("{} min, you earned it", self.s.long_break))
                } else {
                    self.pomo.phase = Phase::Short;
                    ("Break Time!".to_string(), format!("{} minute short break", self.s.short_break))
                }
            }
            _ => {
                self.pomo.phase = Phase::Focus;
                ("Focus Time!".to_string(), format!("Back to work, {} min", self.s.focus))
            }
        };
        self.pomo.reset(&self.s);
        self.pomo.start();
        self.alert(AlertIcon::Glyph('\u{E916}'), gfx::RED, &title, &sub, Vec::new());
    }
}
