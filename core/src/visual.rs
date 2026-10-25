//! Bounded physical-pixel capture planning and small overview motion curves.
pub fn budget_scale(area: f64, bytes: f64) -> f64 {
    (bytes / (area.max(1.) * 4. * 4. / 3.)).sqrt().min(1.)
}
pub fn ease(t: f64) -> f64 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}
pub fn hover(t: f64) -> f64 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_supersampling_on_laptop_nine_by_nine() {
        for (columns, occupied) in [(3., 9.), (9., 81.)] {
            let w = 2256. / columns;
            let h = w / 1.5;
            let area = w * h * 4. * occupied;
            let scale = budget_scale(area, 96. * 1024. * 1024.);
            assert!(area * scale * scale * 4. * 4. / 3. <= 96. * 1024. * 1024. + 1e-6);
            assert!(scale > 0.5, "at least native resolution on this monitor");
        }
    }
    #[test]
    fn motion_is_continuous_bounded_and_settles() {
        for curve in [ease, hover] {
            assert_eq!(curve(-1.), 0.);
            assert_eq!(curve(0.), 0.);
            assert_eq!(curve(1.), 1.);
            assert_eq!(curve(2.), 1.);
            let mut previous = 0.;
            for i in 0..=100 {
                let x = curve(i as f64 / 100.);
                assert!(x >= previous && x - previous < 0.031);
                previous = x;
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct HoverMotion {
    pub from: f64,
    pub to: f64,
    pub start: f64,
}
impl HoverMotion {
    pub fn value(&self, now: f64) -> f64 {
        self.from + (self.to - self.from) * hover((now - self.start) / 120.)
    }
    pub fn active(&self, now: f64) -> bool {
        now < self.start + 120. && self.from != self.to
    }
}
