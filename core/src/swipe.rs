//! Finger tracking and critically damped release motion, independent of Hyprland.
use crate::grid::{Cell, Direction, Grid};
use serde::Serialize;
pub const MAX_SETTLE_MS: f64 = 600.;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Config {
    pub fingers: u32,
    pub distance: f64,
    pub velocity: f64, // logical px / millisecond
    pub natural: bool,
    pub axis_lock: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            fingers: 3,
            distance: 120.,
            velocity: 0.8,
            natural: true,
            axis_lock: 5.,
        }
    }
}
impl Config {
    pub fn validate(self) -> Result<Self, String> {
        if !(2..=5).contains(&self.fingers)
            || !self.distance.is_finite()
            || self.distance <= 0.
            || !self.velocity.is_finite()
            || self.velocity < 0.
            || !self.axis_lock.is_finite()
            || self.axis_lock <= 0.
            || self.axis_lock > self.distance
        {
            return Err("swipe requires 2..5 fingers, positive finite distance/lock and non-negative finite velocity".into());
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}
#[derive(Clone, Copy, Debug)]
struct Spring {
    start: f64,
    from: f64,
    to: f64,
    velocity: f64,
}
#[derive(Debug)]
pub struct Swipe {
    pub monitor: u32,
    pub slot: u32,
    pub origin: u32,
    grid: Grid,
    cell: Cell,
    size: [f64; 2],
    config: Config,
    total: [f64; 2],
    pub axis: Option<Axis>,
    offset: f64,
    velocity: f64,
    last_time: f64,
    spring: Option<Spring>,
    pub committed: bool,
    pub finished: bool,
    fixed_neighbor: Option<Cell>,
}
impl Swipe {
    pub fn new(
        monitor: u32,
        origin: u32,
        grid: Grid,
        size: [f64; 2],
        config: Config,
        now: f64,
    ) -> Option<Self> {
        if !now.is_finite() || size.iter().any(|v| !v.is_finite() || *v <= 0.) {
            return None;
        }
        Some(Self {
            monitor,
            slot: monitor,
            origin,
            grid,
            cell: grid.cell(origin, monitor)?,
            size,
            config,
            total: [0., 0.],
            axis: None,
            offset: 0.,
            velocity: 0.,
            last_time: now,
            spring: None,
            committed: false,
            finished: false,
            fixed_neighbor: None,
        })
    }
    fn index(&self) -> usize {
        usize::from(self.axis == Some(Axis::Vertical))
    }
    fn length(&self) -> f64 {
        self.size[self.index()]
    }
    fn direction(&self) -> Direction {
        match (self.axis, self.offset < 0.) {
            (Some(Axis::Vertical), true) => Direction::Down,
            (Some(Axis::Vertical), false) => Direction::Up,
            (_, true) => Direction::Right,
            (_, false) => Direction::Left,
        }
    }
    pub fn neighbor(&self) -> Option<Cell> {
        if self.spring.is_some() {
            return self.fixed_neighbor;
        }
        if self.axis.is_none() || self.offset == 0. {
            return None;
        }
        self.grid
            .step(self.cell, self.direction())
            .filter(|c| *c != self.cell)
    }
    pub fn neighbor_id(&self) -> u32 {
        self.neighbor()
            .and_then(|c| self.grid.id(c, self.slot).ok())
            .unwrap_or(0)
    }
    pub fn update(&mut self, delta: [f64; 2], now: f64) {
        if self.spring.is_some()
            || self.finished
            || delta.iter().any(|v| !v.is_finite())
            || !now.is_finite()
            || now < self.last_time
        {
            return;
        }
        let sign = if self.config.natural { 1. } else { -1. };
        for (total, d) in self.total.iter_mut().zip(delta) {
            *total = (*total + d.clamp(-100000., 100000.) * sign).clamp(-100000., 100000.);
        }
        if self.axis.is_none()
            && self.total[0].abs().max(self.total[1].abs()) >= self.config.axis_lock
        {
            self.axis = Some(if self.total[0].abs() >= self.total[1].abs() {
                Axis::Horizontal
            } else {
                Axis::Vertical
            });
        }
        let previous = self.offset;
        self.offset = if self.axis.is_some() {
            self.total[self.index()].clamp(-self.length(), self.length())
        } else {
            0.
        };
        let dt = now - self.last_time;
        if dt > 0. && dt < 200. {
            let sample = (self.offset - previous) / dt;
            self.velocity += (sample - self.velocity) * (1. - (-dt / 30.).exp());
        } else if dt >= 200. {
            self.velocity = 0.;
        }
        self.last_time = now;
    }
    pub fn visual_offset(&self) -> f64 {
        if self.neighbor().is_some() || self.spring.is_some() {
            self.offset
        } else {
            let limit = self.length() * 0.15;
            self.offset.signum() * limit * (1. - (-self.offset.abs() / limit).exp())
        }
    }
    pub fn end(&mut self, now: f64, cancelled: bool) -> Option<(u32, Cell)> {
        if self.spring.is_some() || self.finished || !now.is_finite() {
            return None;
        }
        let velocity = self.velocity * (-(now - self.last_time).max(0.) / 50.).exp();
        let neighbor = self.neighbor();
        let commit = !cancelled
            && neighbor.is_some()
            && (self.offset.abs() >= self.config.distance.min(self.length())
                || (self.offset.abs() >= self.config.axis_lock
                    && velocity.signum() == self.offset.signum()
                    && velocity.abs() >= self.config.velocity));
        let from = self.visual_offset();
        self.fixed_neighbor = neighbor;
        self.committed = commit;
        let to = if commit {
            self.offset.signum() * self.length()
        } else {
            0.
        };
        self.offset = from;
        self.spring = Some(Spring {
            start: now,
            from,
            to,
            velocity: velocity * 1000.,
        });
        if self.axis.is_none() || from.abs() < 0.01 {
            self.finished = true;
        }
        if commit {
            neighbor.and_then(|c| self.grid.id(c, self.slot).ok().map(|id| (id, c)))
        } else {
            None
        }
    }
    pub fn tick(&mut self, now: f64) {
        let Some(s) = self.spring else { return };
        if !now.is_finite() {
            return;
        }
        let t = ((now - s.start) / 1000.).max(0.);
        let displacement = s.from - s.to;
        let b = s.velocity + 24. * displacement;
        let decay = (-24. * t).exp();
        let value = s.to + (displacement + b * t) * decay;
        // A one-cell swipe never displays a third workspace or overshoots an edge.
        self.offset = if s.from < 0. {
            value.clamp(-self.length(), 0.)
        } else {
            value.clamp(0., self.length())
        };
        if t >= MAX_SETTLE_MS / 1000.
            || ((self.offset - s.to).abs() < 0.25
                && (b - 24. * (displacement + b * t)).abs() * decay < 5.)
        {
            self.offset = s.to;
            self.finished = true;
        }
    }
    pub fn offsets(&self) -> ([f64; 2], [f64; 2]) {
        let offset = self.visual_offset();
        let mut current = [0., 0.];
        let mut neighbor = [0., 0.];
        current[self.index()] = offset;
        // Spring direction remains the release direction, even near the origin.
        let side = if let Some(s) = self.spring {
            if s.from < 0. {
                1.
            } else {
                -1.
            }
        } else if offset < 0. {
            1.
        } else {
            -1.
        };
        neighbor[self.index()] = offset + side * self.length();
        (current, neighbor)
    }
    pub fn settling(&self) -> bool {
        self.spring.is_some()
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"monitor":self.monitor,"origin":self.origin,"neighbor":self.neighbor_id(),"axis":self.axis,"offset":self.visual_offset(),"settling":self.spring.is_some(),"committed":self.committed})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn swipe(wrap: bool) -> Swipe {
        Swipe::new(
            0,
            1,
            Grid {
                wrap,
                ..Grid::default()
            },
            [1000., 800.],
            Config::default(),
            0.,
        )
        .unwrap()
    }
    #[test]
    fn tracks_one_to_one_and_locks_axis() {
        let mut s = swipe(true);
        s.update([-2., 1.], 10.);
        assert!(s.axis.is_none());
        s.update([-30., -10.], 30.);
        assert_eq!(s.axis, Some(Axis::Horizontal));
        assert_eq!(s.offsets().0, [-32., 0.]);
        assert_eq!(s.neighbor_id(), 11);
        s.update([50., 200.], 50.);
        assert_eq!(s.offsets().0, [18., 0.]);
        assert_eq!(s.neighbor_id(), 71);
    }
    #[test]
    fn natural_up_is_down_and_inversion() {
        let mut s = swipe(true);
        s.update([0., -130.], 200.);
        assert_eq!(s.neighbor_id(), 81);
        assert_eq!(s.end(400., false).unwrap().0, 81);
        let mut s = swipe(true);
        s.config.natural = false;
        s.update([0., -130.], 200.);
        assert_eq!(s.neighbor_id(), 561);
    }
    #[test]
    fn thresholds_velocity_pause_cancel_and_reversal() {
        let mut s = swipe(true);
        s.update([-50., 0.], 200.);
        assert!(s.end(400., false).is_none());
        let mut s = swipe(true);
        s.update([-80., 0.], 10.);
        assert!(s.end(10., false).is_some());
        let mut s = swipe(true);
        s.update([-80., 0.], 10.);
        assert!(s.end(500., false).is_none());
        let mut s = swipe(true);
        s.update([-200., 0.], 20.);
        assert!(s.end(20., true).is_none());
        let mut s = swipe(true);
        s.update([-100., 0.], 20.);
        s.update([90., 0.], 40.);
        assert!(s.end(40., false).is_none());
    }
    #[test]
    fn edges_rubber_band_and_spring_finish() {
        let mut s = swipe(false);
        s.update([500., 0.], 200.);
        assert_eq!(s.neighbor_id(), 0);
        assert!(s.visual_offset() > 0. && s.visual_offset() < 150.);
        assert!(s.end(400., false).is_none());
        let before = s.visual_offset();
        s.tick(400.);
        assert_eq!(s.visual_offset(), before);
        s.tick(1000.);
        assert!(s.finished);
        assert_eq!(s.visual_offset(), 0.);
        let mut s = swipe(true);
        s.update([-200., 0.], 200.);
        assert!(s.end(400., false).is_some());
        s.tick(1000.);
        assert!(s.finished);
        assert_eq!(s.offsets().1, [0., 0.]);
    }
    #[test]
    fn invalid_samples_and_configs_are_ignored() {
        let mut s = swipe(true);
        s.update([f64::NAN, 1.], 10.);
        assert!(s.axis.is_none());
        s.update([1., 0.], -1.);
        assert!(s.axis.is_none());
        assert!(Config {
            velocity: f64::NAN,
            ..Config::default()
        }
        .validate()
        .is_err());
        assert!(Config {
            fingers: 1,
            ..Config::default()
        }
        .validate()
        .is_err());
    }
}
