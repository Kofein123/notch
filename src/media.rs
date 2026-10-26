//! Music: Windows "Global System Media Transport Controls" (SMTC).
//!
//! This is how Nusa sees Yandex Music / Spotify / browsers with zero setup:
//! every player that shows up in the Windows media flyout publishes a session
//! here. A background MTA thread polls the current session, decodes artwork and
//! fetches synced lyrics from LRCLIB, then pokes the UI with a window message.

use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::*;
use windows::Data::Json::*;
use windows::Foundation::Uri;
use windows::Graphics::Imaging::*;
use windows::Media::Control::*;
use windows::Storage::Streams::IRandomAccessStream;
use windows::Web::Http::HttpClient;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub const ART: u32 = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lyrics {
    #[default]
    None,
    Loading,
    Missing,
    Ready,
}

#[derive(Clone, Default)]
pub struct MediaInfo {
    pub has: bool,
    pub title: String,
    pub artist: String,
    pub app: String,
    pub playing: bool,
    pub pos: f64,
    pub dur: f64,
    pub sampled: Option<Instant>,
    pub art: Option<Arc<Vec<u8>>>,
    pub art_id: u64,
    pub lyrics: Option<Arc<Vec<(f64, String)>>>,
    pub lyrics_state: Lyrics,
    pub can_seek: bool,
}

impl MediaInfo {
    /// Current position with extrapolation between samples.
    pub fn position(&self) -> f64 {
        let mut p = self.pos;
        if self.playing {
            if let Some(t) = self.sampled {
                p += t.elapsed().as_secs_f64();
            }
        }
        if self.dur > 0.0 {
            p.clamp(0.0, self.dur)
        } else {
            p.max(0.0)
        }
    }

    pub fn lyric_index(&self) -> Option<usize> {
        let l = self.lyrics.as_ref()?;
        let p = self.position() + 0.25;
        let mut idx = None;
        for (i, (t, _)) in l.iter().enumerate() {
            if *t <= p {
                idx = Some(i);
            } else {
                break;
            }
        }
        idx
    }

    pub fn lyric_line(&self) -> Option<(String, Option<String>)> {
        let l = self.lyrics.as_ref()?;
        let i = self.lyric_index()?;
        let cur = l[i].1.clone();
        let next = l.get(i + 1).map(|x| x.1.clone());
        Some((cur, next))
    }
}

pub enum Cmd {
    Toggle,
    Next,
    Prev,
    Seek(f64),
    Refresh,
}

pub type Shared = Arc<Mutex<MediaInfo>>;

pub fn spawn(hwnd: HWND, msg: u32) -> (Shared, Sender<Cmd>) {
    let shared: Shared = Arc::new(Mutex::new(MediaInfo::default()));
    let (tx, rx) = channel::<Cmd>();
    let st = shared.clone();
    let hwnd_raw = hwnd.0 as isize;
    std::thread::Builder::new()
        .name("media".into())
        .spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let notify = move || {
                let _ = PostMessageW(HWND(hwnd_raw as *mut _), msg, WPARAM(0), LPARAM(0));
            };
            let mgr = loop {
                match GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()) {
                    Ok(m) => break m,
                    Err(_) => std::thread::sleep(Duration::from_secs(3)),
                }
            };
            let mut key = String::new();
            let mut last_props = Instant::now() - Duration::from_secs(10);
            let mut art_retry: Option<Instant> = None;
            let mut art_counter = 0u64;
            loop {
                let cmd = match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                let session = mgr.GetCurrentSession().ok();
                if let (Some(c), Some(s)) = (&cmd, &session) {
                    let _ = match c {
                        Cmd::Toggle => s.TryTogglePlayPauseAsync().and_then(|o| o.get()),
                        Cmd::Next => s.TrySkipNextAsync().and_then(|o| o.get()),
                        Cmd::Prev => s.TrySkipPreviousAsync().and_then(|o| o.get()),
                        Cmd::Seek(sec) => {
                            let start = s.GetTimelineProperties().and_then(|t| t.StartTime()).map(|t| t.Duration).unwrap_or(0);
                            s.TryChangePlaybackPositionAsync(start + (sec * 1e7) as i64).and_then(|o| o.get())
                        }
                        Cmd::Refresh => Ok(true),
                    };
                    std::thread::sleep(Duration::from_millis(120));
                    last_props = Instant::now() - Duration::from_secs(10);
                }
                let Some(s) = session else {
                    let mut m = st.lock().unwrap();
                    if m.has {
                        *m = MediaInfo::default();
                        key.clear();
                        drop(m);
                        notify();
                    }
                    continue;
                };

                // cheap synchronous getters every tick
                let playing = s
                    .GetPlaybackInfo()
                    .and_then(|p| p.PlaybackStatus())
                    .map(|p| p == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
                    .unwrap_or(false);
                let (mut pos, mut dur, mut can_seek) = (0.0f64, 0.0f64, false);
                if let Ok(tl) = s.GetTimelineProperties() {
                    let start = tl.StartTime().map(|t| t.Duration).unwrap_or(0);
                    let end = tl.EndTime().map(|t| t.Duration).unwrap_or(0);
                    let p = tl.Position().map(|t| t.Duration).unwrap_or(0);
                    let maxs = tl.MaxSeekTime().map(|t| t.Duration).unwrap_or(0);
                    dur = (end - start) as f64 / 1e7;
                    pos = (p - start) as f64 / 1e7;
                    can_seek = maxs > 0;
                    if playing {
                        if let Ok(last) = tl.LastUpdatedTime() {
                            let ft = GetSystemTimeAsFileTime();
                            let now = ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64;
                            let delta = (now - last.UniversalTime) as f64 / 1e7;
                            if delta > 0.0 && delta < 6.0 * 3600.0 {
                                pos += delta;
                            }
                        }
                    }
                    if dur > 0.0 {
                        pos = pos.clamp(0.0, dur);
                    }
                }

                // media properties are an async round-trip to the player: throttle
                let mut changed = false;
                if last_props.elapsed() > Duration::from_millis(1500) || art_retry.map_or(false, |t| Instant::now() >= t) {
                    last_props = Instant::now();
                    if let Ok(props) = s.TryGetMediaPropertiesAsync().and_then(|o| o.get()) {
                        let title = props.Title().map(|h| h.to_string()).unwrap_or_default();
                        let artist = props.Artist().map(|h| h.to_string()).unwrap_or_default();
                        let new_key = format!("{title}\u{1}{artist}");
                        let retry_now = art_retry.map_or(false, |t| Instant::now() >= t);
                        if new_key != key || retry_now {
                            let is_new = new_key != key;
                            key = new_key.clone();
                            art_retry = None;
                            let art = props.Thumbnail().ok().and_then(|r| decode_art(&r));
                            if art.is_none() && is_new {
                                art_retry = Some(Instant::now() + Duration::from_millis(1500));
                            }
                            let app = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
                            {
                                let mut m = st.lock().unwrap();
                                if is_new {
                                    m.title = title.clone();
                                    m.artist = artist.clone();
                                    m.app = app;
                                    m.lyrics = None;
                                    m.lyrics_state = Lyrics::Loading;
                                }
                                if art.is_some() || is_new {
                                    art_counter += 1;
                                    m.art = art.map(Arc::new);
                                    m.art_id = art_counter;
                                }
                            }
                            if is_new && !title.is_empty() {
                                fetch_lyrics(st.clone(), new_key, title, artist, dur, hwnd_raw, msg);
                            }
                            changed = true;
                        }
                    }
                }

                let mut m = st.lock().unwrap();
                let drift = (m.position() - pos).abs();
                if !m.has || m.playing != playing || (m.dur - dur).abs() > 0.5 || drift > 1.5 || changed || m.can_seek != can_seek {
                    m.has = true;
                    m.playing = playing;
                    m.dur = dur;
                    m.pos = pos;
                    m.can_seek = can_seek;
                    m.sampled = Some(Instant::now());
                    drop(m);
                    notify();
                }
            }
        })
        .expect("media thread");
    (shared, tx)
}

unsafe fn decode_art(r: &windows::Storage::Streams::IRandomAccessStreamReference) -> Option<Vec<u8>> {
    let stream = r.OpenReadAsync().ok()?.get().ok()?;
    let stream: IRandomAccessStream = stream.cast().ok()?;
    let dec = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let t = BitmapTransform::new().ok()?;
    t.SetScaledWidth(ART).ok()?;
    t.SetScaledHeight(ART).ok()?;
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
    let data = px.DetachPixelData().ok()?;
    let v = data.as_slice().to_vec();
    if v.len() >= (ART * ART * 4) as usize {
        Some(v)
    } else {
        None
    }
}

// ------------------------------------------------------------------ lyrics

fn fetch_lyrics(st: Shared, key: String, title: String, artist: String, dur: f64, hwnd_raw: isize, msg: u32) {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = lrclib(&title, &artist, dur);
        let mut m = st.lock().unwrap();
        if format!("{}\u{1}{}", m.title, m.artist) != key {
            return;
        }
        match result {
            Some(lines) if !lines.is_empty() => {
                m.lyrics = Some(Arc::new(lines));
                m.lyrics_state = Lyrics::Ready;
            }
            _ => m.lyrics_state = Lyrics::Missing,
        }
        drop(m);
        let _ = PostMessageW(HWND(hwnd_raw as *mut _), msg, WPARAM(0), LPARAM(0));
    });
}

fn lrclib(title: &str, artist: &str, dur: f64) -> Option<Vec<(f64, String)>> {
    // Many players append " - Topic", "(feat. X)" etc. Search with a cleaned title.
    let clean_title = strip_brackets(title);
    let first_artist = artist.split([',', '&']).next().unwrap_or(artist).trim().to_string();
    let url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        urlencode(&clean_title),
        urlencode(&first_artist)
    );
    let body = http_get(&url).or_else(|| {
        let q = format!("https://lrclib.net/api/search?q={}", urlencode(&format!("{clean_title} {first_artist}")));
        http_get(&q)
    })?;
    let arr = JsonArray::Parse(&HSTRING::from(body)).ok()?;
    let n = arr.Size().ok()?;
    let mut best: Option<(f64, String)> = None;
    for i in 0..n {
        let Ok(o) = arr.GetObjectAt(i) else { continue };
        let Some(synced) = json_str(&o, "syncedLyrics") else { continue };
        if synced.trim().is_empty() {
            continue;
        }
        let d = o.GetNamedNumber(h!("duration")).unwrap_or(0.0);
        let score = if dur > 0.0 { (d - dur).abs() } else { 0.0 };
        if best.as_ref().map_or(true, |(s, _)| score < *s) {
            best = Some((score, synced));
        }
    }
    let (score, text) = best?;
    if dur > 0.0 && score > 8.0 {
        return None;
    }
    Some(parse_lrc(&text))
}

fn json_str(o: &JsonObject, name: &str) -> Option<String> {
    let v = o.GetNamedValue(&HSTRING::from(name)).ok()?;
    if v.ValueType().ok()? == JsonValueType::String {
        v.GetString().ok().map(|h| h.to_string())
    } else {
        None
    }
}

fn http_get(url: &str) -> Option<String> {
    let client = HttpClient::new().ok()?;
    if let Ok(h) = client.DefaultRequestHeaders() {
        if let Ok(ua) = h.UserAgent() {
            let _ = ua.TryParseAdd(h!("Notch/0.1 (+https://github.com/Kofein123/notch)"));
        }
    }
    let uri = Uri::CreateUri(&HSTRING::from(url)).ok()?;
    let s = client.GetStringAsync(&uri).ok()?.get().ok()?;
    Some(s.to_string())
}

pub fn parse_lrc(text: &str) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let tag = &stripped[..end];
            if let Some(t) = parse_stamp(tag) {
                stamps.push(t);
            }
            rest = &stripped[end + 1..];
        }
        let lyric = rest.trim().to_string();
        for t in stamps {
            out.push((t, lyric.clone()));
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn parse_stamp(tag: &str) -> Option<f64> {
    let (m, s) = tag.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let s: f64 = s.trim().replace(',', ".").parse().ok()?;
    Some(m * 60.0 + s)
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let t = out.trim().to_string();
    if t.is_empty() {
        s.to_string()
    } else {
        t
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub fn fmt_time(sec: f64) -> String {
    let s = sec.max(0.0) as i64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}
