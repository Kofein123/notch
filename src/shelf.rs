//! File shelf: OLE drag & drop into the notch, and dragging items back out.

use std::sync::{Arc, Mutex};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::*;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::gfx::wide0;
use crate::store::ShelfItem;

pub type Inbox = Arc<Mutex<Vec<ShelfItem>>>;

/// wparam values sent with the drag message
pub const DRAG_ENTER: usize = 1;
pub const DRAG_LEAVE: usize = 0;
pub const DRAG_DROP: usize = 2;

#[implement(IDropTarget)]
struct Target {
    hwnd: isize,
    msg: u32,
    inbox: Inbox,
}

impl Target {
    fn post(&self, w: usize) {
        unsafe {
            let _ = PostMessageW(HWND(self.hwnd as *mut _), self.msg, WPARAM(w), LPARAM(0));
        }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, _obj: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            if !effect.is_null() {
                *effect = DROPEFFECT_COPY;
            }
        }
        self.post(DRAG_ENTER);
        Ok(())
    }

    fn DragOver(&self, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            if !effect.is_null() {
                *effect = DROPEFFECT_COPY;
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.post(DRAG_LEAVE);
        Ok(())
    }

    fn Drop(&self, obj: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let mut items = Vec::new();
        if let Some(o) = obj {
            unsafe {
                items.extend(read_files(o));
                if items.is_empty() {
                    if let Some(t) = read_text(o) {
                        if !t.trim().is_empty() {
                            items.push(ShelfItem::Text(t));
                        }
                    }
                }
                if !effect.is_null() {
                    *effect = DROPEFFECT_COPY;
                }
            }
        }
        if !items.is_empty() {
            self.inbox.lock().unwrap().extend(items);
        }
        self.post(DRAG_DROP);
        Ok(())
    }
}

fn fmt(cf: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

unsafe fn read_files(o: &IDataObject) -> Vec<ShelfItem> {
    let mut out = Vec::new();
    let Ok(mut medium) = o.GetData(&fmt(CF_HDROP.0)) else { return out };
    let hdrop = HDROP(medium.u.hGlobal.0);
    let n = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
    for i in 0..n {
        let len = DragQueryFileW(hdrop, i, None);
        let mut buf = vec![0u16; len as usize + 1];
        let got = DragQueryFileW(hdrop, i, Some(&mut buf));
        if got > 0 {
            out.push(ShelfItem::File(String::from_utf16_lossy(&buf[..got as usize])));
        }
    }
    ReleaseStgMedium(&mut medium);
    out
}

unsafe fn read_text(o: &IDataObject) -> Option<String> {
    let mut medium = o.GetData(&fmt(CF_UNICODETEXT.0)).ok()?;
    let h = medium.u.hGlobal;
    let p = GlobalLock(h) as *const u16;
    let mut s = None;
    if !p.is_null() {
        let size = GlobalSize(h) / 2;
        let slice = std::slice::from_raw_parts(p, size);
        let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
        s = Some(String::from_utf16_lossy(&slice[..end]));
        let _ = GlobalUnlock(h);
    }
    ReleaseStgMedium(&mut medium);
    s
}

pub fn register(hwnd: HWND, msg: u32) -> Inbox {
    let inbox: Inbox = Arc::new(Mutex::new(Vec::new()));
    let target: IDropTarget = Target { hwnd: hwnd.0 as isize, msg, inbox: inbox.clone() }.into();
    unsafe {
        let _ = RegisterDragDrop(hwnd, &target);
    }
    std::mem::forget(target);
    inbox
}

// ------------------------------------------------------------------ drag out

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if (keys.0 & MK_LBUTTON.0) == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Starts a modal shell drag of the given files. Must be called without any
/// RefCell borrow held (the drag loop re-enters the window procedure).
pub fn drag_out(paths: &[String]) {
    unsafe {
        let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
        for p in paths {
            let w = wide0(p);
            let pidl = ILCreateFromPathW(PCWSTR(w.as_ptr()));
            if !pidl.is_null() {
                pidls.push(pidl as *const _);
            }
        }
        if pidls.is_empty() {
            return;
        }
        if let Ok(data) = SHCreateDataObject::<_, IDataObject>(None, Some(&pidls), None) {
            let src: IDropSource = Source.into();
            let mut effect = DROPEFFECT_NONE;
            let _ = DoDragDrop(&data, &src, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect);
        }
        for p in pidls {
            ILFree(Some(p));
        }
    }
}
