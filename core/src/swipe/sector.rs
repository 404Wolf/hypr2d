use crate::grid::Direction;
use serde::Serialize;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Sector {
    Right,
    DownRight,
    Down,
    DownLeft,
    Left,
    UpLeft,
    Up,
    UpRight,
}
impl Sector {
    const ALL: [Self; 8] = [
        Self::Right,
        Self::DownRight,
        Self::Down,
        Self::DownLeft,
        Self::Left,
        Self::UpLeft,
        Self::Up,
        Self::UpRight,
    ];
    fn center(self) -> f64 {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as f64 * 45.
    }
    fn width(self, diagonal: f64) -> f64 {
        if self.diagonal() {
            diagonal
        } else {
            45. - diagonal
        }
    }
    pub fn diagonal(self) -> bool {
        matches!(
            self,
            Self::DownRight | Self::DownLeft | Self::UpLeft | Self::UpRight
        )
    }
    pub fn direction(self) -> Direction {
        match self {
            Self::Right => Direction::Right,
            Self::DownRight => Direction::DownRight,
            Self::Down => Direction::Down,
            Self::DownLeft => Direction::DownLeft,
            Self::Left => Direction::Left,
            Self::UpLeft => Direction::UpLeft,
            Self::Up => Direction::Up,
            Self::UpRight => Direction::UpRight,
        }
    }
    pub fn axes(self) -> [bool; 2] {
        let (x, y) = self.direction().delta();
        [x != 0, y != 0]
    }
    fn difference(self, angle: f64) -> f64 {
        (angle - self.center() + 180.).rem_euclid(360.) - 180.
    }
    pub fn classify(angle: f64, diagonal: f64) -> Self {
        Self::ALL
            .into_iter()
            .find(|s| s.difference(angle).abs() <= s.width(diagonal) + 1e-9)
            .unwrap_or(Self::Right)
    }
    pub fn margin(self, angle: f64, diagonal: f64) -> f64 {
        self.width(diagonal) - self.difference(angle).abs()
    }
}
#[derive(Debug, Default)]
pub struct Lock {
    pub selected: Option<Sector>,
    pending: Option<(Sector, f64)>,
}
impl Lock {
    pub fn update(
        &mut self,
        vector: [f64; 2],
        distance: f64,
        travel: f64,
        config: super::Config,
    ) -> Option<Sector> {
        if distance <= config.axis_lock || vector[0].hypot(vector[1]) < 1e-12 {
            return self.selected;
        }
        let angle = vector[1].atan2(vector[0]).to_degrees();
        let candidate = Sector::classify(angle, config.diagonal_width);
        if self.selected.is_none() {
            self.selected = Some(candidate);
            return self.selected;
        }
        if self.selected == Some(candidate)
            || candidate.margin(angle, config.diagonal_width) < config.hysteresis
        {
            self.pending = None;
            return self.selected;
        }
        let accumulated = match self.pending {
            Some((sector, d)) if sector == candidate => d + travel,
            _ => travel,
        };
        if accumulated >= config.hysteresis_distance {
            self.selected = Some(candidate);
            self.pending = None;
        } else {
            self.pending = Some((candidate, accumulated));
        }
        self.selected
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn vector(angle: f64) -> [f64; 2] {
        let a = angle.to_radians();
        [a.cos() * 100., a.sin() * 100.]
    }
    #[test]
    fn wider_cardinals_and_boundary_hysteresis_require_sustained_distance() {
        for (angle, want) in [
            (0., Sector::Right),
            (29., Sector::Right),
            (31., Sector::DownRight),
            (59., Sector::DownRight),
            (61., Sector::Down),
            (179., Sector::Left),
            (-45., Sector::UpRight),
        ] {
            assert_eq!(Sector::classify(angle, 15.), want);
        }
        let mut lock = Lock::default();
        let c = super::super::Config::default();
        assert_eq!(lock.update(vector(0.), 4., 4., c), None);
        assert_eq!(lock.update(vector(0.), 100., 20., c), Some(Sector::Right));
        for angle in [31., 35., 37., 34., 36.] {
            assert_eq!(
                lock.update(vector(angle), 100., 30., c),
                Some(Sector::Right)
            );
        }
        assert_eq!(lock.update(vector(40.), 100., 5., c), Some(Sector::Right));
        assert_eq!(
            lock.update(vector(40.), 100., 8., c),
            Some(Sector::DownRight)
        );
        assert_eq!(
            lock.update(vector(61.), 100., 50., c),
            Some(Sector::DownRight)
        );
        assert_eq!(lock.update(vector(80.), 100., 20., c), Some(Sector::Down));
    }
}
