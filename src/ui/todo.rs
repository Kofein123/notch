//! To-do page: tasks per day, quick add with a start time and duration,
//! "Task Time!" / "Time's Up!" reminders checked every 15 s.

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::gfx::{self, Align, Gfx, R};
use crate::store::{self, Task};

use super::widgets::{self, Hits};
use super::{Act, AddForm, AlertIcon, App, Deferred, Hit, T_CARET};

const DURS: [i32; 7] = [15, 30, 45, 60, 90, 120, 180];
const ROWS: usize = 4;

fn day_label(offset: i32, day: i32) -> String {
    match offset {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        _ => {
            let (_, m, d) = store::civil_from_days(day);
            format!("{d:02}.{m:02}")
        }
    }
}

impl App {
    fn todo_selected_day(&self) -> i32 {
        store::now().day + self.todo_day
    }

    /// Indices into `self.tasks` for the selected day, sorted by start time.
    fn todo_indices(&self) -> Vec<usize> {
        let day = self.todo_selected_day();
        let mut v: Vec<usize> = (0..self.tasks.len()).filter(|&i| self.tasks[i].day == day).collect();
        v.sort_by_key(|&i| (self.tasks[i].start.unwrap_or(i32::MAX), i));
        v
    }

    pub(super) fn draw_todo(&self, g: &Gfx, hits: &mut Hits) {
        let idx = self.todo_indices();
        let done = idx.iter().filter(|&&i| self.tasks[i].done).count();
        let day = self.todo_selected_day();

        // left column
        let count = format!("{}/{}", done, idx.len());
        let cw = g.measure(&count, 28.0, 700);
        g.text(&count, R::new(44.0, 32.0, cw + 2.0, 38.0), 28.0, 700, gfx::WHITE, Align::Left);
        let label = day_label(self.todo_day, day);
        let lw = g.measure(&label, 10.5, 600) + 34.0;
        let cal = R::new((44.0 + cw + 10.0).min(196.0 - lw), 40.0, lw, 22.0);
        widgets::chip(g, cal, &label, Some('\u{E787}'), gfx::CHIP, gfx::WHITE, 10.5);
        hits.add(cal, Hit::TodoDay);
        let sub = match self.todo_day {
            0 => "Task Done Today".to_string(),
            1 => "Task Done Tomorrow".to_string(),
            _ => format!("Task Done {}", label),
        };
        g.text(&sub, R::new(44.0, 68.0, 150.0, 16.0), 11.0, 400, gfx::GRAY, Align::Left);

        let y = 96.0;
        let add = R::new(44.0, y, 74.0, 26.0);
        widgets::chip(g, add, "Add Task", Some('\u{E710}'), gfx::GREEN, gfx::BLACK, 10.5);
        hits.add(add, Hit::TodoAdd);
        let ed = R::new(124.0, y, 74.0, 26.0);
        let (el, ebg) = if self.todo_edit { ("Done", gfx::WHITE) } else { ("Edit Task", gfx::CHIP) };
        widgets::chip(g, ed, el, if self.todo_edit { None } else { Some('\u{E70F}') }, ebg, if self.todo_edit { gfx::BLACK } else { gfx::WHITE }, 10.5);
        hits.add(ed, Hit::TodoEdit);

        // right card
        let card = R::new(206.0, 32.0, 180.0, 92.0);
        g.rrect(card, 12.0, gfx::CARD);
        if let Some(f) = &self.add {
            self.draw_add_form(g, hits, f, card);
            return;
        }
        if idx.is_empty() {
            g.text("No tasks.", card, 11.5, 400, gfx::GRAY, Align::Center);
            return;
        }
        let scroll = self.todo_scroll.min(idx.len().saturating_sub(ROWS));
        for (row, &i) in idx.iter().skip(scroll).take(ROWS).enumerate() {
            let t = &self.tasks[i];
            let ry = card.y + 4.0 + row as f32 * 21.0;
            let cy = ry + 10.5;
            if t.done {
                g.circle(218.0, cy, 6.5, gfx::GREEN);
                g.icon('\u{E73E}', 218.0, cy, 8.0, gfx::BLACK);
            } else {
                g.ring(218.0, cy, 6.0, 1.4, gfx::GRAY);
            }
            hits.add(R::new(208.0, ry, 22.0, 21.0), Hit::TodoCheck(i));
            let tc = if t.done { gfx::DIM } else { gfx::WHITE };
            g.text(&t.title, R::new(230.0, ry, 108.0, 21.0), 11.0, 600, tc, Align::Left);
            if self.todo_edit {
                g.icon('\u{E74D}', 370.0, cy, 11.0, gfx::RED);
                hits.add(R::new(356.0, ry, 28.0, 21.0), Hit::TodoDelete(i));
            } else if let Some(s) = t.start {
                g.text(&store::hhmm(s), R::new(336.0, ry, 44.0, 21.0), 10.0, 600, gfx::GRAY, Align::Right);
            }
        }
        if idx.len() > ROWS {
            let more = format!("+{}", idx.len() - ROWS);
            g.text(&more, R::new(card.x, card.y + card.h - 10.0, card.w - 8.0, 10.0), 8.5, 400, gfx::DIM, Align::Right);
        }
    }

    fn draw_add_form(&self, g: &Gfx, hits: &mut Hits, f: &AddForm, card: R) {
        let input = R::new(card.x + 8.0, card.y + 8.0, card.w - 16.0, 24.0);
        g.rrect(input, 8.0, gfx::CHIP_HI);
        let tr = R::new(input.x + 8.0, input.y, input.w - 16.0, input.h);
        if f.text.is_empty() {
            g.text("Task title", tr, 11.0, 400, gfx::DIM, Align::Left);
        } else {
            g.text(&f.text, tr, 11.0, 600, gfx::WHITE, Align::Left);
        }
        if self.caret_on {
            let cx = (tr.x + g.measure(&f.text, 11.0, 600) + 1.0).min(tr.x + tr.w);
            g.line(cx, input.y + 6.0, cx, input.y + input.h - 6.0, 1.2, gfx::WHITE);
        }
        hits.add(input, Hit::TodoInput);

        let y = card.y + 38.0;
        let start = match f.start {
            Some(m) => store::hhmm(m),
            None => "No time".into(),
        };
        let sr = R::new(card.x + 8.0, y, 84.0, 22.0);
        widgets::chip(g, sr, &start, Some('\u{E823}'), gfx::CHIP, gfx::WHITE, 10.5);
        hits.add(sr, Hit::TodoStartChip);
        let dr = R::new(card.x + 98.0, y, 74.0, 22.0);
        widgets::chip(g, dr, &format!("{} min", f.dur), None, gfx::CHIP, gfx::WHITE, 10.5);
        hits.add(dr, Hit::TodoDurChip);

        let y = card.y + 64.0;
        let cr = R::new(card.x + 8.0, y, 78.0, 22.0);
        widgets::chip(g, cr, "Cancel", None, gfx::CHIP, gfx::WHITE, 10.5);
        hits.add(cr, Hit::TodoCancel);
        let sv = R::new(card.x + 94.0, y, 78.0, 22.0);
        widgets::chip(g, sv, "Save", None, gfx::GREEN, gfx::BLACK, 10.5);
        hits.add(sv, Hit::TodoSave);
    }

    pub(super) fn todo_begin_add(&mut self) -> Option<Deferred> {
        self.add = Some(AddForm { text: String::new(), start: None, dur: 30 });
        self.todo_edit = false;
        self.caret_on = true;
        unsafe {
            SetTimer(self.hwnd, T_CARET, 530, None);
        }
        self.draw();
        Some(Deferred::Focus(true))
    }

    pub(super) fn todo_cancel(&mut self) -> Option<Deferred> {
        self.add = None;
        unsafe {
            let _ = KillTimer(self.hwnd, T_CARET);
        }
        self.draw();
        Some(Deferred::Focus(false))
    }

    pub(super) fn todo_save(&mut self) -> Option<Deferred> {
        let form = self.add.take()?;
        let title = form.text.trim().to_string();
        if !title.is_empty() {
            self.tasks.push(Task {
                day: self.todo_selected_day(),
                start: form.start,
                dur: form.dur,
                done: false,
                fired_start: false,
                fired_end: false,
                snooze_until: None,
                title,
            });
            store::save_tasks(&self.tasks);
        }
        unsafe {
            let _ = KillTimer(self.hwnd, T_CARET);
        }
        self.draw();
        Some(Deferred::Focus(false))
    }

    pub(super) fn todo_cycle_start(&mut self) {
        let now = store::now().minute;
        let next_slot = ((now / 30) + 1) * 30;
        let Some(f) = self.add.as_mut() else { return };
        f.start = match f.start {
            None => Some(if self.todo_day == 0 { next_slot } else { 9 * 60 }),
            Some(m) if m + 30 >= 24 * 60 => None,
            Some(m) => Some(m + 30),
        };
        self.draw();
    }

    pub(super) fn todo_cycle_dur(&mut self) {
        let Some(f) = self.add.as_mut() else { return };
        let i = DURS.iter().position(|&d| d == f.dur).unwrap_or(0);
        f.dur = DURS[(i + 1) % DURS.len()];
        self.draw();
    }

    pub(super) fn todo_toggle(&mut self, i: usize) {
        if let Some(t) = self.tasks.get_mut(i) {
            t.done = !t.done;
            store::save_tasks(&self.tasks);
            self.draw();
        }
    }

    pub(super) fn todo_delete(&mut self, i: usize) {
        if i < self.tasks.len() {
            self.tasks.remove(i);
            store::save_tasks(&self.tasks);
            self.draw();
        }
    }

    fn find_today_task(&mut self, title: &str) -> Option<&mut Task> {
        let today = store::now().day;
        self.tasks.iter_mut().find(|t| t.day == today && t.title == title)
    }

    pub(super) fn todo_later(&mut self, title: &str) {
        let snooze = self.s.snooze.max(1) as i32;
        let now = store::now().minute;
        if let Some(t) = self.find_today_task(title) {
            t.fired_start = false;
            t.snooze_until = Some(now + snooze);
        }
        store::save_tasks(&self.tasks);
    }

    pub(super) fn todo_started(&mut self, title: &str) {
        if let Some(t) = self.find_today_task(title) {
            t.fired_start = true;
            t.snooze_until = None;
        }
        store::save_tasks(&self.tasks);
    }

    pub(super) fn todo_mark_done(&mut self, title: &str) {
        if let Some(t) = self.find_today_task(title) {
            t.done = true;
        }
        store::save_tasks(&self.tasks);
        self.draw();
    }

    pub(super) fn check_todo(&mut self) {
        let now = store::now();
        let mut alerts: Vec<(bool, String)> = Vec::new();
        let mut changed = false;
        for t in self.tasks.iter_mut() {
            if t.day != now.day || t.done {
                continue;
            }
            let Some(start) = t.start else { continue };
            if !t.fired_start && now.minute >= start {
                if let Some(s) = t.snooze_until {
                    if now.minute < s {
                        continue;
                    }
                }
                t.fired_start = true;
                changed = true;
                // don't nag about tasks that started long ago (e.g. PC was off)
                if now.minute - start <= 60 || t.snooze_until.is_some() {
                    alerts.push((true, t.title.clone()));
                }
                t.snooze_until = None;
            }
            let end = start + t.dur.max(1);
            if !t.fired_end && now.minute >= end {
                t.fired_end = true;
                changed = true;
                if now.minute - end <= 30 {
                    alerts.push((false, t.title.clone()));
                }
            }
        }
        if changed {
            store::save_tasks(&self.tasks);
        }
        for (is_start, title) in alerts {
            if is_start {
                if self.s.todo_start {
                    self.alert(
                        AlertIcon::Glyph('\u{E823}'),
                        gfx::GREEN,
                        "Task Time!",
                        &title,
                        vec![("Later".into(), Act::TodoLater(title.clone())), ("Start".into(), Act::TodoStart(title.clone()))],
                    );
                }
            } else if self.s.todo_end {
                self.alert(
                    AlertIcon::Glyph('\u{E916}'),
                    gfx::ORANGE,
                    "Time's Up!",
                    &title,
                    vec![("OK".into(), Act::Dismiss), ("Done".into(), Act::TodoDone(title.clone()))],
                );
            }
        }
    }
}
