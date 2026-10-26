//! Telegram through TDLib's JSON interface.
//!
//! `tdjson.dll` sits next to Notch.exe (the release zip ships it). It is loaded
//! at runtime, so Notch still works without it. TDLib keeps its own encrypted
//! database in `%APPDATA%\Notch\tdlib`; the database key is random and stored
//! protected with DPAPI (only this Windows user can read it).
//!
//! One background thread pumps `td_receive`, keeps a small model (chats,
//! users, the open conversation, pending message peeks) and pokes the window.

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_double, c_int, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use windows::core::*;
use windows::Graphics::Imaging::*;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::Foundation::*;
use windows::Win32::Security::Cryptography::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::store;

pub const AVATAR: u32 = 64;

/// wparam values of the window message
pub const N_REFRESH: usize = 0;
pub const N_PEEK: usize = 1;

type CreateFn = unsafe extern "C" fn() -> c_int;
type SendFn = unsafe extern "C" fn(c_int, *const c_char);
type RecvFn = unsafe extern "C" fn(c_double) -> *const c_char;
type ExecFn = unsafe extern "C" fn(*const c_char) -> *const c_char;

#[derive(Clone, Copy)]
struct Lib {
    create: CreateFn,
    send: SendFn,
    recv: RecvFn,
    exec: ExecFn,
}

fn load_lib() -> Option<Lib> {
    unsafe {
        let exe = std::env::current_exe().ok()?;
        let path = exe.parent()?.join("tdjson.dll");
        if !path.exists() {
            return None;
        }
        let w = crate::gfx::wide0(&path.to_string_lossy());
        let h = LoadLibraryExW(PCWSTR(w.as_ptr()), None, LOAD_WITH_ALTERED_SEARCH_PATH).ok()?;
        let create = GetProcAddress(h, s!("td_create_client_id"))?;
        let send = GetProcAddress(h, s!("td_send"))?;
        let recv = GetProcAddress(h, s!("td_receive"))?;
        let exec = GetProcAddress(h, s!("td_execute"))?;
        Some(Lib {
            create: std::mem::transmute::<_, CreateFn>(create),
            send: std::mem::transmute::<_, SendFn>(send),
            recv: std::mem::transmute::<_, RecvFn>(recv),
            exec: std::mem::transmute::<_, ExecFn>(exec),
        })
    }
}

// ------------------------------------------------------------------ model

#[derive(Clone, Debug, PartialEq)]
pub enum Auth {
    /// tdjson.dll is missing
    NoLib,
    /// api_id / api_hash not entered yet
    NeedKeys,
    Starting,
    WaitPhone,
    WaitCode(String),
    WaitPassword(String),
    Ready,
    /// a step that has to be finished in an official app
    Other(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Private,
    Group,
    Channel,
}

#[derive(Clone, Debug, Default)]
pub struct Msg {
    pub id: i64,
    pub chat_id: i64,
    pub out: bool,
    pub sender: String,
    pub text: String,
    pub date: i64,
    pub pending: bool,
    pub failed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Chat {
    pub id: i64,
    pub title: String,
    pub kind: Kind,
    pub order: i64,
    pub unread: i32,
    pub last: Option<Msg>,
    pub default_mute: bool,
    pub mute_for: i32,
    pub photo_file: i32,
    pub photo: Option<Arc<Vec<u8>>>,
    pub photo_id: u64,
}

#[derive(Clone, Debug)]
pub struct Peek {
    pub chat_id: i64,
    pub title: String,
    /// newest last; group messages are prefixed with the sender
    pub lines: Vec<String>,
}

pub struct State {
    pub auth: Auth,
    pub error: String,
    pub me: i64,
    pub chats: HashMap<i64, Chat>,
    pub users: HashMap<i64, String>,
    /// mute_for of private / group / channel scope defaults
    pub scope_mute: [i32; 3],
    pub open: Option<i64>,
    pub history: Vec<Msg>,
    pub history_more: bool,
    pub loading: bool,
    pub peeks: VecDeque<Peek>,
    /// chat currently on screen in the island (no peek for it)
    pub visible_chat: Option<i64>,
    files: HashMap<i32, i64>,
    photo_counter: u64,
}

impl State {
    fn new(auth: Auth) -> State {
        State {
            auth,
            error: String::new(),
            me: 0,
            chats: HashMap::new(),
            users: HashMap::new(),
            scope_mute: [0; 3],
            open: None,
            history: Vec::new(),
            history_more: true,
            loading: false,
            peeks: VecDeque::new(),
            visible_chat: None,
            files: HashMap::new(),
            photo_counter: 0,
        }
    }

    pub fn is_muted(&self, c: &Chat) -> bool {
        let m = if c.default_mute {
            self.scope_mute[match c.kind {
                Kind::Private => 0,
                Kind::Group => 1,
                Kind::Channel => 2,
            }]
        } else {
            c.mute_for
        };
        m > 0
    }

    /// Chats of the main list, most recent first.
    pub fn sorted(&self) -> Vec<Chat> {
        let mut v: Vec<Chat> = self.chats.values().filter(|c| c.order != 0).cloned().collect();
        v.sort_by(|a, b| b.order.cmp(&a.order));
        v
    }

    /// Unread messages in chats that aren't muted.
    pub fn unread_total(&self) -> i32 {
        self.chats.values().filter(|c| c.order != 0 && !self.is_muted(c)).map(|c| c.unread).sum()
    }
}

pub type Shared = Arc<Mutex<State>>;

// ------------------------------------------------------------------ client

pub struct Tg {
    lib: Option<Lib>,
    client: Arc<AtomicI32>,
    started: Arc<AtomicBool>,
    relogin: Arc<AtomicBool>,
    pub state: Shared,
    hwnd: isize,
    msg: u32,
}

#[derive(Clone)]
struct Ctx {
    lib: Lib,
    client: Arc<AtomicI32>,
    relogin: Arc<AtomicBool>,
    state: Shared,
    hwnd: isize,
    msg: u32,
}

impl Ctx {
    fn send(&self, v: Value) {
        if let Ok(c) = CString::new(v.to_string()) {
            unsafe { (self.lib.send)(self.client.load(Ordering::SeqCst), c.as_ptr()) }
        }
    }
    fn notify(&self, kind: usize) {
        unsafe {
            let _ = PostMessageW(HWND(self.hwnd as *mut _), self.msg, WPARAM(kind), LPARAM(0));
        }
    }
}

fn keys_path() -> std::path::PathBuf {
    store::data_dir().join("tg.txt")
}

fn load_keys() -> Option<(i32, String)> {
    let s = std::fs::read_to_string(keys_path()).ok()?;
    let mut id = None;
    let mut hash = None;
    for line in s.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "api_id" => id = v.trim().parse().ok(),
                "api_hash" => hash = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    Some((id?, hash.filter(|h| h.len() >= 16)?))
}

impl Tg {
    pub fn start(hwnd: HWND, msg: u32) -> Tg {
        let lib = load_lib();
        let auth = if lib.is_none() {
            Auth::NoLib
        } else if load_keys().is_none() {
            Auth::NeedKeys
        } else {
            Auth::Starting
        };
        let tg = Tg {
            lib,
            client: Arc::new(AtomicI32::new(0)),
            started: Arc::new(AtomicBool::new(false)),
            relogin: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(State::new(auth.clone()))),
            hwnd: hwnd.0 as isize,
            msg,
        };
        if auth == Auth::Starting {
            tg.boot();
        }
        tg
    }

    fn ctx(&self) -> Option<Ctx> {
        Some(Ctx {
            lib: self.lib?,
            client: self.client.clone(),
            relogin: self.relogin.clone(),
            state: self.state.clone(),
            hwnd: self.hwnd,
            msg: self.msg,
        })
    }

    /// Creates the client and the receive thread (once).
    fn boot(&self) {
        let Some(ctx) = self.ctx() else { return };
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        unsafe {
            if let Ok(c) = CString::new(r#"{"@type":"setLogVerbosityLevel","new_verbosity_level":1}"#) {
                (ctx.lib.exec)(c.as_ptr());
            }
            ctx.client.store((ctx.lib.create)(), Ordering::SeqCst);
        }
        // any request starts the client
        ctx.send(json!({"@type": "getOption", "name": "version"}));
        std::thread::Builder::new().name("telegram".into()).spawn(move || pump(ctx)).ok();
    }

    fn send(&self, v: Value) {
        if let Some(ctx) = self.ctx() {
            ctx.send(v);
        }
    }

    fn set_error(&self, e: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.error = e.to_string();
        }
    }

    pub fn set_keys(&self, api_id: &str, api_hash: &str) -> bool {
        let id: i32 = match api_id.trim().parse() {
            Ok(v) if v > 0 => v,
            _ => {
                self.set_error("api_id is a number");
                return false;
            }
        };
        let hash = api_hash.trim();
        if hash.len() < 16 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            self.set_error("api_hash is a 32-character code");
            return false;
        }
        let _ = std::fs::write(keys_path(), format!("api_id={id}\napi_hash={hash}\n"));
        if let Ok(mut s) = self.state.lock() {
            s.auth = Auth::Starting;
            s.error.clear();
        }
        if self.started.load(Ordering::SeqCst) {
            // a client already exists (e.g. after logout): restart it
            self.relogin.store(true, Ordering::SeqCst);
            self.send(json!({"@type": "close"}));
        } else {
            self.boot();
        }
        true
    }

    /// Forgets api_id / api_hash (login screen goes back to step 1).
    pub fn reset_keys(&self) {
        let _ = std::fs::remove_file(keys_path());
        if let Ok(mut s) = self.state.lock() {
            if s.auth != Auth::Ready {
                s.auth = Auth::NeedKeys;
            }
            s.error.clear();
        }
    }

    fn auth_req(&self, v: Value) {
        self.set_error("");
        let mut v = v;
        v["@extra"] = json!("auth");
        self.send(v);
    }

    pub fn phone(&self, p: &str) {
        let p: String = p.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
        self.auth_req(json!({"@type": "setAuthenticationPhoneNumber", "phone_number": p}));
    }

    pub fn code(&self, c: &str) {
        let c: String = c.chars().filter(|c| c.is_ascii_digit()).collect();
        self.auth_req(json!({"@type": "checkAuthenticationCode", "code": c}));
    }

    pub fn password(&self, p: &str) {
        self.auth_req(json!({"@type": "checkAuthenticationPassword", "password": p}));
    }

    pub fn logout(&self) {
        self.relogin.store(true, Ordering::SeqCst);
        self.send(json!({"@type": "logOut"}));
        if let Ok(mut s) = self.state.lock() {
            s.auth = Auth::Starting;
            s.chats.clear();
            s.history.clear();
            s.open = None;
            s.peeks.clear();
        }
    }

    pub fn open_chat(&self, chat_id: i64) {
        let prev = {
            let Ok(mut s) = self.state.lock() else { return };
            let prev = s.open.replace(chat_id);
            if prev == Some(chat_id) {
                return;
            }
            s.history.clear();
            s.history_more = true;
            s.loading = true;
            if let Some(c) = s.chats.get(&chat_id) {
                if let Some(m) = &c.last {
                    s.history.push(m.clone());
                }
            }
            prev
        };
        if let Some(p) = prev {
            self.send(json!({"@type": "closeChat", "chat_id": p}));
        }
        self.send(json!({"@type": "openChat", "chat_id": chat_id}));
        self.send(json!({
            "@type": "getChatHistory", "chat_id": chat_id, "from_message_id": 0,
            "offset": 0, "limit": 30, "only_local": false, "@extra": format!("hist:{chat_id}:0")
        }));
    }

    pub fn close_chat(&self) {
        let prev = self.state.lock().ok().and_then(|mut s| {
            s.history.clear();
            s.open.take()
        });
        if let Some(p) = prev {
            self.send(json!({"@type": "closeChat", "chat_id": p}));
        }
    }

    /// Requests older messages of the open chat.
    pub fn load_older(&self) {
        let (chat, oldest) = {
            let Ok(mut s) = self.state.lock() else { return };
            if s.loading || !s.history_more {
                return;
            }
            let Some(chat) = s.open else { return };
            let Some(first) = s.history.first().map(|m| m.id) else { return };
            s.loading = true;
            (chat, first)
        };
        self.send(json!({
            "@type": "getChatHistory", "chat_id": chat, "from_message_id": oldest,
            "offset": 0, "limit": 30, "only_local": false, "@extra": format!("hist:{chat}:older")
        }));
    }

    pub fn mark_read(&self, chat_id: i64) {
        let ids: Vec<i64> = {
            let Ok(s) = self.state.lock() else { return };
            if s.open != Some(chat_id) {
                return;
            }
            s.history.iter().rev().take(50).map(|m| m.id).filter(|id| *id > 0).collect()
        };
        if ids.is_empty() {
            return;
        }
        self.send(json!({
            "@type": "viewMessages", "chat_id": chat_id, "message_ids": ids,
            "source": {"@type": "messageSourceChatHistory"}, "force_read": true
        }));
    }

    pub fn send_text(&self, chat_id: i64, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.send(json!({
            "@type": "sendMessage", "chat_id": chat_id,
            "input_message_content": {
                "@type": "inputMessageText",
                "text": {"@type": "formattedText", "text": text, "entities": []},
                "clear_draft": true
            }
        }));
    }

    pub fn set_visible_chat(&self, c: Option<i64>) {
        if let Ok(mut s) = self.state.lock() {
            s.visible_chat = c;
        }
    }
}

// ------------------------------------------------------------------ receive loop

fn pump(ctx: Ctx) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut decoded: Vec<(i64, String)> = Vec::new();
    loop {
        let p = unsafe { (ctx.lib.recv)(1.0) };
        if p.is_null() {
            continue;
        }
        let raw = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        let Ok(v) = serde_json::from_str::<Value>(&raw) else { continue };
        // only the current client
        if let Some(id) = v.get("@client_id").and_then(|x| x.as_i64()) {
            if id as i32 != ctx.client.load(Ordering::SeqCst) {
                continue;
            }
        }
        let kind = handle(&ctx, &v, &mut decoded);
        // avatars are decoded outside the lock
        for (chat, path) in decoded.drain(..) {
            if let Some(px) = decode_avatar(&path) {
                if let Ok(mut s) = ctx.state.lock() {
                    s.photo_counter += 1;
                    let n = s.photo_counter;
                    if let Some(c) = s.chats.get_mut(&chat) {
                        c.photo = Some(Arc::new(px));
                        c.photo_id = n;
                    }
                }
                ctx.notify(N_REFRESH);
            }
        }
        if let Some(k) = kind {
            ctx.notify(k);
        }
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn i(v: &Value, k: &str) -> i64 {
    match v.get(k) {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(t)) => t.parse().unwrap_or(0),
        _ => 0,
    }
}

fn ty(v: &Value) -> &str {
    v.get("@type").and_then(|x| x.as_str()).unwrap_or("")
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn handle(ctx: &Ctx, v: &Value, decode: &mut Vec<(i64, String)>) -> Option<usize> {
    match ty(v) {
        "updateAuthorizationState" => {
            let st = &v["authorization_state"];
            let auth = match ty(st) {
                "authorizationStateWaitTdlibParameters" => {
                    send_parameters(ctx);
                    Auth::Starting
                }
                "authorizationStateWaitPhoneNumber" => Auth::WaitPhone,
                "authorizationStateWaitCode" => {
                    let info = &st["code_info"];
                    let hint = match ty(&info["type"]) {
                        "authenticationCodeTypeTelegramMessage" => "The code was sent to your Telegram app".to_string(),
                        "authenticationCodeTypeSms" | "authenticationCodeTypeSmsWord" | "authenticationCodeTypeSmsPhrase" => {
                            format!("The code was sent by SMS to {}", s(info, "phone_number"))
                        }
                        "authenticationCodeTypeCall" => "You will get a call with the code".to_string(),
                        _ => "Enter the login code".to_string(),
                    };
                    Auth::WaitCode(hint)
                }
                "authorizationStateWaitPassword" => {
                    let h = s(st, "password_hint");
                    Auth::WaitPassword(if h.is_empty() { String::new() } else { format!("Hint: {h}") })
                }
                "authorizationStateReady" => {
                    ctx.send(json!({"@type": "loadChats", "chat_list": {"@type": "chatListMain"}, "limit": 60}));
                    ctx.send(json!({"@type": "getMe", "@extra": "me"}));
                    Auth::Ready
                }
                "authorizationStateLoggingOut" | "authorizationStateClosing" => Auth::Starting,
                "authorizationStateClosed" => {
                    if ctx.relogin.swap(false, Ordering::SeqCst) {
                        unsafe { ctx.client.store((ctx.lib.create)(), Ordering::SeqCst) };
                        ctx.send(json!({"@type": "getOption", "name": "version"}));
                    }
                    if let Ok(mut s) = ctx.state.lock() {
                        s.chats.clear();
                        s.history.clear();
                        s.open = None;
                    }
                    if load_keys().is_some() {
                        Auth::Starting
                    } else {
                        Auth::NeedKeys
                    }
                }
                "authorizationStateWaitOtherDeviceConfirmation" => {
                    Auth::Other("Confirm the login on another device, then try again".into())
                }
                "authorizationStateWaitRegistration" => Auth::Other("This number has no Telegram account yet".into()),
                _ => Auth::Other("Finish signing in in the Telegram app first".into()),
            };
            if let Ok(mut s) = ctx.state.lock() {
                s.auth = auth;
            }
            Some(N_REFRESH)
        }
        "error" => {
            let extra = s(v, "@extra");
            if extra == "auth" {
                let m = s(v, "message");
                let nice = if m.contains("PHONE_CODE_INVALID") {
                    "Wrong code".to_string()
                } else if m.contains("PHONE_CODE_EXPIRED") {
                    "The code expired, enter the number again".to_string()
                } else if m.contains("PHONE_NUMBER_INVALID") {
                    "Invalid phone number".to_string()
                } else if m.contains("PASSWORD_HASH_INVALID") {
                    "Wrong password".to_string()
                } else if m.contains("API_ID") {
                    "api_id / api_hash are wrong (Reset keys)".to_string()
                } else if m.contains("FLOOD") || m.contains("Too Many") {
                    "Too many attempts, try again later".to_string()
                } else {
                    m
                };
                if let Ok(mut s) = ctx.state.lock() {
                    s.error = nice;
                }
                return Some(N_REFRESH);
            }
            if extra.starts_with("hist:") {
                if let Ok(mut s) = ctx.state.lock() {
                    s.loading = false;
                }
            }
            None
        }
        "user" => {
            if s(v, "@extra") == "me" {
                if let Ok(mut st) = ctx.state.lock() {
                    st.me = i(v, "id");
                }
            }
            put_user(ctx, v);
            None
        }
        "updateUser" => {
            put_user(ctx, &v["user"]);
            None
        }
        "updateNewChat" => {
            let c = &v["chat"];
            let mut chat = Chat {
                id: i(c, "id"),
                title: s(c, "title"),
                kind: match ty(&c["type"]) {
                    "chatTypeBasicGroup" => Kind::Group,
                    "chatTypeSupergroup" => {
                        if c["type"]["is_channel"].as_bool().unwrap_or(false) {
                            Kind::Channel
                        } else {
                            Kind::Group
                        }
                    }
                    _ => Kind::Private,
                },
                unread: i(c, "unread_count") as i32,
                default_mute: c["notification_settings"]["use_default_mute_for"].as_bool().unwrap_or(true),
                mute_for: i(&c["notification_settings"], "mute_for") as i32,
                ..Default::default()
            };
            chat.order = main_order(&c["positions"]).unwrap_or(0);
            let mut st = ctx.state.lock().ok()?;
            chat.last = c.get("last_message").filter(|m| !m.is_null()).map(|m| parse_msg(&st, m, chat.kind));
            let photo = &c["photo"];
            let id = chat.id;
            st.chats.insert(id, chat);
            want_photo(ctx, &mut st, id, photo, decode);
            Some(N_REFRESH)
        }
        "updateChatTitle" => {
            let mut st = ctx.state.lock().ok()?;
            let id = i(v, "chat_id");
            if let Some(c) = st.chats.get_mut(&id) {
                c.title = s(v, "title");
            }
            Some(N_REFRESH)
        }
        "updateChatPhoto" => {
            let mut st = ctx.state.lock().ok()?;
            let id = i(v, "chat_id");
            if let Some(c) = st.chats.get_mut(&id) {
                c.photo = None;
                c.photo_file = 0;
            }
            want_photo(ctx, &mut st, id, &v["photo"], decode);
            Some(N_REFRESH)
        }
        "updateChatPosition" => {
            let p = &v["position"];
            if ty(&p["list"]) != "chatListMain" {
                return None;
            }
            let mut st = ctx.state.lock().ok()?;
            if let Some(c) = st.chats.get_mut(&i(v, "chat_id")) {
                c.order = i(p, "order");
            }
            Some(N_REFRESH)
        }
        "updateChatLastMessage" => {
            let mut st = ctx.state.lock().ok()?;
            let id = i(v, "chat_id");
            let kind = st.chats.get(&id).map(|c| c.kind).unwrap_or_default();
            let last = v.get("last_message").filter(|m| !m.is_null()).map(|m| parse_msg(&st, m, kind));
            let order = main_order(&v["positions"]);
            if let Some(c) = st.chats.get_mut(&id) {
                c.last = last;
                if let Some(o) = order {
                    c.order = o;
                }
            }
            Some(N_REFRESH)
        }
        "updateChatReadInbox" => {
            let mut st = ctx.state.lock().ok()?;
            if let Some(c) = st.chats.get_mut(&i(v, "chat_id")) {
                c.unread = i(v, "unread_count") as i32;
            }
            Some(N_REFRESH)
        }
        "updateChatNotificationSettings" => {
            let mut st = ctx.state.lock().ok()?;
            let n = &v["notification_settings"];
            if let Some(c) = st.chats.get_mut(&i(v, "chat_id")) {
                c.default_mute = n["use_default_mute_for"].as_bool().unwrap_or(true);
                c.mute_for = i(n, "mute_for") as i32;
            }
            Some(N_REFRESH)
        }
        "updateScopeNotificationSettings" => {
            let mut st = ctx.state.lock().ok()?;
            let k = match ty(&v["scope"]) {
                "notificationSettingsScopePrivateChats" => 0,
                "notificationSettingsScopeGroupChats" => 1,
                _ => 2,
            };
            st.scope_mute[k] = i(&v["notification_settings"], "mute_for") as i32;
            Some(N_REFRESH)
        }
        "updateNewMessage" => {
            let m = &v["message"];
            let mut st = ctx.state.lock().ok()?;
            let chat_id = i(m, "chat_id");
            let kind = st.chats.get(&chat_id).map(|c| c.kind).unwrap_or_default();
            let msg = parse_msg(&st, m, kind);
            if st.open == Some(chat_id) && !st.history.iter().any(|h| h.id == msg.id) {
                st.history.push(msg.clone());
            }
            let fresh = now_unix() - msg.date < 120;
            let muted = st.chats.get(&chat_id).map(|c| st.is_muted(c)).unwrap_or(true);
            if msg.out || !fresh || muted || st.visible_chat == Some(chat_id) || kind == Kind::Channel {
                return Some(N_REFRESH);
            }
            let title = st.chats.get(&chat_id).map(|c| c.title.clone()).unwrap_or_default();
            let line = if kind == Kind::Private || msg.sender.is_empty() {
                msg.text.clone()
            } else {
                format!("{}: {}", msg.sender, msg.text)
            };
            if let Some(k) = st.peeks.iter().position(|p| p.chat_id == chat_id) {
                st.peeks[k].lines.push(line);
            } else {
                st.peeks.push_back(Peek { chat_id, title, lines: vec![line] });
                while st.peeks.len() > 8 {
                    st.peeks.pop_front();
                }
            }
            Some(N_PEEK)
        }
        "updateMessageSendSucceeded" => {
            let m = &v["message"];
            let old = i(v, "old_message_id");
            let mut st = ctx.state.lock().ok()?;
            let kind = st.chats.get(&i(m, "chat_id")).map(|c| c.kind).unwrap_or_default();
            let msg = parse_msg(&st, m, kind);
            if let Some(h) = st.history.iter_mut().find(|h| h.id == old) {
                *h = msg;
            }
            Some(N_REFRESH)
        }
        "updateMessageSendFailed" => {
            let old = i(v, "old_message_id");
            let mut st = ctx.state.lock().ok()?;
            if let Some(h) = st.history.iter_mut().find(|h| h.id == old) {
                h.pending = false;
                h.failed = true;
            }
            Some(N_REFRESH)
        }
        "updateDeleteMessages" => {
            if !v["is_permanent"].as_bool().unwrap_or(false) {
                return None;
            }
            let mut st = ctx.state.lock().ok()?;
            if st.open != Some(i(v, "chat_id")) {
                return None;
            }
            let ids: Vec<i64> = v["message_ids"].as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
            st.history.retain(|m| !ids.contains(&m.id));
            Some(N_REFRESH)
        }
        "messages" => {
            let extra = s(v, "@extra");
            let mut parts = extra.split(':');
            if parts.next() != Some("hist") {
                return None;
            }
            let chat: i64 = parts.next()?.parse().ok()?;
            let step = parts.next().unwrap_or("0").to_string();
            let mut st = ctx.state.lock().ok()?;
            st.loading = false;
            if st.open != Some(chat) {
                return None;
            }
            let kind = st.chats.get(&chat).map(|c| c.kind).unwrap_or_default();
            let list: Vec<Msg> = v["messages"]
                .as_array()
                .map(|a| a.iter().filter(|m| !m.is_null()).map(|m| parse_msg(&st, m, kind)).collect())
                .unwrap_or_default();
            if list.is_empty() {
                st.history_more = false;
            }
            for m in list.iter() {
                if !st.history.iter().any(|h| h.id == m.id) {
                    st.history.push(m.clone());
                }
            }
            st.history.sort_by_key(|m| m.id);
            // TDLib answers the first request from its cache: ask again until enough
            let total = st.history.len();
            let oldest = st.history.first().map(|m| m.id).unwrap_or(0);
            if step != "older" {
                let n: u32 = step.parse().unwrap_or(0);
                if total < 20 && !list.is_empty() && n < 3 {
                    st.loading = true;
                    drop(st);
                    ctx.send(json!({
                        "@type": "getChatHistory", "chat_id": chat, "from_message_id": oldest,
                        "offset": 0, "limit": 30, "only_local": false, "@extra": format!("hist:{chat}:{}", n + 1)
                    }));
                }
            }
            Some(N_REFRESH)
        }
        "updateFile" => {
            let f = &v["file"];
            if !f["local"]["is_downloading_completed"].as_bool().unwrap_or(false) {
                return None;
            }
            let id = i(f, "id") as i32;
            let mut st = ctx.state.lock().ok()?;
            if let Some(chat) = st.files.remove(&id) {
                decode.push((chat, s(&f["local"], "path")));
            }
            None
        }
        _ => None,
    }
}

fn put_user(ctx: &Ctx, u: &Value) {
    let name = format!("{} {}", s(u, "first_name"), s(u, "last_name")).trim().to_string();
    if let Ok(mut st) = ctx.state.lock() {
        st.users.insert(i(u, "id"), if name.is_empty() { "Deleted account".into() } else { name });
    }
}

fn main_order(positions: &Value) -> Option<i64> {
    positions.as_array()?.iter().find(|p| ty(&p["list"]) == "chatListMain").map(|p| i(p, "order"))
}

fn want_photo(ctx: &Ctx, st: &mut State, chat: i64, photo: &Value, decode: &mut Vec<(i64, String)>) {
    if photo.is_null() {
        return;
    }
    let f = &photo["small"];
    let id = i(f, "id") as i32;
    if id == 0 {
        return;
    }
    if let Some(c) = st.chats.get_mut(&chat) {
        c.photo_file = id;
    }
    if f["local"]["is_downloading_completed"].as_bool().unwrap_or(false) {
        decode.push((chat, s(&f["local"], "path")));
    } else {
        st.files.insert(id, chat);
        ctx.send(json!({"@type": "downloadFile", "file_id": id, "priority": 1, "offset": 0, "limit": 0, "synchronous": false}));
    }
}

fn text_of(f: &Value) -> String {
    s(f, "text")
}

fn with_caption(label: &str, caption: &Value) -> String {
    let c = text_of(caption);
    if c.is_empty() {
        label.to_string()
    } else {
        format!("{label}, {c}")
    }
}

pub fn content_text(c: &Value) -> String {
    match ty(c) {
        "messageText" => text_of(&c["text"]),
        "messagePhoto" => with_caption("Photo", &c["caption"]),
        "messageVideo" => with_caption("Video", &c["caption"]),
        "messageAnimation" => with_caption("GIF", &c["caption"]),
        "messageSticker" => format!("{} Sticker", s(&c["sticker"], "emoji")).trim().to_string(),
        "messageVoiceNote" => "Voice message".into(),
        "messageVideoNote" => "Video message".into(),
        "messageDocument" => with_caption(&format!("File: {}", s(&c["document"], "file_name")), &c["caption"]),
        "messageAudio" => {
            let a = &c["audio"];
            let t = format!("{} – {}", s(a, "performer"), s(a, "title"));
            format!("Music: {}", t.trim_matches([' ', '–']))
        }
        "messageLocation" | "messageVenue" => "Location".into(),
        "messageContact" => "Contact".into(),
        "messagePoll" => format!("Poll: {}", text_of(&c["poll"]["question"])),
        "messageCall" => "Call".into(),
        "messageDice" => "Dice".into(),
        "messagePinMessage" => "Pinned a message".into(),
        "messageChatAddMembers" | "messageChatJoinByLink" => "Joined the group".into(),
        "messageChatDeleteMember" => "Left the group".into(),
        "messageStory" => "Story".into(),
        _ => "Message".into(),
    }
}

fn parse_msg(st: &State, m: &Value, kind: Kind) -> Msg {
    let sid = &m["sender_id"];
    let sender = match ty(sid) {
        "messageSenderUser" => {
            let uid = i(sid, "user_id");
            st.users.get(&uid).cloned().unwrap_or_default()
        }
        "messageSenderChat" => st.chats.get(&i(sid, "chat_id")).map(|c| c.title.clone()).unwrap_or_default(),
        _ => String::new(),
    };
    let _ = kind;
    let sending = &m["sending_state"];
    Msg {
        id: i(m, "id"),
        chat_id: i(m, "chat_id"),
        out: m["is_outgoing"].as_bool().unwrap_or(false),
        sender,
        text: content_text(&m["content"]).replace(['\r', '\n'], " "),
        date: i(m, "date"),
        pending: ty(sending) == "messageSendingStatePending",
        failed: ty(sending) == "messageSendingStateFailed",
    }
}

fn send_parameters(ctx: &Ctx) {
    let Some((api_id, api_hash)) = load_keys() else {
        if let Ok(mut s) = ctx.state.lock() {
            s.auth = Auth::NeedKeys;
        }
        return;
    };
    let dir = store::data_dir().join("tdlib");
    let _ = std::fs::create_dir_all(&dir);
    let key = db_key(&dir);
    ctx.send(json!({
        "@type": "setTdlibParameters",
        "use_test_dc": false,
        "database_directory": dir.to_string_lossy(),
        "files_directory": dir.join("files").to_string_lossy(),
        "database_encryption_key": base64(&key),
        "use_file_database": true,
        "use_chat_info_database": true,
        "use_message_database": true,
        "use_secret_chats": false,
        "api_id": api_id,
        "api_hash": api_hash,
        "system_language_code": "en",
        "device_model": "Windows PC",
        "system_version": "Windows",
        "application_version": concat!("Notch ", env!("CARGO_PKG_VERSION")),
        "@extra": "auth"
    }));
}

// ------------------------------------------------------------------ database key (DPAPI)

/// Random 32-byte key, stored DPAPI-protected next to the database. If it
/// can't be unprotected (another Windows user, reinstalled OS) the database is
/// wiped and a new key made, which only means logging in again.
fn db_key(dir: &std::path::Path) -> Vec<u8> {
    let path = store::data_dir().join("tg.key");
    if let Ok(blob) = std::fs::read(&path) {
        if let Some(k) = unprotect(&blob) {
            if k.len() == 32 {
                return k;
            }
        }
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::create_dir_all(dir);
    }
    let mut key = vec![0u8; 32];
    unsafe {
        let _ = BCryptGenRandom(BCRYPT_ALG_HANDLE(std::ptr::null_mut()), &mut key, BCRYPT_USE_SYSTEM_PREFERRED_RNG);
    }
    if let Some(p) = protect(&key) {
        let _ = std::fs::write(&path, p);
    }
    key
}

fn protect(data: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(&input, w!("Notch Telegram"), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as *mut _));
        Some(v)
    }
}

fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as *mut _));
        Some(v)
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// ------------------------------------------------------------------ avatars

fn decode_avatar(path: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let stream = InMemoryRandomAccessStream::new().ok()?;
    let w = DataWriter::CreateDataWriter(&stream).ok()?;
    w.WriteBytes(&bytes).ok()?;
    w.StoreAsync().ok()?.get().ok()?;
    let _ = w.DetachStream();
    stream.Seek(0).ok()?;
    let dec = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let t = BitmapTransform::new().ok()?;
    t.SetScaledWidth(AVATAR).ok()?;
    t.SetScaledHeight(AVATAR).ok()?;
    t.SetInterpolationMode(BitmapInterpolationMode::Fant).ok()?;
    let px = dec
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &t,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .ok()?
        .get()
        .ok()?;
    let v = px.DetachPixelData().ok()?.as_slice().to_vec();
    if v.len() >= (AVATAR * AVATAR * 4) as usize {
        Some(v)
    } else {
        None
    }
}

/// Telegram's avatar palette, picked by id.
pub fn avatar_color(id: i64) -> u32 {
    const P: [u32; 7] = [0xe17076, 0xfaa774, 0xa695e7, 0x7bc862, 0x6ec9cb, 0x65aadd, 0xee7aae];
    P[(id.unsigned_abs() % 7) as usize]
}

pub fn initials(title: &str) -> String {
    let mut out = String::new();
    for w in title.split_whitespace().take(2) {
        if let Some(c) = w.chars().find(|c| c.is_alphanumeric()) {
            out.extend(c.to_uppercase());
        }
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}
