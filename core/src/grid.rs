use serde::{Deserialize, Serialize};

/// A named grid encodes a reserved 65536-workspace block, independent of outputs.
pub const VIRTUAL_GRID: u32 = 1 << 30;
pub const GRID_BLOCK: u32 = 1 << 16;
pub fn virtual_base(slot: u32) -> Option<u32> {
    let block = slot.checked_sub(VIRTUAL_GRID)?;
    (block > 0 && block <= 32767).then(|| block * GRID_BLOCK)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grid {
    pub columns: u32,
    pub rows: u32,
    pub wrap: bool,
    pub stride: u32,
}
impl Default for Grid {
    fn default() -> Self {
        Self {
            columns: 8,
            rows: 8,
            wrap: true,
            stride: 10,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Cell {
    pub x: u32,
    pub y: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
}
impl Direction {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "up-left" => Ok(Self::UpLeft),
            "up-right" => Ok(Self::UpRight),
            "down-left" => Ok(Self::DownLeft),
            "down-right" => Ok(Self::DownRight),
            _ => Err(
                "direction must be left/right/up/down/up-left/up-right/down-left/down-right".into(),
            ),
        }
    }
    pub fn delta(self) -> (i64, i64) {
        match self {
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
            Self::Up => (0, -1),
            Self::Down => (0, 1),
            Self::UpLeft => (-1, -1),
            Self::UpRight => (1, -1),
            Self::DownLeft => (-1, 1),
            Self::DownRight => (1, 1),
        }
    }
    pub fn from_delta(x: i64, y: i64) -> Option<Self> {
        match (x.signum(), y.signum()) {
            (-1, 0) => Some(Self::Left),
            (1, 0) => Some(Self::Right),
            (0, -1) => Some(Self::Up),
            (0, 1) => Some(Self::Down),
            (-1, -1) => Some(Self::UpLeft),
            (1, -1) => Some(Self::UpRight),
            (-1, 1) => Some(Self::DownLeft),
            (1, 1) => Some(Self::DownRight),
            _ => None,
        }
    }
    pub fn vertical(self) -> bool {
        matches!(self, Self::Up | Self::Down)
    }
    pub fn positive(self) -> bool {
        let (x, y) = self.delta();
        if x != 0 {
            x > 0
        } else {
            y > 0
        }
    }
}
impl Grid {
    pub fn validate(self, monitor: u32) -> Result<Self, String> {
        if self.columns == 0
            || self.rows == 0
            || self.columns > 256
            || self.rows > 256
            || self.stride == 0
            || (monitor >= self.stride && virtual_base(monitor).is_none())
        {
            return Err(
                "grid requires dimensions 1..256 and stride greater than every monitor id".into(),
            );
        }
        self.id(
            Cell {
                x: self.columns - 1,
                y: self.rows - 1,
            },
            monitor,
        )?;
        Ok(self)
    }
    pub fn id(self, c: Cell, monitor: u32) -> Result<u32, String> {
        if c.x >= self.columns || c.y >= self.rows {
            return Err("cell outside grid".into());
        }
        if let Some(base) = virtual_base(monitor) {
            return self
                .columns
                .checked_mul(c.y)
                .and_then(|n| n.checked_add(c.x))
                .filter(|n| *n < GRID_BLOCK)
                .and_then(|n| base.checked_add(n))
                .filter(|n| *n <= i32::MAX as u32)
                .ok_or_else(|| "named grid workspace overflow".into());
        }
        if monitor >= self.stride {
            return Err("cell or monitor outside grid".into());
        }
        self.columns
            .checked_mul(c.y)
            .and_then(|v| v.checked_add(c.x))
            .and_then(|v| v.checked_mul(self.stride))
            .and_then(|v| v.checked_add(monitor))
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| "workspace id overflow".into())
    }
    pub fn cell(self, id: u32, monitor: u32) -> Option<Cell> {
        if let Some(base) = virtual_base(monitor) {
            let index = id.checked_sub(base)?;
            return (self.columns > 0
                && index < self.columns.checked_mul(self.rows)?
                && index < GRID_BLOCK)
                .then(|| Cell {
                    x: index % self.columns,
                    y: index / self.columns,
                });
        }
        if monitor >= self.stride {
            return None;
        }
        let n = id.checked_sub(monitor.checked_add(1)?)?;
        if self.stride == 0 || self.columns == 0 || n % self.stride != 0 {
            return None;
        }
        let index = n / self.stride;
        let c = Cell {
            x: index % self.columns,
            y: index / self.columns,
        };
        (c.y < self.rows).then_some(c)
    }
    pub fn step(self, cell: Cell, dir: Direction) -> Option<Cell> {
        if self.columns == 0 || self.rows == 0 {
            return None;
        }
        let (dx, dy) = dir.delta();
        let advance = |value: u32, delta: i64, size: u32| {
            let next = i64::from(value) + delta;
            if self.wrap {
                next.rem_euclid(size.into()) as u32
            } else {
                next.clamp(0, i64::from(size) - 1) as u32
            }
        };
        let (x, y) = (
            advance(cell.x, dx, self.columns),
            advance(cell.y, dy, self.rows),
        );
        if x == cell.x && y == cell.y {
            return None;
        }

        Some(Cell { x, y })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_numbering_and_roundtrip() {
        let g = Grid::default();
        for m in 0..10 {
            for y in 0..8 {
                for x in 0..8 {
                    let c = Cell { x, y };
                    let id = g.id(c, m).unwrap();
                    assert_eq!(id, 10 * (y * 8 + x) + m + 1);
                    assert_eq!(g.cell(id, m), Some(c));
                    assert_eq!(g.cell(id, (m + 1) % 10), None);
                }
            }
        }
        assert_eq!(g.cell(0, 0), None);
        assert_eq!(g.cell(641, 0), None);
    }
    #[test]
    fn wrap_edges_and_custom_grid() {
        let mut g = Grid {
            columns: 3,
            rows: 2,
            stride: 12,
            wrap: true,
        };
        assert_eq!(
            g.step(Cell { x: 0, y: 0 }, Direction::Left),
            Some(Cell { x: 2, y: 0 })
        );
        assert_eq!(
            g.step(Cell { x: 0, y: 0 }, Direction::Up),
            Some(Cell { x: 0, y: 1 })
        );
        assert_eq!(
            g.step(Cell { x: 2, y: 1 }, Direction::Right),
            Some(Cell { x: 0, y: 1 })
        );
        assert_eq!(g.id(Cell { x: 2, y: 1 }, 4).unwrap(), 65);
        g.wrap = false;
        assert!(g.step(Cell { x: 0, y: 0 }, Direction::Left).is_none());
        assert!(g.validate(12).is_err());
    }
}

#[cfg(test)]
mod slot_capacity_tests {
    use super::*;
    #[test]
    fn slots_outside_stride_cannot_alias_an_existing_grid() {
        let grid = Grid::default();
        assert!(grid.cell(13, 12).is_none());
        assert!(grid.id(Cell { x: 0, y: 0 }, 12).is_err());
    }
}

#[cfg(test)]
mod diagonal_tests {
    use super::*;
    #[test]
    fn diagonal_wrap_and_independent_edge_clamps() {
        let mut g = Grid::default();
        assert_eq!(
            g.step(Cell { x: 0, y: 0 }, Direction::UpLeft),
            Some(Cell { x: 7, y: 7 })
        );
        assert_eq!(
            g.step(Cell { x: 7, y: 7 }, Direction::DownRight),
            Some(Cell { x: 0, y: 0 })
        );
        g.wrap = false;
        assert_eq!(
            g.step(Cell { x: 0, y: 0 }, Direction::UpRight),
            Some(Cell { x: 1, y: 0 })
        );
        assert_eq!(
            g.step(Cell { x: 0, y: 0 }, Direction::DownLeft),
            Some(Cell { x: 0, y: 1 })
        );
        assert_eq!(g.step(Cell { x: 0, y: 0 }, Direction::UpLeft), None);
        for (text, dir) in [
            ("up-left", Direction::UpLeft),
            ("up-right", Direction::UpRight),
            ("down-left", Direction::DownLeft),
            ("down-right", Direction::DownRight),
        ] {
            assert_eq!(Direction::parse(text).unwrap(), dir);
            let (x, y) = dir.delta();
            assert_eq!(Direction::from_delta(x, y), Some(dir));
        }
    }
}
