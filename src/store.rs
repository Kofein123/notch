//! Persistence: plain `key=value` / line-based text files in `%APPDATA%\Notch`,
//! the same approach Nusa uses (settings.txt, todo.txt, water.txt, shelf.txt).

use std::collections::HashMap;
use std::path::PathBuf;

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("Notch");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn read_kv(name: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Ok(s) = std::fs::read_to_string(data_dir().join(name)) {
        for line in s.lines() {
            if let Some((k, v)) = line.split_once('=') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    m
}

fn write_atomic(name: &str, content: &str) {
    let dir = data_dir();
    let tmp = dir.join(format!("{name}.tmp"));
    if std::fs::write(&tmp, content).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(name));
    }
}

// ------------------------------------------------------------------ time

#[derive(Clone, Copy, Debug)]
pub struct Now {
    pub day: i32,     // days since 1970-01-01 (local)
    pub minute: i32,  // minutes since local midnight
    pub second: i32,
    pub weekday: i32, // 0 = Sunday
    pub st: SYSTEMTIME,
}

pub fn now() -> Now {
    let st = unsafe { GetLocalTime() };
    let day = days_from_civil(st.wYear as i32, st.wMonth as i32, st.wDay as i32);
    Now {
        day,
        minute: st.wHour as i32 * 60 + st.wMinute as i32,
        second: st.wSecond as i32,
        weekday: st.wDayOfWeek as i32,
        st,
    }
}

pub fn days_from_civil(y: i32, m: i32, d: i32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i32) -> (i32, i32, i32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn hhmm(min: i32) -> String {
    let m = min.rem_euclid(24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

// ------------------------------------------------------------------ settings

#[derive(Clone, Debug)]
pub struct Settings {
    pub autostart: bool,
    pub size: u32, // 0 = auto, else percent
    pub minimal: bool,
    pub mascot: bool,
    pub sounds: bool,
    pub hide_fullscreen: bool,
    pub display: usize,
    pub open_click: bool,
    pub fast_animations: bool,
    pub lyrics: bool,
    pub lyrics_in_pill: bool,
    pub todo_start: bool,
    pub todo_end: bool,
    pub snooze: u32,
    pub week_monday: bool,
    pub focus: u32,
    pub short_break: u32,
    pub long_break: u32,
    pub water_on: bool,
    pub water_goal: u32,   // ml
    pub water_bottle: u32, // ml
    pub water_glass: u32,  // ml per "Drink"
    pub water_from: i32,   // hour
    pub water_until: i32,  // hour
    pub hud_volume: bool,
    pub hud_brightness: bool,
    pub hud_caps: bool,
    pub hud_language: bool,
    pub popup_battery: bool,
    pub popup_wifi: bool,
    pub popup_bluetooth: bool,
    pub notif_badge: bool,
    pub shelf_clear: bool,
    pub last_page: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autostart: true,
            size: 0,
            minimal: false,
            mascot: true,
            sounds: true,
            hide_fullscreen: true,
            display: 0,
            open_click: false,
            fast_animations: false,
            lyrics: true,
            lyrics_in_pill: true,
            todo_start: true,
            todo_end: true,
            snooze: 10,
            week_monday: true,
            focus: 25,
            short_break: 5,
            long_break: 15,
            water_on: true,
            water_goal: 1500,
            water_bottle: 500,
            water_glass: 250,
            water_from: 8,
            water_until: 22,
            hud_volume: true,
            hud_brightness: true,
            hud_caps: true,
            hud_language: true,
            popup_battery: true,
            popup_wifi: true,
            popup_bluetooth: true,
            notif_badge: true,
            shelf_clear: false,
            last_page: 0,
        }
    }
}

macro_rules! kv_fields {
    ($($name:ident),* $(,)?) => {
        impl Settings {
            pub fn load() -> Settings {
                let m = read_kv("settings.txt");
                let mut s = Settings::default();
                $( if let Some(v) = m.get(stringify!($name)) { if let Ok(p) = v.parse() { s.$name = p; } } )*
                s
            }
            pub fn save(&self) {
                let mut out = String::new();
                $( out.push_str(&format!("{}={}\n", stringify!($name), self.$name)); )*
                write_atomic("settings.txt", &out);
            }
        }
    };
}

kv_fields!(
    autostart, size, minimal, mascot, sounds, hide_fullscreen, display, open_click, fast_animations,
    lyrics, lyrics_in_pill, todo_start, todo_end, snooze, week_monday, focus, short_break, long_break,
    water_on, water_goal, water_bottle, water_glass, water_from, water_until, hud_volume, hud_brightness,
    hud_caps, hud_language, popup_battery, popup_wifi, popup_bluetooth, notif_badge, shelf_clear, last_page,
);

// ------------------------------------------------------------------ to-do

#[derive(Clone, Debug)]
pub struct Task {
    pub day: i32,
    pub start: Option<i32>, // minute of day
    pub dur: i32,           // minutes
    pub done: bool,
    pub fired_start: bool,
    pub fired_end: bool,
    pub snooze_until: Option<i32>, // minute of day
    pub title: String,
}

pub fn load_tasks() -> Vec<Task> {
    let mut v = Vec::new();
    if let Ok(s) = std::fs::read_to_string(data_dir().join("todo.txt")) {
        for line in s.lines() {
            let p: Vec<&str> = line.splitn(7, '|').collect();
            if p.len() < 7 {
                continue;
            }
            let start: i32 = p[1].parse().unwrap_or(-1);
            v.push(Task {
                day: p[0].parse().unwrap_or(0),
                start: if start >= 0 { Some(start) } else { None },
                dur: p[2].parse().unwrap_or(30),
                done: p[3] == "1",
                fired_start: p[4] == "1",
                fired_end: p[5] == "1",
                snooze_until: None,
                title: p[6].replace("\\n", " "),
            });
        }
    }
    v
}

pub fn save_tasks(tasks: &[Task]) {
    let mut out = String::new();
    for t in tasks {
        out.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|{}\n",
            t.day,
            t.start.unwrap_or(-1),
            t.dur,
            t.done as u8,
            t.fired_start as u8,
            t.fired_end as u8,
            t.title.replace('\n', " ").replace('|', "/")
        ));
    }
    write_atomic("todo.txt", &out);
}

// ------------------------------------------------------------------ water

#[derive(Clone, Debug, Default)]
pub struct Water {
    pub day: i32,
    pub taken: u32,
    pub since_refill: u32,
    pub fired_slot: i32,
    pub snooze_until: i32,
    /// per-hour intake for the little chart (24 buckets, ml)
    pub hours: Vec<u32>,
}

pub fn load_water(today: i32) -> Water {
    let m = read_kv("water.txt");
    let mut w = Water { day: today, fired_slot: -1, hours: vec![0; 24], ..Default::default() };
    let day: i32 = m.get("day").and_then(|v| v.parse().ok()).unwrap_or(today);
    if day == today {
        w.taken = m.get("taken").and_then(|v| v.parse().ok()).unwrap_or(0);
        w.fired_slot = m.get("fired").and_then(|v| v.parse().ok()).unwrap_or(-1);
        if let Some(h) = m.get("hours") {
            let parsed: Vec<u32> = h.split(',').filter_map(|x| x.parse().ok()).collect();
            if parsed.len() == 24 {
                w.hours = parsed;
            }
        }
    }
    w.since_refill = m.get("bottle").and_then(|v| v.parse().ok()).unwrap_or(0);
    w
}

pub fn save_water(w: &Water) {
    let hours: Vec<String> = w.hours.iter().map(|x| x.to_string()).collect();
    write_atomic(
        "water.txt",
        &format!("day={}\ntaken={}\nbottle={}\nfired={}\nhours={}\n", w.day, w.taken, w.since_refill, w.fired_slot, hours.join(",")),
    );
}

// ------------------------------------------------------------------ shelf

#[derive(Clone, Debug)]
pub enum ShelfItem {
    File(String),
    Text(String),
}

impl ShelfItem {
    pub fn label(&self) -> String {
        match self {
            ShelfItem::File(p) => std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.clone()),
            ShelfItem::Text(t) => t.chars().take(40).collect::<String>().replace(['\n', '\r'], " "),
        }
    }
}

pub fn load_shelf() -> Vec<ShelfItem> {
    let mut v = Vec::new();
    if let Ok(s) = std::fs::read_to_string(data_dir().join("shelf.txt")) {
        for line in s.lines() {
            if let Some(t) = line.strip_prefix("text:") {
                v.push(ShelfItem::Text(t.replace("\\n", "\n")));
            } else if let Some(p) = line.strip_prefix("file:") {
                if std::path::Path::new(p).exists() {
                    v.push(ShelfItem::File(p.to_string()));
                }
            }
        }
    }
    v
}

pub fn save_shelf(items: &[ShelfItem]) {
    let mut out = String::new();
    for it in items {
        match it {
            ShelfItem::File(p) => out.push_str(&format!("file:{p}\n")),
            ShelfItem::Text(t) => out.push_str(&format!("text:{}\n", t.replace('\r', "").replace('\n', "\\n"))),
        }
    }
    write_atomic("shelf.txt", &out);
}
