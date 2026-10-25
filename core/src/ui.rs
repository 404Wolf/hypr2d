//! Overview geometry and interaction policy, independent of compositor objects.
use crate::{ffi, grid::Cell, guarded, Core};
use std::collections::BTreeSet;
const WIDTH: f64 = 1000.0;
const HEIGHT: f64 = 700.0;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}
impl Rect {
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}
#[derive(Clone, PartialEq)]
struct Window {
    id: String,
    workspace: String,
    monitor: u32,
    rect: Rect,
    floating: bool,
    title: String,
}
fn window_snapshot(windows: &[ffi::UIWindow]) -> Vec<Window> {
    windows
        .iter()
        .map(|w| Window {
            id: w.id.clone(),
            workspace: w.workspace.clone(),
            monitor: w.monitor,
            rect: Rect {
                x: w.x,
                y: w.y,
                w: w.width,
                h: w.height,
            },
            floating: w.floating,
            title: w.title.clone(),
        })
        .collect()
}
struct CellView {
    key: String,
    monitor: u32,
    rect: Rect,
    label: String,
    group: usize,
    x: u32,
    y: u32,
    occupied: bool,
    shown: bool,
}
#[derive(Default)]
pub struct Overview {
    windows: Vec<Window>,
    cells: Vec<CellView>,
    headers: Vec<(Rect, String)>,
    selected: usize,
    hover: Option<usize>,
    started: f64,
    closing: Option<f64>,
}
impl Core {
    pub fn overview_new(
        &self,
        outputs: &[ffi::UIOutput],
        specials: &[ffi::UISpecial],
        windows: &[ffi::UIWindow],
        now: f64,
    ) -> Box<Overview> {
        guarded(|| {
            let mut ui = Overview {
                started: now,
                windows: window_snapshot(windows),
                ..Default::default()
            };
            let min_x = outputs.iter().map(|m| m.x).fold(f64::INFINITY, f64::min);
            let min_y = outputs.iter().map(|m| m.y).fold(f64::INFINITY, f64::min);
            let max_x = outputs
                .iter()
                .map(|m| m.x + m.width)
                .fold(f64::NEG_INFINITY, f64::max);
            let max_y = outputs
                .iter()
                .map(|m| m.y + m.height)
                .fold(f64::NEG_INFINITY, f64::max);
            let scale = (960.0 / (max_x - min_x).max(1.0)).min(530.0 / (max_y - min_y).max(1.0));
            for (group, m) in outputs.iter().enumerate() {
                let grid = self
                    .grids
                    .get(&m.name)
                    .or_else(|| self.grids.get(&m.id.to_string()))
                    .copied()
                    .unwrap_or(crate::grid::Grid {
                        stride: self.stride,
                        ..Default::default()
                    });
                let panel = Rect {
                    x: 20.0 + (m.x - min_x) * scale,
                    y: 24.0 + (m.y - min_y) * scale,
                    w: m.width * scale,
                    h: m.height * scale,
                };
                ui.headers.push((
                    Rect {
                        x: panel.x + 6.0,
                        y: panel.y,
                        w: panel.w - 12.0,
                        h: 24.0,
                    },
                    m.name.clone(),
                ));
                let ratio = m.height / m.width;
                let w = ((panel.w - 16.0) / grid.columns as f64)
                    .min((panel.h - 40.0) / grid.rows as f64 / ratio)
                    .max(4.0);
                let h = w * ratio;
                for y in 0..grid.rows {
                    for x in 0..grid.columns {
                        if let Ok(id) = grid.id(Cell { x, y }, m.id) {
                            let key = id.to_string();
                            let occupied = windows.iter().any(|win| win.workspace == key);
                            if m.focused && m.workspace == id {
                                ui.selected = ui.cells.len();
                            }
                            ui.cells.push(CellView {
                                key,
                                monitor: m.id,
                                rect: Rect {
                                    x: panel.x + 8.0 + x as f64 * w,
                                    y: panel.y + 30.0 + y as f64 * h,
                                    w: (w - 3.0).max(1.0),
                                    h: ((w - 3.0) * ratio).max(1.0),
                                },
                                label: id.to_string(),
                                group,
                                x,
                                y,
                                occupied,
                                shown: m.workspace == id,
                            });
                        }
                    }
                }
            }
            ui.headers.push((
                Rect {
                    x: 24.0,
                    y: 575.0,
                    w: 950.0,
                    h: 22.0,
                },
                "Special workspaces".into(),
            ));
            let w = (950.0 / specials.len().max(1) as f64).min(150.0);
            for (i, s) in specials.iter().enumerate() {
                ui.cells.push(CellView {
                    key: s.name.clone(),
                    monitor: s.monitor,
                    rect: Rect {
                        x: 24.0 + i as f64 * w,
                        y: 603.0,
                        w: w - 5.0,
                        h: 72.0,
                    },
                    label: s.name.strip_prefix("special:").unwrap_or(&s.name).into(),
                    group: outputs.len(),
                    x: i as u32,
                    y: 0,
                    occupied: windows.iter().any(|win| win.workspace == s.name),
                    shown: s.shown,
                });
            }
            Box::new(ui)
        })
    }
}
impl Overview {
    pub fn overview_windows(&mut self, windows: &[ffi::UIWindow]) -> Vec<String> {
        guarded(|| {
            let replacement = window_snapshot(windows);
            let mut dirty = BTreeSet::new();
            for w in self
                .windows
                .iter()
                .filter(|w| !replacement.contains(w))
                .chain(replacement.iter().filter(|w| !self.windows.contains(w)))
            {
                dirty.insert(w.workspace.clone());
            }
            self.windows = replacement;
            for c in &mut self.cells {
                c.occupied = self.windows.iter().any(|w| w.workspace == c.key);
            }
            dirty.into_iter().collect()
        })
    }
    pub fn overview_reconcile(&mut self, mut replacement: Box<Overview>) -> Vec<String> {
        guarded(|| {
            let mut dirty = BTreeSet::new();
            for w in self
                .windows
                .iter()
                .filter(|w| !replacement.windows.contains(w))
                .chain(
                    replacement
                        .windows
                        .iter()
                        .filter(|w| !self.windows.contains(w)),
                )
            {
                dirty.insert(w.workspace.clone());
            }
            if self.cells.len() != replacement.cells.len()
                || self
                    .cells
                    .iter()
                    .zip(&replacement.cells)
                    .any(|(a, b)| a.key != b.key || a.rect != b.rect || a.monitor != b.monitor)
            {
                for c in &replacement.cells {
                    dirty.insert(c.key.clone());
                }
            }
            let selected = self.cells.get(self.selected).map(|c| c.key.clone());
            replacement.started = self.started;
            replacement.closing = self.closing;
            if let Some(i) = replacement
                .cells
                .iter()
                .position(|c| Some(&c.key) == selected.as_ref())
            {
                replacement.selected = i;
            }
            *self = *replacement;
            dirty.into_iter().collect()
        })
    }
    pub fn overview_state(&self, width: f64, height: f64, now: f64) -> String {
        guarded(|| {
            let f = self.overview_frame(width, height, now);
            serde_json::json!({"open":f.alive,"alpha":f.alpha,"cells":f.cells.iter().map(|c|serde_json::json!({"key":c.key,"monitor":c.monitor,"rect":[c.rect.x,c.rect.y,c.rect.width,c.rect.height],"selected":c.selected,"hovered":c.hovered,"occupied":c.occupied,"shown":c.shown,"label":c.label})).collect::<Vec<_>>(),"panels":self.headers.iter().map(|(r,t)|serde_json::json!({"label":t,"rect":[r.x,r.y,r.w,r.h]})).collect::<Vec<_>>()}).to_string()
        })
    }
    pub fn overview_frame(&self, width: f64, height: f64, now: f64) -> ffi::UIFrame {
        guarded(|| {
            let (scale, ox, oy) = Self::transform(width, height);
            let convert = |r: Rect| ffi::UIRect {
                x: ox + r.x * scale,
                y: oy + r.y * scale,
                width: r.w * scale,
                height: r.h * scale,
            };
            let progress = if let Some(t) = self.closing {
                1.0 - ((now - t) / 100.0).clamp(0.0, 1.0)
            } else {
                ((now - self.started) / 100.0).clamp(0.0, 1.0)
            };
            ffi::UIFrame {
                alive: self.closing.is_none() || progress > 0.0,
                animating: progress > 0.0 && progress < 1.0,
                alpha: progress * progress * (3.0 - 2.0 * progress),
                cells: self
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| ffi::UICell {
                        key: c.key.clone(),
                        monitor: c.monitor,
                        rect: convert(c.rect),
                        label: c.label.clone(),
                        selected: i == self.selected,
                        hovered: self.hover == Some(i),
                        occupied: c.occupied,
                        shown: c.shown,
                    })
                    .collect(),
                labels: self
                    .headers
                    .iter()
                    .map(|(r, text)| ffi::UILabel {
                        rect: convert(*r),
                        text: text.clone(),
                    })
                    .collect(),
            }
        })
    }
    fn transform(width: f64, height: f64) -> (f64, f64, f64) {
        let s = (width / WIDTH).min(height / HEIGHT);
        (s, (width - WIDTH * s) / 2.0, (height - HEIGHT * s) / 2.0)
    }
    pub fn overview_pointer(
        &mut self,
        width: f64,
        height: f64,
        x: f64,
        y: f64,
        click: bool,
    ) -> String {
        guarded(|| {
            if self.closing.is_some() {
                return String::new();
            }
            let (s, ox, oy) = Self::transform(width, height);
            self.hover = self
                .cells
                .iter()
                .position(|c| c.rect.contains((x - ox) / s, (y - oy) / s));
            if click {
                if let Some(i) = self.hover {
                    self.selected = i;
                    return self.cells[i].key.clone();
                }
            }
            String::new()
        })
    }
    pub fn overview_key(&mut self, key: u32, shift: bool, now: f64) -> String {
        guarded(|| {
            if self.cells.is_empty() || self.closing.is_some() {
                return String::new();
            }
            match key {
                0xff1b => {
                    self.closing = Some(now);
                }
                0xff0d => return self.cells[self.selected].key.clone(),
                0xff09 | 0xfe20 => {
                    let groups = self.cells.iter().map(|c| c.group).max().unwrap_or(0) + 1;
                    let current = self.cells[self.selected].group;
                    for step in 1..=groups {
                        let group = if shift || key == 0xfe20 {
                            (current + groups - step) % groups
                        } else {
                            (current + step) % groups
                        };
                        if let Some(i) = self.cells.iter().position(|c| c.group == group) {
                            self.selected = i;
                            break;
                        }
                    }
                }
                _ => {
                    let (dx, dy) = match key {
                        0xff51 | 104 => (-1, 0),
                        0xff53 | 108 => (1, 0),
                        0xff52 | 107 => (0, -1),
                        0xff54 | 106 => (0, 1),
                        _ => return String::new(),
                    };
                    let c = &self.cells[self.selected];
                    let cols = self
                        .cells
                        .iter()
                        .filter(|o| o.group == c.group)
                        .map(|o| o.x)
                        .max()
                        .unwrap_or(0)
                        + 1;
                    let rows = self
                        .cells
                        .iter()
                        .filter(|o| o.group == c.group)
                        .map(|o| o.y)
                        .max()
                        .unwrap_or(0)
                        + 1;
                    let x = (c.x as i32 + dx).rem_euclid(cols as i32) as u32;
                    let y = (c.y as i32 + dy).rem_euclid(rows as i32) as u32;
                    if let Some(i) = self
                        .cells
                        .iter()
                        .position(|o| o.group == c.group && o.x == x && o.y == y)
                    {
                        self.selected = i;
                    }
                }
            }
            String::new()
        })
    }
    pub fn overview_close(&mut self, now: f64) {
        guarded(|| self.closing = Some(now));
    }
}
impl Default for ffi::UIFrame {
    fn default() -> Self {
        Self {
            alive: false,
            animating: false,
            alpha: 0.0,
            cells: vec![],
            labels: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(id: u32, x: f64, y: f64) -> ffi::UIOutput {
        ffi::UIOutput {
            id,
            name: id.to_string(),
            x,
            y,
            width: 1280.0,
            height: 720.0,
            workspace: id + 1,
            focused: id == 0,
        }
    }
    #[test]
    fn physical_layout_negative_positions_and_keyboard_groups() {
        let mut ui = Core::default().overview_new(
            &[output(0, 0.0, 0.0), output(1, -1280.0, -720.0)],
            &[ffi::UISpecial {
                monitor: 1,
                name: "special:empty".into(),
                shown: true,
            }],
            &[],
            0.0,
        );
        let f = ui.overview_frame(1000.0, 700.0, 101.0);
        assert_eq!(f.cells.len(), 129);
        assert!(f.cells[64].rect.x < f.cells[0].rect.x && f.cells[64].rect.y < f.cells[0].rect.y);
        ui.overview_key(0xff09, false, 101.0);
        assert_eq!(ui.overview_key(0xff0d, false, 101.0), "2");
        ui.overview_key(0xff09, false, 101.0);
        assert_eq!(ui.overview_key(0xff0d, false, 101.0), "special:empty");
        assert!(!f.cells[128].occupied && f.cells[128].shown);
        ui.overview_key(0xff09, true, 101.0);
        assert_eq!(ui.overview_key(0xff0d, false, 101.0), "2");
    }
    #[test]
    fn hit_testing_scaled_canvas_and_animation_cleanup() {
        let mut ui = Core::default().overview_new(&[output(0, 0.0, 0.0)], &[], &[], 0.0);
        let f = ui.overview_frame(1280.0, 720.0, 101.0);
        let c = &f.cells[9];
        assert_eq!(
            ui.overview_pointer(1280.0, 720.0, c.rect.x + 1.0, c.rect.y + 1.0, true),
            "91"
        );
        assert!(ui
            .overview_pointer(1280.0, 720.0, 0.0, 0.0, true)
            .is_empty());
        assert!(ui.overview_frame(1280.0, 720.0, 50.0).animating);
        ui.overview_key(0xff1b, false, 110.0);
        assert!(ui.overview_frame(1280.0, 720.0, 150.0).alive);
        assert!(!ui.overview_frame(1280.0, 720.0, 211.0).alive);
    }
}
