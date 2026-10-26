//! Browser downloads.
//!
//! The "Notch Downloads" extension (embedded below, unpacked into
//! `%APPDATA%\Notch\chrome-extension`) reports `chrome.downloads` events over
//! native messaging. The browser starts `Notch.exe chrome-extension://…` as the
//! native host; that process just relays each JSON message to the running
//! island with WM_COPYDATA.

use std::io::Read;
use std::time::Instant;

use serde_json::Value;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::wide0;
use crate::store;

pub const HOST: &str = "com.notch.host";
pub const EXT_ID: &str = "pbhgikiagdfeejahlihhdgcbhcaklkih";
pub const COPYDATA_TAG: usize = 0x4E44; // "ND"

const EXT_MANIFEST: &str = include_str!("../extension/manifest.json");
const EXT_JS: &str = include_str!("../extension/background.js");

// ------------------------------------------------------------------ native host process

pub fn is_host_invocation() -> bool {
    std::env::args().skip(1).any(|a| a.starts_with("chrome-extension://"))
}

/// Runs as the browser's native messaging host: reads length-prefixed JSON
/// from stdin and forwards it to the island until the browser closes the pipe.
pub fn run_host() {
    let mut stdin = std::io::stdin().lock();
    loop {
        let mut len = [0u8; 4];
        if stdin.read_exact(&mut len).is_err() {
            return;
        }
        let n = u32::from_le_bytes(len) as usize;
        if n == 0 || n > 4 << 20 {
            return;
        }
        let mut buf = vec![0u8; n];
        if stdin.read_exact(&mut buf).is_err() {
            return;
        }
        unsafe {
            let Ok(hwnd) = FindWindowW(w!("NotchWindow"), w!("Notch")) else { continue };
            let cds = COPYDATASTRUCT { dwData: COPYDATA_TAG, cbData: buf.len() as u32, lpData: buf.as_mut_ptr() as *mut _ };
            let mut res = 0usize;
            SendMessageTimeoutW(
                hwnd,
                WM_COPYDATA,
                WPARAM(0),
                LPARAM(&cds as *const _ as isize),
                SMTO_ABORTIFHUNG,
                1000,
                Some(&mut res),
            );
        }
    }
}

// ------------------------------------------------------------------ registration

pub fn extension_dir() -> std::path::PathBuf {
    store::data_dir().join("chrome-extension")
}

fn write_if_changed(path: &std::path::Path, content: &str) {
    if std::fs::read_to_string(path).ok().as_deref() != Some(content) {
        let _ = std::fs::write(path, content);
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Unpacks the extension and registers the native host for Chrome-family
/// browsers (per user, no admin rights needed). Cheap; runs at every start.
pub fn register() {
    let dir = extension_dir();
    let _ = std::fs::create_dir_all(&dir);
    write_if_changed(&dir.join("manifest.json"), EXT_MANIFEST);
    write_if_changed(&dir.join("background.js"), EXT_JS);

    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let host = format!(
        "{{\n  \"name\": \"{HOST}\",\n  \"description\": \"Notch island\",\n  \"path\": {},\n  \"type\": \"stdio\",\n  \"allowed_origins\": [\"chrome-extension://{EXT_ID}/\"]\n}}\n",
        json_str(&exe)
    );
    let manifest = store::data_dir().join("native-host.json");
    write_if_changed(&manifest, &host);

    let value = wide0(&manifest.to_string_lossy());
    for root in [
        "Software\\Google\\Chrome\\NativeMessagingHosts\\",
        "Software\\Microsoft\\Edge\\NativeMessagingHosts\\",
        "Software\\BraveSoftware\\Brave-Browser\\NativeMessagingHosts\\",
        "Software\\Yandex\\YandexBrowser\\NativeMessagingHosts\\",
        "Software\\Vivaldi\\NativeMessagingHosts\\",
        "Software\\Chromium\\NativeMessagingHosts\\",
    ] {
        let key = wide0(&format!("{root}{HOST}"));
        unsafe {
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR::null(),
                REG_SZ.0,
                Some(value.as_ptr() as *const _),
                (value.len() * 2) as u32,
            );
        }
    }
}

/// Opens the browser's extensions page and puts the unpacked folder path on
/// the clipboard for the "Load unpacked" dialog.
pub fn open_install(hwnd: HWND) {
    crate::sys::copy_text(hwnd, &extension_dir().to_string_lossy());
    unsafe {
        let r = ShellExecuteW(None, w!("open"), w!("chrome"), w!("chrome://extensions/"), PCWSTR::null(), SW_SHOWNORMAL);
        if (r.0 as isize) <= 32 {
            let r = ShellExecuteW(None, w!("open"), w!("msedge"), w!("edge://extensions/"), PCWSTR::null(), SW_SHOWNORMAL);
            if (r.0 as isize) <= 32 {
                crate::sys::open_path(&extension_dir().to_string_lossy());
            }
        }
    }
}

// ------------------------------------------------------------------ model

#[derive(Clone, Debug, PartialEq)]
pub enum DlState {
    Active,
    Paused,
    Done,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Dl {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub received: u64,
    pub total: u64,
    pub state: DlState,
    pub speed: f64,
    sample: (Instant, u64),
}

impl Dl {
    pub fn frac(&self) -> Option<f32> {
        if self.total > 0 {
            Some((self.received as f64 / self.total as f64).clamp(0.0, 1.0) as f32)
        } else {
            None
        }
    }
}

pub enum Event {
    None,
    Changed,
    Done(Dl),
    Failed(Dl),
}

#[derive(Default)]
pub struct Downloads {
    pub items: Vec<Dl>,
    pub connected: bool,
}

fn name_of(path: &str, url: &str) -> String {
    let p = std::path::Path::new(path);
    if let Some(n) = p.file_name() {
        return n.to_string_lossy().to_string();
    }
    let tail = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
    if tail.is_empty() {
        "Download".into()
    } else {
        tail.to_string()
    }
}

impl Downloads {
    pub fn active(&self) -> Vec<&Dl> {
        self.items.iter().filter(|d| matches!(d.state, DlState::Active | DlState::Paused)).collect()
    }

    fn upsert(&mut self, it: &Value) -> Option<usize> {
        let id = it.get("id")?.as_i64()?;
        let path = it.get("file").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let url = it.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let received = it.get("received").and_then(|v| v.as_u64()).unwrap_or(0);
        let total = it.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
        let paused = it.get("paused").and_then(|v| v.as_bool()).unwrap_or(false);
        let idx = match self.items.iter().position(|d| d.id == id) {
            Some(i) => i,
            None => {
                self.items.push(Dl {
                    id,
                    path: String::new(),
                    name: String::new(),
                    received: 0,
                    total: 0,
                    state: DlState::Active,
                    speed: 0.0,
                    sample: (Instant::now(), 0),
                });
                self.items.len() - 1
            }
        };
        let d = &mut self.items[idx];
        if !path.is_empty() || d.name.is_empty() {
            d.path = path.clone();
            d.name = name_of(&path, url);
        }
        let now = Instant::now();
        let dt = (now - d.sample.0).as_secs_f64();
        if dt >= 0.5 && received >= d.sample.1 {
            let inst = (received - d.sample.1) as f64 / dt;
            d.speed = if d.speed == 0.0 { inst } else { d.speed * 0.6 + inst * 0.4 };
            d.sample = (now, received);
        }
        d.received = received;
        if total > 0 {
            d.total = total;
        }
        if matches!(d.state, DlState::Active | DlState::Paused) {
            d.state = if paused { DlState::Paused } else { DlState::Active };
        }
        Some(idx)
    }

    /// Applies one message from the extension.
    pub fn on_message(&mut self, raw: &str) -> Event {
        let Ok(v) = serde_json::from_str::<Value>(raw) else { return Event::None };
        self.connected = true;
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "hello" => Event::Changed,
            "created" | "changed" => {
                if let Some(it) = v.get("item") {
                    self.upsert(it);
                }
                Event::Changed
            }
            "progress" => {
                let items = v.get("items").and_then(|a| a.as_array()).cloned().unwrap_or_default();
                let ids: Vec<i64> = items.iter().filter_map(|it| it.get("id").and_then(|x| x.as_i64())).collect();
                for it in &items {
                    self.upsert(it);
                }
                // anything Chrome no longer reports as in progress and we never saw finish
                for d in self.items.iter_mut() {
                    if matches!(d.state, DlState::Active | DlState::Paused) && !ids.contains(&d.id) && d.sample.0.elapsed().as_secs() > 5 {
                        d.state = DlState::Failed(String::new());
                    }
                }
                self.items.retain(|d| !matches!(d.state, DlState::Failed(ref e) if e.is_empty()));
                Event::Changed
            }
            "complete" => {
                let Some(idx) = v.get("item").and_then(|it| self.upsert(it)) else { return Event::None };
                let mut d = self.items.remove(idx);
                d.state = DlState::Done;
                if d.total > 0 {
                    d.received = d.total;
                }
                Event::Done(d)
            }
            "interrupted" => {
                let Some(it) = v.get("item") else { return Event::None };
                let err = it.get("error").and_then(|e| e.as_str()).unwrap_or("").to_string();
                let Some(idx) = self.upsert(it) else { return Event::None };
                let mut d = self.items.remove(idx);
                if err == "USER_CANCELED" || err.is_empty() {
                    return Event::Changed;
                }
                d.state = DlState::Failed(err);
                Event::Failed(d)
            }
            "erased" => {
                if let Some(id) = v.get("id").and_then(|x| x.as_i64()) {
                    self.items.retain(|d| d.id != id);
                }
                Event::Changed
            }
            _ => Event::None,
        }
    }
}

pub fn fmt_size(b: u64) -> String {
    let b = b as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", b / 1024.0 / 1024.0 / 1024.0)
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{} B", b as u64)
    }
}

pub fn failure_text(code: &str) -> String {
    match code {
        c if c.starts_with("NETWORK") => "Network error".into(),
        c if c.starts_with("FILE_NO_SPACE") => "Disk is full".into(),
        c if c.starts_with("FILE_ACCESS_DENIED") => "No access to the folder".into(),
        c if c.starts_with("FILE_VIRUS") || c.starts_with("FILE_BLOCKED") || c.starts_with("FILE_SECURITY") => {
            "Blocked by the browser".into()
        }
        c if c.starts_with("SERVER") => "Server error".into(),
        "" => "Failed".into(),
        c => c.replace('_', " ").to_lowercase(),
    }
}

/// Explorer window with the file selected.
pub fn show_in_folder(path: &str) {
    unsafe {
        let args = wide0(&format!("/select,\"{path}\""));
        ShellExecuteW(None, w!("open"), w!("explorer.exe"), PCWSTR(args.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
    }
}
