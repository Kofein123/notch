//! System sensors that feed the HUD and popups.
//!
//! Recovered from Nusa's import table:
//! * volume / output device  -> MMDevice API (IAudioEndpointVolume)
//! * brightness, battery, AC -> RegisterPowerSettingNotification
//! * Caps Lock / volume keys -> WH_KEYBOARD_LL hook + GetKeyState
//! * input language          -> GetKeyboardLayout of the foreground thread
//! * Wi-Fi                   -> Windows.Networking.Connectivity.NetworkInformation
//! * full screen             -> SHQueryUserNotificationState + foreground rect
//! * notifications badge     -> Windows.UI.Notifications.Management.UserNotificationListener

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::core::*;
use windows::Networking::Connectivity::*;
use windows::UI::Notifications::Management::*;
use windows::UI::Notifications::NotificationKinds;
use windows::Win32::Devices::FunctionDiscovery::{PKEY_Device_EnumeratorName, PKEY_Device_FriendlyName};
use windows::Win32::Foundation::*;
use windows::Win32::Globalization::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Power::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::SystemServices::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::wide0;

// ------------------------------------------------------------------ audio

pub struct Audio {
    enumr: Option<IMMDeviceEnumerator>,
    vol: Option<IAudioEndpointVolume>,
    pub device_id: String,
    pub device_name: String,
    pub device_bt: bool,
}

impl Audio {
    pub fn new() -> Audio {
        let enumr: Option<IMMDeviceEnumerator> = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok() };
        let mut a = Audio { enumr, vol: None, device_id: String::new(), device_name: String::new(), device_bt: false };
        a.refresh_device();
        a
    }

    /// Re-reads the default output device. Returns true when it changed.
    pub fn refresh_device(&mut self) -> bool {
        unsafe {
            let Some(e) = &self.enumr else { return false };
            let Ok(dev) = e.GetDefaultAudioEndpoint(eRender, eMultimedia) else {
                self.vol = None;
                return false;
            };
            let id = dev.GetId().ok().map(|p| {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            });
            let id = id.unwrap_or_default();
            if id == self.device_id && self.vol.is_some() {
                return false;
            }
            let changed = !self.device_id.is_empty() && id != self.device_id;
            self.device_id = id;
            self.vol = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok();
            if let Ok(store) = dev.OpenPropertyStore(STGM_READ) {
                self.device_name = store.GetValue(&PKEY_Device_FriendlyName).map(|v| v.to_string()).unwrap_or_default();
                let en = store.GetValue(&PKEY_Device_EnumeratorName).map(|v| v.to_string()).unwrap_or_default();
                self.device_bt = en.to_ascii_uppercase().starts_with("BTH");
            }
            changed
        }
    }

    pub fn volume(&self) -> Option<(f32, bool)> {
        unsafe {
            let v = self.vol.as_ref()?;
            let level = v.GetMasterVolumeLevelScalar().ok()?;
            let mute = v.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            Some((level, mute))
        }
    }

    pub fn set_volume(&self, level: f32) {
        unsafe {
            if let Some(v) = &self.vol {
                let _ = v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
                if level > 0.0 {
                    let _ = v.SetMute(false, std::ptr::null());
                }
            }
        }
    }
}

// ------------------------------------------------------------------ power

pub fn register_power(hwnd: HWND) {
    unsafe {
        let h = HANDLE(hwnd.0);
        for g in [GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING, GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS] {
            let _ = RegisterPowerSettingNotification(h, &g, DEVICE_NOTIFY_WINDOW_HANDLE);
        }
    }
}

pub enum PowerEvent {
    Ac(bool),
    Battery(u32),
    Brightness(u32),
}

/// Decodes a WM_POWERBROADCAST / PBT_POWERSETTINGCHANGE lparam.
pub unsafe fn decode_power(lparam: LPARAM) -> Option<PowerEvent> {
    let p = lparam.0 as *const POWERBROADCAST_SETTING;
    if p.is_null() {
        return None;
    }
    let s = &*p;
    if s.DataLength < 4 {
        return None;
    }
    let data = std::ptr::read_unaligned(s.Data.as_ptr() as *const u32);
    if s.PowerSetting == GUID_ACDC_POWER_SOURCE {
        Some(PowerEvent::Ac(data == 0))
    } else if s.PowerSetting == GUID_BATTERY_PERCENTAGE_REMAINING {
        Some(PowerEvent::Battery(data))
    } else if s.PowerSetting == GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS {
        Some(PowerEvent::Brightness(data))
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Battery {
    pub has: bool,
    pub percent: u32,
    pub ac: bool,
    pub charging: bool,
}

pub fn battery() -> Battery {
    unsafe {
        let mut s = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut s).is_err() {
            return Battery::default();
        }
        let has = s.BatteryFlag != 128 && s.BatteryFlag != 255;
        Battery {
            has,
            percent: if s.BatteryLifePercent <= 100 { s.BatteryLifePercent as u32 } else { 100 },
            ac: s.ACLineStatus == 1,
            charging: (s.BatteryFlag & 8) != 0,
        }
    }
}

// ------------------------------------------------------------------ keyboard

static HOOK_HWND: AtomicIsize = AtomicIsize::new(0);
static HOOK_MSG: AtomicIsize = AtomicIsize::new(0);
static CAPS_DOWN: AtomicBool = AtomicBool::new(false);
pub static CAPS_ON: AtomicBool = AtomicBool::new(false);

pub fn install_keyboard_hook(hwnd: HWND, msg: u32) {
    HOOK_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
    HOOK_MSG.store(msg as isize, Ordering::Relaxed);
    unsafe {
        CAPS_ON.store((GetKeyState(VK_CAPITAL.0 as i32) & 1) != 0, Ordering::Relaxed);
        let _ = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), None, 0);
    }
}

unsafe extern "system" fn kb_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let down = wparam.0 as u32 == WM_KEYDOWN || wparam.0 as u32 == WM_SYSKEYDOWN;
        let up = wparam.0 as u32 == WM_KEYUP || wparam.0 as u32 == WM_SYSKEYUP;
        let vk = k.vkCode;
        let hwnd = HWND(HOOK_HWND.load(Ordering::Relaxed) as *mut _);
        let msg = HOOK_MSG.load(Ordering::Relaxed) as u32;
        if vk == VK_CAPITAL.0 as u32 {
            if down && !CAPS_DOWN.swap(true, Ordering::Relaxed) {
                let on = !CAPS_ON.load(Ordering::Relaxed);
                CAPS_ON.store(on, Ordering::Relaxed);
                let _ = PostMessageW(hwnd, msg, WPARAM(vk as usize), LPARAM(on as isize));
            } else if up {
                CAPS_DOWN.store(false, Ordering::Relaxed);
            }
        } else if down
            && (vk == VK_VOLUME_UP.0 as u32 || vk == VK_VOLUME_DOWN.0 as u32 || vk == VK_VOLUME_MUTE.0 as u32)
        {
            let _ = PostMessageW(hwnd, msg, WPARAM(vk as usize), LPARAM(0));
        } else if up && (vk == VK_LSHIFT.0 as u32 || vk == VK_RSHIFT.0 as u32 || vk == VK_SPACE.0 as u32 || vk == VK_LMENU.0 as u32) {
            // layout switch hotkeys: let the UI re-check the layout shortly
            let _ = PostMessageW(hwnd, msg, WPARAM(0), LPARAM(0));
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Two-letter code of the foreground window's keyboard layout ("EN", "RU", "KK"...).
pub fn input_language() -> String {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, None);
        let hkl = GetKeyboardLayout(tid);
        let langid = (hkl.0 as usize & 0xffff) as u32;
        let mut buf = [0u16; 85];
        let n = LCIDToLocaleName(langid, Some(&mut buf), 0);
        if n <= 1 {
            return String::new();
        }
        let name = String::from_utf16_lossy(&buf[..(n as usize - 1)]);
        name.split('-').next().unwrap_or("").to_ascii_uppercase()
    }
}

// ------------------------------------------------------------------ network

#[derive(Clone, Debug, PartialEq)]
pub enum Net {
    Unknown,
    Connected(String, bool), // profile name, is wifi
    Limited(String),
    Disconnected,
}

pub fn network() -> Net {
    let Ok(p) = NetworkInformation::GetInternetConnectionProfile() else { return Net::Disconnected };
    let name = p.ProfileName().map(|h| h.to_string()).unwrap_or_default();
    let wifi = p.IsWlanConnectionProfile().unwrap_or(false);
    match p.GetNetworkConnectivityLevel() {
        Ok(NetworkConnectivityLevel::InternetAccess) => Net::Connected(name, wifi),
        Ok(NetworkConnectivityLevel::None) => Net::Disconnected,
        Ok(_) => Net::Limited(name),
        Err(_) => Net::Unknown,
    }
}

pub fn watch_network(hwnd: HWND, msg: u32) {
    let raw = hwnd.0 as isize;
    let handler = NetworkStatusChangedEventHandler::new(move |_| {
        unsafe {
            let _ = PostMessageW(HWND(raw as *mut _), msg, WPARAM(0), LPARAM(0));
        }
        Ok(())
    });
    let _ = NetworkInformation::NetworkStatusChanged(&handler);
    std::mem::forget(handler);
}

// ------------------------------------------------------------------ notifications

/// Polls the notification center count on a background thread (needs the
/// user's consent the first time; silently gives up otherwise).
pub fn watch_notifications(hwnd: HWND, msg: u32) {
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(l) = UserNotificationListener::Current() else { return };
        let status = l.RequestAccessAsync().and_then(|o| o.get());
        if status != Ok(UserNotificationListenerAccessStatus::Allowed) {
            return;
        }
        let mut last = u32::MAX;
        loop {
            if let Ok(list) = l.GetNotificationsAsync(NotificationKinds::Toast).and_then(|o| o.get()) {
                let n = list.Size().unwrap_or(0);
                if n != last {
                    last = n;
                    let _ = PostMessageW(HWND(raw as *mut _), msg, WPARAM(n as usize), LPARAM(0));
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    });
}

// ------------------------------------------------------------------ full screen

pub fn fullscreen_on(mon: RECT) -> bool {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return true;
            }
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid() || !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() {
            return false;
        }
        let mut cls = [0u16; 64];
        let n = GetClassNameW(fg, &mut cls);
        let class = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "NotchWindow") {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        r.left <= mon.left && r.top <= mon.top && r.right >= mon.right && r.bottom >= mon.bottom
    }
}

/// What the foreground window does under the island.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    /// nothing big up there
    Free,
    /// a maximised / full-screen window whose title-bar drag area is under the
    /// island (Telegram, Explorer): the island can stay, it blocks nothing
    Caption,
    /// tabs, toolbars or a full-screen app under the island (Chrome, games):
    /// the island should get out of the way
    Blocked,
}

/// Checks the foreground window against the island area. `probes` are screen
/// points (physical px) along the top of the island.
pub fn cover_state(mon: RECT, probes: &[(i32, i32)]) -> Cover {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return Cover::Blocked;
            }
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid() || !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() {
            return Cover::Free;
        }
        let mut cls = [0u16; 64];
        let n = GetClassNameW(fg, &mut cls);
        let class = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "NotchWindow") {
            return Cover::Free;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return Cover::Free;
        }
        let min_x = probes.iter().map(|p| p.0).min().unwrap_or(0);
        let max_x = probes.iter().map(|p| p.0).max().unwrap_or(0);
        let touches_top = r.top <= mon.top + 2 && r.bottom > mon.top + 40;
        if !touches_top || r.left > min_x || r.right < max_x {
            return Cover::Free;
        }
        for &(x, y) in probes {
            let lp = LPARAM((((y as u16) as isize) << 16) | ((x as u16) as isize));
            let mut res = 0usize;
            let ok = SendMessageTimeoutW(fg, WM_NCHITTEST, WPARAM(0), lp, SMTO_ABORTIFHUNG, 60, Some(&mut res));
            if ok.0 == 0 || res as u32 != HTCAPTION {
                return Cover::Blocked;
            }
        }
        Cover::Caption
    }
}

// ------------------------------------------------------------------ monitors

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub handle: HMONITOR,
    pub rect: RECT,
    pub primary: bool,
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let v = &mut *(data.0 as *mut Vec<Monitor>);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(h, &mut mi).as_bool() {
            v.push(Monitor { handle: h, rect: mi.rcMonitor, primary: (mi.dwFlags & MONITORINFOF_PRIMARY) != 0 });
        }
        TRUE
    }
    let mut v: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    // primary first
    v.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
    v
}

pub fn monitor_scale(h: HMONITOR) -> f32 {
    unsafe {
        let (mut x, mut y) = (96u32, 96u32);
        if windows::Win32::UI::HiDpi::GetDpiForMonitor(h, windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() {
            x as f32 / 96.0
        } else {
            1.0
        }
    }
}

// ------------------------------------------------------------------ autostart

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let v = wide0(&format!("\"{exe}\""));
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                w!("Notch"),
                REG_SZ.0,
                Some(v.as_ptr() as *const _),
                (v.len() * 2) as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Notch"));
        }
    }
}

// ------------------------------------------------------------------ misc

pub fn chime(kind: &str) {
    unsafe {
        let alias = wide0(match kind {
            "alarm" => "SystemExclamation",
            "soft" => "SystemAsterisk",
            _ => "SystemNotification",
        });
        let _ = windows::Win32::Media::Audio::PlaySoundW(
            PCWSTR(alias.as_ptr()),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// "пн 26 окт. 2026" - follows the Windows regional format like Nusa.
pub fn date_string() -> String {
    unsafe {
        let mut buf = [0u16; 64];
        let n = GetDateFormatEx(PCWSTR::null(), ENUM_DATE_FORMATS_FLAGS(0), None, w!("ddd d MMM yyyy"), Some(&mut buf), PCWSTR::null());
        if n > 1 {
            String::from_utf16_lossy(&buf[..n as usize - 1])
        } else {
            String::new()
        }
    }
}

pub fn time_string() -> String {
    unsafe {
        let mut buf = [0u16; 32];
        let n = GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, None, PCWSTR::null(), Some(&mut buf));
        if n > 1 {
            String::from_utf16_lossy(&buf[..n as usize - 1])
        } else {
            String::new()
        }
    }
}

pub fn open_path(p: &str) {
    unsafe {
        let w = wide0(p);
        ShellExecuteW(None, w!("open"), PCWSTR(w.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn copy_text(hwnd: HWND, text: &str) {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    unsafe {
        let w = wide0(text);
        if OpenClipboard(hwnd).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(h);
                let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(h.0));
            }
        }
        let _ = CloseClipboard();
    }
}
