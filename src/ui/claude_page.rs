//! Claude page: is Claude Desktop writing an answer right now, and when did
//! the last one finish.

use std::time::{Duration, Instant};

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::claude;
use crate::gfx::{self, Align, Color, Gfx, R};
use crate::store;

use super::widgets::{self, Hits};
use super::{Act, AlertIcon, App, Hit, T_SPIN};

pub const ORANGE: Color = Color::hex(0xd97757);

#[derive(Default)]
pub struct ClaudeUi {
    pub state: usize,
    pub since: Option<Instant>,
    pub last: Option<(Instant, Duration)>,
    pub unseen: bool,
    pub day: i32,
    pub count: u32,
    pub total: Duration,
}

pub fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn ago(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    if s < 45 {
        "just now".into()
    } else if s < 3600 {
        format!("{} min ago", (s + 30) / 60)
    } else {
        format!("{} h ago", s / 3600)
    }
}

/// Claude's mark: eight rounded rays; spinning while `spin` is Some.
pub fn spark(g: &Gfx, cx: f32, cy: f32, r: f32, c: Color, spin: Option<f32>) {
    let rot = spin.unwrap_or(0.0);
    for k in 0..8 {
        let a = rot + k as f32 * std::f32::consts::TAU / 8.0;
        let pulse = match spin {
            Some(t) => 0.75 + 0.25 * ((t * 3.0 - k as f32 * 0.6).sin() * 0.5 + 0.5),
            None => 1.0,
        };
        let r0 = r * 0.22;
        let r1 = r * pulse;
        g.line(cx + a.cos() * r0, cy + a.sin() * r0, cx + a.cos() * r1, cy + a.sin() * r1, (r * 0.2).max(1.6), c);
    }
}

impl App {
    pub(super) fn draw_claude(&self, g: &Gfx, hits: &mut Hits) {
        let c = &self.claude;
        let working = c.state == claude::ST_WORKING;
        let closed = c.state == claude::ST_CLOSED;
        let (cx, cy) = (84.0, 74.0);
        g.circle(cx, cy, 34.0, ORANGE.a(if closed { 0.06 } else { 0.14 }));
        spark(g, cx, cy, 22.0, if closed { gfx::DIM } else { ORANGE }, if working { Some(self.t()) } else { None });

        let x = 140.0;
        let (title, tc, sub) = if !self.s.claude_on {
            ("Tracking is off", gfx::GRAY, "Turn it on in Settings → Claude".to_string())
        } else if closed {
            ("Claude is closed", gfx::GRAY, "Open the Claude app to track answers".to_string())
        } else if working {
            let el = c.since.map(|t| fmt_dur(t.elapsed())).unwrap_or_default();
            ("Writing an answer…", ORANGE, format!("for {el}"))
        } else if let Some((t, d)) = c.last {
            (
                if c.unseen { "Answer is ready" } else { "Idle" },
                if c.unseen { gfx::GREEN } else { gfx::WHITE },
                format!("Last answer {} · took {}", ago(t), fmt_dur(d)),
            )
        } else {
            ("Idle", gfx::WHITE, "Waiting for your next message".to_string())
        };
        if c.unseen && !working {
            g.icon('\u{E73E}', x + 7.0, 52.0, 13.0, gfx::GREEN);
            g.text(title, R::new(x + 20.0, 40.0, 240.0, 24.0), 17.0, 700, tc, Align::Left);
        } else {
            g.text(title, R::new(x, 40.0, 260.0, 24.0), 17.0, 700, tc, Align::Left);
        }
        g.text(&sub, R::new(x, 64.0, 250.0, 16.0), 11.0, 400, gfx::GRAY, Align::Left);
        let today = if c.day == store::now().day && c.count > 0 {
            let avg = c.total / c.count.max(1);
            format!("Today: {} answers · avg {}", c.count, fmt_dur(avg))
        } else {
            "Today: no answers yet".into()
        };
        g.text(&today, R::new(x, 82.0, 250.0, 16.0), 10.5, 400, gfx::DIM, Align::Left);

        let br = R::new(x, 102.0, 108.0, 24.0);
        widgets::chip(g, br, "Open Claude", None, ORANGE, gfx::WHITE, 11.0);
        hits.add(br, Hit::ClaudeOpen);
    }

    /// State change reported by the watcher thread.
    pub fn on_claude(&mut self, state: usize) {
        let prev = self.claude.state;
        self.claude.state = state;
        let now = Instant::now();
        if state == claude::ST_WORKING && prev != claude::ST_WORKING {
            self.claude.since = Some(now);
            self.claude.unseen = false;
            unsafe {
                SetTimer(self.hwnd, T_SPIN, 70, None);
            }
        }
        if prev == claude::ST_WORKING && state != claude::ST_WORKING {
            unsafe {
                let _ = KillTimer(self.hwnd, T_SPIN);
            }
            if let Some(start) = self.claude.since.take() {
                let took = now - start;
                // a closed window mid-answer isn't a finished answer; blips aren't either
                if state == claude::ST_IDLE && took >= Duration::from_secs(3) {
                    let today = store::now().day;
                    if self.claude.day != today {
                        self.claude.day = today;
                        self.claude.count = 0;
                        self.claude.total = Duration::ZERO;
                    }
                    self.claude.count += 1;
                    self.claude.total += took;
                    self.claude.last = Some((now, took));
                    let looking = claude::is_foreground();
                    self.claude.unseen = !looking;
                    if !looking && self.s.claude_popup {
                        self.alert(
                            AlertIcon::Glyph('\u{E73E}'),
                            ORANGE,
                            "Claude finished",
                            &format!("The answer took {}", fmt_dur(took)),
                            vec![("Later".into(), Act::Dismiss), ("Open".into(), Act::OpenClaude)],
                        );
                        // a finished answer shouldn't hang around for a minute
                        if let Some(a) = self.alerts.last_mut() {
                            a.until = now + Duration::from_secs(8);
                        }
                    }
                }
            }
        }
        self.kick();
    }

    pub(super) fn claude_open(&mut self) {
        self.claude.unseen = false;
        claude::open();
        self.draw();
    }

    /// Called every second: reading Claude clears the "unseen" dot.
    pub(super) fn claude_second(&mut self) {
        if self.claude.unseen && claude::is_foreground() {
            self.claude.unseen = false;
            self.draw();
        }
    }
}
