#![windows_subsystem = "windows"]
#![allow(dead_code)]

//! Notch: a living notch (dynamic island) for Windows 10/11.

mod anim;
mod claude;
mod downloads;
mod gfx;
mod media;
mod shelf;
mod store;
mod sys;
mod tg;
mod ui;

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use store::Settings;
use ui::{App, Deferred};

const WM_MOUSELEAVE: u32 = 0x02A3;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static TRAY_ICON: AtomicIsize = AtomicIsize::new(0);

/// Runs `f` with the app if it isn't already borrowed. Calls such as
/// SetWindowPos, DoDragDrop and TrackPopupMenu re-enter the window
/// procedure; those nested messages simply fall through to DefWindowProc.
fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|cell| {
        let mut b = cell.try_borrow_mut().ok()?;
        let app = b.as_mut()?;
        Some(f(app))
    })
}

// ------------------------------------------------------------------ tray

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: ui::WM_TRAY,
        hIcon: HICON(TRAY_ICON.load(Ordering::Relaxed) as *mut _),
        ..Default::default()
    };
    let tip = gfx::wide("Notch");
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    nid
}

fn tray_add(hwnd: HWND) {
    let nid = tray_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn tray_remove(hwnd: HWND) {
    let nid = tray_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

// ------------------------------------------------------------------ menu

use ui::settings::{toggle_field, TOGGLE_LABELS};

const ID_TOGGLE: usize = 1000;
const ID_SIZE: usize = 2000;
const ID_SCREEN: usize = 2100;
const ID_FOCUS: usize = 2200;
const ID_SHORT: usize = 2300;
const ID_LONG: usize = 2400;
const ID_GOAL: usize = 2500; // + liters * 10
const ID_GLASS: usize = 2600; // + ml / 10
const ID_SNOOZE: usize = 2700;
const ID_FOLDER: usize = 3000;
const ID_QUIT: usize = 3001;
const ID_SETTINGS: usize = 3002;

unsafe fn item(m: HMENU, id: usize, label: &str, checked: bool) {
    let w = gfx::wide0(label);
    let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
    let _ = AppendMenuW(m, flags, id, PCWSTR(w.as_ptr()));
}

unsafe fn toggle(m: HMENU, s: &Settings, i: usize) {
    let mut c = s.clone();
    let on = toggle_field(&mut c, i).map(|b| *b).unwrap_or(false);
    item(m, ID_TOGGLE + i, TOGGLE_LABELS[i], on);
}

unsafe fn sep(m: HMENU) {
    let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
}

unsafe fn grayed(m: HMENU, label: &str) {
    let w = gfx::wide0(label);
    let _ = AppendMenuW(m, MF_STRING | MF_GRAYED, 0, PCWSTR(w.as_ptr()));
}

unsafe fn submenu(parent: HMENU, label: &str) -> HMENU {
    let sub = CreatePopupMenu().unwrap_or_default();
    let w = gfx::wide0(label);
    let _ = AppendMenuW(parent, MF_POPUP, sub.0 as usize, PCWSTR(w.as_ptr()));
    sub
}

unsafe fn build_menu(s: &Settings) -> Option<HMENU> {
    let m = CreatePopupMenu().ok()?;
    grayed(m, concat!("Notch ", env!("CARGO_PKG_VERSION")));
    item(m, ID_SETTINGS, "Settings…", false);
    sep(m);
    for i in 0..=6 {
        toggle(m, s, i);
    }
    sep(m);

    let size = submenu(m, "Size");
    for (pct, label) in [(0u32, "Auto"), (90, "90%"), (100, "100%"), (110, "110%"), (125, "125%"), (150, "150%")] {
        item(size, ID_SIZE + pct as usize, label, s.size == pct);
    }
    let screen = submenu(m, "Screen");
    for (i, mon) in sys::monitors().iter().enumerate() {
        let label = if mon.primary { format!("Screen {} (primary)", i + 1) } else { format!("Screen {}", i + 1) };
        item(screen, ID_SCREEN + i, &label, s.display == i);
    }
    sep(m);

    let music = submenu(m, "Music");
    toggle(music, s, 7);
    toggle(music, s, 21);
    toggle(music, s, 8);

    let pomo = submenu(m, "Pomodoro");
    for v in [15u32, 20, 25, 30, 45, 50, 60] {
        item(pomo, ID_FOCUS + v as usize, &format!("Focus {v} min"), s.focus == v);
    }
    sep(pomo);
    for v in [3u32, 5, 10] {
        item(pomo, ID_SHORT + v as usize, &format!("Short break {v} min"), s.short_break == v);
    }
    sep(pomo);
    for v in [10u32, 15, 20, 30] {
        item(pomo, ID_LONG + v as usize, &format!("Long break {v} min"), s.long_break == v);
    }

    let water = submenu(m, "Water");
    toggle(water, s, 9);
    sep(water);
    for l10 in [15u32, 20, 25, 30] {
        item(water, ID_GOAL + l10 as usize, &format!("Goal {:.1} L", l10 as f32 / 10.0), s.water_goal == l10 * 100);
    }
    sep(water);
    for ml in [150u32, 200, 250, 300, 500] {
        item(water, ID_GLASS + (ml / 10) as usize, &format!("Glass {ml} ml"), s.water_glass == ml);
    }

    let todo = submenu(m, "To-do");
    toggle(todo, s, 10);
    toggle(todo, s, 11);
    sep(todo);
    for v in [5u32, 10, 15, 30] {
        item(todo, ID_SNOOZE + v as usize, &format!("Later = {v} min"), s.snooze == v);
    }

    let hud = submenu(m, "HUD");
    for i in 12..=15 {
        toggle(hud, s, i);
    }
    sep(hud);
    for i in 16..=19 {
        toggle(hud, s, i);
    }

    let shelf = submenu(m, "Shelf");
    toggle(shelf, s, 20);

    sep(m);
    item(m, ID_FOLDER, "Open data folder", false);
    item(m, ID_QUIT, "Quit", false);
    Some(m)
}

/// Returns true when the user chose Quit.
fn apply_command(a: &mut App, id: usize) -> bool {
    let mut layout = false;
    if id == ID_SETTINGS {
        a.open_settings();
        return false;
    }
    {
        let s = a.settings_mut();
        match id {
            ID_QUIT => return true,
            ID_FOLDER => {
                sys::open_path(&store::data_dir().to_string_lossy());
                return false;
            }
            i if (ID_TOGGLE..ID_TOGGLE + TOGGLE_LABELS.len()).contains(&i) => {
                if let Some(f) = toggle_field(s, i - ID_TOGGLE) {
                    *f = !*f;
                }
            }
            i if (ID_SIZE..ID_SIZE + 100).contains(&i) => {
                s.size = (i - ID_SIZE) as u32;
                layout = true;
            }
            i if (ID_SCREEN..ID_SCREEN + 100).contains(&i) => {
                s.display = i - ID_SCREEN;
                layout = true;
            }
            i if (ID_FOCUS..ID_FOCUS + 100).contains(&i) => s.focus = (i - ID_FOCUS) as u32,
            i if (ID_SHORT..ID_SHORT + 100).contains(&i) => s.short_break = (i - ID_SHORT) as u32,
            i if (ID_LONG..ID_LONG + 100).contains(&i) => s.long_break = (i - ID_LONG) as u32,
            i if (ID_GOAL..ID_GOAL + 100).contains(&i) => s.water_goal = (i - ID_GOAL) as u32 * 100,
            i if (ID_GLASS..ID_GLASS + 100).contains(&i) => s.water_glass = (i - ID_GLASS) as u32 * 10,
            i if (ID_SNOOZE..ID_SNOOZE + 100).contains(&i) => s.snooze = (i - ID_SNOOZE) as u32,
            _ => return false,
        }
    }
    a.settings_changed(layout);
    false
}

fn set_activatable(hwnd: HWND, on: bool) {
    sys::set_activatable(hwnd, on);
}

fn show_menu(hwnd: HWND) {
    let Some(s) = with_app(|a| a.settings().clone()) else { return };
    unsafe {
        let Some(m) = build_menu(&s) else { return };
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        set_activatable(hwnd, true);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, None);
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(m);
        let editing = with_app(|a| a.wants_activation()).unwrap_or(false);
        if !editing {
            set_activatable(hwnd, false);
        }
        if cmd.0 != 0 {
            let quit = with_app(|a| apply_command(a, cmd.0 as usize)).unwrap_or(false);
            if quit {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

fn run_deferred(hwnd: HWND, d: Option<Deferred>) {
    match d {
        Some(Deferred::DragOut(paths, idx)) => {
            shelf::drag_out(&paths);
            with_app(|a| a.after_drag(idx));
        }
        Some(Deferred::Menu) => show_menu(hwnd),
        Some(Deferred::Quit) => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        Some(Deferred::Focus(on)) => {
            set_activatable(hwnd, on);
            if on {
                unsafe {
                    let _ = SetForegroundWindow(hwnd);
                    let _ = SetFocus(hwnd);
                }
            }
        }
        None => {}
    }
}

// ------------------------------------------------------------------ window procedure

fn xy(lp: LPARAM) -> (i32, i32) {
    let x = (lp.0 & 0xffff) as u16 as i16 as i32;
    let y = ((lp.0 >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled: Option<LRESULT> = match msg {
        WM_TIMER => with_app(|a| a.on_timer(wp.0)).map(|d| {
            run_deferred(hwnd, d);
            LRESULT(0)
        }),
        WM_MOUSEMOVE => {
            let (x, y) = xy(lp);
            with_app(|a| a.on_mouse_move(x, y)).map(|d| {
                run_deferred(hwnd, d);
                LRESULT(0)
            })
        }
        WM_MOUSELEAVE => with_app(|a| a.on_mouse_leave()).map(|_| LRESULT(0)),
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = xy(lp);
            with_app(|a| a.on_lbutton_down(x, y)).map(|d| {
                run_deferred(hwnd, d);
                LRESULT(0)
            })
        }
        WM_LBUTTONUP => {
            let (x, y) = xy(lp);
            with_app(|a| a.on_lbutton_up(x, y)).map(|_| LRESULT(0))
        }
        WM_RBUTTONUP => {
            let (x, y) = xy(lp);
            with_app(|a| a.on_rbutton_up(x, y)).map(|d| {
                run_deferred(hwnd, d);
                LRESULT(0)
            })
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xffff) as u16 as i16 as i32;
            with_app(|a| a.on_wheel(delta)).map(|_| LRESULT(0))
        }
        WM_SETCURSOR => {
            if with_app(|a| a.cursor_hand()).unwrap_or(false) {
                if let Ok(c) = LoadCursorW(None, IDC_HAND) {
                    SetCursor(c);
                    Some(LRESULT(1))
                } else {
                    None
                }
            } else {
                None
            }
        }
        WM_MOUSEACTIVATE => {
            let act = with_app(|a| a.wants_activation()).unwrap_or(false);
            Some(LRESULT(if act { MA_ACTIVATE as isize } else { MA_NOACTIVATE as isize }))
        }
        WM_CHAR => with_app(|a| a.on_char(wp.0 as u32)).map(|d| {
            run_deferred(hwnd, d);
            LRESULT(0)
        }),
        WM_KEYDOWN => with_app(|a| a.on_keydown(wp.0 as u32)).map(|d| {
            run_deferred(hwnd, d);
            LRESULT(0)
        }),
        WM_POWERBROADCAST => {
            if wp.0 as u32 == PBT_POWERSETTINGCHANGE {
                if let Some(ev) = sys::decode_power(lp) {
                    with_app(|a| a.on_power(ev));
                }
            }
            Some(LRESULT(1))
        }
        WM_DPICHANGED | WM_DISPLAYCHANGE => with_app(|a| a.place()).map(|_| LRESULT(0)),
        WM_SETTINGCHANGE => {
            with_app(|a| a.draw());
            None
        }
        ui::WM_MEDIA => with_app(|a| a.on_media()).map(|_| LRESULT(0)),
        ui::WM_KEYHOOK => with_app(|a| a.on_keyhook(wp.0, lp.0 != 0)).map(|_| LRESULT(0)),
        ui::WM_NET => with_app(|a| a.on_net()).map(|_| LRESULT(0)),
        ui::WM_NOTIF => with_app(|a| a.on_notif(wp.0 as u32)).map(|_| LRESULT(0)),
        ui::WM_DRAG => with_app(|a| a.on_drag(wp.0)).map(|_| LRESULT(0)),
        ui::WM_TG => with_app(|a| a.on_tg(wp.0)).map(|_| LRESULT(0)),
        ui::WM_CLAUDE => with_app(|a| a.on_claude(wp.0)).map(|_| LRESULT(0)),
        WM_COPYDATA => {
            let cds = lp.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT;
            if !cds.is_null() && (*cds).dwData == downloads::COPYDATA_TAG && !(*cds).lpData.is_null() {
                let bytes = std::slice::from_raw_parts((*cds).lpData as *const u8, (*cds).cbData as usize);
                let text = String::from_utf8_lossy(bytes).into_owned();
                // may arrive while the app is busy (menu, drag): retry shortly
                if with_app(|a| a.on_download_msg(&text)).is_none() {
                    PENDING.with(|p| p.borrow_mut().push(text));
                    let _ = PostMessageW(hwnd, WM_APP + 99, WPARAM(0), LPARAM(0));
                }
                Some(LRESULT(1))
            } else {
                None
            }
        }
        m if m == WM_APP + 99 => {
            let list: Vec<String> = PENDING.with(|p| p.borrow_mut().drain(..).collect());
            for t in list {
                if with_app(|a| a.on_download_msg(&t)).is_none() {
                    PENDING.with(|p| p.borrow_mut().push(t));
                }
            }
            Some(LRESULT(0))
        }
        ui::WM_TRAY => {
            match (lp.0 as u32) & 0xffff {
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                WM_LBUTTONUP => {
                    with_app(|a| a.toggle_open());
                }
                _ => {}
            }
            Some(LRESULT(0))
        }
        WM_DESTROY => {
            tray_remove(hwnd);
            with_app(|a| a.shutdown());
            PostQuitMessage(0);
            Some(LRESULT(0))
        }
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            tray_add(hwnd);
            Some(LRESULT(0))
        }
        _ => None,
    };
    match handled {
        Some(r) => r,
        None => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ------------------------------------------------------------------ entry point

fn main() {
    // launched by a browser as the downloads native-messaging host
    if downloads::is_host_invocation() {
        downloads::run_host();
        return;
    }
    unsafe {
        // single instance
        let mutex = CreateMutexW(None, TRUE, w!("Local\\Notch.SingleInstance"));
        if mutex.is_err() || GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = OleInitialize(None);

        let hinst: HINSTANCE = GetModuleHandleW(None).map(|h| h.into()).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("NotchWindow"),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return;
        }
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("NotchWindow"),
            w!("Notch"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            hinst,
            None,
        ) else {
            return;
        };

        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        TRAY_ICON.store(gfx::make_tray_icon().0 as isize, Ordering::Relaxed);

        let app = App::new(hwnd);
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        with_app(|a| a.draw());
        tray_add(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|cell| *cell.borrow_mut() = None);
        let _ = mutex;
    }
}
