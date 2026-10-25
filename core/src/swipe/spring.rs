#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub start: f64,
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub velocity: [f64; 2],
    pub omega: f64,
}
impl Spring {
    pub fn deadline(self) -> f64 {
        (18000. / self.omega).max(super::MAX_SETTLE_MS)
    }
    pub fn sample(self, now: f64) -> ([f64; 2], [f64; 2]) {
        if now - self.start >= self.deadline() {
            return (self.to, [0., 0.]);
        }
        let t = ((now - self.start) / 1000.).max(0.);
        let decay = (-self.omega * t).exp();
        let mut pos = [0.; 2];
        let mut vel = [0.; 2];
        for i in 0..2 {
            let d = self.from[i] - self.to[i];
            let b = self.velocity[i] + self.omega * d;
            pos[i] = self.to[i] + (d + b * t) * decay;
            vel[i] = (b - self.omega * (d + b * t)) * decay;
        }
        (pos, vel)
    }
    pub fn finished(self, now: f64) -> bool {
        let (p, v) = self.sample(now);
        now - self.start >= self.deadline()
            || (0..2).all(|i| (p[i] - self.to[i]).abs() < 0.25 && v[i].abs() < 5.)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analytic_two_dimensional_spring_is_position_and_velocity_continuous() {
        let s = Spring {
            start: 10.,
            from: [200., -75.],
            to: [-1280., 720.],
            velocity: [-800., 300.],
            omega: 24.,
        };
        let (p, v) = s.sample(10.);
        assert_eq!(p, s.from);
        assert_eq!(v, s.velocity);
        let (next, _) = s.sample(10.001);
        for i in 0..2 {
            assert!(((next[i] - p[i]) / 0.000001 - v[i]).abs() < 1.);
        }
        assert!(s.finished(10. + s.deadline()));
        assert_eq!(s.sample(10. + s.deadline()), (s.to, [0., 0.]));
    }
}
