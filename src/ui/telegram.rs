//! Telegram page: sign-in, chat list, conversation with quick reply, and the
//! message peek that drops out of the pill when someone writes.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::gfx::{self, Align, Color, Gfx, R};
use crate::store;
use crate::tg::{self, Auth, Chat, Kind, Msg, Peek};

use super::widgets::{self, Hits};
use super::{App, Deferred, Hit, T_CARET, T_PEEK};

pub const TG_BLUE: Color = Color::hex(0x2aabee);
const BUBBLE_OUT: Color = Color::hex(0x2b6cb0);
pub const W: f32 = 480.0;
pub const H: f32 = 232.0;
const X0: f32 = 44.0;
const X1: f32 = 436.0;
const ROW_H: f32 = 36.0;
const ROWS: usize = 5;

pub const PEEK_W: f32 = 400.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    ApiId,
    ApiHash,
    Phone,
    Code,
    Password,
    Reply,
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
    pub scroll: usize,
    pub msg_scroll: usize,
    pub text: String,
    pub focus: bool,
    pub api_id: String,
    pub peek: Option<(Peek, Instant)>,
    pub queue: VecDeque<Peek>,
    pub avatars: std::collections::HashMap<i64, (u64, u64, windows::Win32::Graphics::Direct2D::ID2D1Bitmap)>,
}

impl TgUi {
    pub fn new(client: tg::Tg) -> TgUi {
        let mut t = TgUi {
            client,
            view: View::default(),
            open: None,
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
        if prev != self.view.auth && self.field() != Some(Field::Reply) {
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
            Auth::Ready => self.open.map(|_| Field::Reply),
            _ => None,
        }
    }

    fn chat(&self, id: i64) -> Option<&Chat> {
        self.view.chats.iter().find(|c| c.id == id)
    }
}

/// Clamps `text` to `lines` lines of `width`, adding an ellipsis.
pub fn clamp(g: &Gfx, text: &str, width: f32, size: f32, weight: i32, lines: usize) -> (String, f32) {
    let line_h = g.measure_wrap("Ag", width, size, weight).max(1.0);
    let limit = line_h * lines as f32 + 1.0;
    let h = g.measure_wrap(text, width, size, weight);
    if h <= limit {
        return (text.to_string(), h);
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if g.measure_wrap(&s, width, size, weight) <= limit {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let s = chars[..lo].iter().collect::<String>().trim_end().to_string() + "…";
    (s, line_h * lines as f32)
}

fn hhmm_unix(t: i64) -> String {
    // local time of a unix timestamp, via today's offset
    let now = store::now();
    let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let local_now = now.day as i64 * 86400 + now.minute as i64 * 60 + now.second as i64;
    let off = local_now - now_unix;
    let local = t + off;
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

    // -------------------------------------------------------------- page

    pub(super) fn draw_telegram(&self, g: &Gfx, hits: &mut Hits) {
        let t = &self.tgu;
        match &t.view.auth {
            Some(Auth::Ready) => match t.open {
                Some(id) => self.draw_conversation(g, hits, id),
                None => self.draw_chat_list(g, hits),
            },
            _ => self.draw_login(g, hits),
        }
    }

    fn draw_input(&self, g: &Gfx, hits: &mut Hits, r: R, placeholder: &str, masked: bool) {
        let t = &self.tgu;
        g.rrect(r, r.h / 2.0, gfx::CHIP_HI);
        if t.focus {
            g.rrect_stroke(r.inset(0.5), r.h / 2.0, TG_BLUE.a(0.8), 1.0);
        }
        let tr = R::new(r.x + 14.0, r.y, r.w - 28.0, r.h);
        let shown: String = if masked { "•".repeat(t.text.chars().count()) } else { t.text.clone() };
        // keep the end of long text visible
        let mut visible = shown.clone();
        while g.measure(&visible, 12.0, 500) > tr.w - 4.0 && !visible.is_empty() {
            visible.remove(0);
        }
        if shown.is_empty() {
            g.text(placeholder, tr, 12.0, 400, gfx::DIM, Align::Left);
        } else {
            g.text(&visible, tr, 12.0, 500, gfx::WHITE, Align::Left);
        }
        if t.focus && self.caret_on {
            let cx = tr.x + g.measure(&visible, 12.0, 500) + 1.0;
            g.line(cx, r.y + 8.0, cx, r.y + r.h - 8.0, 1.2, gfx::WHITE);
        }
        hits.add(r, Hit::TgInput);
    }

    fn draw_login(&self, g: &Gfx, hits: &mut Hits) {
        let t = &self.tgu;
        let auth = t.view.auth.clone().unwrap_or(Auth::NoLib);
        let field = t.field();
        let (step, title, desc): (&str, &str, String) = match (&auth, field) {
            (Auth::NoLib, _) => (
                "",
                "Telegram module missing",
                "tdjson.dll isn't next to Notch.exe. Download Notch-windows-x64.zip from Releases and keep both files in one folder.".into(),
            ),
            (_, Some(Field::ApiId)) => (
                "1 / 5",
                "Connect Telegram",
                "Telegram needs your own API key. Open my.telegram.org → API development tools, create any app and paste its api_id here.".into(),
            ),
            (_, Some(Field::ApiHash)) => ("2 / 5", "API hash", "Now paste api_hash from the same page (32 letters and digits).".into()),
            (_, Some(Field::Phone)) => ("3 / 5", "Phone number", "Your Telegram number in international format, like +7 701 123 45 67.".into()),
            (Auth::WaitCode(h), _) => ("4 / 5", "Login code", h.clone()),
            (Auth::WaitPassword(h), _) => (
                "5 / 5",
                "Cloud password",
                if h.is_empty() { "Two-step verification is on: enter your password.".into() } else { format!("Two-step verification is on. {h}") },
            ),
            (Auth::Other(m), _) => ("", "Almost there", m.clone()),
            _ => ("", "Connecting…", "Starting Telegram, one moment.".into()),
        };
        g.text(title, R::new(X0, 38.0, 300.0, 24.0), 15.0, 700, gfx::WHITE, Align::Left);
        if !step.is_empty() {
            g.text(step, R::new(X1 - 60.0, 38.0, 60.0, 24.0), 11.0, 600, gfx::GRAY, Align::Right);
        }
        g.text_wrap(&desc, R::new(X0, 64.0, X1 - X0, 36.0), 11.0, 400, gfx::GRAY, Align::Left);

        if let Some(f) = field {
            let placeholder = match f {
                Field::ApiId => "api_id, e.g. 1234567",
                Field::ApiHash => "api_hash",
                Field::Phone => "+7 …",
                Field::Code => "12345",
                Field::Password => "Password",
                Field::Reply => "",
            };
            self.draw_input(g, hits, R::new(X0, 104.0, 300.0, 32.0), placeholder, f == Field::Password);
            let nb = R::new(X0 + 308.0, 104.0, X1 - X0 - 308.0, 32.0);
            widgets::chip(g, nb, "Next", None, TG_BLUE, gfx::WHITE, 12.0);
            hits.add(nb, Hit::TgSubmit);
        }
        if !t.view.error.is_empty() {
            g.text(&t.view.error, R::new(X0 + 4.0, 140.0, X1 - X0, 18.0), 11.0, 600, gfx::RED, Align::Left);
        }
        // helper chips
        let mut x = X0;
        let y = 164.0;
        let mut chip = |g: &Gfx, hits: &mut Hits, label: &str, h: Hit| {
            let w = g.measure(label, 10.5, 600) + 22.0;
            let r = R::new(x, y, w, 22.0);
            widgets::chip(g, r, label, None, gfx::CHIP, gfx::WHITE, 10.5);
            hits.add(r, h);
            x += w + 6.0;
        };
        match field {
            Some(Field::ApiId) | Some(Field::ApiHash) => chip(g, hits, "Open my.telegram.org", Hit::TgOpenSite),
            _ => {}
        }
        if matches!(field, Some(Field::ApiHash)) || matches!(auth, Auth::WaitPhone | Auth::WaitCode(_) | Auth::WaitPassword(_) | Auth::Other(_)) {
            chip(g, hits, "Start over", Hit::TgResetKeys);
        }
        if field.is_some() {
            g.text("Ctrl+V pastes", R::new(X1 - 120.0, y, 120.0, 22.0), 10.0, 400, gfx::DIM, Align::Right);
        }
        g.text(
            "The session stays on this PC, encrypted with your Windows account.",
            R::new(X0, 192.0, X1 - X0, 16.0),
            9.5,
            400,
            gfx::DIM,
            Align::Left,
        );
    }

    fn draw_chat_list(&self, g: &Gfx, hits: &mut Hits) {
        let t = &self.tgu;
        let chats = &t.view.chats;
        if chats.is_empty() {
            g.text("Loading chats…", R::new(X0, 100.0, X1 - X0, 24.0), 12.0, 600, gfx::GRAY, Align::Center);
            return;
        }
        let scroll = t.scroll.min(chats.len().saturating_sub(ROWS));
        for (k, c) in chats.iter().skip(scroll).take(ROWS).enumerate() {
            let y = 36.0 + k as f32 * ROW_H;
            let r = R::new(X0 - 8.0, y + 1.0, X1 - X0 + 16.0, ROW_H - 2.0);
            if c.unread > 0 {
                g.rrect(r, 10.0, gfx::WHITE.a(0.04));
            }
            self.avatar(g, c.id, &c.title, X0 + 14.0, y + ROW_H / 2.0, 28.0);
            let muted = t.view.muted.contains(&c.id);
            let time = c.last.as_ref().map(|m| hhmm_unix(m.date)).unwrap_or_default();
            let tw = g.measure(&time, 10.0, 400);
            let mut name_w = X1 - (X0 + 36.0) - tw - 10.0;
            if muted {
                name_w -= 14.0;
            }
            let nw = g.measure(&c.title, 12.5, 600).min(name_w);
            g.text(&c.title, R::new(X0 + 36.0, y + 3.0, nw + 2.0, 17.0), 12.5, 600, gfx::WHITE, Align::Left);
            if muted {
                g.icon('\u{E74F}', X0 + 36.0 + nw + 10.0, y + 11.5, 9.0, gfx::DIM);
            }
            g.text(&time, R::new(X1 - tw - 2.0, y + 3.0, tw + 2.0, 17.0), 10.0, 400, gfx::GRAY, Align::Right);
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
            let mut pw = X1 - (X0 + 36.0);
            if c.unread > 0 {
                let n = if c.unread > 999 { "999+".to_string() } else { c.unread.to_string() };
                let bw = (g.measure(&n, 9.5, 700) + 10.0).max(18.0);
                let br = R::new(X1 - bw, y + 19.0, bw, 15.0);
                g.rrect(br, 7.5, if muted { gfx::DIM } else { TG_BLUE });
                g.text(&n, br, 9.5, 700, gfx::WHITE, Align::Center);
                pw -= bw + 8.0;
            }
            g.text(&preview, R::new(X0 + 36.0, y + 18.0, pw, 16.0), 11.0, 400, gfx::GRAY, Align::Left);
            hits.add(r, Hit::TgChat(c.id));
        }
        if chats.len() > ROWS {
            // scroll indicator
            let track = ROW_H * ROWS as f32 - 8.0;
            let frac = ROWS as f32 / chats.len() as f32;
            let pos = scroll as f32 / (chats.len() - ROWS) as f32;
            let h = (track * frac).max(14.0);
            g.rrect(R::new(X1 + 10.0, 40.0 + (track - h) * pos, 3.0, h), 1.5, gfx::WHITE.a(0.25));
        }
    }

    fn draw_conversation(&self, g: &Gfx, hits: &mut Hits, id: i64) {
        let t = &self.tgu;
        let title = t.chat(id).map(|c| c.title.clone()).unwrap_or_default();
        let kind = t.chat(id).map(|c| c.kind).unwrap_or_default();
        // top bar
        g.circle(X0 + 8.0, 48.0, 11.0, gfx::CHIP_HI);
        g.icon('\u{E76B}', X0 + 8.0, 48.0, 8.5, gfx::WHITE);
        hits.add(R::new(X0 - 6.0, 36.0, 28.0, 26.0), Hit::TgBack);
        self.avatar(g, id, &title, X0 + 36.0, 48.0, 22.0);
        g.text(&title, R::new(X0 + 52.0, 38.0, 300.0, 20.0), 13.0, 700, gfx::WHITE, Align::Left);

        // messages, newest at the bottom
        let top = 64.0;
        let mut y = 182.0;
        let maxw = 270.0;
        let mut more_above = t.view.history_more;
        let list: Vec<&Msg> = t.view.history.iter().rev().skip(t.msg_scroll).collect();
        if list.is_empty() {
            g.text("No messages yet", R::new(X0, 104.0, X1 - X0, 20.0), 11.5, 400, gfx::GRAY, Align::Center);
        }
        g.clip(R::new(X0 - 8.0, top, X1 - X0 + 16.0, 182.0 - top));
        for (k, m) in list.iter().enumerate() {
            let (text, th) = clamp(g, &m.text, maxw, 11.5, 400, 4);
            let show_name = !m.out && kind == Kind::Group && !m.sender.is_empty();
            let name_h = if show_name { 14.0 } else { 0.0 };
            let time = hhmm_unix(m.date);
            let time_w = g.measure(&time, 8.5, 400) + if m.out { 14.0 } else { 0.0 };
            let line_w = g.measure(&text, 11.5, 400).min(maxw);
            let name_w = if show_name { g.measure(&m.sender, 10.0, 700).min(maxw) } else { 0.0 };
            let bw = (line_w + time_w + 24.0).max(name_w + 18.0).min(maxw + 26.0);
            let bh = th + name_h + 10.0;
            if y - bh < top {
                more_above = more_above || k < list.len();
                break;
            }
            let bx = if m.out { X1 - bw } else { X0 };
            let br = R::new(bx, y - bh, bw, bh);
            g.rrect(br, 10.0, if m.out { BUBBLE_OUT } else { gfx::CHIP_HI });
            if show_name {
                let c = Color::hex(tg::avatar_color(m.sender.len() as i64 * 31 + m.sender.chars().next().map(|c| c as i64).unwrap_or(0)));
                g.text(&m.sender, R::new(bx + 9.0, y - bh + 4.0, name_w + 2.0, 13.0), 10.0, 700, c, Align::Left);
            }
            let tc = if m.failed { gfx::RED } else { gfx::WHITE };
            g.text_wrap(&text, R::new(bx + 9.0, y - bh + 5.0 + name_h, maxw, th + 4.0), 11.5, 400, tc, Align::Left);
            let ty = y - 14.0;
            g.text(&time, R::new(bx + bw - time_w - 8.0, ty, time_w, 12.0), 8.5, 400, gfx::WHITE.a(0.55), Align::Left);
            if m.out {
                let glyph = if m.pending { '\u{E823}' } else if m.failed { '\u{E783}' } else { '\u{E73E}' };
                g.icon(glyph, bx + bw - 12.0, ty + 6.0, 8.0, gfx::WHITE.a(0.7));
            }
            y -= bh + 5.0;
        }
        g.unclip();
        if more_above {
            for k in 0..3 {
                g.circle(240.0 - 8.0 + k as f32 * 8.0, top + 3.0, 1.8, gfx::GRAY);
            }
        }

        // reply bar
        self.draw_input(g, hits, R::new(X0, 188.0, X1 - X0 - 36.0, 28.0), "Message…", false);
        let can_send = !t.text.trim().is_empty();
        g.circle(X1 - 14.0, 202.0, 14.0, if can_send { TG_BLUE } else { gfx::CHIP_HI });
        g.icon('\u{E724}', X1 - 14.0, 202.0, 11.0, gfx::WHITE);
        hits.add(R::new(X1 - 30.0, 186.0, 32.0, 32.0), Hit::TgSend);
    }

    // -------------------------------------------------------------- peek

    pub(super) fn peek_height(&self) -> f32 {
        let Some((p, _)) = &self.tgu.peek else { return 62.0 };
        let n = p.lines.len().min(3) as f32;
        let dots = if p.lines.len() > 3 { 10.0 } else { 0.0 };
        44.0 + n * 17.0 + dots
    }

    pub(super) fn draw_peek(&self, g: &Gfx, hits: &mut Hits) {
        let Some((p, _)) = &self.tgu.peek else { return };
        let h = self.peek_height();
        hits.add(R::new(0.0, 0.0, PEEK_W, h), Hit::TgPeek);
        self.avatar(g, p.chat_id, &p.title, 34.0, 32.0, 34.0);
        let x = 62.0;
        let w = PEEK_W - x - 70.0;
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
            g.text(text, R::new(x, y, w + 50.0, 17.0), 11.5, 400, gfx::WHITE.a(0.85), Align::Left);
            y += 17.0;
        }
        // reply chip + close
        let rr = R::new(PEEK_W - 70.0, 10.0, 54.0, 22.0);
        widgets::chip(g, rr, "Reply", None, TG_BLUE, gfx::WHITE, 10.5);
        hits.add(rr, Hit::TgPeekReply);
        if p.lines.len() > 1 {
            let n = format!("{}", p.lines.len());
            g.text(&n, R::new(PEEK_W - 70.0, 34.0, 54.0, 14.0), 9.5, 600, gfx::GRAY, Align::Center);
        }
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
            if self.expanded && self.page == super::P_TG {
                if self.tgu.view.chats.iter().any(|c| c.id == id && c.unread > 0) {
                    self.tgu.client.mark_read(id);
                }
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
        } else {
            let max = self.tgu.view.chats.len().saturating_sub(ROWS);
            self.tgu.scroll = if up { self.tgu.scroll.saturating_sub(1) } else { (self.tgu.scroll + 1).min(max) };
        }
        self.draw();
    }
}
