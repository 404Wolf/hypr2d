//! Sequence ownership is fixed at begin: a pinch that opens the overview must
//! finish natively; a cancellation pinch stays claimed even after it closes.
#[derive(Default)]
pub struct Pinch {
    active: bool,
    claimed: bool,
    fired: bool,
}
impl Pinch {
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn event(
        &mut self,
        phase: u8,
        fingers: u32,
        scale: f64,
        open: bool,
        threshold: f64,
    ) -> (bool, bool) {
        match phase {
            0 => {
                self.active = true;
                self.claimed = open && fingers == 4;
                self.fired = false;
                (self.claimed, false)
            }
            1 => {
                let close = self.active
                    && self.claimed
                    && !self.fired
                    && open
                    && scale.is_finite()
                    && scale > 0.0
                    && (scale - 1.0).abs() >= threshold;
                self.fired |= close;
                (self.active && self.claimed, close)
            }
            2 => {
                let claimed = self.active && self.claimed;
                *self = Self::default();
                (claimed, false)
            }
            _ => (false, false),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owns_whole_cancel_sequence_in_both_directions() {
        for scale in [0.9, 1.1] {
            let mut p = Pinch::default();
            assert_eq!(p.event(0, 4, 1., true, 0.08), (true, false));
            for tiny in [1.02, 0.97, f64::NAN, 0., f64::INFINITY] {
                assert_eq!(p.event(1, 4, tiny, true, 0.08), (true, false));
            }
            assert_eq!(p.event(1, 4, scale, true, 0.08), (true, true));
            assert_eq!(p.event(1, 4, scale, false, 0.08), (true, false));
            assert_eq!(p.event(2, 0, 1., false, 0.08), (true, false));
            assert!(!p.active());
        }
    }
    #[test]
    fn opening_sequence_wrong_fingers_and_configured_threshold() {
        let mut p = Pinch::default();
        assert_eq!(p.event(0, 4, 1., false, 0.08), (false, false));
        assert_eq!(p.event(1, 4, 1.3, true, 0.08), (false, false));
        assert_eq!(p.event(2, 0, 1., true, 0.08), (false, false));
        assert_eq!(p.event(0, 3, 1., true, 0.08), (false, false));
        assert_eq!(p.event(1, 3, 0.5, true, 0.08), (false, false));
        p.event(2, 0, 1., true, 0.08);
        p.event(0, 4, 1., true, 0.2);
        assert_eq!(p.event(1, 4, 1.1, true, 0.2), (true, false));
        assert_eq!(p.event(1, 4, 1.3, true, 0.2), (true, true));
    }
}
