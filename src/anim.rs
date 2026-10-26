//! Tiny spring animation used for every moving part of the island.

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub v: f32,
    pub target: f32,
    vel: f32,
    stiffness: f32,
    damping: f32,
}

impl Spring {
    /// `bounce` 0 = critically damped, ~0.25 = a little overshoot (iOS-like).
    pub fn new(v: f32, stiffness: f32, bounce: f32) -> Spring {
        let damping = 2.0 * stiffness.sqrt() * (1.0 - bounce).clamp(0.05, 1.0);
        Spring { v, target: v, vel: 0.0, stiffness, damping }
    }

    pub fn set(&mut self, t: f32) {
        self.target = t;
    }

    pub fn snap(&mut self, t: f32) {
        self.v = t;
        self.target = t;
        self.vel = 0.0;
    }

    pub fn speed(&mut self, mul: f32) {
        let base_k = self.stiffness;
        let bounce = 1.0 - self.damping / (2.0 * base_k.sqrt());
        self.stiffness = base_k * mul;
        self.damping = 2.0 * self.stiffness.sqrt() * (1.0 - bounce);
    }

    /// Advances the spring; returns true while still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        // sub-step for stability
        let n = ((dt / 0.004).ceil() as i32).clamp(1, 16);
        let h = dt / n as f32;
        for _ in 0..n {
            let f = -self.stiffness * (self.v - self.target) - self.damping * self.vel;
            self.vel += f * h;
            self.v += self.vel * h;
        }
        if (self.v - self.target).abs() < 0.05 && self.vel.abs() < 0.5 {
            self.v = self.target;
            self.vel = 0.0;
            return false;
        }
        true
    }

    pub fn moving(&self) -> bool {
        self.v != self.target || self.vel != 0.0
    }
}

/// Linear fade 0..1 that moves at a fixed speed.
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    pub v: f32,
    pub target: f32,
    pub per_sec: f32,
}

impl Fade {
    pub fn new(v: f32, per_sec: f32) -> Fade {
        Fade { v, target: v, per_sec }
    }
    pub fn step(&mut self, dt: f32) -> bool {
        if self.v == self.target {
            return false;
        }
        let d = self.per_sec * dt;
        if (self.target - self.v).abs() <= d {
            self.v = self.target;
        } else if self.target > self.v {
            self.v += d;
        } else {
            self.v -= d;
        }
        true
    }
}

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}
