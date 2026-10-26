//! Thin Direct2D / DirectWrite layer.
//!
//! The notch is a per-pixel-alpha layered window. Every frame is drawn with a
//! D2D DC render target into a 32bpp premultiplied DIB and pushed to the window
//! with `UpdateLayeredWindow`. Fully transparent pixels are click-through, so the
//! window can be bigger than the visible island.
//!
//! All coordinates are logical pixels (DIPs); the render target DPI is set to
//! `96 * scale`, so D2D handles HiDPI for us.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::*;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub const fn hex(rgb: u32) -> Color {
        Color(
            ((rgb >> 16) & 0xff) as f32 / 255.0,
            ((rgb >> 8) & 0xff) as f32 / 255.0,
            (rgb & 0xff) as f32 / 255.0,
            1.0,
        )
    }
    pub const fn a(self, a: f32) -> Color {
        Color(self.0, self.1, self.2, self.3 * a)
    }
}

pub const WHITE: Color = Color(1.0, 1.0, 1.0, 1.0);
pub const BLACK: Color = Color(0.0, 0.0, 0.0, 1.0);
pub const GRAY: Color = Color::hex(0x8e8e93);
pub const DIM: Color = Color::hex(0x5a5a5e);
pub const CHIP: Color = Color::hex(0x1f1f22);
pub const CHIP_HI: Color = Color::hex(0x2c2c30);
pub const CARD: Color = Color::hex(0x161618);
pub const RED: Color = Color::hex(0xff4d55);
pub const GREEN: Color = Color::hex(0x30d158);
pub const BLUE: Color = Color::hex(0x2f8cff);
pub const ORANGE: Color = Color::hex(0xff9f0a);
pub const YELLOW: Color = Color::hex(0xffd60a);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> R {
        R { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
    pub fn inset(&self, d: f32) -> R {
        R::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

fn col(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn wide0(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct Gfx {
    pub d2d: ID2D1Factory,
    pub dw: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    dash: ID2D1StrokeStyle,
    formats: RefCell<HashMap<(u32, i32, bool, bool), IDWriteTextFormat>>,
    text_family: Vec<u16>,
    icon_family: Vec<u16>,
    pub has_fluent: bool,
    pub alpha: f32,
    mem_dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    pub pw: i32,
    pub ph: i32,
    pub scale: f32,
    /// Bumped whenever the render target is recreated; bitmaps from an older
    /// generation must be recreated.
    pub generation: u64,
}

impl Gfx {
    pub fn new(pw: i32, ph: i32, scale: f32) -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let rt = Self::make_rt(&d2d)?;
            let brush = rt.CreateSolidColorBrush(&col(WHITE), None)?;
            let round = d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            let dash = d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_CUSTOM,
                    dashOffset: 0.0,
                },
                Some(&[3.0, 3.0]),
            )?;

            // Icon font: Segoe Fluent Icons (Win11) or Segoe MDL2 Assets (Win10).
            let mut coll: Option<IDWriteFontCollection> = None;
            dw.GetSystemFontCollection(&mut coll, false)?;
            let mut has_fluent = false;
            if let Some(c) = &coll {
                let mut idx = 0u32;
                let mut exists = BOOL(0);
                let _ = c.FindFamilyName(w!("Segoe Fluent Icons"), &mut idx, &mut exists);
                has_fluent = exists.as_bool();
            }
            let icon_family = if has_fluent { wide0("Segoe Fluent Icons") } else { wide0("Segoe MDL2 Assets") };

            let screen = GetDC(None);
            let mem_dc = CreateCompatibleDC(screen);
            ReleaseDC(None, screen);

            let mut g = Gfx {
                d2d,
                dw,
                rt,
                brush,
                round,
                dash,
                formats: RefCell::new(HashMap::new()),
                text_family: wide0("Segoe UI"),
                icon_family,
                has_fluent,
                alpha: 1.0,
                mem_dc,
                bmp: HBITMAP::default(),
                old: HGDIOBJ::default(),
                pw: 0,
                ph: 0,
                scale,
                generation: 1,
            };
            g.resize(pw, ph, scale)?;
            Ok(g)
        }
    }

    unsafe fn make_rt(d2d: &ID2D1Factory) -> Result<ID2D1DCRenderTarget> {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = d2d.CreateDCRenderTarget(&props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        Ok(rt)
    }

    pub fn resize(&mut self, pw: i32, ph: i32, scale: f32) -> Result<()> {
        unsafe {
            if !self.bmp.is_invalid() {
                SelectObject(self.mem_dc, self.old);
                let _ = DeleteObject(self.bmp);
            }
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: pw,
                    biHeight: -ph,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            self.bmp = CreateDIBSection(self.mem_dc, &bi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            self.old = SelectObject(self.mem_dc, self.bmp);
            self.pw = pw;
            self.ph = ph;
            self.scale = scale;
            self.formats.borrow_mut().clear();
        }
        Ok(())
    }

    pub fn begin(&self) -> Result<()> {
        unsafe {
            let rc = RECT { left: 0, top: 0, right: self.pw, bottom: self.ph };
            self.rt.BindDC(self.mem_dc, &rc)?;
            self.rt.SetDpi(96.0 * self.scale, 96.0 * self.scale);
            self.rt.BeginDraw();
            self.rt.Clear(Some(&col(Color(0.0, 0.0, 0.0, 0.0))));
            self.rt.SetTransform(&identity());
        }
        Ok(())
    }

    /// Finishes the frame. Returns false when the target had to be recreated
    /// (caller should redraw).
    pub fn end(&mut self) -> bool {
        unsafe {
            match self.rt.EndDraw(None, None) {
                Ok(()) => true,
                Err(e) if e.code() == D2DERR_RECREATE_TARGET => {
                    if let Ok(rt) = Self::make_rt(&self.d2d) {
                        if let Ok(b) = rt.CreateSolidColorBrush(&col(WHITE), None) {
                            self.rt = rt;
                            self.brush = b;
                            self.generation += 1;
                        }
                    }
                    false
                }
                Err(_) => true,
            }
        }
    }

    pub fn present(&self, hwnd: HWND, x: i32, y: i32) {
        unsafe {
            let dst = POINT { x, y };
            let src = POINT { x: 0, y: 0 };
            let size = SIZE { cx: self.pw, cy: self.ph };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                None,
                Some(&dst),
                Some(&size),
                self.mem_dc,
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }

    // ---------------------------------------------------------------- state

    pub fn translate(&self, dx: f32, dy: f32) {
        unsafe {
            let mut m = identity();
            m.M31 = dx;
            m.M32 = dy;
            self.rt.SetTransform(&m);
        }
    }

    pub fn reset_transform(&self) {
        unsafe { self.rt.SetTransform(&identity()) }
    }

    pub fn clip(&self, r: R) {
        unsafe { self.rt.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) }
    }

    pub fn unclip(&self) {
        unsafe { self.rt.PopAxisAlignedClip() }
    }

    fn set(&self, c: Color) -> &ID2D1SolidColorBrush {
        unsafe {
            self.brush.SetColor(&col(c.a(self.alpha)));
        }
        &self.brush
    }

    // ---------------------------------------------------------------- shapes

    pub fn rect(&self, r: R, c: Color) {
        unsafe { self.rt.FillRectangle(&r.d2d(), self.set(c)) }
    }

    pub fn rrect(&self, r: R, rad: f32, c: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.set(c)) }
    }

    pub fn rrect_stroke(&self, r: R, rad: f32, c: Color, width: f32) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.rt.DrawRoundedRectangle(&rr, self.set(c), width, None) }
    }

    pub fn rrect_dashed(&self, r: R, rad: f32, c: Color, width: f32) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.rt.DrawRoundedRectangle(&rr, self.set(c), width, &self.dash) }
    }

    pub fn circle(&self, cx: f32, cy: f32, rad: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: pt(cx, cy), radiusX: rad, radiusY: rad };
        unsafe { self.rt.FillEllipse(&e, self.set(c)) }
    }

    pub fn ring(&self, cx: f32, cy: f32, rad: f32, width: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: pt(cx, cy), radiusX: rad, radiusY: rad };
        unsafe { self.rt.DrawEllipse(&e, self.set(c), width, None) }
    }

    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: Color) {
        unsafe { self.rt.DrawLine(pt(x0, y0), pt(x1, y1), self.set(c), width, &self.round) }
    }

    /// Progress arc starting at 12 o'clock, clockwise. `frac` in 0..=1.
    pub fn arc(&self, cx: f32, cy: f32, rad: f32, width: f32, frac: f32, c: Color) {
        let frac = frac.clamp(0.0, 1.0);
        if frac <= 0.001 {
            return;
        }
        if frac >= 0.999 {
            self.ring(cx, cy, rad, width, c);
            return;
        }
        let a = frac * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let end = pt(cx + rad * a.cos(), cy + rad * a.sin());
        unsafe {
            let Ok(path) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            sink.BeginFigure(pt(cx, cy - rad), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: end,
                size: D2D_SIZE_F { width: rad, height: rad },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if frac > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            self.rt.DrawGeometry(&path, self.set(c), width, &self.round);
        }
    }

    fn notch_path(&self, cx: f32, w: f32, h: f32, r: f32, ear: f32) -> Option<ID2D1PathGeometry> {
        let x0 = cx - w / 2.0;
        let x1 = cx + w / 2.0;
        let r = r.min(h - ear).min(w / 2.0).max(0.0);
        unsafe {
            let path = self.d2d.CreatePathGeometry().ok()?;
            let sink = path.Open().ok()?;
            sink.BeginFigure(pt(x0 - ear, 0.0), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(pt(x1 + ear, 0.0));
            sink.AddArc(&arc_seg(x1, ear, ear, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
            sink.AddLine(pt(x1, h - r));
            sink.AddArc(&arc_seg(x1 - r, h, r, D2D1_SWEEP_DIRECTION_CLOCKWISE));
            sink.AddLine(pt(x0 + r, h));
            sink.AddArc(&arc_seg(x0, h - r, r, D2D1_SWEEP_DIRECTION_CLOCKWISE));
            sink.AddLine(pt(x0, ear));
            sink.AddArc(&arc_seg(x0 - ear, 0.0, ear, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close().ok()?;
            Some(path)
        }
    }

    /// The island silhouette: flush with the top edge, rounded bottom corners
    /// and concave "ears" that melt into the screen edge.
    pub fn notch(&self, cx: f32, w: f32, h: f32, r: f32, ear: f32, c: Color) {
        let Some(path) = self.notch_path(cx, w, h, r, ear) else { return };
        unsafe { self.rt.FillGeometry(&path, self.set(c), None) }
    }

    /// Vertical tint over the island: transparent at `y0`, `c` at the bottom.
    pub fn notch_tint(&self, cx: f32, w: f32, h: f32, r: f32, ear: f32, y0: f32, c: Color) {
        let Some(path) = self.notch_path(cx, w, h, r, ear) else { return };
        let a = self.alpha;
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color: col(c.a(0.0)) },
            D2D1_GRADIENT_STOP { position: 1.0, color: col(c.a(a)) },
        ];
        unsafe {
            let Ok(coll) = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(cx, y0), endPoint: pt(cx, h) };
            let Ok(brush) = self.rt.CreateLinearGradientBrush(&props, None, &coll) else { return };
            self.rt.FillGeometry(&path, &brush, None);
        }
    }

    /// Simple water drop (Segoe icon fonts don't have a good one).
    pub fn drop_icon(&self, cx: f32, cy: f32, size: f32, c: Color) {
        let h = size;
        let w = size * 0.72;
        let top = cy - h / 2.0;
        let rad = w / 2.0;
        let ccy = cy + h / 2.0 - rad;
        unsafe {
            let Ok(path) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            sink.BeginFigure(pt(cx, top), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: pt(cx + rad * 0.2, top + h * 0.2),
                point2: pt(cx + rad, ccy - rad * 0.5),
                point3: pt(cx + rad, ccy),
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: pt(cx - rad, ccy),
                size: D2D_SIZE_F { width: rad, height: rad },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: pt(cx - rad, ccy - rad * 0.5),
                point2: pt(cx - rad * 0.2, top + h * 0.2),
                point3: pt(cx, top),
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            self.rt.FillGeometry(&path, self.set(c), None);
        }
    }

    // ---------------------------------------------------------------- text

    fn format(&self, size: f32, weight: i32, icon: bool) -> Option<IDWriteTextFormat> {
        self.format_ex(size, weight, icon, false)
    }

    fn format_ex(&self, size: f32, weight: i32, icon: bool, wrap: bool) -> Option<IDWriteTextFormat> {
        let key = ((size * 10.0) as u32, weight, icon, wrap);
        if let Some(f) = self.formats.borrow().get(&key) {
            return Some(f.clone());
        }
        unsafe {
            let fam = if icon { &self.icon_family } else { &self.text_family };
            let f = self
                .dw
                .CreateTextFormat(
                    PCWSTR(fam.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT(weight),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!(""),
                )
                .ok()?;
            if wrap {
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
                self.formats.borrow_mut().insert(key, f.clone());
                return Some(f);
            }
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = f.SetTrimming(&trim, &sign);
            }
            self.formats.borrow_mut().insert(key, f.clone());
            Some(f)
        }
    }

    /// Single-line text, vertically centred in `r`, trimmed with an ellipsis.
    pub fn text(&self, s: &str, r: R, size: f32, weight: i32, c: Color, align: Align) {
        if s.is_empty() {
            return;
        }
        let Some(f) = self.format(size, weight, false) else { return };
        unsafe {
            let _ = f.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            self.rt.DrawText(
                &wide(s),
                &f,
                &r.d2d(),
                self.set(c),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// Multi-line text, top-aligned in `r`, wrapped at word boundaries.
    pub fn text_wrap(&self, s: &str, r: R, size: f32, weight: i32, c: Color, align: Align) {
        if s.is_empty() {
            return;
        }
        let Some(f) = self.format_ex(size, weight, false, true) else { return };
        unsafe {
            let _ = f.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            self.rt.DrawText(&wide(s), &f, &r.d2d(), self.set(c), D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    /// Height of `s` wrapped to `width`.
    pub fn measure_wrap(&self, s: &str, width: f32, size: f32, weight: i32) -> f32 {
        let s = if s.is_empty() { " " } else { s };
        let Some(f) = self.format_ex(size, weight, false, true) else { return size * 1.3 };
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&wide(s), &f, width, 4000.0) else { return size * 1.3 };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_ok() {
                m.height
            } else {
                size * 1.3
            }
        }
    }

    pub fn measure(&self, s: &str, size: f32, weight: i32) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let Some(f) = self.format(size, weight, false) else { return 0.0 };
        unsafe {
            let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            let Ok(layout) = self.dw.CreateTextLayout(&wide(s), &f, 4000.0, 200.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_ok() {
                m.widthIncludingTrailingWhitespace
            } else {
                0.0
            }
        }
    }

    /// Draws an icon glyph from Segoe Fluent Icons / MDL2 Assets centred at (cx, cy).
    pub fn icon(&self, glyph: char, cx: f32, cy: f32, size: f32, c: Color) {
        let Some(f) = self.format(size, 400, true) else { return };
        let mut buf = [0u16; 2];
        let s = glyph.encode_utf16(&mut buf);
        let r = R::new(cx - size, cy - size, size * 2.0, size * 2.0);
        unsafe {
            let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            self.rt.DrawText(s, &f, &r.d2d(), self.set(c), D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    // ---------------------------------------------------------------- bitmaps

    /// Creates a bitmap from premultiplied BGRA pixels.
    pub fn bitmap(&self, bgra: &[u8], w: u32, h: u32) -> Option<ID2D1Bitmap> {
        if bgra.len() < (w * h * 4) as usize {
            return None;
        }
        unsafe {
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            self.rt
                .CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(bgra.as_ptr() as *const _), w * 4, &props)
                .ok()
        }
    }

    /// Draws a bitmap into a rounded rect.
    pub fn image(&self, bmp: &ID2D1Bitmap, r: R, rad: f32) {
        unsafe {
            let sz = bmp.GetSize();
            if sz.width <= 0.0 || sz.height <= 0.0 {
                return;
            }
            let Ok(brush) = self.rt.CreateBitmapBrush(bmp, None, None) else { return };
            let mut m = identity();
            m.M11 = r.w / sz.width;
            m.M22 = r.h / sz.height;
            m.M31 = r.x;
            m.M32 = r.y;
            brush.SetTransform(&m);
            brush.SetOpacity(self.alpha);
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            self.rt.FillRoundedRectangle(&rr, &brush);
        }
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        unsafe {
            if !self.bmp.is_invalid() {
                SelectObject(self.mem_dc, self.old);
                let _ = DeleteObject(self.bmp);
            }
            let _ = DeleteDC(self.mem_dc);
        }
    }
}

fn identity() -> Matrix3x2 {
    Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 }
}

fn arc_seg(x: f32, y: f32, r: f32, dir: D2D1_SWEEP_DIRECTION) -> D2D1_ARC_SEGMENT {
    D2D1_ARC_SEGMENT {
        point: pt(x, y),
        size: D2D_SIZE_F { width: r, height: r },
        rotationAngle: 0.0,
        sweepDirection: dir,
        arcSize: D2D1_ARC_SIZE_SMALL,
    }
}

/// Builds a 32x32 tray icon procedurally (black pill with a soft white dot).
pub fn make_tray_icon() -> HICON {
    const S: i32 = 32;
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: S,
                biHeight: -S,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(color) = CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
        };
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (S * S) as usize);
        // pill: x 2..30, y 9..23, radius 7
        for y in 0..S {
            for x in 0..S {
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                let (l, r, t, b, rad) = (2.0f32, 30.0f32, 9.0f32, 23.0f32, 7.0f32);
                let cx = fx.clamp(l + rad, r - rad);
                let cy = fy.clamp(t + rad, b - rad);
                let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                let a = (rad - d + 0.5).clamp(0.0, 1.0);
                let mut rgb = 0.07f32;
                // white dot on the right
                let dd = ((fx - 22.0).powi(2) + (fy - 16.0).powi(2)).sqrt();
                let dot = (3.2 - dd + 0.5).clamp(0.0, 1.0);
                rgb = rgb * (1.0 - dot) + 1.0 * dot;
                let ai = (a * 255.0) as u32;
                let ci = (rgb * a * 255.0) as u32;
                px[(y * S + x) as usize] = (ai << 24) | (ci << 16) | (ci << 8) | ci;
            }
        }
        let mask = CreateBitmap(S, S, 1, 1, None);
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(mask);
        let _ = DeleteObject(color);
        icon
    }
}
