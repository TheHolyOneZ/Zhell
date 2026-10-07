#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub stiffness: f32,
    pub damping: f32,
}

pub const SOFT: Params = Params { stiffness: 180.0, damping: 24.0 };

pub const DEFAULT: Params = Params { stiffness: 300.0, damping: 30.0 };

pub const SNAPPY: Params = Params { stiffness: 500.0, damping: 35.0 };

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    pub params: Params,
}

impl Spring {
    pub fn new(value: f32, target: f32, params: Params) -> Self {
        Self { value, velocity: 0.0, target, params }
    }

    pub fn step(&mut self, dt: f32, instant: bool) -> bool {
        if instant {
            self.settle();
            return false;
        }

        let mut left = dt.clamp(0.0, 0.1);
        while left > 0.0 {
            let h = left.min(1.0 / 240.0);
            let force = -self.params.stiffness * (self.value - self.target) - self.params.damping * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            left -= h;
        }
        if (self.value - self.target).abs() < 0.001 && self.velocity.abs() < 0.01 {
            self.settle();
            return false;
        }
        true
    }

    pub fn settle(&mut self) {
        self.value = self.target;
        self.velocity = 0.0;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Slide {
    pub x: Spring,
    pub w: Spring,
}

impl Slide {
    pub fn new(x: f32, w: f32) -> Self {
        Self { x: Spring::new(x, x, SNAPPY), w: Spring::new(w, w, SNAPPY) }
    }

    pub fn to(&mut self, x: f32, w: f32) {
        self.x.target = x;
        self.w.target = w;
    }

    pub fn step(&mut self, dt: f32, instant: bool) -> bool {
        let a = self.x.step(dt, instant);
        let b = self.w.step(dt, instant);
        a || b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn springs_settle_on_target() {
        for p in [SOFT, DEFAULT, SNAPPY] {
            let mut s = Spring::new(0.0, 1.0, p);
            let mut t = 0.0;
            while s.step(1.0 / 60.0, false) {
                t += 1.0 / 60.0;
                assert!(t < 2.0, "{p:?} never settled");
            }
            assert_eq!(s.value, 1.0);
        }
    }

    #[test]
    fn default_spring_overshoots_a_little() {
        let mut s = Spring::new(0.0, 1.0, DEFAULT);
        let mut peak: f32 = 0.0;
        while s.step(1.0 / 120.0, false) {
            peak = peak.max(s.value);
        }
        assert!(peak > 1.0 && peak < 1.06, "{peak}");
    }

    #[test]
    fn reduced_motion_jumps() {
        let mut s = Spring::new(0.0, 5.0, SOFT);
        assert!(!s.step(0.016, true));
        assert_eq!(s.value, 5.0);
    }
}
