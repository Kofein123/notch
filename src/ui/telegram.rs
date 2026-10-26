//! Telegram page, sized like every other page (420 x 140).
//!
//! * Inbox (default): up to three chats that need you — unread first, then the
//!   ones you talked to today. Click to reply, right-click to mark as read.
//! * Reply: the last messages of one chat, quick-reply chips and an input.
//! * All chats: behind the list button in the header, with search.
//! * Peek: when a message arrives the pill drops a card with the sender and as
//!   many of the latest lines as fit; Reply / Read right there.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::gfx::{self, Align, Color, Gfx, R};
use crate::store;
use crate::tg::{self, Auth, Chat, Kind, Msg, Peek};

use super::widgets::{self, Hits};
use super::{App, Deferred, Hit, T_CARET, T_PEEK};

pub const TG_BLUE: Color = Color::hex(0x2aabee);
const X0: f32 = 44.0;
const X1: f32 = 376.0;
const ROW_H: f32 = 28.0;

pub const PEEK_W: f32 = 400.0;

/// One-tap answers in the reply view.
pub const QUICK: [&str; 3] = ["Ок", "Спасибо!", "Позже отвечу"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    ApiId,
    ApiHash,
    Phone,
    Code,
    Password,
    Reply,
    Search,
}

/// What the page shows, cloned from the TDLib thread's model on every update.
#[derive(Default)]
pub struct View {
    pub auth: Option<Auth>,
    pub error: String,
    pub chats: Vec<Chat>,
    pub history: Vec<Msg>,
    pub history_more: bool,
    pub unread: i32,
    pub muted: Vec<i64>,
}

pub struct TgUi {
    pub client: tg::Tg,
    pub view: View,
    pub open: Option<i64>,
    /// "All chats" list instead of the inbox
    pub all: bool,
    pub scroll: usize,
    pub msg_scroll: usize,
    pub text: String,
    pub focus: bool,
    pub api_id: String,
    pub peek: Option<(Peek, Instant)>,
    pub queue: VecDeque<Peek>,
    pub avatars: std::collections::HashMap<i64, (u64, u64, windows::Win32::Graphics::Direct2D::ID2D1Bitmap)>,
}

fn today_unix_start() -> i64 {
    let now = store::now();
    let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    now_unix - (now.minute as i64 * 60 + now.second as i64)
}

impl TgUi {
    pub fn new(client: tg::Tg) -> TgUi {
        let mut t = TgUi {
            client,
            view: View::default(),
            open: None,
            all: false,
            scroll: 0,
            msg_scroll: 0,
            text: String::new(),
            focus: false,
            api_id: String::new(),
            peek: None,
            queue: VecDeque::new(),
            avatars: std::collections::HashMap::new(),
        };
        t.refresh();
        t
    }

    pub fn refresh(&mut self) {
        let Ok(s) = self.client.state.lock() else { return };
        let prev = self.view.auth.clone();
        self.view.auth = Some(s.auth.clone());
        self.view.error = s.error.clone();
        self.view.chats = s.sorted();
        self.view.muted = s.chats.values().filter(|c| s.is_muted(c)).map(|c| c.id).collect();
        self.view.history = s.history.clone();
        self.view.history_more = s.history_more;
        self.view.unread = s.unread_total();
        drop(s);
        if prev != self.view.auth && !matches!(self.field(), Some(Field::Reply | Field::Search)) {
            self.text.clear();
        }
    }

    pub fn ready(&self) -> bool {
        self.view.auth == Some(Auth::Ready)
    }

    /// The input the page currently shows.
    pub fn field(&self) -> Option<Field> {
        match self.view.auth.as_ref()? {
            Auth::NeedKeys => Some(if self.api_id.is_empty() { Field::ApiId } else { Field::ApiHash }),
            Auth::WaitPhone => Some(Field::Phone),
            Auth::WaitCode(_) => Some(Field::Code),
            Auth::WaitPassword(_) => Some(Field::Password),
            Auth::Ready => {
                if self.open.is_some() {
                    Some(Field::Reply)
                } else if self.all {
                    Some(Field::Search)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn chat(&self, id: i64) -> Option<&Chat> {
        self.view.chats.iter().find(|c| c.id == id)
    }

    fn muted(&self, c: &Chat) -> bool {
        self.view.muted.contains(&c.id)
    }

    /// Chats that need you: unread (not muted, not channels) first, then
    /// people you talked to today. At most `n`.
    pub fn inbox(&self, n: usize) -> Vec<&Chat> {
        let today = today_unix_start();
        let mut v: Vec<&Chat> = self
            .view
            .chats
            .iter()
            .filter(|c| c.kind != Kind::Channel && !self.muted(c))
            .filter(|c| c.unread > 0 || c.last.as_ref().map_or(false, |m| m.date >= today))
            .collect();
        v.sort_by_key(|c| (c.unread == 0, -(c.last.as_ref().map_or(0, |m| m.date))));
        v.truncate(n);
        v
    }

    /// The "All chats" list filtered by the search text.
    pub fn filtered(&self) -> Vec<&Chat> {
        let q = self.text.trim().to_lowercase();
        self.view.chats.iter().filter(|c| q.is_empty() || c.title.to_lowercase().contains(&q)).collect()
    }
}

fn local_offset() -> i64 {
    let now = store::now();
    let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let local_now = now.day as i64 * 86400 + now.minute as i64 * 60 + now.second as i64;
    local_now - now_unix
}

fn hhmm_unix(t: i64) -> String {
    let now = store::now();
    let local = t + local_offset();
    let day = local.div_euclid(86400);
    if day == now.day as i64 {
        store::hhmm((local.rem_euclid(86400) / 60) as i32)
    } else if day == now.day as i64 - 1 {
        "yesterday".into()
    } else {
        let (_, m, d) = store::civil_from_days(day as i32);
        format!("{d:02}.{m:02}")
    }
}

impl App {
    fn avatar(&self, g: &Gfx, chat_id: i64, title: &str, cx: f32, cy: f32, d: f32) {
        if let Some((_, _, bmp)) = self.tgu.avatars.get(&chat_id) {
            g.image(bmp, R::new(cx - d / 2.0, cy - d / 2.0, d, d), d / 2.0);
            return;
        }
        g.circle(cx, cy, d / 2.0, Color::hex(tg::avatar_color(chat_id)));
        g.text(&tg::initials(title), R::new(cx - d / 2.0, cy - d / 2.0, d, d), d * 0.38, 700, gfx::WHITE, Align::Center);
    }

    /// Creates D2D bitmaps for avatars that arrived since the last frame.
    pub(super) fn ensure_avatars(&mut self, g: &Gfx) {
        let gen = g.generation;
        let Ok(s) = self.tgu.client.state.lock() else { return };
        for c in s.chats.values() {
            let Some(px) = &c.photo else { continue };
            let fresh = matches!(self.tgu.avatars.get(&c.id), Some((pid, gn, _)) if *pid == c.photo_id && *gn == gen);
            if fresh {
                continue;
            }
            if let Some(b) = g.bitmap(px, tg::AVATAR, tg::AVATAR) {
                self.tgu.avatars.insert(c.id, (c.photo_id, gen, b));
            }
        }
    }

    /// Header extras of the Telegram page: unread count and the list button.
    pub(super) fn draw_tg_header(&self, g: &Gfx, hits: &mut Hits, pw: f32) {
        if !self.tgu.ready() {
            return;
        }
        let cy = 19.0;
        let bx = pw - 34.0;
        let active = self.tgu.all && self.tgu.open.is_none();
        g.circle(bx, cy, 11.0, if active { TG_BLUE } else { gfx::CHIP_HI });
        g.icon('\u{E8FD}', bx, cy, 10.0, gfx::WHITE);
        hits.add(R::new(bx - 13.0, cy - 13.0, 26.0, 26.0), Hit::TgAll);
        if self.tgu.view.unread > 0 {
            let n = if self.tgu.view.unread > 99 { "99+".to_string() } else { self.tgu.view.unread.to_string() };
            let w = (g.measure(&n, 9.5, 700) + 10.0).max(18.0);
            let r = R::new(bx - 16.0 - w, cy - 8.0, w, 16.0);
            g.rrect(r, 8.0, TG_BLUE);
            g.text(&n, r, 9.5, 700, gfx::WHITE, Align::Center);
        }
    }

    // -------------------------------------------------------------- page

    pub(super) fn draw_telegram(&self, g: &Gfx, hits: &mut Hits) {
        let t = &self.tgu;
        if t.view.auth != Some(Auth::Ready) {
            self.draw_login(g, hits);
        } else if let Some(id) = t.open {
            self.draw_reply(g, hits, id);
        } else if t.all {
            self.draw_all(g, hits);
        } else {
            self.draw_inbox(g, hits);
        }
    }

    fn draw_input(&self, g: &Gfx, hits: &mut Hits, r: R, placeholder: &str, masked: bool) {
        let t = &self.tgu;
        let size = 11.5;
        g.rrect(r, r.h / 2.0, gfx::CHIP_HI);
        if t.focus {
            g.rrect_stroke(r.inset(0.5), r.h / 2.0, TG_BLUE.a(0.8), 1.0);
        }
        let tr = R::new(r.x + 12.0, r.y, r.w - 24.0, r.h);
        let shown: String = if masked { "•".repeat(t.text.chars().count()) } else { t.text.clone() };
        // keep the end of long text visible
        let mut visible = shown.clone();
        while g.measure(&visible, size, 500) > tr.w - 4.0 && !visible.is_empty() {
            visible.remove(0);
        }
        if shown.is_empty() {
            g.text(placeholder, tr, size, 400, gfx::DIM, Align::Left);
        } else {
            g.text(&visible, tr, size, 500, gfx::WHITE, Align::Left);
        }
        if t.focus && self.caret_on {
            let cx = tr.x + g.measure(&visible, size, 500) + 1.0;
            g.line(cx, r.y + 6.0, cx, r.y + r.h - 6.0, 1.2, gfx::WHITE);
        }
        hits.add(r, Hit::TgInput);
    }

    fn draw_login(&self, g: &Gfx, hits: &mut Hits) {
        let t = &self.tgu;
        let auth = t.view.auth.clone().unwrap_or(Auth::NoLib);
        let field = t.field();
        let (step, title, desc): (&str, &str, String) = match (&auth, field) {
            (Auth::NoLib, _) => ("", "Telegram module missing", "Keep tdjson.dll from the release zip next to Notch.exe.".into()),
            (_, Some(Field::ApiId)) => (
                "1/5",
                "Connect Telegram",
                "my.telegram.org → API development tools → create an app, paste api_id.".into(),
            ),
            (_, Some(Field::ApiHash)) => ("2/5", "API hash", "Paste api_hash from the same page.".into()),
            (_, Some(Field::Phone)) => ("3/5", "Phone number", "International format, like +7 701 123 45 67.".into()),
            (Auth::WaitCode(h), _) => ("4/5", "Login code", h.clone()),
            (Auth::WaitPassword(h), _) => ("5/5", "Cloud password", if h.is_empty() { "Two-step verification is on.".into() } else { h.clone() }),
            (Auth::Other(m), _) => ("", "Almost there", m.clone()),
            _ => ("", "Connecting…", "Starting Telegram, one moment.".into()),
        };
        g.text(title, R::new(X0, 36.0, 220.0, 20.0), 13.0, 700, gfx::WHITE, Align::Left);
        if !step.is_empty() {
            g.text(step, R::new(X1 - 40.0, 36.0, 40.0, 20.0), 10.5, 600, gfx::GRAY, Align::Right);
        }
        let err = !t.view.error.is_empty();
        let (line, color) = if err { (t.view.error.as_str(), gfx::RED) } else { (desc.as_str(), gfx::GRAY) };
        g.text(line, R::new(X0, 56.0, X1 - X0, 16.0), 10.5, if err { 600 } else { 400 }, color, Align::Left);

        if let Some(f) = field {
            let placeholder = match f {
                Field::ApiId => "api_id, e.g. 1234567",
                Field::ApiHash => "api_hash",
                Field::Phone => "+7 …",
                Field::Code => "12345",
                Field::Password => "Password",
                _ => "",
            };
            self.draw_input(g, hits, R::new(X0, 78.0, X1 - X0 - 72.0, 26.0), placeholder, f == Field::Password);
            let nb = R::new(X1 - 64.0, 78.0, 64.0, 26.0);
            widgets::chip(g, nb, "Next", None, TG_BLUE, gfx::WHITE, 11.5);
            hits.add(nb, Hit::TgSubmit);
        }
        // helper chips
        let mut x = X0;
        let y = 108.0;
        let mut chip = |g: &Gfx, hits: &mut Hits, label: &str, h: Hit| {
            let w = g.measure(label, 9.5, 600) + 18.0;
            let r = R::new(x, y, w, 17.0);
            widgets::chip(g, r, label, None, gfx::CHIP, gfx::WHITE, 9.5);
            hits.add(r, h);
            x += w + 5.0;
        };
        if matches!(field, Some(Field::ApiId | Field::ApiHash)) {
            chip(g, hits, "Open my.telegram.org", Hit::TgOpenSite);
        }
        if matches!(field, Some(Field::ApiHash)) || matches!(auth, Auth::WaitPhone | Auth::WaitCode(_) | Auth::WaitPassword(_) | Auth::Other(_)) {
            chip(g, hits, "Start over", Hit::TgResetKeys);
        }
        if field.is_some() {
            g.text("Ctrl+V pastes", R::new(X1 - 90.0, y, 90.0, 17.0), 9.5, 400, gfx::DIM, Align::Right);
        }
    }

    /// One compact chat row: avatar, name + preview on one line, time / badge.
    fn draw_row(&self, g: &Gfx, hits: &mut Hits, c: &Chat, y: f32) {
        let t = &self.tgu;
        let r = R::new(X0 - 6.0, y + 1.0, X1 - X0 + 12.0, ROW_H - 2.0);
        if c.unread > 0 && !t.muted(c) {
            g.rrect(r, 9.0, gfx::WHITE.a(0.05));
        }
        let cy = y + ROW_H / 2.0;
        self.avatar(g, c.id, &c.title, X0 + 10.0, cy, 22.0);
        // right side: badge or time
        let mut right = X1;
        if c.unread > 0 {
            let n = if c.unread > 99 { "99+".to_string() } else { c.unread.to_string() };
            let bw = (g.measure(&n, 9.0, 700) + 9.0).max(16.0);
            let br = R::new(right - bw, cy - 7.5, bw, 15.0);
            g.rrect(br, 7.5, if t.muted(c) { gfx::DIM } else { TG_BLUE });
            g.text(&n, br, 9.0, 700, gfx::WHITE, Align::Center);
            right -= bw + 6.0;
        }
        let time = c.last.as_ref().map(|m| hhmm_unix(m.date)).unwrap_or_default();
        let tw = g.measure(&time, 9.5, 400);
        g.text(&time, R::new(right - tw - 1.0, y, tw + 2.0, ROW_H), 9.5, 400, gfx::GRAY, Align::Right);
        right -= tw + 8.0;
        // name, then the preview after it
        let x = X0 + 28.0;
        let name_w = g.measure(&c.title, 12.0, 600).min((right - x) * 0.55);
        g.text(&c.title, R::new(x, y, name_w + 2.0, ROW_H), 12.0, 600, gfx::WHITE, Align::Left);
        let preview = c
            .last
            .as_ref()
            .map(|m| {
                if m.out {
                    format!("You: {}", m.text)
                } else if c.kind == Kind::Group && !m.sender.is_empty() {
                    format!("{}: {}", m.sender.split(' ').next().unwrap_or(""), m.text)
                } else {
                    m.text.clone()
                }
            })
            .unwrap_or_default();
        let px = x + name_w + 8.0;
        if right - px > 20.0 {
            g.text(&preview, R::new(px, y, right - px, ROW_H), 11.0, 400, gfx::GRAY, Align::Left);
        }
        hits.add(r, Hit::TgChat(c.id));
    }

    fn draw_inbox(&self, g: &Gfx, hits: &mut Hits) {
        let t = &self.tgu;
        if t.view.chats.is_empty() {
            g.text("Loading chats…", R::new(X0, 60.0, X1 - X0, 24.0), 12.0, 600, gfx::GRAY, Align::Center);
            return;
        }
        let list = t.inbox(3);
        if list.is_empty() {
            g.icon('\u{E73E}', 210.0, 62.0, 18.0, gfx::GREEN);
            g.text("All caught up", R::new(X0, 76.0, X1 - X0, 20.0), 12.5, 700, gfx::WHITE, Align::Center);
            g.text("No new messages today", R::new(X0, 94.0, X1 - X0, 16.0), 10.5, 400, gfx::GRAY, Align::Center);
            return;
        }
        for (k, c) in list.iter().enumerate() {
            self.draw_row(g, hits, c, 36.0 + k as f32 * ROW_H);
        }
    }

    fn draw_all(&self, g: &Gfx, hits: &mut Hits) {
        let t = &self.tgu;
        self.draw_input(g, hits, R::new(X0, 36.0, X1 - X0, 22.0), "Search chats", false);
        let list = t.filtered();
        if list.is_empty() {
            g.text("Nothing found", R::new(X0, 74.0, X1 - X0, 24.0), 11.5, 600, gfx::GRAY, Align::Center);
            return;
        }
        let rows = 2;
        let scroll = t.scroll.min(list.len().saturating_sub(rows));
        for (k, c) in list.iter().skip(scroll).take(rows).enumerate() {
            self.draw_row(g, hits, c, 62.0 + k as f32 * ROW_H);
        }
        if list.len() > rows {
            let track = ROW_H * rows as f32 - 6.0;
            let h = (track * rows as f32 / list.len() as f32).max(10.0);
            let pos = scroll as f32 / (list.len() - rows) as f32;
            g.rrect(R::new(X1 + 8.0, 65.0 + (track - h) * pos, 2.5, h), 1.25, gfx::WHITE.a(0.25));
        }
    }

    fn draw_reply(&self, g: &Gfx, hits: &mut Hits, id: i64) {
        let t = &self.tgu;
        let title = t.chat(id).map(|c| c.title.clone()).unwrap_or_default();
        let kind = t.chat(id).map(|c| c.kind).unwrap_or_default();
        // top bar: back, avatar, name ... quick replies
        g.circle(X0 + 6.0, 46.0, 9.0, gfx::CHIP_HI);
        g.icon('\u{E76B}', X0 + 6.0, 46.0, 7.5, gfx::WHITE);
        hits.add(R::new(X0 - 6.0, 34.0, 24.0, 24.0), Hit::TgBack);
        self.avatar(g, id, &title, X0 + 28.0, 46.0, 18.0);
        let mut qx = X1;
        let mut quick: Vec<(R, usize)> = Vec::new();
        for (k, q) in QUICK.iter().enumerate().rev() {
            let w = g.measure(q, 9.5, 600) + 16.0;
            qx -= w;
            quick.push((R::new(qx, 37.0, w, 18.0), k));
            qx -= 4.0;
        }
        let name_w = (qx - (X0 + 42.0) - 6.0).max(40.0);
        g.text(&title, R::new(X0 + 42.0, 36.0, name_w, 20.0), 12.0, 700, gfx::WHITE, Align::Left);
        for (r, k) in quick {
            widgets::chip(g, r, QUICK[k], None, gfx::CHIP, gfx::WHITE, 9.5);
            hits.add(r, Hit::TgQuick(k as u8));
        }

        // the last two messages, one line each
        let lines: Vec<&Msg> = t.view.history.iter().rev().skip(t.msg_scroll).take(2).collect();
        let older = t.view.history.len() > t.msg_scroll + 2 || t.view.history_more;
        if lines.is_empty() {
            g.text("No messages yet", R::new(X0, 60.0, X1 - X0, 34.0), 11.0, 400, gfx::GRAY, Align::Left);
        }
        let mut y = 81.0;
        for m in lines.iter() {
            let who = if m.out {
                "You".to_string()
            } else if kind == Kind::Group && !m.sender.is_empty() {
                m.sender.split(' ').next().unwrap_or("").to_string()
            } else {
                String::new()
            };
            let time = hhmm_unix(m.date);
            let tw = g.measure(&time, 9.0, 400);
            g.text(&time, R::new(X1 - tw - 1.0, y, tw + 2.0, 18.0), 9.0, 400, gfx::DIM, Align::Right);
            let mut x = X0;
            if !who.is_empty() {
                let ww = g.measure(&who, 11.0, 700);
                let c = if m.out { TG_BLUE } else { Color::hex(tg::avatar_color(m.sender.len() as i64 * 7 + 3)) };
                g.text(&who, R::new(x, y, ww + 2.0, 18.0), 11.0, 700, c, Align::Left);
                x += ww + 6.0;
            }
            let tc = if m.failed { gfx::RED } else if m.out { gfx::WHITE.a(0.7) } else { gfx::WHITE };
            g.text(&m.text, R::new(x, y, X1 - tw - 8.0 - x, 18.0), 11.0, 400, tc, Align::Left);
            if m.out && m.pending {
                g.icon('\u{E823}', X1 - tw - 12.0, y + 9.0, 7.5, gfx::DIM);
            }
            y -= 18.0;
        }
        if older && !lines.is_empty() {
            for k in 0..3 {
                g.circle(205.0 + k as f32 * 5.0, 60.0, 1.3, gfx::DIM);
            }
        }

        // input + send
        self.draw_input(g, hits, R::new(X0, 100.0, X1 - X0 - 30.0, 24.0), "Reply…", false);
        let can = !t.text.trim().is_empty();
        g.circle(X1 - 12.0, 112.0, 12.0, if can { TG_BLUE } else { gfx::CHIP_HI });
        g.icon('\u{E724}', X1 - 12.0, 112.0, 9.5, gfx::WHITE);
        hits.add(R::new(X1 - 26.0, 98.0, 28.0, 28.0), Hit::TgSend);
    }

    // -------------------------------------------------------------- peek

    pub(super) fn peek_height(&self) -> f32 {
        let Some((p, _)) = &self.tgu.peek else { return 62.0 };
        let n = p.lines.len().min(3) as f32;
        let dots = if p.lines.len() > 3 { 10.0 } else { 0.0 };
        (42.0 + n * 17.0 + dots).max(64.0)
    }

    pub(super) fn draw_peek(&self, g: &Gfx, hits: &mut Hits) {
        let Some((p, _)) = &self.tgu.peek else { return };
        let h = self.peek_height();
        hits.add(R::new(0.0, 0.0, PEEK_W, h), Hit::TgPeek);
        self.avatar(g, p.chat_id, &p.title, 34.0, 32.0, 34.0);
        let x = 62.0;
        let w = PEEK_W - x - 112.0;
        g.text(&p.title, R::new(x, 10.0, w, 20.0), 13.0, 700, gfx::WHITE, Align::Left);
        let mut y = 30.0;
        if p.lines.len() > 3 {
            for k in 0..3 {
                g.circle(x + 3.0 + k as f32 * 6.0, y + 4.0, 1.6, gfx::GRAY);
            }
            y += 10.0;
        }
        let start = p.lines.len().saturating_sub(3);
        for line in &p.lines[start..] {
            let text = if self.s.tg_preview { line.as_str() } else { "New message" };
            g.text(text, R::new(x, y, PEEK_W - x - 20.0, 17.0), 11.5, 400, gfx::WHITE.a(0.85), Align::Left);
            y += 17.0;
        }
        // Read / Reply
        let rr = R::new(PEEK_W - 66.0, 10.0, 50.0, 20.0);
        widgets::chip(g, rr, "Reply", None, TG_BLUE, gfx::WHITE, 10.0);
        hits.add(rr, Hit::TgPeekReply);
        let rd = R::new(PEEK_W - 66.0 - 46.0, 10.0, 42.0, 20.0);
        widgets::chip(g, rd, "Read", None, gfx::CHIP_HI, gfx::WHITE, 10.0);
        hits.add(rd, Hit::TgPeekRead);
    }

    /// New messages from the TDLib thread.
    pub fn on_tg(&mut self, kind: usize) {
        self.tgu.refresh();
        if kind == tg::N_PEEK {
            let fresh: Vec<Peek> = self.tgu.client.state.lock().map(|mut s| s.peeks.drain(..).collect()).unwrap_or_default();
            if self.s.tg_popups {
                for p in fresh {
                    // merge into what is shown / queued
                    if let Some((cur, until)) = &mut self.tgu.peek {
                        if cur.chat_id == p.chat_id {
                            cur.lines.extend(p.lines);
                            *until = Instant::now() + Duration::from_secs(7);
                            continue;
                        }
                    }
                    if let Some(q) = self.tgu.queue.iter_mut().find(|q| q.chat_id == p.chat_id) {
                        q.lines.extend(p.lines);
                    } else {
                        self.tgu.queue.push_back(p);
                    }
                }
                if self.tgu.peek.is_none() {
                    self.next_peek();
                } else if self.s.sounds {
                    crate::sys::chime("soft");
                }
            }
        }
        // the open conversation is being read: keep marking it
        if let Some(id) = self.tgu.open {
            if self.expanded && self.page == super::P_TG && self.tgu.view.chats.iter().any(|c| c.id == id && c.unread > 0) {
                self.tgu.client.mark_read(id);
            }
        }
        self.kick();
    }

    fn next_peek(&mut self) {
        // nobody wants a peek about the chat that is open on screen
        while let Some(p) = self.tgu.queue.pop_front() {
            if self.expanded && self.page == super::P_TG && self.tgu.open == Some(p.chat_id) {
                continue;
            }
            if self.s.sounds {
                crate::sys::chime("soft");
            }
            self.tgu.peek = Some((p, Instant::now() + Duration::from_secs(7)));
            unsafe {
                SetTimer(self.hwnd, T_PEEK, 500, None);
            }
            break;
        }
        self.kick();
    }

    pub(super) fn peek_tick(&mut self) {
        let Some((_, until)) = &self.tgu.peek else {
            unsafe {
                let _ = KillTimer(self.hwnd, T_PEEK);
            }
            return;
        };
        // hovering keeps it
        if self.cursor_in_island() {
            return;
        }
        if Instant::now() >= *until {
            self.close_peek();
        }
    }

    fn close_peek(&mut self) {
        self.tgu.peek = None;
        if self.tgu.queue.is_empty() {
            unsafe {
                let _ = KillTimer(self.hwnd, T_PEEK);
            }
            self.kick();
        } else {
            self.next_peek();
        }
    }

    // -------------------------------------------------------------- actions

    pub(super) fn tg_open_chat(&mut self, id: i64, focus: bool) -> Option<Deferred> {
        self.tgu.open = Some(id);
        self.tgu.msg_scroll = 0;
        self.tgu.text.clear();
        self.tgu.client.open_chat(id);
        self.tgu.client.set_visible_chat(Some(id));
        self.tgu.client.mark_read(id);
        self.tgu.refresh();
        self.draw();
        if focus {
            return self.tg_focus(true);
        }
        None
    }

    pub(super) fn tg_back(&mut self) -> Option<Deferred> {
        self.tgu.open = None;
        self.tgu.text.clear();
        self.tgu.client.close_chat();
        self.tgu.client.set_visible_chat(None);
        self.tgu.refresh();
        self.draw();
        self.tg_focus(false)
    }

    pub(super) fn tg_toggle_all(&mut self) -> Option<Deferred> {
        let mut d = None;
        if self.tgu.open.is_some() {
            d = self.tg_back();
            self.tgu.all = true;
        } else {
            self.tgu.all = !self.tgu.all;
            self.tgu.text.clear();
            if self.tgu.focus {
                d = self.tg_focus(false);
            }
        }
        self.tgu.scroll = 0;
        self.draw();
        d
    }

    pub(super) fn tg_focus(&mut self, on: bool) -> Option<Deferred> {
        if self.tgu.focus == on {
            return if on { Some(Deferred::Focus(true)) } else { None };
        }
        self.tgu.focus = on;
        self.caret_on = true;
        unsafe {
            if on {
                SetTimer(self.hwnd, T_CARET, 530, None);
            } else if self.add.is_none() {
                let _ = KillTimer(self.hwnd, T_CARET);
            }
        }
        self.draw();
        Some(Deferred::Focus(on))
    }

    pub(super) fn tg_peek_open(&mut self) -> Option<Deferred> {
        let (p, _) = self.tgu.peek.take()?;
        unsafe {
            let _ = KillTimer(self.hwnd, T_PEEK);
        }
        self.tgu.queue.retain(|q| q.chat_id != p.chat_id);
        self.page = super::P_TG;
        self.expanded = true;
        self.hud = None;
        self.kick();
        self.tg_open_chat(p.chat_id, true)
    }

    pub(super) fn tg_peek_read(&mut self) {
        if let Some((p, _)) = &self.tgu.peek {
            self.tgu.client.mark_chat_read(p.chat_id);
        }
        self.close_peek();
    }

    pub(super) fn tg_mark_read(&mut self, id: i64) {
        self.tgu.client.mark_chat_read(id);
    }

    pub(super) fn tg_quick(&mut self, k: u8) -> Option<Deferred> {
        let id = self.tgu.open?;
        let text = QUICK.get(k as usize)?;
        self.tgu.client.send_text(id, text);
        self.tgu.msg_scroll = 0;
        self.draw();
        None
    }

    pub(super) fn tg_submit(&mut self) -> Option<Deferred> {
        let text = self.tgu.text.trim().to_string();
        match self.tgu.field() {
            Some(Field::ApiId) => {
                if text.parse::<i32>().map(|v| v > 0).unwrap_or(false) {
                    self.tgu.api_id = text;
                    self.tgu.text.clear();
                    if let Ok(mut s) = self.tgu.client.state.lock() {
                        s.error.clear();
                    }
                } else if let Ok(mut s) = self.tgu.client.state.lock() {
                    s.error = "api_id is a number from my.telegram.org".into();
                }
            }
            Some(Field::ApiHash) => {
                let id = self.tgu.api_id.clone();
                if self.tgu.client.set_keys(&id, &text) {
                    self.tgu.text.clear();
                }
            }
            Some(Field::Phone) => self.tgu.client.phone(&text),
            Some(Field::Code) => self.tgu.client.code(&text),
            Some(Field::Password) => {
                self.tgu.client.password(&self.tgu.text.clone());
                self.tgu.text.clear();
            }
            Some(Field::Reply) => {
                if let Some(id) = self.tgu.open {
                    if !text.is_empty() {
                        self.tgu.client.send_text(id, &text);
                        self.tgu.text.clear();
                        self.tgu.msg_scroll = 0;
                    }
                }
            }
            Some(Field::Search) => {
                // Enter opens the first match
                if let Some(id) = self.tgu.filtered().first().map(|c| c.id) {
                    self.tgu.text.clear();
                    return self.tg_open_chat(id, true);
                }
            }
            None => {}
        }
        self.tgu.refresh();
        self.draw();
        None
    }

    pub(super) fn tg_reset_keys(&mut self) {
        self.tgu.api_id.clear();
        self.tgu.text.clear();
        self.tgu.client.reset_keys();
        if self.tgu.ready() {
            return;
        }
        self.tgu.refresh();
        self.draw();
    }

    pub(super) fn tg_key_char(&mut self, ch: u32) {
        if ch == 22 {
            // Ctrl+V
            let clip = crate::sys::clipboard_text(self.hwnd).unwrap_or_default();
            let clip = clip.replace(['\r', '\n'], " ");
            self.tgu.text.push_str(clip.trim());
            self.caret_on = true;
            self.draw();
            return;
        }
        let t = &mut self.tgu;
        match ch {
            8 => {
                t.text.pop();
            }
            // Ctrl+Backspace: delete a word
            127 => {
                let trimmed = t.text.trim_end().to_string();
                let cut = trimmed.rfind(' ').map(|i| i + 1).unwrap_or(0);
                t.text.truncate(cut);
            }
            13 | 27 | 9 => {}
            c if c >= 32 => {
                if let Some(c) = char::from_u32(c) {
                    if t.text.chars().count() < 4000 {
                        t.text.push(c);
                    }
                }
            }
            _ => {}
        }
        if self.tgu.all {
            self.tgu.scroll = 0;
        }
        self.caret_on = true;
        self.draw();
    }

    pub(super) fn tg_wheel(&mut self, up: bool) {
        if self.tgu.open.is_some() {
            if up {
                self.tgu.msg_scroll += 1;
                let n = self.tgu.view.history.len();
                if self.tgu.msg_scroll + 6 >= n {
                    self.tgu.client.load_older();
                }
                self.tgu.msg_scroll = self.tgu.msg_scroll.min(n.saturating_sub(1));
            } else {
                self.tgu.msg_scroll = self.tgu.msg_scroll.saturating_sub(1);
            }
        } else if self.tgu.all {
            let max = self.tgu.filtered().len().saturating_sub(2);
            self.tgu.scroll = if up { self.tgu.scroll.saturating_sub(1) } else { (self.tgu.scroll + 1).min(max) };
        }
        self.draw();
    }
}
