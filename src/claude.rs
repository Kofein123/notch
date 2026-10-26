//! Claude Desktop status.
//!
//! The app has no API for this, so a background thread looks at its window
//! through UI Automation: while Claude is writing an answer the chat shows a
//! "Stop response" button. Present -> working, gone -> idle. Two idle polls in
//! a row are needed before "working" ends, so a redraw can't fake a finish.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ST_CLOSED: usize = 0;
pub const ST_IDLE: usize = 1;
pub const ST_WORKING: usize = 2;

fn exe_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) else { return String::new() };
        let mut buf = [0u16; 520];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..n as usize]);
        full.rsplit('\\').next().unwrap_or("").to_ascii_lowercase()
    }
}

/// The biggest visible top-level window of claude.exe.
pub fn find_window() -> Option<HWND> {
    struct Acc {
        best: Option<(HWND, i64)>,
    }
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let acc = &mut *(lp.0 as *mut Acc);
        if !IsWindowVisible(h).as_bool() || GetWindowTextLengthW(h) == 0 {
            return TRUE;
        }
        let mut cls = [0u16; 64];
        let n = GetClassNameW(h, &mut cls);
        if String::from_utf16_lossy(&cls[..n.max(0) as usize]) != "Chrome_WidgetWin_1" {
            return TRUE;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid as *mut u32));
        if exe_name(pid) != "claude.exe" {
            return TRUE;
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
        if acc.best.map_or(true, |(_, a)| area > a) {
            acc.best = Some((h, area));
        }
        TRUE
    }
    let mut acc = Acc { best: None };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut acc as *mut _ as isize));
    }
    acc.best.map(|b| b.0)
}

/// True when the foreground window belongs to Claude.
pub fn is_foreground() -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid as *mut u32));
        exe_name(pid) == "claude.exe"
    }
}

/// Brings Claude to the front (or starts it through its URL protocol).
pub fn open() {
    unsafe {
        if let Some(h) = find_window() {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            if SetForegroundWindow(h).as_bool() {
                return;
            }
        }
        windows::Win32::UI::Shell::ShellExecuteW(None, w!("open"), w!("claude://"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

unsafe fn condition(uia: &IUIAutomation) -> Result<IUIAutomationCondition> {
    let button = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0))?;
    let flags = PropertyConditionFlags(PropertyConditionFlags_IgnoreCase.0 | PropertyConditionFlags_MatchSubstring.0);
    let names = [
        uia.CreatePropertyConditionEx(UIA_NamePropertyId, &VARIANT::from("Stop response"), flags)?,
        uia.CreatePropertyConditionEx(UIA_NamePropertyId, &VARIANT::from("Stop generating"), flags)?,
        uia.CreatePropertyConditionEx(UIA_NamePropertyId, &VARIANT::from("Остановить"), flags)?,
        uia.CreatePropertyCondition(UIA_NamePropertyId, &VARIANT::from("Stop"))?,
    ];
    let any = uia.CreateOrConditionFromNativeArray(&names.map(Some))?;
    uia.CreateAndCondition(&button, &any)
}

pub fn spawn(hwnd: HWND, msg: u32, enabled: Arc<AtomicBool>) {
    let raw = hwnd.0 as isize;
    std::thread::Builder::new()
        .name("claude".into())
        .spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(uia) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else { return };
            let Ok(cond) = condition(&uia) else { return };
            let mut last = usize::MAX;
            let mut idle_streak = 0;
            loop {
                if !enabled.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                let state = match find_window() {
                    None => ST_CLOSED,
                    Some(w) => {
                        let working = uia
                            .ElementFromHandle(w)
                            .and_then(|e| e.FindFirst(TreeScope_Descendants, &cond))
                            .is_ok();
                        if working {
                            idle_streak = 0;
                            ST_WORKING
                        } else {
                            idle_streak += 1;
                            if last == ST_WORKING && idle_streak < 2 {
                                ST_WORKING
                            } else {
                                ST_IDLE
                            }
                        }
                    }
                };
                if state != last {
                    last = state;
                    let _ = PostMessageW(HWND(raw as *mut _), msg, WPARAM(state), LPARAM(0));
                }
                std::thread::sleep(Duration::from_millis(if state == ST_CLOSED { 3000 } else { 1000 }));
            }
        })
        .ok();
}
