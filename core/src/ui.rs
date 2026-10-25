//! Overview geometry and interaction policy, independent of compositor objects.
use crate::{ffi, grid::Cell, guarded, Core};
use std::collections::BTreeSet;
mod drag;

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
#[derive(Clone)]
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
    visible: bool,
    annotated: bool,
}
#[derive(Clone, Default)]
pub struct Overview {
    annotations: std::collections::BTreeMap<String, hypr2d_client::Annotation>,
    regions: Vec<(u32, u32, String)>,
    fit: bool,
    gap: f64,
    font_height: f64,
    gap_pixels: f64,
    header_widths: std::collections::HashMap<String, f64>,
    header_height: f64,
    header_gap: f64,
    panels: Vec<Rect>,
    windows: Vec<Window>,
    outputs: Vec<(u32, f64, f64)>,
    canvas_height: f64,
    drag: Option<drag::Drag>,
    pointer: (f64, f64),
    pressed: Option<usize>,
    cancelled_release: bool,
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
                fit: self.theme.fit,
                gap: f64::from(self.theme.gap),
                font_height: self
                    .ui_font_height
                    .unwrap_or(f64::from(self.theme.font_size) + 4.),
                gap_pixels: 8.,
                header_widths: self.ui_header_widths.clone(),
                header_height: 20.,
                header_gap: 8.,
                started: now,
                windows: window_snapshot(windows),
                outputs: outputs.iter().map(|m| (m.id, m.width, m.height)).collect(),
                ..Default::default()
            };
            if outputs.is_empty() {
                ui.closing = Some(now - 100.0);
                return Box::new(ui);
            }
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
            let scale = (984.0 / (max_x - min_x).max(1.0)).min(530.0 / (max_y - min_y).max(1.0));
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
                    x: 8.0 + (m.x - min_x) * scale,
                    y: 24.0 + (m.y - min_y) * scale,
                    w: m.width * scale,
                    h: m.height * scale,
                };
                ui.panels.push(panel);
                ui.headers.push((
                    Rect {
                        x: panel.x + 6.0,
                        y: panel.y,
                        w: panel.w - 12.0,
                        h: 24.0,
                    },
                    m.name.clone(),
                ));
                for y in 0..grid.rows {
                    for x in 0..grid.columns {
                        if let Ok(id) = grid.id(Cell { x, y }, m.id) {
                            let key = id.to_string();
                            let occupied = windows.iter().any(|win| win.workspace == key);
                            if m.focused && m.workspace == id {
                                ui.selected = ui.cells.len();
                            }
                            ui.cells.push(CellView {
                                key: key.clone(),
                                monitor: m.id,
                                rect: Rect::default(),
                                label: cell_label(windows, &key),
                                group,
                                x,
                                y,
                                occupied,
                                shown: m.workspace == id,
                                visible: true,
                                annotated: false,
                            });
                        }
                    }
                }
            }
            ui.headers
                .push((Rect::default(), "Special workspaces".into()));
            for (i, s) in specials.iter().enumerate() {
                ui.cells.push(CellView {
                    key: s.name.clone(),
                    monitor: s.monitor,
                    rect: Rect::default(),
                    label: special_label(&s.name, cell_label(windows, &s.name)),
                    group: outputs.len(),
                    x: i as u32,
                    y: 0,
                    occupied: windows.iter().any(|win| win.workspace == s.name),
                    shown: s.shown,
                    visible: true,
                    annotated: false,
                });
            }
            for c in &mut ui.cells {
                let target = if c.key.starts_with("special:") {
                    hypr2d_client::Target::Special {
                        name: c.key.clone(),
                    }
                } else {
                    hypr2d_client::Target::Cell {
                        monitor: c.monitor,
                        x: c.x,
                        y: c.y,
                    }
                };
                if let Some(a) = self.extensions.annotation(&target) {
                    ui.annotations.insert(c.key.clone(), a.clone());
                    c.annotated = true;
                }
            }
            for w in windows {
                if let Some(a) = self.extensions.annotation(&hypr2d_client::Target::Window {
                    address: w.id.clone(),
                }) {
                    ui.annotations.insert(w.id.clone(), a.clone());
                }
            }
            for m in outputs {
                for (row, title) in self.extensions.row_titles(m.id) {
                    ui.regions.push((m.id, row, title));
                }
            }
            ui.reflow();
            Box::new(ui)
        })
    }
}
impl Overview {
    fn for_viewport(&self, width: f64, height: f64) -> Self {
        self.scaled_viewport(width, height, 1.)
    }
    fn scaled_viewport(&self, width: f64, height: f64, scale: f64) -> Self {
        let mut layout = self.clone();
        layout.font_height *= scale;
        layout.gap_pixels *= scale;
        for width in layout.header_widths.values_mut() {
            *width *= scale;
        }
        layout.fit_headers(width, height);
        layout
    }
    fn fit_headers(&mut self, width: f64, height: f64) {
        for _ in 0..12 {
            let scale = self.transform(width, height).0.max(0.001);
            let h = self.font_height / scale;
            let gap = self.gap_pixels / scale;
            if (self.header_height - h).abs() < 1e-9 && (self.header_gap - gap).abs() < 1e-9 {
                break;
            }
            self.header_height = h;
            self.header_gap = gap;
            self.reflow();
        }
    }
    fn reflow(&mut self) {
        let header = self.header_height + self.header_gap;
        for (g, p) in self.panels.iter().enumerate() {
            let cells: Vec<_> = self
                .cells
                .iter()
                .enumerate()
                .filter(|(_, c)| c.group == g)
                .collect();
            let cols = cells.iter().map(|(_, c)| c.x + 1).max().unwrap_or(1);
            let rows = cells.iter().map(|(_, c)| c.y + 1).max().unwrap_or(1);
            let used: Vec<_> = cells
                .iter()
                .filter(|(i, c)| {
                    c.occupied
                        || c.shown
                        || c.annotated
                        || self
                            .regions
                            .iter()
                            .any(|(m, y, _)| *m == c.monitor && *y == c.y)
                        || *i == self.selected
                })
                .collect();
            let bounds = |axis: bool, n: u32| {
                if !self.fit {
                    return (0, n);
                }
                let lo = used
                    .iter()
                    .map(|(_, c)| if axis { c.x } else { c.y })
                    .min()
                    .unwrap_or(0)
                    .saturating_sub(1);
                let hi = (used
                    .iter()
                    .map(|(_, c)| if axis { c.x } else { c.y })
                    .max()
                    .unwrap_or(0)
                    + 2)
                .min(n);
                let count = (hi - lo).max(3.min(n));
                (lo.min(n - count), count)
            };
            let (x0, nx) = bounds(true, cols);
            let (y0, ny) = bounds(false, rows);
            let ratio = self.outputs[g].2 / self.outputs[g].1;
            let w = ((p.w - self.gap * f64::from(nx)) / f64::from(nx))
                .min((p.h - header - self.gap * f64::from(ny - 1)) / f64::from(ny) / ratio)
                .max(1.);
            let grid_w = w * f64::from(nx) + self.gap * f64::from(nx - 1);
            let grid_h = w * ratio * f64::from(ny) + self.gap * f64::from(ny - 1);
            let left = p.x + (p.w - grid_w) / 2.;
            let top = p.y + header + (p.h - header - grid_h) / 2.;
            self.headers[g].0.y = top - header;
            self.headers[g].0.h = self.header_height;
            self.headers[g].0.x = left;
            self.headers[g].0.w = grid_w;
            for c in self.cells.iter_mut().filter(|c| c.group == g) {
                c.visible = c.x >= x0 && c.x < x0 + nx && c.y >= y0 && c.y < y0 + ny;
                c.rect = Rect {
                    x: left + (c.x as f64 - x0 as f64) * (w + self.gap),
                    y: top + (c.y as f64 - y0 as f64) * (w * ratio + self.gap),
                    w,
                    h: w * ratio,
                };
            }
        }
        let normal: Vec<_> = self
            .cells
            .iter()
            .filter(|c| c.group < self.panels.len() && c.visible)
            .collect();
        let left = normal
            .iter()
            .map(|c| c.rect.x)
            .fold(f64::INFINITY, f64::min);
        let right = normal
            .iter()
            .map(|c| c.rect.x + c.rect.w)
            .fold(f64::NEG_INFINITY, f64::max);
        let bottom = normal
            .iter()
            .map(|c| c.rect.y + c.rect.h)
            .fold(0., f64::max);
        if let Some((r, _)) = self.headers.last_mut() {
            *r = Rect {
                x: left,
                y: bottom + self.gap,
                w: right - left,
                h: self.header_height,
            };
        }
        let n = self
            .cells
            .iter()
            .filter(|c| c.group == self.panels.len())
            .count()
            .max(1) as f64;
        let mut x = left;
        for c in self
            .cells
            .iter_mut()
            .filter(|c| c.group == self.panels.len())
        {
            let ratio = self
                .outputs
                .iter()
                .find(|o| o.0 == c.monitor)
                .map(|o| o.2 / o.1)
                .unwrap_or(2. / 3.);
            let w = ((right - left - self.gap * (n - 1.)) / n).min(150.);
            c.rect = Rect {
                x,
                y: bottom + header + self.gap,
                w,
                h: w * ratio,
            };
            x += w + self.gap;
        }
        self.canvas_height = self
            .cells
            .iter()
            .filter(|c| c.visible)
            .map(|c| c.rect.y + c.rect.h)
            .fold(bottom + header, f64::max)
            + 28.;
    }
    /// Only visible occupied cells retain textures. Shared cache uses the largest
    /// presentation across outputs; 64 MiB RGBA8+mip payload caps extreme layouts.
    pub fn overview_captures(&self, viewports: &[ffi::UIRect]) -> Vec<ffi::ProbeCell> {
        guarded(|| {
            let mut plans = std::collections::BTreeMap::<String, ffi::ProbeCell>::new();
            for viewport in viewports {
                let frame = self.overview_frame_scaled(
                    viewport.width,
                    viewport.height,
                    viewport.x.max(1.),
                    0.,
                );
                for cell in frame.cells.into_iter().filter(|c| c.occupied) {
                    let w = cell.rect.width.ceil().max(1.) as u32;
                    let h = cell.rect.height.ceil().max(1.) as u32;
                    let entry = plans
                        .entry(cell.key.clone())
                        .or_insert_with(|| ffi::ProbeCell {
                            monitor: cell.monitor,
                            workspace: cell.key.parse().unwrap_or(0),
                            special: if cell.key.starts_with("special:") {
                                cell.key
                            } else {
                                String::new()
                            },
                            x: 0.,
                            y: 0.,
                            width: 0.,
                            height: 0.,
                            capture_width: 0,
                            capture_height: 0,
                        });
                    entry.capture_width = entry.capture_width.max(w);
                    entry.capture_height = entry.capture_height.max(h);
                    entry.width = entry.capture_width.into();
                    entry.height = entry.capture_height.into();
                }
            }
            let area: f64 = plans
                .values()
                .map(|p| f64::from(p.capture_width) * f64::from(p.capture_height))
                .sum();
            let factor = (64. * 1024. * 1024. / (area.max(1.) * 4. * 4. / 3.))
                .sqrt()
                .min(1.);
            plans
                .into_values()
                .map(|mut p| {
                    if factor < 1. {
                        p.capture_width =
                            (f64::from(p.capture_width) * factor).floor().max(1.) as u32;
                        p.capture_height =
                            (f64::from(p.capture_height) * factor).floor().max(1.) as u32;
                    }
                    p
                })
                .collect()
        })
    }
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
            self.validate_drag();
            for c in &mut self.cells {
                c.occupied = self.windows.iter().any(|w| w.workspace == c.key);
                let app = owned_label(&self.windows, &c.key);
                c.label = if c.key.starts_with("special:") {
                    special_label(&c.key, app)
                } else {
                    app
                };
            }
            self.reflow();
            dirty.into_iter().collect()
        })
    }
    pub fn overview_reconcile(&mut self, mut replacement: Box<Overview>) -> Vec<String> {
        guarded(|| {
            replacement.fit = self.fit;
            replacement.header_height = self.header_height;
            replacement.header_gap = self.header_gap;
            replacement.reflow();
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
            let layout_changed = self.cells.len() != replacement.cells.len()
                || self
                    .cells
                    .iter()
                    .zip(&replacement.cells)
                    .any(|(a, b)| a.key != b.key || a.rect != b.rect || a.monitor != b.monitor);
            if layout_changed {
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
            if !layout_changed {
                replacement.drag = self.drag.take();
                replacement.pressed = self.pressed;
                replacement.pointer = self.pointer;
                replacement.hover = self.hover;
                replacement.cancelled_release = self.cancelled_release;
                replacement.validate_drag();
            } else if self.drag.is_some() {
                replacement.cancelled_release = true;
            }
            *self = *replacement;
            dirty.into_iter().collect()
        })
    }
    pub fn overview_frame_scaled(
        &self,
        width: f64,
        height: f64,
        scale: f64,
        now: f64,
    ) -> ffi::UIFrame {
        guarded(|| {
            self.scaled_viewport(width, height, scale)
                .frame(width, height, now)
        })
    }
    pub fn overview_state_scaled(&self, width: f64, height: f64, scale: f64, now: f64) -> String {
        guarded(|| {
            self.scaled_viewport(width, height, scale)
                .state_frame(width, height, now)
        })
    }
    pub fn overview_state(&self, width: f64, height: f64, now: f64) -> String {
        guarded(|| {
            self.for_viewport(width, height)
                .state_frame(width, height, now)
        })
    }
    fn state_frame(&self, width: f64, height: f64, now: f64) -> String {
        guarded(|| {
            let f = self.overview_frame(width, height, now);
            let (s, ox, oy) = self.transform(width, height);
            serde_json::json!({"labels":f.labels.iter().map(|l|serde_json::json!({"text":l.text,"rect":[l.rect.x,l.rect.y,l.rect.width,l.rect.height]})).collect::<Vec<_>>(),"fit":self.fit,"pointer":[self.pointer.0*s+ox,self.pointer.1*s+oy],"dragging":f.ghost.visible,"target":f.target,"windows":f.windows.iter().map(|w|serde_json::json!({"id":w.id,"workspace":w.workspace,"rect":[w.rect.x,w.rect.y,w.rect.width,w.rect.height],"floating":w.floating,"target":w.target})).collect::<Vec<_>>(),"open":f.alive,"alpha":f.alpha,"cells":f.cells.iter().map(|c|serde_json::json!({"key":c.key,"monitor":c.monitor,"rect":[c.rect.x,c.rect.y,c.rect.width,c.rect.height],"selected":c.selected,"hovered":c.hovered,"occupied":c.occupied,"shown":c.shown,"label":c.label})).collect::<Vec<_>>(),"panels":f.labels.iter().take(self.headers.len()).map(|l|serde_json::json!({"label":l.text,"rect":[l.rect.x,l.rect.y,l.rect.width,l.rect.height]})).collect::<Vec<_>>()}).to_string()
        })
    }
    pub fn overview_frame(&self, width: f64, height: f64, now: f64) -> ffi::UIFrame {
        guarded(|| self.for_viewport(width, height).frame(width, height, now))
    }
    fn frame(&self, width: f64, height: f64, now: f64) -> ffi::UIFrame {
        guarded(|| {
            let (scale, ox, oy) = self.transform(width, height);
            let convert = |r: Rect| ffi::UIRect {
                x: ox + r.x * scale,
                y: oy + r.y * scale,
                width: r.w * scale,
                height: r.h * scale,
            };
            let (ghost, windows, target) = self.drag_frame(convert);
            let progress = if let Some(t) = self.closing {
                1.0 - ((now - t) / 100.0).clamp(0.0, 1.0)
            } else {
                ((now - self.started) / 100.0).clamp(0.0, 1.0)
            };
            ffi::UIFrame {
                ghost,
                windows,
                target,
                alive: self.closing.is_none() || progress > 0.0,
                animating: progress > 0.0 && progress < 1.0,
                alpha: progress * progress * (3.0 - 2.0 * progress),
                cells: self
                    .cells
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.visible)
                    .map(|(i, c)| ffi::UICell {
                        key: c.key.clone(),
                        monitor: c.monitor,
                        rect: convert(c.rect),
                        label: annotation_text(&c.label, self.annotations.get(&c.key)),
                        colour: self
                            .annotations
                            .get(&c.key)
                            .and_then(|a| a.colour.as_deref())
                            .map(parse_colour)
                            .unwrap_or(0),
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
                        rect: {
                            let mut rect = convert(*r);
                            if let Some(width) = self.header_widths.get(text) {
                                rect.width = rect.width.min(*width);
                            }
                            rect
                        },
                        text: text.clone(),
                        colour: 0,
                        band: false,
                    })
                    .chain(self.annotation_labels(convert))
                    .collect(),
            }
        })
    }
    fn annotation_labels(&self, convert: impl Fn(Rect) -> ffi::UIRect) -> Vec<ffi::UILabel> {
        let mut labels = vec![];
        for c in self.cells.iter().filter(|c| c.visible) {
            if let Some(a) = self.annotations.get(&c.key) {
                let text = badge(a);
                if !text.trim().is_empty() {
                    let w = c.rect.w.min(60.);
                    let h = c.rect.h.min(16.);
                    labels.push(ffi::UILabel {
                        rect: convert(Rect {
                            x: c.rect.x + c.rect.w - w,
                            y: c.rect.y + c.rect.h - h,
                            w,
                            h,
                        }),
                        text,
                        colour: a.colour.as_deref().map(parse_colour).unwrap_or(0),
                        band: true,
                    });
                }
            }
        }
        for w in &self.windows {
            if let Some(a) = self.annotations.get(&w.id) {
                if let Some(r) = self.visible_window_rect(w) {
                    let tag = badge(a);
                    if !tag.trim().is_empty() {
                        let width = r.w.min(60.);
                        let h = r.h.min(16.);
                        labels.push(ffi::UILabel {
                            rect: convert(Rect {
                                x: r.x + r.w - width,
                                y: r.y + r.h - h,
                                w: width,
                                h,
                            }),
                            text: tag,
                            colour: a.colour.as_deref().map(parse_colour).unwrap_or(0),
                            band: true,
                        });
                    }
                    labels.push(ffi::UILabel {
                        rect: convert(Rect { h: 18., ..r }),
                        text: annotation_text("", Some(a)),
                        colour: a.colour.as_deref().map(parse_colour).unwrap_or(0),
                        band: true,
                    });
                }
            }
        }
        for (m, row, title) in &self.regions {
            if let Some(c) = self
                .cells
                .iter()
                .find(|c| c.visible && c.monitor == *m && c.y == *row)
            {
                labels.push(ffi::UILabel {
                    rect: convert(Rect {
                        w: self
                            .cells
                            .iter()
                            .filter(|o| o.visible && o.monitor == *m && o.y == *row)
                            .map(|o| o.rect.w + 3.)
                            .sum::<f64>()
                            - 3.,
                        h: 18.,
                        ..c.rect
                    }),
                    text: title.clone(),
                    colour: 0,
                    band: true,
                });
            }
        }
        if let Some(c) = self.hover.and_then(|i| self.cells.get(i)) {
            if let Some(t) = self
                .hit_window(None)
                .and_then(|w| self.annotations.get(&w.id))
                .and_then(|a| a.tooltip.as_ref())
                .or_else(|| {
                    self.annotations
                        .get(&c.key)
                        .and_then(|a| a.tooltip.as_ref())
                })
            {
                labels.push(ffi::UILabel {
                    rect: convert(Rect {
                        x: 24.,
                        y: self.canvas_height - 22.,
                        w: 950.,
                        h: 20.,
                    }),
                    text: t.clone(),
                    colour: 0,
                    band: true,
                });
            }
        }
        labels
    }
    fn transform(&self, width: f64, height: f64) -> (f64, f64, f64) {
        let bounds = self
            .cells
            .iter()
            .filter(|c| c.visible)
            .map(|c| c.rect)
            .chain(self.headers.iter().map(|(r, _)| *r));
        let (left, top, right, bottom) = bounds.fold(
            (
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ),
            |(l, t, r, b), rect| {
                (
                    l.min(rect.x),
                    t.min(rect.y),
                    r.max(rect.x + rect.w),
                    b.max(rect.y + rect.h),
                )
            },
        );
        if !left.is_finite() {
            return (1., 0., 0.);
        }
        let w = (right - left).max(1.);
        let h = (bottom - top).max(1.);
        let s = ((width - 24.).max(1.) / w).min((height - 24.).max(1.) / h);
        (
            s,
            (width - w * s) / 2. - left * s,
            (height - h * s) / 2. - top * s,
        )
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
            self.fit_headers(width, height);
            let (s, ox, oy) = self.transform(width, height);
            self.pointer = ((x - ox) / s, (y - oy) / s);
            self.drag_motion(s);
            self.hover = self
                .cells
                .iter()
                .position(|c| c.visible && c.rect.contains((x - ox) / s, (y - oy) / s));
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
            if self.closing.is_some() {
                return String::new();
            }
            if self.cells.is_empty() {
                if key == 0xff1b {
                    self.closing = Some(now);
                }
                return String::new();
            }
            if self.drag.is_some() && key != 0xff1b {
                return String::new();
            }
            match key {
                97 => {
                    self.fit = !self.fit;
                    self.reflow();
                }
                0xff1b => {
                    if self.drag.take().is_some() {
                        self.pressed = None;
                        self.cancelled_release = true;
                    } else {
                        self.closing = Some(now);
                    }
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
            self.reflow();
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
            ghost: ffi::UIGhost::default(),
            windows: vec![],
            target: String::new(),
            alive: false,
            animating: false,
            alpha: 0.0,
            cells: vec![],
            labels: vec![],
        }
    }
}

fn short_app(class: &str) -> String {
    let name = class
        .trim()
        .strip_suffix(".desktop")
        .unwrap_or(class.trim());
    let name = name
        .rsplit(['.', '/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_lowercase();
    if name.is_empty() {
        "window".into()
    } else {
        name
    }
}
// Numeric special identifiers are key bindings, not useful display names.
fn special_label(key: &str, app: String) -> String {
    let name = key.strip_prefix("special:").unwrap_or(key);
    if !name.is_empty() && name.parse::<u32>().is_err() {
        name.into()
    } else if !app.is_empty() {
        app
    } else {
        String::new()
    }
}
fn app_label<'a>(apps: impl Iterator<Item = &'a str>) -> String {
    let apps: Vec<_> = apps.map(short_app).collect();
    let mut counts = std::collections::HashMap::new();
    for (i, app) in apps.iter().enumerate() {
        let entry = counts.entry(app).or_insert((0, usize::MAX - i));
        entry.0 += 1;
    }
    let Some((app, _)) = counts.into_iter().max_by_key(|(_, rank)| *rank) else {
        return String::new();
    };
    if apps.len() > 1 {
        format!("{app} +{}", apps.len() - 1)
    } else {
        app.clone()
    }
}
fn owned_label(windows: &[Window], key: &str) -> String {
    app_label(
        windows
            .iter()
            .filter(|w| w.workspace == key)
            .map(|w| w.title.as_str()),
    )
}
fn cell_label(windows: &[ffi::UIWindow], key: &str) -> String {
    app_label(
        windows
            .iter()
            .filter(|w| w.workspace == key)
            .map(|w| w.title.as_str()),
    )
}

fn parse_colour(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0) | 0x1000000
}
fn annotation_text(base: &str, a: Option<&hypr2d_client::Annotation>) -> String {
    a.and_then(|a| a.label.clone())
        .unwrap_or_else(|| base.into())
}
fn badge(a: &hypr2d_client::Annotation) -> String {
    format!(
        "{}{}",
        a.badge
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(4)
            .collect::<String>(),
        if a.dot { " ●" } else { "" }
    )
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
    fn edp_headers_are_text_sized_flush_and_eight_pixels_above_each_panel() {
        let mut output = output(0, 8934., 6682.);
        output.name = "eDP-1".into();
        output.width = 2256.;
        output.height = 1504.;
        let mut core = Core::default();
        core.ui_font_height(19.);
        core.ui_header_width("eDP-1", 52.);
        core.ui_header_width("Special workspaces", 148.);
        let mut ui = core.overview_new(
            &[output],
            &[ffi::UISpecial {
                monitor: 0,
                name: "special:8".into(),
                shown: false,
            }],
            &[],
            0.,
        );
        for full in [false, true] {
            ui.fit = !full;
            ui.reflow();
            for scale in [1., 2.] {
                let f = ui.overview_frame_scaled(2256. * scale, 1504. * scale, scale, 101.);
                for (label, special) in [("eDP-1", false), ("Special workspaces", true)] {
                    let header = f.labels.iter().find(|l| l.text == label).unwrap();
                    let cells: Vec<_> = f
                        .cells
                        .iter()
                        .filter(|c| c.key.starts_with("special:") == special)
                        .collect();
                    let left = cells.iter().map(|c| c.rect.x).fold(f64::INFINITY, f64::min);
                    let top = cells.iter().map(|c| c.rect.y).fold(f64::INFINITY, f64::min);
                    assert!((header.rect.x - left).abs() < 1e-6);
                    assert!((header.rect.height - 19. * scale).abs() < 1e-6);
                    assert!((top - header.rect.y - header.rect.height - 8. * scale).abs() < 1e-6);
                }
                let state: serde_json::Value = serde_json::from_str(&ui.overview_state_scaled(
                    2256. * scale,
                    1504. * scale,
                    scale,
                    101.,
                ))
                .unwrap();
                for (header, label) in state["panels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(state["labels"].as_array().unwrap())
                {
                    assert_eq!(header["rect"], label["rect"]);
                }
            }
        }
    }
    #[test]
    fn flush_cells_configurable_gap_and_special_names() {
        assert_eq!(crate::theme::Theme::default().gap, 0);
        assert!(crate::theme::Theme::parse(r#"{"gap":33}"#).is_err());
        assert_eq!(
            special_label("special:8", "obsidian +1".into()),
            "obsidian +1"
        );
        assert_eq!(special_label("special:music", "spotify".into()), "music");
        assert_eq!(special_label("special:6", String::new()), "");
        for gap in [0., 6.] {
            let mut core = Core::default();
            core.theme.gap = gap as u32;
            let ui = core.overview_new(
                &[output(0, 8934., 6682.)],
                &[
                    ffi::UISpecial {
                        monitor: 0,
                        name: "special:8".into(),
                        shown: false,
                    },
                    ffi::UISpecial {
                        monitor: 0,
                        name: "special:6".into(),
                        shown: false,
                    },
                ],
                &[],
                0.,
            );
            let a = ui.cells[0].rect;
            let b = ui.cells[1].rect;
            assert!((b.x - a.x - a.w - gap).abs() < 1e-6);
            let specials: Vec<_> = ui
                .cells
                .iter()
                .filter(|c| c.key.starts_with("special:"))
                .collect();
            assert!(
                (specials[1].rect.x - specials[0].rect.x - specials[0].rect.w - gap).abs() < 1e-6
            );
        }
    }
    #[test]
    fn edp_three_two_nonzero_origin_center_gaps_and_pixel_captures() {
        let mut output = output(0, 8934., 6682.);
        output.name = "eDP-1".into();
        output.width = 2256.;
        output.height = 1504.;
        let windows = [ffi::UIWindow {
            id: "group-current".into(),
            workspace: "1".into(),
            monitor: 0,
            x: 0.,
            y: 0.,
            width: 2256.,
            height: 1504.,
            floating: false,
            title: "kitty".into(),
        }];
        let core = Core::default();
        let mut ui = core.overview_new(
            &[output],
            &[ffi::UISpecial {
                monitor: 0,
                name: "special:chat".into(),
                shown: false,
            }],
            &windows,
            0.,
        );
        let viewport = [ffi::UIRect {
            x: 0.,
            y: 0.,
            width: 2256.,
            height: 1504.,
        }];
        let mut previous_width = 0;
        for full in [false, true, false] {
            ui.fit = !full;
            ui.reflow();
            let f = ui.overview_frame(2256., 1504., 101.);
            let rects: Vec<_> = f
                .cells
                .iter()
                .map(|c| &c.rect)
                .chain(f.labels.iter().map(|l| &l.rect))
                .collect();
            let l = rects.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
            let r = rects.iter().map(|r| r.x + r.width).fold(0., f64::max);
            let t = rects.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
            let b = rects.iter().map(|r| r.y + r.height).fold(0., f64::max);
            assert!((l - (2256. - r)).abs() < 1e-6);
            assert!((t - (1504. - b)).abs() < 1e-6);
            let a = &f.cells[0].rect;
            let right = &f.cells[1].rect;
            let below = &f.cells[if full { 8 } else { 3 }].rect;
            assert!((a.width / a.height - 1.5).abs() < 1e-6);
            assert!(((right.x - a.x - a.width) - (below.y - a.y - a.height)).abs() < 1e-6);
            let plan = ui.overview_captures(&viewport);
            assert_eq!(plan.len(), 1);
            assert_eq!(plan[0].capture_width, a.width.ceil() as u32);
            assert_eq!(plan[0].capture_height, a.height.ceil() as u32);
            if full {
                assert!(plan[0].capture_width < previous_width);
            }
            previous_width = plan[0].capture_width;
            let double = ui.overview_captures(&[ffi::UIRect {
                x: 0.,
                y: 0.,
                width: 4512.,
                height: 3008.,
            }]);
            assert!(double[0].capture_width > plan[0].capture_width);
        }
        // Translation of the physical desktop must not translate local UI.
        let mut origin = super::tests::output(0, 0., 0.);
        origin.name = "eDP-1".into();
        origin.width = 2256.;
        origin.height = 1504.;
        let other = core.overview_new(
            &[origin],
            &[ffi::UISpecial {
                monitor: 0,
                name: "special:chat".into(),
                shown: false,
            }],
            &windows,
            0.,
        );
        assert_eq!(ui.cells[0].rect, other.cells[0].rect);
    }
    #[test]
    fn capture_cache_budget_and_largest_scaled_output() {
        let mut laptop = output(0, 8934., 6682.);
        laptop.name = "eDP-1".into();
        laptop.width = 2256.;
        laptop.height = 1504.;
        let windows: Vec<_> = (0..64)
            .map(|i| ffi::UIWindow {
                id: i.to_string(),
                workspace: (10 * i + 1).to_string(),
                monitor: 0,
                x: 0.,
                y: 0.,
                width: 2256.,
                height: 1504.,
                floating: false,
                title: "kitty".into(),
            })
            .collect();
        let ui = Core::default().overview_new(&[laptop], &[], &windows, 0.);
        let viewport = |width, height| ffi::UIRect {
            x: 0.,
            y: 0.,
            width,
            height,
        };
        let normal = ui.overview_captures(&[viewport(2256., 1504.)]);
        let scaled = ui.overview_captures(&[viewport(2256., 1504.), viewport(4512., 3008.)]);
        assert!(scaled[0].capture_width > normal[0].capture_width);
        let extreme = ui.overview_captures(&[viewport(30000., 20000.)]);
        assert_eq!(extreme.len(), 64);
        let bytes: f64 = extreme
            .iter()
            .map(|c| f64::from(c.capture_width) * f64::from(c.capture_height) * 4. * 4. / 3.)
            .sum();
        assert!(bytes <= 64. * 1024. * 1024.);
    }
    #[test]
    fn short_app_labels_and_centered_visible_composition() {
        assert_eq!(app_label([].into_iter()), "");
        assert_eq!(
            app_label(["org.mozilla.Firefox.desktop"].into_iter()),
            "firefox"
        );
        assert_eq!(
            app_label(["org.mozilla.Firefox", "kitty", "kitty"].into_iter()),
            "kitty +2"
        );
        assert_eq!(app_label(["", ""].into_iter()), "window +1");
        for fit in [true, false] {
            let core = Core {
                theme: crate::theme::Theme {
                    fit,
                    ..Default::default()
                },
                ..Default::default()
            };
            let ui = core.overview_new(
                &[output(0, 0., 0.), output(1, 1280., 0.)],
                &[ffi::UISpecial {
                    monitor: 0,
                    name: "special:chat".into(),
                    shown: false,
                }],
                &[],
                0.,
            );
            let f = ui.overview_frame(1280., 720., 101.);
            let top = f
                .labels
                .iter()
                .map(|l| l.rect.y)
                .fold(f64::INFINITY, f64::min);
            let bottom = f
                .cells
                .iter()
                .map(|c| c.rect.y + c.rect.height)
                .fold(0., f64::max);
            assert!((top - (720. - bottom)).abs() < 0.001);
            assert!(f
                .cells
                .iter()
                .filter(|c| !c.key.starts_with("special:"))
                .all(|c| c.label.is_empty()));
        }
    }
    #[test]
    fn fit_margin_minimum_and_toggle_preserve_aspect() {
        let mut ui = Core::default().overview_new(&[output(0, 0., 0.)], &[], &[], 0.);
        let f = ui.overview_frame(1280., 720., 101.);
        assert_eq!(f.cells.len(), 9);
        assert!((f.cells[0].rect.width / f.cells[0].rect.height - 1280. / 720.).abs() < 0.001);
        ui.overview_key(97, false, 101.);
        assert_eq!(ui.overview_frame(1280., 720., 101.).cells.len(), 64);
        ui.overview_key(97, false, 101.);
        assert_eq!(ui.overview_frame(1280., 720., 101.).cells.len(), 9);
        assert!(crate::theme::Theme::parse(r##"{"label":"#oops"}"##).is_err());
    }
    #[test]
    fn physical_layout_negative_positions_and_keyboard_groups() {
        let mut ui = Core {
            theme: crate::theme::Theme {
                fit: false,
                ..Default::default()
            },
            ..Default::default()
        }
        .overview_new(
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
        let mut ui = Core {
            theme: crate::theme::Theme {
                fit: false,
                ..Default::default()
            },
            ..Default::default()
        }
        .overview_new(&[output(0, 0.0, 0.0)], &[], &[], 0.0);
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
