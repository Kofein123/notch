//! Shelf page: files and text dropped onto the island, dragged back out later.

use std::time::Instant;

use windows::Win32::UI::WindowsAndMessaging::SetTimer;

use crate::gfx::{self, Align, Gfx, R};
use crate::store::{self, ShelfItem};
use crate::sys;

use super::widgets::{self, Hits};
use super::{App, Hit, T_COPIED};

const TILE_W: f32 = 60.0;
const TILE_GAP: f32 = 6.0;
const VISIBLE: usize = 5;

impl App {
    pub(super) fn draw_shelf(&self, g: &Gfx, hits: &mut Hits) {
        let area = R::new(40.0, 34.0, 340.0, 90.0);
        if self.shelf.is_empty() {
            let c = if self.drag_over { gfx::BLUE } else { gfx::GRAY.a(0.6) };
            g.rrect_dashed(area, 14.0, c, 1.4);
            if self.drag_over {
                g.icon('\u{E896}', area.cx(), 64.0, 18.0, gfx::BLUE);
                g.text("Release to add", R::new(area.x, 78.0, area.w, 20.0), 13.0, 600, gfx::BLUE, Align::Center);
            } else {
                g.text("Drop files here", R::new(area.x, 58.0, area.w, 20.0), 13.0, 600, gfx::WHITE, Align::Center);
                g.text("Drag out anytime · click to open or copy", R::new(area.x, 80.0, area.w, 16.0), 10.5, 400, gfx::GRAY, Align::Center);
            }
            return;
        }

        // header corner: Clear all
        let cr = R::new(316.0, 10.0, 62.0, 20.0);
        widgets::chip(g, cr, "Clear all", None, gfx::CHIP, gfx::GRAY, 10.0);
        hits.add(cr, Hit::ShelfClear);

        let n = self.shelf.len();
        let scroll = self.shelf_scroll.min(n.saturating_sub(VISIBLE));
        let shown = (n - scroll).min(VISIBLE);
        let total = shown as f32 * TILE_W + (shown.saturating_sub(1)) as f32 * TILE_GAP;
        let mut x = 210.0 - total / 2.0;
        for i in scroll..scroll + shown {
            let it = &self.shelf[i];
            let r = R::new(x, 36.0, TILE_W, 70.0);
            g.rrect(r, 12.0, gfx::CARD);
            let (glyph, c) = match it {
                ShelfItem::File(p) => (widgets::file_glyph(p), gfx::WHITE),
                ShelfItem::Text(_) => ('\u{E8C8}', gfx::YELLOW),
            };
            g.icon(glyph, r.cx(), 60.0, 20.0, c);
            let copied = matches!(self.copied, Some((ci, _)) if ci == i);
            if copied {
                g.text("Copied", R::new(r.x + 2.0, 82.0, r.w - 4.0, 16.0), 10.0, 700, gfx::GREEN, Align::Center);
            } else {
                g.text(&it.label(), R::new(r.x + 4.0, 82.0, r.w - 8.0, 16.0), 9.5, 400, gfx::GRAY, Align::Center);
            }
            hits.add(r, Hit::ShelfTile(i));
            x += TILE_W + TILE_GAP;
        }
        if n > VISIBLE {
            let more = format!("{} items · scroll", n);
            g.text(&more, R::new(40.0, 108.0, 340.0, 14.0), 9.5, 400, gfx::DIM, Align::Center);
        }
        if self.drag_over {
            g.rrect_dashed(area, 14.0, gfx::BLUE, 1.4);
            g.rrect(area.inset(2.0), 12.0, gfx::BLACK.a(0.7));
            g.text("Release to add", R::new(area.x, 66.0, area.w, 20.0), 13.0, 600, gfx::BLUE, Align::Center);
        }
    }

    pub(super) fn shelf_add(&mut self, items: Vec<ShelfItem>) {
        if items.is_empty() {
            return;
        }
        for it in items {
            let dup = self.shelf.iter().any(|e| match (e, &it) {
                (ShelfItem::File(a), ShelfItem::File(b)) => a.eq_ignore_ascii_case(b),
                (ShelfItem::Text(a), ShelfItem::Text(b)) => a == b,
                _ => false,
            });
            if !dup {
                self.shelf.push(it);
            }
        }
        self.shelf_scroll = self.shelf.len().saturating_sub(VISIBLE);
        store::save_shelf(&self.shelf);
    }

    pub(super) fn shelf_remove(&mut self, i: usize) {
        if i < self.shelf.len() {
            self.shelf.remove(i);
            store::save_shelf(&self.shelf);
            self.draw();
        }
    }

    pub(super) fn shelf_clear(&mut self) {
        self.shelf.clear();
        self.shelf_scroll = 0;
        store::save_shelf(&self.shelf);
        self.draw();
    }

    pub(super) fn shelf_click(&mut self, i: usize) {
        let Some(it) = self.shelf.get(i).cloned() else { return };
        match it {
            ShelfItem::File(p) => sys::open_path(&p),
            ShelfItem::Text(t) => {
                sys::copy_text(self.hwnd, &t);
                self.copied = Some((i, Instant::now()));
                unsafe {
                    SetTimer(self.hwnd, T_COPIED, 1200, None);
                }
                self.draw();
            }
        }
    }

    /// File to drag out for a tile; text cards become a small .txt file.
    pub(super) fn shelf_drag_path(&self, i: usize) -> Option<String> {
        match self.shelf.get(i)? {
            ShelfItem::File(p) => Some(p.clone()),
            ShelfItem::Text(t) => {
                let dir = store::data_dir().join("cards");
                let _ = std::fs::create_dir_all(&dir);
                let mut name: String = t
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
                    .take(32)
                    .collect::<String>()
                    .trim()
                    .to_string();
                if name.is_empty() {
                    name = "Text".into();
                }
                let path = dir.join(format!("{name}.txt"));
                std::fs::write(&path, t.replace('\n', "\r\n")).ok()?;
                Some(path.to_string_lossy().to_string())
            }
        }
    }

    /// Called after a drag-out finished (the borrow was released for it).
    pub fn after_drag(&mut self, i: usize) {
        if self.s.shelf_clear && i < self.shelf.len() {
            self.shelf.remove(i);
            store::save_shelf(&self.shelf);
        }
        self.kick();
    }
}
