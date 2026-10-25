//! Eight-sector tracking, inertial projection and continuous 2D critical springs.
mod sector;
mod spring;
use crate::grid::{Cell, Direction, Grid};
pub use sector::Sector;
use serde::Serialize;
use spring::Spring;
pub const MAX_SETTLE_MS: f64 = 750.;
const WINDOW_MS: f64 = 50.;
fn sign(v: f64) -> i64 {
    if v > 1e-9 {
        1
    } else if v < -1e-9 {
        -1
    } else {
        0
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Config {
    pub fingers: u32,
    pub distance: f64,
    pub velocity: f64,
    pub natural: bool,
    pub axis_lock: f64,
    pub diagonal_width: f64,
    pub hysteresis: f64,
    pub hysteresis_distance: f64,
    pub projection_decel: f64,
    pub spring_stiffness: f64,
    pub sensitivity: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            fingers: 3,
            distance: 120.,
            velocity: 0.8,
            natural: true,
            axis_lock: 5.,
            diagonal_width: 15.,
            hysteresis: 8.,
            hysteresis_distance: 12.,
            projection_decel: 0.0006,
            spring_stiffness: 576.,
            sensitivity: 1.,
        }
    }
}
impl Config {
    pub fn validate(self) -> Result<Self, String> {
        let finite = [
            self.distance,
            self.velocity,
            self.axis_lock,
            self.diagonal_width,
            self.hysteresis,
            self.hysteresis_distance,
            self.projection_decel,
            self.spring_stiffness,
            self.sensitivity,
        ]
        .iter()
        .all(|v| v.is_finite());
        if !finite
            || !(2..=5).contains(&self.fingers)
            || self.sensitivity <= 0.
            || self.distance <= 0.
            || self.velocity < 0.
            || self.axis_lock <= 0.
            || self.axis_lock > self.distance
            || self.diagonal_width <= 0.
            || self.diagonal_width >= 22.5
            || self.hysteresis < 0.
            || self.hysteresis >= self.diagonal_width
            || self.hysteresis_distance <= 0.
            || self.projection_decel <= 0.
            || !(1.0..=10000.0).contains(&self.spring_stiffness)
        {
            return Err("invalid swipe settings: finite positive sensitivity and thresholds, diagonal_width 0..22.5, hysteresis below diagonal width, positive projection_decel and spring_stiffness 1..10000 required".into());
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Axis {
    Horizontal,
    Vertical,
    Diagonal,
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
    smoothed: [f64; 2],
    velocity: [f64; 2],
    last_time: f64,
    lock: sector::Lock,
    pub axis: Option<Axis>,
    tracked: [f64; 2],
    tracking_velocity: [f64; 2],
    screen: [f64; 2],
    screen_velocity: [f64; 2],
    bias: Option<Spring>,
    spring: Option<Spring>,
    target: Option<Cell>,
    pub committed: bool,
    pub finished: bool,
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
        if !now.is_finite()
            || size
                .iter()
                .any(|n| !n.is_finite() || *n <= 0. || *n > 100000.)
        {
            return None;
        }
        Some(Self {
            monitor,
            slot: monitor,
            origin,
            grid: grid.validate(monitor).ok()?,
            cell: grid.cell(origin, monitor)?,
            size,
            config: config.validate().ok()?,
            total: [0.; 2],
            smoothed: [0.; 2],
            velocity: [0.; 2],
            last_time: now,
            lock: sector::Lock::default(),
            axis: None,
            tracked: [0.; 2],
            tracking_velocity: [0.; 2],
            screen: [0.; 2],
            screen_velocity: [0.; 2],
            bias: None,
            spring: None,
            target: None,
            committed: false,
            finished: false,
        })
    }
    fn axes(&self) -> [bool; 2] {
        self.lock.selected.map_or([false; 2], Sector::axes)
    }
    fn desired(&self) -> [f64; 2] {
        let axes = self.axes();
        std::array::from_fn(|i| {
            if axes[i] {
                self.total[i].clamp(-self.size[i], self.size[i])
            } else {
                0.
            }
        })
    }
    fn finger_velocity(&self, now: f64) -> [f64; 2] {
        let decay = (-(now - self.last_time).max(0.) / WINDOW_MS).exp();
        self.velocity.map(|v| v * decay)
    }
    fn desired_velocity(&self, now: f64) -> [f64; 2] {
        let v = self.finger_velocity(now);
        let axes = self.axes();
        std::array::from_fn(|i| {
            if axes[i] && self.total[i].abs() < self.size[i] {
                v[i] * 1000.
            } else {
                0.
            }
        })
    }
    fn axis_neighbor(&self, i: usize, offset: f64) -> Option<Cell> {
        let direction = if i == 0 {
            Direction::from_delta(if offset < 0. { 1 } else { -1 }, 0)
        } else {
            Direction::from_delta(0, if offset < 0. { 1 } else { -1 })
        }?;
        self.grid
            .step(self.cell, direction)
            .filter(|c| *c != self.cell)
    }
    fn refresh_tracking(&mut self, now: f64) {
        let desired = self.desired();
        let velocity = self.desired_velocity(now);
        if self.bias.is_some_and(|s| s.finished(now)) {
            self.bias = None;
        }
        let (bias, bias_v) = self.bias.map_or(([0.; 2], [0.; 2]), |s| s.sample(now));
        for i in 0..2 {
            self.tracked[i] = (desired[i] + bias[i]).clamp(-self.size[i], self.size[i]);
            self.tracking_velocity[i] = velocity[i] + bias_v[i];
            if self.tracked[i].abs() > 1e-9 && self.axis_neighbor(i, self.tracked[i]).is_none() {
                let limit = self.size[i] * 0.15;
                let derivative = (-self.tracked[i].abs() / limit).exp();
                self.screen[i] = self.tracked[i].signum() * limit * (1. - derivative);
                self.screen_velocity[i] = self.tracking_velocity[i] * derivative;
            } else {
                self.screen[i] = self.tracked[i];
                self.screen_velocity[i] = self.tracking_velocity[i];
            }
        }
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
        let dt = (now - self.last_time).max(1.);
        let alpha = 1. - (-dt / WINDOW_MS).exp();
        let sign = if self.config.natural { 1. } else { -1. };
        // Gain precedes displacement/velocity estimation and every screen-space
        // threshold. Bound the gained sample as well as its physical input.
        let delta = delta.map(|v| {
            (v.clamp(-100000., 100000.) * sign * self.config.sensitivity).clamp(-100000., 100000.)
        });
        for (i, d) in delta.into_iter().enumerate() {
            self.total[i] = (self.total[i] + d).clamp(-100000., 100000.);
            self.smoothed[i] += (self.total[i] - self.smoothed[i]) * alpha;
            self.velocity[i] += (d / dt - self.velocity[i]) * alpha;
        }
        self.last_time = now;
        self.refresh_tracking(now);
        let previous = self.lock.selected;
        let old_axes = self.axes();
        let old = self.tracked;
        let old_v = self.tracking_velocity;
        let sector = self.lock.update(
            self.smoothed.map(|v| -v),
            self.total[0].hypot(self.total[1]),
            delta[0].hypot(delta[1]),
            self.config,
        );
        self.axis = sector.map(|s| match s.axes() {
            [true, false] => Axis::Horizontal,
            [false, true] => Axis::Vertical,
            _ => Axis::Diagonal,
        });
        if previous.is_some() && old_axes != self.axes() {
            let desired = self.desired();
            let v = self.desired_velocity(now);
            self.bias = Some(Spring {
                start: now,
                from: std::array::from_fn(|i| old[i] - desired[i]),
                to: [0.; 2],
                velocity: std::array::from_fn(|i| old_v[i] - v[i]),
                omega: self.config.spring_stiffness.sqrt() * 1.5,
            });
        }
        self.refresh_tracking(now);
    }
    pub fn visual_offset(&self) -> f64 {
        if self.axis == Some(Axis::Vertical) {
            self.screen[1]
        } else {
            self.screen[0]
        }
    }
    pub fn neighbor(&self) -> Option<Cell> {
        if self.spring.is_some() && self.target.is_some() {
            return self.target;
        }
        let dir = Direction::from_delta(-sign(self.screen[0]), -sign(self.screen[1]))?;
        self.grid.step(self.cell, dir).filter(|c| *c != self.cell)
    }
    pub fn neighbor_id(&self) -> u32 {
        self.neighbor()
            .and_then(|c| self.grid.id(c, self.slot).ok())
            .unwrap_or(0)
    }
    pub fn end(&mut self, now: f64, cancelled: bool) -> Option<(u32, Cell)> {
        if self.spring.is_some() || self.finished || !now.is_finite() || now < self.last_time {
            return None;
        }
        self.refresh_tracking(now);
        let axes = self.axes();
        let velocity = self.finger_velocity(now);
        let speed = velocity[0].hypot(velocity[1]);
        let projection = if speed >= self.config.velocity {
            speed / (2. * self.config.projection_decel)
        } else {
            0.
        };
        let projected: [f64; 2] =
            std::array::from_fn(|i| self.tracked[i] + velocity[i] * projection);
        let step: [i64; 2] = std::array::from_fn(|i| {
            if !cancelled
                && axes[i]
                && projected[i].abs() > self.size[i] * 0.5
                && projected[i].abs() >= self.config.distance.min(self.size[i])
            {
                -projected[i].signum() as i64
            } else {
                0
            }
        });
        self.target = Direction::from_delta(step[0], step[1])
            .and_then(|d| self.grid.step(self.cell, d))
            .filter(|c| *c != self.cell);
        self.committed = self.target.is_some();
        let to = self.target.map_or([0.; 2], |c| {
            [
                if c.x != self.cell.x {
                    -(step[0] as f64) * self.size[0]
                } else {
                    0.
                },
                if c.y != self.cell.y {
                    -(step[1] as f64) * self.size[1]
                } else {
                    0.
                },
            ]
        });
        self.spring = Some(Spring {
            start: now,
            from: self.screen,
            to,
            velocity: self.screen_velocity,
            omega: self.config.spring_stiffness.sqrt(),
        });
        self.bias = None;
        if self.axis.is_none()
            || (0..2).all(|i| self.screen[i].abs() < 0.01 && self.screen_velocity[i].abs() < 1.)
        {
            self.finished = true;
        }
        self.target
            .and_then(|c| self.grid.id(c, self.slot).ok().map(|id| (id, c)))
    }
    pub fn tick(&mut self, now: f64) {
        if !now.is_finite() {
            return;
        }
        if let Some(s) = self.spring {
            let (p, v) = s.sample(now);
            self.screen = std::array::from_fn(|i| p[i].clamp(-self.size[i], self.size[i]));
            self.screen_velocity = v;
            if s.finished(now) {
                self.screen = s.to;
                self.finished = true;
            }
        } else {
            self.refresh_tracking(now);
        }
    }
    pub fn offsets(&self) -> ([f64; 2], [f64; 2]) {
        let neighbor = self.neighbor().map_or(self.screen, |c| {
            let direction = if self.spring.is_some() && self.committed {
                self.spring
                    .map(|s| [-sign(s.to[0]), -sign(s.to[1])])
                    .unwrap_or([0; 2])
            } else {
                [-sign(self.screen[0]), -sign(self.screen[1])]
            };
            [
                self.screen[0]
                    + if c.x != self.cell.x {
                        direction[0] as f64 * self.size[0]
                    } else {
                        0.
                    },
                self.screen[1]
                    + if c.y != self.cell.y {
                        direction[1] as f64 * self.size[1]
                    } else {
                        0.
                    },
            ]
        });
        (self.screen, neighbor)
    }
    /// The quadrant currently on screen, independent of an inertial snap target.
    pub fn views(&self) -> Vec<(u32, [f64; 2])> {
        let mut views = vec![(self.origin, self.screen)];
        let dx = if self.screen[0].abs() > 1e-9 {
            -sign(self.screen[0])
        } else {
            0
        };
        let dy = if self.screen[1].abs() > 1e-9 {
            -sign(self.screen[1])
        } else {
            0
        };
        for (x, y) in [(dx, 0), (0, dy), (dx, dy)] {
            if let Some(c) = Direction::from_delta(x, y).and_then(|d| self.grid.step(self.cell, d))
            {
                if c == self.cell {
                    continue;
                }
                let Some(id) = self.grid.id(c, self.slot).ok() else {
                    continue;
                };
                if views.iter().any(|v| v.0 == id) {
                    continue;
                }
                views.push((
                    id,
                    [
                        self.screen[0]
                            + if c.x != self.cell.x {
                                x as f64 * self.size[0]
                            } else {
                                0.
                            },
                        self.screen[1]
                            + if c.y != self.cell.y {
                                y as f64 * self.size[1]
                            } else {
                                0.
                            },
                    ],
                ));
            }
        }
        views
    }
    pub fn settling(&self) -> bool {
        self.spring.is_some() || self.bias.is_some()
    }
    pub fn deadline(&self) -> f64 {
        self.spring.map_or(MAX_SETTLE_MS, Spring::deadline)
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"monitor":self.monitor,"origin":self.origin,"neighbor":self.neighbor_id(),"axis":self.axis,"sector":self.lock.selected,"offset":self.visual_offset(),"offsets":self.screen,"velocity":self.velocity,"settling":self.spring.is_some(),"retargeting":self.bias.is_some(),"committed":self.committed,"views":self.views().iter().map(|(id,offset)|serde_json::json!({"workspace":id,"offset":offset})).collect::<Vec<_>>()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gesture(wrap: bool) -> Swipe {
        let g = Grid {
            wrap,
            ..Default::default()
        };
        Swipe::new(
            0,
            g.id(Cell { x: 3, y: 3 }, 0).unwrap(),
            g,
            [1000., 800.],
            Config::default(),
            0.,
        )
        .unwrap()
    }
    #[test]
    fn sensitivity_scales_axes_lock_velocity_and_screen_space_commit_thresholds() {
        let make = |sensitivity, size| {
            Swipe::new(
                0,
                271,
                Grid::default(),
                size,
                Config {
                    sensitivity,
                    ..Default::default()
                },
                0.,
            )
            .unwrap()
        };
        let mut base = make(1., [200., 200.]);
        base.update([-100., 0.], 200.);
        assert!(base.end(1000., false).is_none());
        let mut gain = make(1.3, [200., 200.]);
        gain.update([-100., 0.], 200.);
        assert_eq!(gain.offsets().0, [-130., 0.]);
        assert_eq!(gain.end(1000., false).unwrap().1, Cell { x: 4, y: 3 });
        let mut diagonal = make(1.3, [2256., 1504.]);
        diagonal.update([-1000., -1000.], 200.);
        assert_eq!(diagonal.offsets().0, [-1300., -1300.]);
        assert_eq!(diagonal.end(1000., false).unwrap().1, Cell { x: 4, y: 4 });
        let mut locked = make(1.3, [1000., 800.]);
        locked.update([-4., 0.], 20.);
        assert_eq!(locked.axis, Some(Axis::Horizontal));
        let mut unlocked = make(1., [1000., 800.]);
        unlocked.update([-4., 0.], 20.);
        assert_eq!(unlocked.axis, None);
        let mut flick = make(1.3, [1000., 800.]);
        flick.update([-40., 0.], 10.);
        assert!(
            flick.end(10., false).is_some(),
            "gained speed crosses the screen-space velocity threshold"
        );
        let mut slow = make(1., [1000., 800.]);
        slow.update([-40., 0.], 10.);
        assert!(slow.end(10., false).is_none());
        for sensitivity in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(Config {
                sensitivity,
                ..Default::default()
            }
            .validate()
            .is_err());
        }
    }
    #[test]
    fn all_eight_sectors_track_and_snap_with_two_or_four_views() {
        for (d, dx, dy) in [
            (Direction::Left, 700., 0.),
            (Direction::Right, -700., 0.),
            (Direction::Up, 0., 560.),
            (Direction::Down, 0., -560.),
            (Direction::UpLeft, 700., 560.),
            (Direction::UpRight, -700., 560.),
            (Direction::DownLeft, 700., -560.),
            (Direction::DownRight, -700., -560.),
        ] {
            let mut s = gesture(true);
            s.update([dx, dy], 200.);
            assert_eq!(s.offsets().0, [dx, dy]);
            assert_eq!(s.views().len(), if dx != 0. && dy != 0. { 4 } else { 2 });
            let want = s.grid.step(s.cell, d).unwrap();
            assert_eq!(s.end(600., false).unwrap().1, want);
            let p = s.offsets().0;
            s.tick(600.);
            assert_eq!(s.offsets().0, p, "release position continuity");
            s.tick(1600.);
            assert!(s.finished);
            assert_eq!(s.offsets().1, [0., 0.]);
        }
    }
    #[test]
    fn projection_handles_flick_pause_cancel_and_laptop_axis_normalization() {
        let mut slow = gesture(true);
        slow.update([-80., 0.], 200.);
        assert!(slow.end(400., false).is_none());
        let mut flick = gesture(true);
        flick.update([-80., 0.], 10.);
        assert!(flick.end(10., false).is_some());
        let mut paused = gesture(true);
        paused.update([-80., 0.], 10.);
        assert!(paused.end(500., false).is_none());
        let mut cancelled = gesture(true);
        cancelled.update([-700., -560.], 20.);
        assert!(cancelled.end(20., true).is_none());
        let g = Grid::default();
        let mut laptop = Swipe::new(0, 271, g, [2256., 1504.], Config::default(), 0.).unwrap();
        laptop.update([-1000., -1000.], 200.);
        assert_eq!(laptop.axis, Some(Axis::Diagonal));
        assert_eq!(
            laptop.end(1000., false).unwrap().1,
            Cell { x: 3, y: 4 },
            "only the vertical normalized axis crossed its snap threshold"
        );
    }
    #[test]
    fn clamped_diagonal_degrades_to_valid_cardinal_and_resists_each_axis() {
        let g = Grid {
            wrap: false,
            ..Default::default()
        };
        let mut s = Swipe::new(0, 1, g, [1000., 800.], Config::default(), 0.).unwrap();
        s.update([700., -560.], 200.);
        assert!(s.screen[0] > 0. && s.screen[0] < 150.);
        assert_eq!(s.screen[1], -560.);
        assert_eq!(s.views().len(), 2);
        assert_eq!(s.end(600., false).unwrap().1, Cell { x: 0, y: 1 });
        let mut blocked = Swipe::new(0, 1, g, [1000., 800.], Config::default(), 0.).unwrap();
        blocked.update([700., 560.], 200.);
        assert!(blocked.screen[0] < 150. && blocked.screen[1] < 120.);
        assert!(blocked.end(600., false).is_none());
        blocked.tick(1600.);
        assert_eq!(blocked.offsets().0, [0., 0.]);
        assert!(blocked.finished);
    }
    #[test]
    fn retarget_preserves_position_and_release_velocity() {
        let mut s = gesture(true);
        s.update([-200., 0.], 20.);
        let before = s.screen;
        s.update([0., -800.], 220.);
        assert_eq!(s.axis, Some(Axis::Vertical));
        assert_eq!(s.screen, before, "retarget opens an axis without a jump");
        s.tick(240.);
        assert_eq!(s.views().len(), 4);
        let before = s.screen;
        let velocity = s.screen_velocity;
        s.end(240., false);
        s.tick(240.);
        assert_eq!(s.screen, before);
        s.tick(240.001);
        for i in 0..2 {
            assert!(((s.screen[i] - before[i]) / 0.000001 - velocity[i]).abs() < 2.);
        }
        let mut settled = gesture(true);
        settled.update([-200., 0.], 20.);
        settled.update([0., -800.], 220.);
        settled.tick(900.);
        assert_eq!(settled.screen[0], 0.);
        assert_eq!(settled.views().len(), 2);
    }
    #[test]
    fn sloppy_horizontal_is_cardinal_and_inversion_is_consistent() {
        let mut s = gesture(true);
        s.update([-700., -120.], 200.);
        assert_eq!(s.axis, Some(Axis::Horizontal));
        assert_eq!(s.screen, [-700., 0.]);
        assert_eq!(s.views().len(), 2);
        assert_eq!(s.end(600., false).unwrap().1, Cell { x: 4, y: 3 });
        let mut s = gesture(true);
        s.config.natural = false;
        s.update([0., -560.], 200.);
        assert_eq!(s.end(600., false).unwrap().1, Cell { x: 3, y: 2 });
    }
    #[test]
    fn visible_quadrant_covers_corners_even_when_projection_reverses() {
        let mut s = gesture(true);
        s.update([-700., -560.], 20.);
        s.update([600., 500.], 40.);
        assert_eq!(s.end(40., false).unwrap().1, Cell { x: 2, y: 2 });
        for time in (40..900).step_by(5) {
            s.tick(f64::from(time));
            let views = s.views();
            assert!(views.len() <= 4);
            for point in [[0., 0.], [999., 0.], [0., 799.], [999., 799.], [500., 400.]] {
                assert!(
                    views.iter().any(|(_, p)| point[0] >= p[0]
                        && point[0] < p[0] + 1000.
                        && point[1] >= p[1]
                        && point[1] < p[1] + 800.),
                    "uncovered {point:?} at {time}: {views:?}"
                );
            }
        }
    }
    #[test]
    fn invalid_samples_and_settings_do_not_enter_the_renderer() {
        let mut s = gesture(true);
        let initial = s.status();
        s.update([f64::NAN, 1.], 10.);
        s.update([1., 0.], -1.);
        assert_eq!(s.status(), initial);
        for config in [
            Config {
                diagonal_width: 22.5,
                ..Default::default()
            },
            Config {
                hysteresis: 15.,
                ..Default::default()
            },
            Config {
                projection_decel: 0.,
                ..Default::default()
            },
            Config {
                spring_stiffness: f64::NAN,
                ..Default::default()
            },
            Config {
                fingers: 1,
                ..Default::default()
            },
        ] {
            assert!(config.validate().is_err());
        }
    }
}
