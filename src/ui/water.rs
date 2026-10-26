//! Water reminder: intake, evenly spread reminders over the active hours,
//! bottle refill prompt.

use crate::gfx::{self, Align, Gfx, R};
use crate::store;

use super::widgets::{self, Hits};
use super::{Act, AlertIcon, App, Hit};

fn liters(ml: u32) -> String {
    let l = ml as f32 / 1000.0;
    if (l * 10.0).fract().abs() < 0.01 {
        format!("{:.1}", l)
    } else {
        format!("{:.2}", l)
    }
}

impl App {
    /// Reminder times (minute of day) spread evenly over the active hours.
    fn water_slots(&self) -> Vec<i32> {
        let from = self.s.water_from.clamp(0, 23) * 60;
        let until = self.s.water_until.clamp(1, 24) * 60;
        if until <= from {
            return Vec::new();
        }
        let glass = self.s.water_glass.max(50);
        let n = ((self.s.water_goal + glass - 1) / glass).max(1) as i32;
        let step = (until - from) as f32 / n as f32;
        (0..n).map(|i| from + (step * (i as f32 + 0.5)).round() as i32).collect()
    }

    fn water_next(&self) -> Option<i32> {
        if self.water.taken >= self.s.water_goal || !self.s.water_on {
            return None;
        }
        let now = store::now().minute;
        if self.water_snoozed && self.water.snooze_until > now {
            return Some(self.water.snooze_until);
        }
        self.water_slots().into_iter().enumerate().find(|(i, t)| *i as i32 > self.water.fired_slot && *t > now).map(|(_, t)| t)
    }

    pub(super) fn draw_water(&self, g: &Gfx, hits: &mut Hits) {
        if self.water_settings {
            self.draw_water_settings(g, hits);
            return;
        }
        let goal = self.s.water_goal.max(1);
        let taken = self.water.taken;
        let frac = (taken as f32 / goal as f32).min(1.0);

        let amount = format!("{} / {} L", liters(taken), liters(goal));
        let aw = g.measure(&amount, 20.0, 700);
        g.text(&amount, R::new(44.0, 32.0, aw + 2.0, 30.0), 20.0, 700, gfx::WHITE, Align::Left);
        let sx = (44.0 + aw + 12.0).min(250.0);
        widgets::switch(g, sx, 39.0, self.s.water_on, gfx::BLUE);
        hits.add(R::new(sx - 2.0, 35.0, 32.0, 24.0), Hit::WaterReminders);

        let next = match self.water_next() {
            Some(t) => format!("Next {}", store::hhmm(t)),
            None if taken >= goal => "Goal reached".into(),
            None if !self.s.water_on => "Reminders off".into(),
            None => "No more reminders".into(),
        };
        let line = format!("Today · {}% · {}", (frac * 100.0).round() as i32, next);
        g.text(&line, R::new(44.0, 60.0, 260.0, 16.0), 11.0, 400, gfx::GRAY, Align::Left);
        widgets::bar(g, 44.0, 84.0, 256.0, 4.0, frac, gfx::BLUE);

        // buttons
        let y = 96.0;
        let dr = R::new(44.0, y, 84.0, 26.0);
        g.rrect(dr, 13.0, gfx::BLUE);
        g.drop_icon(dr.x + 22.0, dr.cy(), 12.0, gfx::WHITE);
        g.text("Drink", R::new(dr.x + 32.0, y, 48.0, 26.0), 12.0, 600, gfx::WHITE, Align::Left);
        hits.add(dr, Hit::WaterDrink);
        let st = R::new(136.0, y, 92.0, 26.0);
        widgets::chip(g, st, "Settings", Some('\u{E713}'), gfx::CHIP, gfx::WHITE, 11.5);
        hits.add(st, Hit::WaterSettings);

        // gauge
        let (cx, cy) = (346.0, 76.0);
        g.ring(cx, cy, 30.0, 6.0, gfx::BLUE.a(0.2));
        g.arc(cx, cy, 30.0, 6.0, frac, gfx::BLUE);
        g.drop_icon(cx, cy, 20.0, gfx::BLUE);
    }

    fn draw_water_settings(&self, g: &Gfx, hits: &mut Hits) {
        let label = |g: &Gfx, t: &str, y: f32| g.text(t, R::new(44.0, y, 60.0, 22.0), 11.0, 600, gfx::GRAY, Align::Left);
        let row = |g: &Gfx, hits: &mut Hits, y: f32, opts: &[u32], cur: u32, mk: fn(u32) -> Hit| {
            let mut x = 104.0;
            for &v in opts {
                let t = format!("{} L", liters(v));
                let w = g.measure(&t, 11.0, 600) + 20.0;
                let on = v == cur;
                widgets::chip(
                    g,
                    R::new(x, y, w, 22.0),
                    &t,
                    None,
                    if on { gfx::BLUE } else { gfx::CHIP },
                    if on { gfx::WHITE } else { gfx::GRAY },
                    11.0,
                );
                hits.add(R::new(x, y, w, 22.0), mk(v));
                x += w + 6.0;
            }
        };
        label(g, "Goal", 34.0);
        row(g, hits, 34.0, &[1500, 2000, 2500, 3000], self.s.water_goal, Hit::WaterGoal);
        label(g, "Bottle", 62.0);
        row(g, hits, 62.0, &[500, 750, 1000, 1500], self.s.water_bottle, Hit::WaterBottle);
        label(g, "Hours", 90.0);
        let from = format!("From {:02}:00", self.s.water_from);
        let until = format!("Until {:02}:00", self.s.water_until.min(24));
        let fw = g.measure(&from, 11.0, 600) + 20.0;
        let uw = g.measure(&until, 11.0, 600) + 20.0;
        widgets::chip(g, R::new(104.0, 90.0, fw, 22.0), &from, None, gfx::CHIP, gfx::WHITE, 11.0);
        hits.add(R::new(104.0, 90.0, fw, 22.0), Hit::WaterFrom);
        widgets::chip(g, R::new(110.0 + fw, 90.0, uw, 22.0), &until, None, gfx::CHIP, gfx::WHITE, 11.0);
        hits.add(R::new(110.0 + fw, 90.0, uw, 22.0), Hit::WaterUntil);
        let done = R::new(318.0, 90.0, 60.0, 22.0);
        widgets::chip(g, done, "Done", None, gfx::WHITE, gfx::BLACK, 11.0);
        hits.add(done, Hit::WaterDone);
    }

    pub(super) fn drink(&mut self) {
        let today = store::now();
        if self.water.day != today.day {
            self.water = store::load_water(today.day);
        }
        let glass = self.s.water_glass.max(50);
        self.water.taken += glass;
        self.water.since_refill += glass;
        let h = (today.minute / 60).clamp(0, 23) as usize;
        if self.water.hours.len() == 24 {
            self.water.hours[h] += glass;
        }
        // drinking counts for every reminder that is already due
        let now = today.minute;
        let due = self.water_slots().iter().filter(|t| **t <= now).count() as i32 - 1;
        self.water.fired_slot = self.water.fired_slot.max(due);
        self.water_snoozed = false;
        store::save_water(&self.water);
        if self.water.since_refill >= self.s.water_bottle.max(100) {
            self.alert(
                AlertIcon::Drop,
                gfx::BLUE,
                "Time to Refill",
                "Your bottle is probably empty",
                vec![("Refilled".into(), Act::WaterRefilled)],
            );
        } else {
            self.kick();
        }
    }

    pub(super) fn water_snooze(&mut self) {
        let now = store::now().minute;
        self.water.snooze_until = now + self.s.snooze.max(1) as i32;
        self.water_snoozed = true;
        self.kick();
    }

    pub(super) fn water_refilled(&mut self) {
        self.water.since_refill = 0;
        store::save_water(&self.water);
        self.kick();
    }

    pub(super) fn check_water(&mut self) {
        let now = store::now();
        if self.water.day != now.day {
            self.water = store::load_water(now.day);
            self.water_snoozed = false;
        }
        if !self.s.water_on || self.water.taken >= self.s.water_goal {
            return;
        }
        if self.alerts.iter().any(|a| a.title == "Time to Drink") {
            return;
        }
        if self.water_snoozed {
            if now.minute >= self.water.snooze_until {
                self.water_snoozed = false;
                self.drink_alert();
            }
            return;
        }
        let slots = self.water_slots();
        let due = slots.iter().filter(|t| **t <= now.minute).count() as i32 - 1;
        if due > self.water.fired_slot {
            self.water.fired_slot = due;
            store::save_water(&self.water);
            // only remind if the user is actually behind schedule
            let expected = (due as u32 + 1) * self.s.water_glass.max(50);
            if self.water.taken < expected {
                self.drink_alert();
            }
        }
    }

    fn drink_alert(&mut self) {
        let sub = format!("{} ml · {} / {} L", self.s.water_glass, liters(self.water.taken), liters(self.s.water_goal));
        self.alert(
            AlertIcon::Drop,
            gfx::BLUE,
            "Time to Drink",
            &sub,
            vec![("Snooze".into(), Act::WaterSnooze), ("Drank".into(), Act::WaterDrank)],
        );
    }
}
