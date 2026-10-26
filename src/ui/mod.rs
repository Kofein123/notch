//! The island: state, layers, springs, hit-testing and timers. Pages live in
//! their own files and only paint + react to hits.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{Fade, Spring};
use crate::gfx::{self, Align, Color, Gfx, R};
use crate::media::{self, MediaInfo};
use crate::shelf;
use crate::store::{self, Settings, ShelfItem, Task};
use crate::sys;

mod hud;
mod media_page;
mod pomodoro;
pub mod settings;
mod shelf_page;
mod todo;
mod water;
mod widgets;

pub use pomodoro::Pomo;
pub use widgets::Hits;

// ------------------------------------------------------------------ layout

/// Logical canvas: the layered window is this big; transparent pixels are
/// click-through, so only the island itself catches the mouse.
pub const CW: f32 = 560.0;
pub const CH: f32 = 340.0;
pub const CX: f32 = CW / 2.0;
pub const EAR: f32 = 9.0;

pub const PAGES: usize = 5;
pub const P_MUSIC: usize = 0;
pub const P_POMO: usize = 1;
pub const P_WATER: usize = 2;
pub const P_SHELF: usize = 3;
pub const P_TODO: usize = 4;

// ------------------------------------------------------------------ messages / timers

pub const WM_MEDIA: u32 = WM_APP + 1;
pub const WM_KEYHOOK: u32 = WM_APP + 2;
pub const WM_NET: u32 = WM_APP + 3;
pub const WM_NOTIF: u32 = WM_APP + 4;
pub const WM_DRAG: u32 = WM_APP + 5;
pub const WM_TRAY: u32 = WM_APP + 6;

pub const T_CLOCK: usize = 1;
pub const T_BG: usize = 3;
pub const T_CLOSE: usize = 5;
pub const T_HUD: usize = 6;
pub const T_FOLLOW: usize = 7;
pub const T_POMO: usize = 8;
pub const T_TODO: usize = 9;
pub const T_CARET: usize = 10;
pub const T_WATER: usize = 11;
pub const T_ANIM: usize = 12;
pub const T_MEDIA: usize = 13;
pub const T_HOVER: usize = 14;
pub const T_ALERT: usize = 15;
pub const T_NET: usize = 16;
pub const T_COPIED: usize = 17;
pub const T_LANG: usize = 18;
pub const T_BLINK: usize = 19;
pub const T_PET: usize = 20;

// ------------------------------------------------------------------ types

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Ghost,
    Settings,
    Pill,
    Expanded,
    Hud,
    Alert,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Island,
    PrevPage,
    NextPage,
    Page(usize),
    AlertBody,
    AlertBtn(usize),
    // music
    Lyrics,
    Prev,
    Toggle,
    Next,
    VolumeBtn,
    Seek(R),
    Volume(R),
    // pomodoro
    PomoPlay,
    PomoReset,
    PomoShort,
    PomoFocus,
    // water
    WaterDrink,
    WaterSettings,
    WaterReminders,
    WaterGoal(u32),
    WaterBottle(u32),
    WaterFrom,
    WaterUntil,
    WaterDone,
    // shelf
    ShelfTile(usize),
    ShelfClear,
    // to-do
    TodoDay,
    TodoAdd,
    TodoEdit,
    TodoCheck(usize),
    TodoDelete(usize),
    TodoInput,
    TodoStartChip,
    TodoDurChip,
    TodoCancel,
    TodoSave,
    // settings
    OpenSettings,
    SetClose,
    SetQuit,
    SetFolder,
    SetCat(u8),
    SetToggle(u8),
    SetChoice(settings::Field, i32),
    SetStep(settings::Field, i8),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Dismiss,
    TodoLater(String),
    TodoStart(String),
    TodoDone(String),
    WaterDrank,
    WaterSnooze,
    WaterRefilled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlertIcon {
    Glyph(char),
    Drop,
}

#[derive(Clone, Debug)]
pub struct Alert {
    pub icon: AlertIcon,
    pub color: Color,
    pub title: String,
    pub sub: String,
    pub buttons: Vec<(String, Act)>,
    pub until: Instant,
}

#[derive(Clone, Debug)]
pub struct Hud {
    pub icon: char,
    pub color: Color,
    pub title: String,
    pub value: Option<f32>,
    pub right: String,
    pub right_color: Color,
}

#[derive(Clone, Debug)]
enum Press {
    Seek(R),
    Volume(R),
    Tile { idx: usize, x: f32, y: f32 },
}

pub struct AddForm {
    pub text: String,
    pub start: Option<i32>,
    pub dur: i32,
}

/// Work the window procedure must do after the app borrow is released
/// (these calls re-enter the window procedure).
pub enum Deferred {
    DragOut(Vec<String>, usize),
    Menu,
    Focus(bool),
    Quit,
}

// ------------------------------------------------------------------ app

pub struct App {
    pub hwnd: HWND,
    gfx: Option<Gfx>,
    pub s: Settings,
    pub scale: f32,
    win_x: i32,
    win_y: i32,
    mon: RECT,
    started: Instant,

    // shape + layers
    sw: Spring,
    sh: Spring,
    sr: Spring,
    layer: Layer,
    fade: Fade,
    page: usize,
    page_shown: usize,
    page_fade: Fade,
    expanded: bool,
    last_anim: Instant,
    anim_on: bool,
    hits: Vec<(R, Hit)>,
    tracking: bool,
    press: Option<Press>,
    hidden: bool,
    ticks: u32,
    blink: bool,

    // media
    media_shared: media::Shared,
    media_tx: Sender<media::Cmd>,
    pub m: MediaInfo,
    art: Option<ID2D1Bitmap>,
    art_key: (u64, u64),
    art_tint: Option<Color>,
    tint_id: u64,
    lyr: Spring,
    track: String,
    wide_shown: bool,
    wheel_acc: i32,
    settings_open: bool,
    set_cat: usize,
    auto_hide: bool,
    body: Fade,
    pet: bool,
    pet_x: Spring,
    pet_y: Spring,
    pet_fade: Fade,
    pet_happy: Option<Instant>,
    rng: u32,
    vol_mode: bool,
    seek_preview: Option<f64>,
    audio: sys::Audio,

    // system
    batt: sys::Battery,
    notif: u32,
    hud: Option<Hud>,
    alerts: Vec<Alert>,
    last_lang: String,
    last_net: Option<sys::Net>,
    brightness: Option<u32>,
    ac: Option<bool>,
    batt_pct: Option<u32>,

    // pages
    pomo: Pomo,
    water: store::Water,
    water_settings: bool,
    water_snoozed: bool,
    shelf: Vec<ShelfItem>,
    inbox: shelf::Inbox,
    drag_over: bool,
    shelf_scroll: usize,
    copied: Option<(usize, Instant)>,
    tasks: Vec<Task>,
    todo_day: i32,
    todo_edit: bool,
    todo_scroll: usize,
    add: Option<AddForm>,
    caret_on: bool,
}

fn now_ms_to_next_minute() -> u32 {
    let st = store::now().st;
    let ms = (60 - st.wSecond as i64) * 1000 - st.wMilliseconds as i64 + 30;
    ms.max(100) as u32
}

impl App {
    pub fn new(hwnd: HWND) -> App {
        let s = Settings::load();
        let today = store::now().day;
        let (media_shared, media_tx) = media::spawn(hwnd, WM_MEDIA);
        sys::install_keyboard_hook(hwnd, WM_KEYHOOK);
        sys::register_power(hwnd);
        sys::watch_network(hwnd, WM_NET);
        sys::watch_notifications(hwnd, WM_NOTIF);
        let inbox = shelf::register(hwnd, WM_DRAG);
        if s.autostart {
            sys::set_autostart(true);
        }
        let page = s.last_page.min(PAGES - 1);
        let mut pomo = Pomo::default();
        pomo.reset(&s);
        let mut app = App {
            hwnd,
            gfx: None,
            scale: 1.0,
            win_x: 0,
            win_y: 0,
            mon: RECT::default(),
            started: Instant::now(),
            sw: Spring::new(172.0, 260.0, 0.18),
            sh: Spring::new(30.0, 260.0, 0.18),
            sr: Spring::new(13.0, 260.0, 0.1),
            layer: Layer::Pill,
            fade: Fade::new(1.0, 7.0),
            page,
            page_shown: page,
            page_fade: Fade::new(1.0, 9.0),
            expanded: false,
            last_anim: Instant::now(),
            anim_on: false,
            hits: Vec::new(),
            tracking: false,
            press: None,
            hidden: false,
            ticks: 0,
            blink: false,
            media_shared,
            media_tx,
            m: MediaInfo::default(),
            art: None,
            art_key: (0, 0),
            art_tint: None,
            tint_id: 0,
            lyr: Spring::new(0.0, 160.0, 0.0),
            track: String::new(),
            wide_shown: false,
            wheel_acc: 0,
            settings_open: false,
            set_cat: 0,
            auto_hide: false,
            body: Fade::new(1.0, 6.0),
            pet: false,
            pet_x: Spring::new(0.0, 60.0, 0.25),
            pet_y: Spring::new(0.0, 60.0, 0.25),
            pet_fade: Fade::new(0.0, 2.5),
            pet_happy: None,
            rng: (Instant::now().elapsed().subsec_nanos() ^ 0x9e37_79b9) | 1,
            vol_mode: false,
            seek_preview: None,
            audio: sys::Audio::new(),
            batt: sys::battery(),
            notif: 0,
            hud: None,
            alerts: Vec::new(),
            last_lang: sys::input_language(),
            last_net: Some(sys::network()),
            brightness: None,
            ac: None,
            batt_pct: None,
            pomo,
            water: store::load_water(today),
            water_settings: false,
            water_snoozed: false,
            shelf: store::load_shelf(),
            inbox,
            drag_over: false,
            shelf_scroll: 0,
            copied: None,
            tasks: store::load_tasks(),
            todo_day: 0,
            todo_edit: false,
            todo_scroll: 0,
            add: None,
            caret_on: true,
            s,
        };
        app.apply_speed();
        app.place();
        let (w, h, r) = app.target_shape();
        app.sw.snap(w);
        app.sh.snap(h);
        app.sr.snap(r);
        unsafe {
            SetTimer(hwnd, T_CLOCK, now_ms_to_next_minute(), None);
            SetTimer(hwnd, T_BG, 1000, None);
            SetTimer(hwnd, T_TODO, 15_000, None);
            SetTimer(hwnd, T_WATER, 15_000, None);
        }
        app.check_todo();
        app
    }

    fn apply_speed(&mut self) {
        let mul = if self.s.fast_animations { 1.8 } else { 1.0 };
        self.sw = Spring::new(self.sw.v, 260.0, 0.18);
        self.sh = Spring::new(self.sh.v, 260.0, 0.18);
        self.sr = Spring::new(self.sr.v, 260.0, 0.1);
        for sp in [&mut self.sw, &mut self.sh, &mut self.sr] {
            sp.speed(mul);
        }
        self.fade.per_sec = 7.0 * mul;
        self.page_fade.per_sec = 9.0 * mul;
    }

    // -------------------------------------------------------------- window placement

    /// Sizes the canvas for the chosen monitor and puts it at the top centre.
    pub fn place(&mut self) {
        let mons = sys::monitors();
        let Some(mon) = mons.get(self.s.display.min(mons.len().saturating_sub(1))).copied() else { return };
        let dpi = sys::monitor_scale(mon.handle);
        let size = if self.s.size == 0 { 1.0 } else { self.s.size as f32 / 100.0 };
        let scale = (dpi * size).max(0.5);
        let pw = (CW * scale).ceil() as i32;
        let ph = (CH * scale).ceil() as i32;
        self.mon = mon.rect;
        self.scale = scale;
        self.win_x = mon.rect.left + ((mon.rect.right - mon.rect.left) - pw) / 2;
        self.win_y = mon.rect.top;
        match &mut self.gfx {
            Some(g) => {
                let _ = g.resize(pw, ph, scale);
            }
            None => self.gfx = Gfx::new(pw, ph, scale).ok(),
        }
        self.art = None;
        self.art_key = (0, 0);
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, self.win_x, self.win_y, pw, ph, SWP_NOACTIVATE);
        }
        self.draw();
    }

    fn to_logical(&self, px: i32, py: i32) -> (f32, f32) {
        (px as f32 / self.scale, py as f32 / self.scale)
    }

    fn cursor_logical(&self) -> (f32, f32) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        self.to_logical(p.x - self.win_x, p.y - self.win_y)
    }

    fn island_rect(&self) -> R {
        R::new(CX - self.sw.v / 2.0 - EAR, 0.0, self.sw.v + 2.0 * EAR, self.sh.v + 2.0)
    }

    fn cursor_in_island(&self) -> bool {
        let (x, y) = self.cursor_logical();
        self.island_rect().contains(x, y)
    }

    // -------------------------------------------------------------- state machine

    fn desired_layer(&self) -> Layer {
        if !self.alerts.is_empty() {
            Layer::Alert
        } else if self.settings_open {
            Layer::Settings
        } else if self.expanded {
            Layer::Expanded
        } else if self.hud.is_some() {
            Layer::Hud
        } else if self.auto_hide && self.pill_lyric().is_none() {
            Layer::Ghost
        } else {
            Layer::Pill
        }
    }

    fn pill_lyric(&self) -> Option<String> {
        if self.s.minimal || !self.m.playing || !self.s.lyrics || !self.s.lyrics_view || !self.s.lyrics_in_pill {
            return None;
        }
        let (cur, _) = self.m.lyric_line()?;
        if cur.trim().is_empty() {
            None
        } else {
            Some(cur)
        }
    }

    fn pill_size(&self) -> (f32, f32) {
        if self.s.minimal {
            let w = if self.pomo.running { 150.0 } else { 128.0 };
            return (w, 26.0);
        }
        let w = if self.pomo.running {
            210.0
        } else if self.pill_lyric().is_some() {
            330.0
        } else {
            172.0
        };
        (w, 30.0)
    }

    fn layer_size(&self, l: Layer) -> (f32, f32, f32) {
        match l {
            Layer::Alert => (380.0, 78.0, 26.0),
            Layer::Expanded => (if self.wide_target() { 520.0 } else { 420.0 }, 140.0, 30.0),
            Layer::Hud => (hud::HUD_W, 32.0, 14.0),
            Layer::Ghost => (200.0, 8.0, 4.0),
            Layer::Settings => (settings::W, settings::H, 30.0),
            Layer::Pill => {
                let (w, h) = self.pill_size();
                (w, h, if self.s.minimal { 12.0 } else { 13.0 })
            }
        }
    }

    /// Music page in the wide lyrics view (shape target).
    fn wide_target(&self) -> bool {
        self.page == P_MUSIC && self.s.lyrics_view && self.m.has
    }

    fn content_changing(&self) -> bool {
        self.page != self.page_shown || self.wide_target() != self.wide_shown
    }

    fn target_shape(&self) -> (f32, f32, f32) {
        self.layer_size(self.desired_layer())
    }

    /// Re-targets the springs and redraws; starts the frame timer if needed.
    pub fn kick(&mut self) {
        let (w, h, r) = self.target_shape();
        self.sw.set(w);
        self.sh.set(h);
        self.sr.set(r);
        let want = self.desired_layer();
        if want != self.layer {
            self.fade.target = 0.0;
        }
        self.body.target = if want == Layer::Ghost { 0.0 } else { 1.0 };
        if self.content_changing() {
            self.page_fade.target = 0.0;
        }
        self.pet_fade.target = if self.pet_visible() { 1.0 } else { 0.0 };
        let moving = self.pet_fade.v != self.pet_fade.target || self.pet_x.moving() || self.pet_y.moving() || self.body.v != self.body.target || self.lyr.moving() || self.content_changing() || self.sw.moving() || self.sh.moving() || self.sr.moving() || self.fade.v != self.fade.target || self.page_fade.v != self.page_fade.target || want != self.layer;
        if moving && !self.anim_on {
            self.anim_on = true;
            self.last_anim = Instant::now();
            unsafe {
                SetTimer(self.hwnd, T_ANIM, 16, None);
            }
        }
        self.draw();
    }

    fn step_anim(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_anim).as_secs_f32().clamp(0.001, 0.05);
        self.last_anim = now;
        let mut moving = false;
        moving |= self.sw.step(dt);
        moving |= self.sh.step(dt);
        moving |= self.sr.step(dt);
        moving |= self.lyr.step(dt);
        let want = self.desired_layer();
        self.body.target = if want == Layer::Ghost { 0.0 } else { 1.0 };
        moving |= self.body.step(dt);
        self.pet_fade.target = if self.pet_visible() { 1.0 } else { 0.0 };
        moving |= self.pet_fade.step(dt);
        moving |= self.pet_x.step(dt);
        moving |= self.pet_y.step(dt);
        if want != self.layer {
            self.fade.target = 0.0;
            // swap once the old layer has faded and the shape is mostly there
            if self.fade.v <= 0.0 {
                self.layer = want;
                self.fade.target = 1.0;
            }
        } else {
            self.fade.target = 1.0;
        }
        moving |= self.fade.step(dt);
        if self.content_changing() {
            self.page_fade.target = 0.0;
            if self.page_fade.v <= 0.0 {
                self.page_shown = self.page;
                self.wide_shown = self.wide_target();
                self.page_fade.target = 1.0;
            }
        } else {
            self.page_fade.target = 1.0;
        }
        moving |= self.page_fade.step(dt);
        moving |= want != self.layer || self.content_changing();
        if !moving {
            self.anim_on = false;
            unsafe {
                let _ = KillTimer(self.hwnd, T_ANIM);
            }
        }
        self.draw();
    }

    pub fn expand(&mut self) {
        if self.expanded {
            return;
        }
        self.expanded = true;
        self.hud = None;
        self.kick();
    }

    pub fn collapse(&mut self) {
        if !self.expanded {
            return;
        }
        self.expanded = false;
        self.vol_mode = false;
        self.water_settings = false;
        self.todo_edit = false;
        self.kick();
    }

    pub fn toggle_open(&mut self) {
        if self.expanded {
            self.collapse();
        } else {
            self.expand();
        }
    }

    pub fn set_page(&mut self, p: usize) {
        let p = p % PAGES;
        if p == self.page {
            return;
        }
        self.page = p;
        self.vol_mode = false;
        self.water_settings = false;
        self.todo_edit = false;
        self.s.last_page = p;
        self.s.save();
        self.kick();
    }

    fn editing(&self) -> bool {
        self.add.is_some()
    }

    // -------------------------------------------------------------- alerts / hud

    pub fn push_alert(&mut self, a: Alert) {
        if self.s.sounds {
            sys::chime(if a.buttons.is_empty() { "soft" } else { "alarm" });
        }
        self.alerts.push(a);
        unsafe {
            SetTimer(self.hwnd, T_ALERT, 500, None);
        }
        self.kick();
    }

    pub fn alert(&mut self, icon: AlertIcon, color: Color, title: &str, sub: &str, buttons: Vec<(String, Act)>) {
        let secs = if buttons.is_empty() { 6 } else { 60 };
        self.push_alert(Alert {
            icon,
            color,
            title: title.to_string(),
            sub: sub.to_string(),
            buttons,
            until: Instant::now() + Duration::from_secs(secs),
        });
    }

    fn dismiss_alert(&mut self) {
        if !self.alerts.is_empty() {
            self.alerts.remove(0);
            if let Some(a) = self.alerts.first_mut() {
                let left = a.until.saturating_duration_since(Instant::now());
                if left < Duration::from_secs(3) {
                    a.until = Instant::now() + Duration::from_secs(4);
                }
            }
        }
        if self.alerts.is_empty() {
            unsafe {
                let _ = KillTimer(self.hwnd, T_ALERT);
            }
        }
        self.kick();
    }

    fn run_act(&mut self, act: Act) {
        self.dismiss_alert();
        match act {
            Act::Dismiss => {}
            Act::TodoLater(t) => self.todo_later(&t),
            Act::TodoStart(t) => self.todo_started(&t),
            Act::TodoDone(t) => self.todo_mark_done(&t),
            Act::WaterDrank => self.drink(),
            Act::WaterSnooze => self.water_snooze(),
            Act::WaterRefilled => self.water_refilled(),
        }
    }

    pub fn show_hud(&mut self, h: Hud, ms: u32) {
        if self.expanded || !self.alerts.is_empty() || self.hidden {
            return;
        }
        self.hud = Some(h);
        unsafe {
            SetTimer(self.hwnd, T_HUD, ms, None);
        }
        self.kick();
    }

    // -------------------------------------------------------------- drawing

    pub fn draw(&mut self) {
        let Some(mut g) = self.gfx.take() else { return };
        self.ensure_art(&g);
        for _ in 0..2 {
            let _ = g.begin();
            self.hits = self.paint(&mut g);
            if g.end() {
                break;
            }
            // render target recreated: device bitmaps are gone
            self.art = None;
            self.art_key = (0, 0);
            self.ensure_art(&g);
        }
        g.present(self.hwnd, self.win_x, self.win_y);
        self.gfx = Some(g);
    }

    fn ensure_art(&mut self, g: &Gfx) {
        let key = (self.m.art_id, g.generation);
        if key == self.art_key {
            return;
        }
        self.art_key = key;
        self.art = self.m.art.as_ref().and_then(|px| g.bitmap(px, media::ART, media::ART));
    }

    fn paint(&self, g: &mut Gfx) -> Vec<(R, Hit)> {
        let (w, h, r) = (self.sw.v, self.sh.v, self.sr.v);
        g.alpha = 1.0;
        // auto-hidden: practically invisible, but alpha > 0 still catches the hover
        g.notch(CX, w, h, r, EAR, gfx::BLACK.a(self.body.v.max(2.0 / 255.0)));
        if self.layer == Layer::Pill && self.pet_fade.v > 0.0 {
            g.alpha = crate::anim::ease_out(self.fade.v) * self.pet_fade.v;
            g.notch_glow(CX, w, h, r, EAR, gfx::Color::hex(0x2ec8ff), gfx::Color::hex(0x8a5cff));
            g.alpha = 1.0;
        }
        if let Some(tint) = self.art_tint {
            let fade = crate::anim::ease_out(self.fade.v);
            if self.layer == Layer::Expanded && self.wide_shown && self.page_shown == P_MUSIC {
                g.alpha = fade * crate::anim::ease_out(self.page_fade.v);
                g.notch_tint(CX, w, h, r, EAR, h * 0.3, tint.a(0.8));
            } else if self.layer == Layer::Pill && self.m.playing && self.pill_lyric().is_some() {
                g.alpha = fade;
                g.notch_tint(CX, w, h, r, EAR, 0.0, tint.a(0.7));
            }
            g.alpha = 1.0;
        }

        let lw = match self.layer {
            Layer::Expanded => {
                if self.wide_shown && self.page_shown == P_MUSIC {
                    520.0
                } else {
                    420.0
                }
            }
            l => self.layer_size(l).0,
        };
        let ox = (CX - lw / 2.0).round();
        let mut hits = Hits { ox, on: self.fade.v > 0.5 && self.layer == self.desired_layer(), v: Vec::new() };
        hits.add(R::new(CX - w / 2.0 - ox, 0.0, w, h), Hit::Island);

        g.translate(ox, 0.0);
        g.clip(R::new(CX - w / 2.0 - ox, 0.0, w, h));
        g.alpha = crate::anim::ease_out(self.fade.v);
        match self.layer {
            Layer::Pill => self.draw_pill(g, lw),
            Layer::Hud => {
                if let Some(hd) = &self.hud {
                    hud::draw_hud(g, hd);
                }
            }
            Layer::Alert => {
                if let Some(a) = self.alerts.first() {
                    hud::draw_alert(g, a, &mut hits);
                }
            }
            Layer::Expanded => self.draw_expanded(g, &mut hits),
            Layer::Settings => self.draw_settings(g, &mut hits),
            Layer::Ghost => {}
        }
        g.unclip();
        g.reset_transform();
        g.alpha = 1.0;
        hits.v
    }

    fn t(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    fn pet_visible(&self) -> bool {
        self.pet || self.pet_happy.map_or(false, |t| Instant::now() < t)
    }

    fn draw_pet(&self, g: &Gfx, w: f32, h: f32) {
        let cx = w / 2.0 + self.pet_x.v;
        let cy = h / 2.0 + self.pet_y.v;
        let happy = self.pet_happy.map_or(false, |t| Instant::now() < t);
        for dx in [-8.0f32, 8.0] {
            let ex = cx + dx;
            if happy {
                g.line(ex - 4.0, cy + 2.0, ex, cy - 2.5, 2.2, gfx::WHITE);
                g.line(ex, cy - 2.5, ex + 4.0, cy + 2.0, 2.2, gfx::WHITE);
            } else {
                let eh = if self.blink { 2.0 } else { 12.0 };
                g.rrect(R::new(ex - 4.5, cy - eh / 2.0 - 1.5, 9.0, eh + 3.0), 4.5, gfx::WHITE.a(0.22));
                g.rrect(R::new(ex - 3.0, cy - eh / 2.0, 6.0, eh), 3.0, gfx::WHITE);
            }
        }
    }

    fn draw_pill(&self, g: &mut Gfx, w: f32) {
        let (_, h) = self.pill_size();
        let cy = h / 2.0;
        if self.pet_fade.v > 0.0 {
            let base = g.alpha;
            g.alpha = base * self.pet_fade.v;
            self.draw_pet(g, w, h);
            g.alpha = base * (1.0 - self.pet_fade.v);
            if g.alpha <= 0.01 {
                g.alpha = base;
                return;
            }
        }
        // left: artwork while playing, otherwise the battery ring
        if self.m.playing && self.art.is_some() {
            if let Some(bmp) = &self.art {
                let s = h - 10.0;
                g.image(bmp, R::new(13.0, 5.0, s, s), 5.0);
            }
        } else {
            widgets::battery_ring(g, 21.0, cy, if self.s.minimal { 6.5 } else { 7.5 }, &self.batt);
        }
        // centre
        let centre = R::new(38.0, 0.0, w - 76.0, h);
        if self.pomo.running {
            let txt = self.pomo.text(&self.s);
            let tw = g.measure(&txt, 12.5, 700);
            let x = centre.cx() - (tw + 18.0) / 2.0;
            g.icon('\u{E916}', x + 6.0, cy, 11.0, gfx::RED);
            g.text(&txt, R::new(x + 18.0, 0.0, tw + 2.0, h), 12.5, 700, gfx::RED, Align::Left);
        } else if let Some(line) = self.pill_lyric() {
            g.text(&line, centre, 11.5, 600, gfx::WHITE, Align::Center);
        } else {
            let size = if self.s.minimal { 11.0 } else { 12.5 };
            g.text(&sys::time_string(), centre, size, 600, gfx::WHITE, Align::Center);
        }
        // right
        if self.m.playing {
            widgets::equalizer(g, w - 15.0 - widgets::EQ_W, cy, true, self.t(), gfx::WHITE);
        } else if self.s.mascot {
            widgets::mascot(g, w - 22.0, cy, self.blink);
        }
        if self.s.notif_badge && self.notif > 0 {
            g.circle(w - 9.0, 8.0, 2.6, gfx::RED);
        }
    }

    fn draw_expanded(&self, g: &mut Gfx, hits: &mut Hits) {
        let base = g.alpha;
        let page = self.page_shown;
        let wide = page == P_MUSIC && self.wide_shown;
        g.alpha = base * crate::anim::ease_out(self.page_fade.v);
        if wide {
            let on = hits.on;
            hits.on = on && self.page_fade.v > 0.5 && !self.content_changing();
            self.draw_music_wide(g, hits);
            hits.on = on;
            g.alpha = base;
            return;
        }
        g.alpha = base;
        self.draw_header(g, page);
        g.icon('\u{E713}', 400.0, 19.0, 12.0, gfx::GRAY);
        hits.add(R::new(386.0, 5.0, 28.0, 28.0), Hit::OpenSettings);
        g.alpha = base * crate::anim::ease_out(self.page_fade.v);
        let page_hits = self.page_fade.v > 0.5 && !self.content_changing();
        let on = hits.on;
        hits.on = on && page_hits;
        match page {
            P_MUSIC => self.draw_music(g, hits),
            P_POMO => self.draw_pomo(g, hits),
            P_WATER => self.draw_water(g, hits),
            P_SHELF => self.draw_shelf(g, hits),
            _ => self.draw_todo(g, hits),
        }
        hits.on = on;
        g.alpha = base;
        // arrows + dots
        if !self.editing() {
            for (x, glyph) in [(22.0, '\u{E76B}'), (398.0, '\u{E76C}')] {
                g.circle(x, 70.0, 10.0, gfx::CHIP_HI);
                g.icon(glyph, x, 70.0, 8.5, gfx::WHITE);
            }
            hits.add(R::new(8.0, 54.0, 28.0, 32.0), Hit::PrevPage);
            hits.add(R::new(384.0, 54.0, 28.0, 32.0), Hit::NextPage);
        }
        let n = PAGES as f32;
        let total = (n - 1.0) * 10.0 + 14.0;
        let mut x = 210.0 - total / 2.0;
        for i in 0..PAGES {
            let wdt = if i == self.page { 14.0 } else { 5.0 };
            g.rrect(R::new(x, 130.5, wdt, 5.0), 2.5, if i == self.page { gfx::WHITE } else { gfx::DIM });
            hits.add(R::new(x - 2.5, 126.0, wdt + 5.0, 14.0), Hit::Page(i));
            x += wdt + 5.0;
        }
    }

    fn draw_header(&self, g: &Gfx, page: usize) {
        let cy = 19.0;
        // left: title
        let (a, b, bc) = match page {
            P_MUSIC => ("", "", gfx::WHITE),
            P_POMO => ("Pomodoro", "", gfx::WHITE),
            P_WATER => ("Water ", "Reminder", gfx::BLUE),
            P_SHELF => ("Shelf", "", gfx::WHITE),
            _ => ("Todo", "List", gfx::GREEN),
        };
        let aw = g.measure(a, 13.0, 700);
        g.text(a, R::new(30.0, cy - 9.0, aw + 2.0, 18.0), 13.0, 700, gfx::WHITE, Align::Left);
        if !b.is_empty() {
            let bw = g.measure(b, 13.0, 700);
            g.text(b, R::new(30.0 + aw, cy - 9.0, bw + 2.0, 18.0), 13.0, 700, bc, Align::Left);
        }
        // centre: battery + date + time
        let date = sys::date_string();
        let time = sys::time_string();
        let dw = g.measure(&date, 10.5, 400);
        let tw = g.measure(&time, 10.5, 700);
        let total = 14.0 + 6.0 + dw + 5.0 + tw;
        let x = 216.0 - total / 2.0;
        widgets::battery_ring(g, x + 7.0, cy, 6.0, &self.batt);
        g.text(&date, R::new(x + 20.0, cy - 8.0, dw + 2.0, 16.0), 10.5, 400, gfx::GRAY, Align::Left);
        g.text(&time, R::new(x + 25.0 + dw, cy - 8.0, tw + 2.0, 16.0), 10.5, 700, gfx::WHITE, Align::Left);
        // right: equalizer + bell (shelf uses the corner for "Clear all")
        if page != P_SHELF {
            if self.m.playing {
                widgets::equalizer(g, 346.0, cy, true, self.t(), gfx::WHITE);
            }
            if self.s.notif_badge && self.notif > 0 {
                widgets::bell(g, 328.0, cy, self.notif);
            }
        }
    }

    pub fn cursor_hand(&self) -> bool {
        let (x, y) = self.cursor_logical();
        matches!(self.find_hit(x, y), Some(h) if h != Hit::Island && h != Hit::AlertBody)
    }

    fn find_hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| *h)
    }

    // -------------------------------------------------------------- mouse

    pub fn on_mouse_move(&mut self, px: i32, py: i32) -> Option<Deferred> {
        let (x, y) = self.to_logical(px, py);
        if !self.tracking {
            self.tracking = true;
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
        }
        unsafe {
            let _ = KillTimer(self.hwnd, T_CLOSE);
        }
        if !self.expanded && self.alerts.is_empty() && !self.s.open_click {
            unsafe {
                SetTimer(self.hwnd, T_HOVER, 120, None);
            }
        }
        match self.press.clone() {
            Some(Press::Seek(r)) => {
                if self.m.dur > 0.0 {
                    self.seek_preview = Some((((x - r.x) / r.w).clamp(0.0, 1.0) as f64) * self.m.dur);
                    self.draw();
                }
            }
            Some(Press::Volume(r)) => {
                self.audio.set_volume(((x - r.x) / r.w).clamp(0.0, 1.0));
                self.draw();
            }
            Some(Press::Tile { idx, x: x0, y: y0 }) => {
                if (x - x0).abs() > 5.0 || (y - y0).abs() > 5.0 {
                    self.press = None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    if let Some(path) = self.shelf_drag_path(idx) {
                        return Some(Deferred::DragOut(vec![path], idx));
                    }
                }
            }
            None => {}
        }
        None
    }

    pub fn on_mouse_leave(&mut self) {
        self.tracking = false;
        unsafe {
            let _ = KillTimer(self.hwnd, T_HOVER);
            SetTimer(self.hwnd, T_CLOSE, 250, None);
        }
    }

    pub fn on_lbutton_down(&mut self, px: i32, py: i32) -> Option<Deferred> {
        let (x, y) = self.to_logical(px, py);
        let hit = self.find_hit(x, y)?;
        match hit {
            Hit::Seek(r) => {
                self.press = Some(Press::Seek(r));
                unsafe { SetCapture(self.hwnd) };
                if self.m.dur > 0.0 {
                    self.seek_preview = Some((((x - r.x) / r.w).clamp(0.0, 1.0) as f64) * self.m.dur);
                }
                self.draw();
                None
            }
            Hit::Volume(r) => {
                self.press = Some(Press::Volume(r));
                unsafe { SetCapture(self.hwnd) };
                self.audio.set_volume(((x - r.x) / r.w).clamp(0.0, 1.0));
                self.draw();
                None
            }
            Hit::ShelfTile(idx) => {
                self.press = Some(Press::Tile { idx, x, y });
                unsafe { SetCapture(self.hwnd) };
                None
            }
            h => self.click(h),
        }
    }

    pub fn on_lbutton_up(&mut self, px: i32, py: i32) {
        let (x, _y) = self.to_logical(px, py);
        let p = self.press.take();
        unsafe {
            let _ = ReleaseCapture();
        }
        match p {
            Some(Press::Seek(r)) => {
                if self.m.dur > 0.0 {
                    let sec = (((x - r.x) / r.w).clamp(0.0, 1.0) as f64) * self.m.dur;
                    let _ = self.media_tx.send(media::Cmd::Seek(sec));
                    if let Ok(mut m) = self.media_shared.lock() {
                        m.pos = sec;
                        m.sampled = Some(Instant::now());
                    }
                    self.m.pos = sec;
                    self.m.sampled = Some(Instant::now());
                }
                self.seek_preview = None;
                self.draw();
            }
            Some(Press::Tile { idx, .. }) => self.shelf_click(idx),
            _ => {}
        }
    }

    pub fn on_rbutton_up(&mut self, px: i32, py: i32) -> Option<Deferred> {
        let (x, y) = self.to_logical(px, py);
        match self.find_hit(x, y) {
            Some(Hit::ShelfTile(i)) => {
                self.shelf_remove(i);
                None
            }
            Some(Hit::TodoCheck(i)) => {
                self.todo_delete(i);
                None
            }
            Some(_) => Some(Deferred::Menu),
            None => None,
        }
    }

    /// Moves the lyrics scroll target to the current line.
    fn follow_lyrics(&mut self) {
        let Some(i) = self.m.lyric_index() else { return };
        let t = i as f32;
        if self.lyr.target != t {
            if (t - self.lyr.v).abs() > 4.0 || !(self.expanded && self.wide_shown) {
                self.lyr.snap(t);
            } else {
                self.lyr.set(t);
                if !self.anim_on {
                    self.anim_on = true;
                    self.last_anim = Instant::now();
                    unsafe {
                        SetTimer(self.hwnd, T_ANIM, 16, None);
                    }
                }
            }
        }
    }

    pub fn on_wheel(&mut self, delta: i32) {
        if !self.expanded {
            return;
        }
        // over the arrows, the page dots or the header row: the wheel flips pages
        let (x, y) = self.cursor_logical();
        let over_nav = matches!(self.find_hit(x, y), Some(Hit::PrevPage | Hit::NextPage | Hit::Page(_)))
            || (y < 32.0 && self.island_rect().contains(x, y) && !self.wide_shown);
        if over_nav && !self.editing() {
            if (self.wheel_acc > 0) != (delta > 0) {
                self.wheel_acc = 0;
            }
            self.wheel_acc += delta;
            if self.wheel_acc.abs() >= 120 {
                let next = self.wheel_acc < 0;
                self.wheel_acc = 0;
                self.set_page(if next { self.page + 1 } else { self.page + PAGES - 1 });
            }
            return;
        }
        self.wheel_acc = 0;
        let up = delta > 0;
        match self.page {
            P_MUSIC => {
                if let Some((v, _)) = self.audio.volume() {
                    self.vol_mode = true;
                    self.audio.set_volume(v + if up { 0.02 } else { -0.02 });
                    self.draw();
                }
            }
            P_TODO => {
                if up {
                    self.todo_scroll = self.todo_scroll.saturating_sub(1);
                } else {
                    self.todo_scroll += 1;
                }
                self.draw();
            }
            P_SHELF => {
                if up {
                    self.shelf_scroll = self.shelf_scroll.saturating_sub(1);
                } else {
                    self.shelf_scroll += 1;
                }
                self.draw();
            }
            _ => {}
        }
    }

    fn click(&mut self, h: Hit) -> Option<Deferred> {
        match h {
            Hit::Island => {
                if !self.expanded && self.alerts.is_empty() {
                    unsafe {
                        let _ = KillTimer(self.hwnd, T_HOVER);
                    }
                    self.expand();
                }
            }
            Hit::PrevPage => self.set_page(self.page + PAGES - 1),
            Hit::NextPage => self.set_page(self.page + 1),
            Hit::Page(i) => self.set_page(i),
            Hit::AlertBody => self.dismiss_alert(),
            Hit::AlertBtn(i) => {
                let act = self.alerts.first().and_then(|a| a.buttons.get(i)).map(|b| b.1.clone());
                if let Some(a) = act {
                    self.run_act(a);
                }
            }
            Hit::Lyrics => {
                self.s.lyrics_view = !self.s.lyrics_view;
                self.vol_mode = false;
                self.lyr.snap(self.m.lyric_index().unwrap_or(0) as f32);
                self.s.save();
                self.kick();
            }
            Hit::Prev => self.media_cmd(media::Cmd::Prev),
            Hit::Toggle => self.media_cmd(media::Cmd::Toggle),
            Hit::Next => self.media_cmd(media::Cmd::Next),
            Hit::VolumeBtn => {
                self.vol_mode = !self.vol_mode;
                self.draw();
            }
            Hit::Seek(_) | Hit::Volume(_) | Hit::ShelfTile(_) => {}
            Hit::PomoPlay => self.pomo_toggle(),
            Hit::PomoReset => self.pomo_reset(),
            Hit::PomoShort => self.pomo_select(pomodoro::Phase::Short),
            Hit::PomoFocus => self.pomo_select(pomodoro::Phase::Focus),
            Hit::WaterDrink => self.drink(),
            Hit::WaterSettings => {
                self.water_settings = true;
                self.draw();
            }
            Hit::WaterDone => {
                self.water_settings = false;
                self.draw();
            }
            Hit::WaterReminders => {
                self.s.water_on = !self.s.water_on;
                self.s.save();
                self.draw();
            }
            Hit::WaterGoal(v) => {
                self.s.water_goal = v;
                self.s.save();
                self.draw();
            }
            Hit::WaterBottle(v) => {
                self.s.water_bottle = v;
                self.s.save();
                self.draw();
            }
            Hit::WaterFrom => {
                self.s.water_from = if self.s.water_from >= 12 { 5 } else { self.s.water_from + 1 };
                self.s.save();
                self.draw();
            }
            Hit::WaterUntil => {
                self.s.water_until = if self.s.water_until >= 24 { 17 } else { self.s.water_until + 1 };
                self.s.save();
                self.draw();
            }
            Hit::ShelfClear => self.shelf_clear(),
            Hit::TodoDay => {
                self.todo_day = (self.todo_day + 1) % 7;
                self.todo_scroll = 0;
                self.draw();
            }
            Hit::TodoAdd => return self.todo_begin_add(),
            Hit::TodoEdit => {
                self.todo_edit = !self.todo_edit;
                self.draw();
            }
            Hit::TodoCheck(i) => self.todo_toggle(i),
            Hit::TodoDelete(i) => self.todo_delete(i),
            Hit::TodoInput => return Some(Deferred::Focus(true)),
            Hit::TodoStartChip => self.todo_cycle_start(),
            Hit::TodoDurChip => self.todo_cycle_dur(),
            Hit::TodoCancel => return self.todo_cancel(),
            Hit::TodoSave => return self.todo_save(),
            Hit::OpenSettings => self.open_settings(),
            h @ (Hit::SetClose
            | Hit::SetQuit
            | Hit::SetFolder
            | Hit::SetCat(_)
            | Hit::SetToggle(_)
            | Hit::SetChoice(..)
            | Hit::SetStep(..)) => return self.settings_click(h),
        }
        None
    }

    fn media_cmd(&mut self, c: media::Cmd) {
        let _ = self.media_tx.send(c);
    }

    // -------------------------------------------------------------- keyboard (to-do input)

    pub fn on_char(&mut self, ch: u32) -> Option<Deferred> {
        let form = self.add.as_mut()?;
        match ch {
            8 => {
                form.text.pop();
            }
            13 | 27 => {}
            c if c >= 32 => {
                if let Some(c) = char::from_u32(c) {
                    if form.text.chars().count() < 80 {
                        form.text.push(c);
                    }
                }
            }
            _ => {}
        }
        self.caret_on = true;
        self.draw();
        None
    }

    pub fn on_keydown(&mut self, vk: u32) -> Option<Deferred> {
        self.add.as_ref()?;
        if vk == VK_RETURN.0 as u32 {
            self.todo_save()
        } else if vk == VK_ESCAPE.0 as u32 {
            self.todo_cancel()
        } else {
            None
        }
    }

    pub fn wants_activation(&self) -> bool {
        self.editing()
    }

    // -------------------------------------------------------------- app messages

    pub fn on_media(&mut self) {
        let snap = self.media_shared.lock().map(|m| m.clone()).unwrap_or_default();
        let was_playing = self.m.playing;
        self.m = snap;
        if let Some(g) = self.gfx.take() {
            self.ensure_art(&g);
            self.gfx = Some(g);
        }
        if self.m.art_id != self.tint_id {
            self.tint_id = self.m.art_id;
            self.art_tint = self.m.art.as_ref().and_then(|px| art_tint(px));
        }
        let track = format!("{}\u{1}{}", self.m.title, self.m.artist);
        if track != self.track {
            self.track = track;
            self.lyr.snap(0.0);
        }
        self.follow_lyrics();
        if self.m.playing != was_playing {
            unsafe {
                if self.m.playing {
                    SetTimer(self.hwnd, T_MEDIA, 80, None);
                } else {
                    let _ = KillTimer(self.hwnd, T_MEDIA);
                }
            }
        }
        self.kick();
    }

    pub fn on_keyhook(&mut self, vk: usize, on: bool) {
        let vk = vk as u16;
        if vk == VK_CAPITAL.0 {
            if self.s.hud_caps {
                self.show_hud(
                    Hud {
                        icon: '\u{E765}',
                        color: gfx::WHITE,
                        title: "Caps Lock".into(),
                        value: None,
                        right: if on { "ON".into() } else { "OFF".into() },
                        right_color: if on { gfx::GREEN } else { gfx::GRAY },
                    },
                    1400,
                );
            }
        } else if vk == VK_VOLUME_UP.0 || vk == VK_VOLUME_DOWN.0 || vk == VK_VOLUME_MUTE.0 {
            unsafe {
                SetTimer(self.hwnd, T_FOLLOW, 60, None);
            }
        } else {
            unsafe {
                SetTimer(self.hwnd, T_LANG, 150, None);
            }
        }
    }

    fn volume_hud(&mut self) {
        if !self.s.hud_volume {
            return;
        }
        if let Some((v, mute)) = self.audio.volume() {
            let pct = if mute { 0.0 } else { v };
            self.show_hud(
                Hud {
                    icon: if mute || v <= 0.001 { '\u{E74F}' } else { '\u{E767}' },
                    color: gfx::WHITE,
                    title: String::new(),
                    value: Some(pct),
                    right: format!("{}%", (pct * 100.0).round() as i32),
                    right_color: gfx::WHITE,
                },
                1400,
            );
        }
    }

    fn check_language(&mut self) {
        let l = sys::input_language();
        if !l.is_empty() && l != self.last_lang {
            self.last_lang = l.clone();
            if self.s.hud_language {
                let name = sys::input_language_name();
                self.show_hud(
                    Hud { icon: '\u{E774}', color: gfx::WHITE, title: if name.is_empty() { l.clone() } else { name }, value: None, right: l, right_color: gfx::WHITE },
                    1400,
                );
            }
        }
    }

    pub fn on_power(&mut self, ev: sys::PowerEvent) {
        self.batt = sys::battery();
        match ev {
            sys::PowerEvent::Brightness(b) => {
                let prev = self.brightness.replace(b);
                if prev.is_some() && prev != Some(b) && self.s.hud_brightness {
                    self.show_hud(
                        Hud {
                            icon: '\u{E706}',
                            color: gfx::WHITE,
                            title: String::new(),
                            value: Some(b as f32 / 100.0),
                            right: format!("{b}%"),
                            right_color: gfx::WHITE,
                        },
                        1400,
                    );
                }
            }
            sys::PowerEvent::Ac(on) => {
                let prev = self.ac.replace(on);
                if prev.is_some() && prev != Some(on) && self.s.popup_battery && self.batt.has {
                    let pct = format!("{}%", self.batt.percent);
                    self.show_hud(
                        Hud {
                            icon: if on { '\u{E945}' } else { '\u{E83F}' },
                            color: if on { gfx::GREEN } else { gfx::WHITE },
                            title: if on { "Charger connected".into() } else { "Charger unplugged".into() },
                            value: None,
                            right: pct,
                            right_color: gfx::GRAY,
                        },
                        1800,
                    );
                }
            }
            sys::PowerEvent::Battery(p) => {
                let prev = self.batt_pct.replace(p);
                let Some(prev) = prev else { return };
                if !self.s.popup_battery || prev == p {
                    self.kick();
                    return;
                }
                let on_ac = self.batt.ac;
                let msg = if !on_ac && p <= 10 && prev > 10 {
                    Some(("Battery critical", gfx::RED))
                } else if !on_ac && p <= 20 && prev > 20 {
                    Some(("Low battery", gfx::RED))
                } else if on_ac && p >= 100 && prev < 100 {
                    Some(("Fully charged", gfx::GREEN))
                } else {
                    None
                };
                if let Some((t, c)) = msg {
                    self.show_hud(
                        Hud { icon: '\u{E83F}', color: c, title: t.into(), value: None, right: format!("{p}%"), right_color: c },
                        1800,
                    );
                } else {
                    self.kick();
                }
            }
        }
    }

    pub fn on_net(&mut self) {
        unsafe {
            SetTimer(self.hwnd, T_NET, 1500, None);
        }
    }

    fn check_net(&mut self) {
        let n = sys::network();
        if Some(&n) == self.last_net.as_ref() || n == sys::Net::Unknown {
            return;
        }
        self.last_net = Some(n.clone());
        if !self.s.popup_wifi {
            return;
        }
        let (title, right, c, icon) = match &n {
            sys::Net::Connected(name, wifi) => (
                if *wifi { "Wi-Fi".to_string() } else { "Network".to_string() },
                if name.is_empty() { "Connected".to_string() } else { name.clone() },
                gfx::GREEN,
                if *wifi { '\u{E701}' } else { '\u{E839}' },
            ),
            sys::Net::Limited(_) => ("No internet access".into(), String::new(), gfx::ORANGE, '\u{E871}'),
            _ => ("Disconnected".into(), String::new(), gfx::RED, '\u{E871}'),
        };
        self.show_hud(Hud { icon, color: c, title, value: None, right, right_color: gfx::GRAY }, 3200);
    }

    pub fn on_notif(&mut self, n: u32) {
        if n != self.notif {
            self.notif = n;
            self.draw();
        }
    }

    pub fn on_drag(&mut self, kind: usize) {
        match kind {
            shelf::DRAG_ENTER => {
                self.drag_over = true;
                self.page = P_SHELF;
                self.expand();
                self.kick();
            }
            shelf::DRAG_DROP => {
                self.drag_over = false;
                let items: Vec<ShelfItem> = self.inbox.lock().map(|mut v| v.drain(..).collect()).unwrap_or_default();
                self.shelf_add(items);
                self.kick();
            }
            _ => {
                self.drag_over = false;
                self.draw();
                unsafe {
                    SetTimer(self.hwnd, T_CLOSE, 250, None);
                }
            }
        }
    }

    // -------------------------------------------------------------- timers

    pub fn on_timer(&mut self, id: usize) -> Option<Deferred> {
        match id {
            T_ANIM => self.step_anim(),
            T_CLOCK => {
                unsafe {
                    SetTimer(self.hwnd, T_CLOCK, now_ms_to_next_minute(), None);
                }
                self.on_minute();
            }
            T_BG => self.on_second(),
            T_HOVER => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_HOVER);
                }
                if self.cursor_in_island() && self.alerts.is_empty() && !self.s.open_click {
                    self.expand();
                }
            }
            T_CLOSE => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_CLOSE);
                }
                if !self.cursor_in_island() && !self.editing() && self.press.is_none() && !self.drag_over {
                    self.collapse();
                }
            }
            T_HUD => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_HUD);
                }
                self.hud = None;
                self.kick();
            }
            T_FOLLOW => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_FOLLOW);
                }
                self.audio.refresh_device();
                self.volume_hud();
            }
            T_LANG => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_LANG);
                }
                self.check_language();
            }
            T_NET => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_NET);
                }
                self.check_net();
            }
            T_POMO => self.pomo_tick(),
            T_TODO => self.check_todo(),
            T_WATER => self.check_water(),
            T_CARET => {
                self.caret_on = !self.caret_on;
                self.draw();
            }
            T_MEDIA => {
                if self.m.playing {
                    self.follow_lyrics();
                    // lyrics can change the pill width
                    let (w, _, _) = self.target_shape();
                    if (w - self.sw.target).abs() > 0.5 {
                        self.kick();
                    } else if !self.hidden {
                        self.draw();
                    }
                } else {
                    unsafe {
                        let _ = KillTimer(self.hwnd, T_MEDIA);
                    }
                }
            }
            T_ALERT => {
                if let Some(a) = self.alerts.first() {
                    if Instant::now() >= a.until {
                        self.dismiss_alert();
                    }
                } else {
                    unsafe {
                        let _ = KillTimer(self.hwnd, T_ALERT);
                    }
                }
            }
            T_COPIED => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_COPIED);
                }
                self.copied = None;
                self.draw();
            }
            T_PET => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_PET);
                }
                self.kick();
            }
            T_BLINK => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_BLINK);
                }
                self.blink = false;
                self.draw();
            }
            _ => {}
        }
        None
    }

    fn on_minute(&mut self) {
        let today = store::now().day;
        if self.water.day != today {
            self.water = store::load_water(today);
        }
        self.batt = sys::battery();
        self.draw();
    }

    fn on_second(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);
        // a maximised / full-screen app under the island: stay if only its
        // title bar is there, otherwise tuck away until hovered
        let ah = self.s.hide_fullscreen && {
            let cx = self.win_x + (CX * self.scale) as i32;
            let y = self.win_y + (10.0 * self.scale) as i32;
            let dx = (70.0 * self.scale) as i32;
            sys::cover_state(self.mon, &[(cx - dx, y), (cx, y), (cx + dx, y)]) == sys::Cover::Blocked
        };
        if ah != self.auto_hide {
            self.auto_hide = ah;
            self.kick();
        }
        if self.ticks % 5 == 0 {
            unsafe {
                let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        if self.ticks % 3 == 0 && self.audio.refresh_device() && self.audio.device_bt && self.s.popup_bluetooth {
            let name = self.audio.device_name.clone();
            self.show_hud(
                Hud { icon: '\u{E7F6}', color: gfx::WHITE, title: "Connected".into(), value: None, right: name, right_color: gfx::GRAY },
                1800,
            );
        }
        self.check_language();
        self.update_pet();
        if self.s.mascot && !self.expanded && !self.m.playing && self.ticks % 4 == 0 && self.layer == Layer::Pill {
            self.blink = true;
            self.draw();
            unsafe {
                SetTimer(self.hwnd, T_BLINK, 140, None);
            }
        }
    }

    fn rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// The idle pet: after 45 s without input the pill turns into a pair of
    /// wandering eyes; it waves (^^) when you come back.
    fn update_pet(&mut self) {
        let can = self.s.mascot
            && !self.m.playing
            && !self.pomo.running
            && !self.expanded
            && !self.settings_open
            && self.alerts.is_empty()
            && self.hud.is_none()
            && !self.auto_hide;
        let want = can && sys::idle_ms() >= 45_000;
        if want != self.pet {
            self.pet = want;
            if want {
                self.pet_x.snap(0.0);
                self.pet_y.snap(0.0);
            } else if can {
                self.pet_happy = Some(Instant::now() + Duration::from_millis(1600));
                unsafe {
                    SetTimer(self.hwnd, T_PET, 1650, None);
                }
            }
            self.kick();
            return;
        }
        if self.pet && self.ticks % 2 == 0 {
            let r = self.rand();
            if r % 10 == 0 {
                // a little happy moment
                self.pet_happy = Some(Instant::now() + Duration::from_millis(1200));
                unsafe {
                    SetTimer(self.hwnd, T_PET, 1250, None);
                }
            }
            let x = (self.rand() % 81) as f32 - 40.0;
            let y = (self.rand() % 7) as f32 - 3.0;
            self.pet_x.set(x);
            self.pet_y.set(y);
            self.kick();
        }
    }

    // -------------------------------------------------------------- tray menu commands

    pub fn settings(&self) -> &Settings {
        &self.s
    }

    /// Applies a settings change made from the tray menu.
    pub fn settings_changed(&mut self, layout: bool) {
        self.s.save();
        sys::set_autostart(self.s.autostart);
        self.apply_speed();
        if layout {
            self.place();
        }
        if self.pomo.running == false {
            self.pomo.reset(&self.s);
        }
        self.kick();
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        &mut self.s
    }

    pub fn shutdown(&mut self) {
        self.s.save();
        store::save_tasks(&self.tasks);
        store::save_water(&self.water);
        store::save_shelf(&self.shelf);
    }
}

/// Accent colour for the lyrics gradient: saturation-weighted average of the
/// artwork (premultiplied BGRA), normalised to a mid brightness.
fn art_tint(px: &[u8]) -> Option<Color> {
    let (mut r, mut g, mut b, mut wsum) = (0f32, 0f32, 0f32, 0f32);
    let mut sat_sum = 0f32;
    let mut n = 0f32;
    for p in px.chunks_exact(4) {
        let a = p[3] as f32 / 255.0;
        if a < 0.5 {
            continue;
        }
        let bb = p[0] as f32 / 255.0 / a;
        let gg = p[1] as f32 / 255.0 / a;
        let rr = p[2] as f32 / 255.0 / a;
        let mx = rr.max(gg).max(bb);
        let mn = rr.min(gg).min(bb);
        let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        let w = sat * sat * mx + 0.01;
        r += rr * w;
        g += gg * w;
        b += bb * w;
        wsum += w;
        sat_sum += sat;
        n += 1.0;
    }
    if wsum <= 0.0 || n == 0.0 {
        return None;
    }
    let (r, g, b) = (r / wsum, g / wsum, b / wsum);
    let mx = r.max(g).max(b).max(0.001);
    // greyscale covers get a dim neutral tint
    let target = if sat_sum / n < 0.08 { 0.28 } else { 0.62 };
    let k = target / mx;
    Some(Color((r * k).min(1.0), (g * k).min(1.0), (b * k).min(1.0), 1.0))
}
