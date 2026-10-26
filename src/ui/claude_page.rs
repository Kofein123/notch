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
    /// minute of day the current answer started
    pub started_at: Option<i32>,
    /// (finished at, minute of day; took)
    pub recent: Vec<(i32, Duration)>,
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
        let closed = c.state == claude::ST_CLOSED || !self.s.claude_on;
        let x0 = 44.0;
        let x1 = 376.0;

        // row 1: mark, status, timer
        g.circle(x0 + 12.0, 50.0, 15.0, ORANGE.a(if closed { 0.06 } else { 0.16 }));
        spark(g, x0 + 12.0, 50.0, 10.0, if closed { gfx::DIM } else { ORANGE }, if working { Some(self.t()) } else { None });
        let (title, tc, sub) = if !self.s.claude_on {
            ("Tracking is off", gfx::GRAY, "Turn it on in Settings → Claude".to_string())
        } else if c.state == claude::ST_CLOSED {
            ("Claude is closed", gfx::GRAY, "Open the app to track answers".to_string())
        } else if working {
            let started = c.started_at.map(store::hhmm).unwrap_or_default();
            ("Writing an answer…", ORANGE, format!("Started at {started}"))
        } else if c.unseen {
            let (t, d) = c.last.unwrap_or((Instant::now(), Duration::ZERO));
            ("Answer is ready", gfx::GREEN, format!("Finished {} · took {}", ago(t), fmt_dur(d)))
        } else if c.day == store::now().day && c.count > 0 {
            let avg = c.total / c.count.max(1);
            ("Idle", gfx::WHITE, format!("Today: {} answers · avg {}", c.count, fmt_dur(avg)))
        } else {
            ("Idle", gfx::WHITE, "Waiting for your next message".to_string())
        };
        g.text(title, R::new(x0 + 34.0, 37.0, 200.0, 20.0), 14.0, 700, tc, Align::Left);
        g.text(&sub, R::new(x0 + 34.0, 56.0, 220.0, 16.0), 10.5, 400, gfx::GRAY, Align::Left);
        let timer = if working {
            c.since.map(|t| fmt_dur(t.elapsed()))
        } else if !closed {
            c.last.map(|(_, d)| fmt_dur(d))
        } else {
            None
        };
        if let Some(t) = timer {
            g.text(&t, R::new(x1 - 90.0, 36.0, 90.0, 24.0), 18.0, 700, if working { ORANGE } else { gfx::DIM }, Align::Right);
        }

        // row 2: recent answers + open
        let br = R::new(x1 - 92.0, 92.0, 92.0, 24.0);
        widgets::chip(g, br, "Open Claude", None, ORANGE, gfx::WHITE, 10.5);
        hits.add(br, Hit::ClaudeOpen);
        if c.recent.is_empty() {
            g.text("Answers you get today show up here", R::new(x0, 92.0, 220.0, 24.0), 10.0, 400, gfx::DIM, Align::Left);
        } else {
            let mut x = x0;
            for (min, d) in c.recent.iter().rev().take(3) {
                let label = format!("{} · {}", store::hhmm(*min), fmt_dur(*d));
                let w = g.measure(&label, 9.5, 600) + 16.0;
                if x + w > br.x - 6.0 {
                    break;
                }
                widgets::chip(g, R::new(x, 95.0, w, 18.0), &label, None, gfx::CHIP, gfx::GRAY, 9.5);
                x += w + 5.0;
            }
        }
    }

    /// State change reported by the watcher thread.
    pub fn on_claude(&mut self, state: usize) {
        let prev = self.claude.state;
        self.claude.state = state;
        let now = Instant::now();
        if state == claude::ST_WORKING && prev != claude::ST_WORKING {
            self.claude.since = Some(now);
            self.claude.started_at = Some(store::now().minute);
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
                        self.claude.recent.clear();
                    }
                    self.claude.count += 1;
                    self.claude.total += took;
                    self.claude.last = Some((now, took));
                    self.claude.recent.push((store::now().minute, took));
                    if self.claude.recent.len() > 20 {
                        self.claude.recent.remove(0);
                    }
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
