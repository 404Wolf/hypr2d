use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}
impl Direction {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            _ => Err("direction must be left/right/up/down".into()),
        }
    }
    pub fn vertical(self) -> bool {
        matches!(self, Self::Up | Self::Down)
    }
    pub fn positive(self) -> bool {
        matches!(self, Self::Right | Self::Down)
    }
}
impl Grid {
    pub fn validate(self, monitor: u32) -> Result<Self, String> {
        if self.columns == 0
            || self.rows == 0
            || self.columns > 256
            || self.rows > 256
            || self.stride == 0
            || monitor >= self.stride
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
        if c.x >= self.columns || c.y >= self.rows || monitor >= self.stride {
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
        let (dx, dy) = match dir {
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
        };
        let (x, y) = (i64::from(cell.x) + dx, i64::from(cell.y) + dy);
        if !self.wrap && (x < 0 || y < 0 || x >= self.columns.into() || y >= self.rows.into()) {
            return None;
        }
        Some(Cell {
            x: x.rem_euclid(self.columns.into()) as u32,
            y: y.rem_euclid(self.rows.into()) as u32,
        })
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
