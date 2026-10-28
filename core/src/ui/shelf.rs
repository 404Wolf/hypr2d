//! Paged grid shelf. Miniatures reuse the main workspace texture cache.
use super::*;
use std::collections::BTreeMap;
#[derive(Clone, Default)]
pub(super) struct State {
    pub offset: usize,
    pub area: Rect,
    pub(super) headers: BTreeMap<usize, Rect>,
    captions: BTreeMap<usize, Rect>,
    minis: Vec<CellView>,
    backgrounds: Vec<(usize, Rect, u32, u32)>,
    capacity: usize,
    title: Rect,
}
impl Overview {
    pub(super) fn refresh_panel_counts(&mut self) {
        self.panel_counts = self
            .group_slots
            .iter()
            .zip(&self.grid_specs)
            .map(|(slot, grid)| {
                self.windows
                    .iter()
                    .filter(|w| {
                        w.workspace
                            .parse()
                            .ok()
                            .and_then(|id| grid.cell(id, *slot))
                            .is_some()
                    })
                    .map(|w| w.group_size.max(1) as usize)
                    .sum()
            })
            .collect();
    }
    pub(super) fn ensure_expanded_cells(&mut self) {
        let Some(g) = self.expanded else { return };
        if !self.materialized.insert(g) {
            return;
        }
        let grid = self.grid_specs[g];
        let present: BTreeSet<_> = self
            .cells
            .iter()
            .filter(|c| c.group == g)
            .map(|c| (c.x, c.y))
            .collect();
        for y in 0..grid.rows {
            for x in 0..grid.columns {
                if present.contains(&(x, y)) {
                    continue;
                }
                let Ok(id) = grid.id(Cell { x, y }, self.group_slots[g]) else {
                    continue;
                };
                let key = id.to_string();
                self.cells.push(CellView {
                    occupied: self.windows.iter().any(|w| w.workspace == key),
                    label: owned_label(&self.windows, &key),
                    key,
                    monitor: self.outputs[g].0,
                    rect: Rect::default(),
                    group: g,
                    x,
                    y,
                    shown: self.group_current[g] == id,
                    visible: false,
                    annotated: false,
                });
            }
        }
    }
    pub(super) fn layout_shelf(&mut self, left: f64, right: f64, bottom: f64, _n: f64) {
        let left = if left.is_finite() { left } else { 8. };
        let right = if right.is_finite() {
            right.max(left + 168.)
        } else {
            992.
        };
        let width = right - left;
        let h = self.header_height;
        let gap = self.header_gap;
        let special = self.panels.len();
        let count = self.cells.iter().filter(|c| c.group == special).count();
        let special_width = if count == 0 {
            0.
        } else {
            (count as f64 * 150.).min(width * 0.48)
        };
        let start = left + special_width + if count > 0 { gap } else { 0. };
        let y = bottom + h + 2. * gap;
        if count > 0 {
            let w = (special_width / count as f64).max(1.);
            for (i, c) in self
                .cells
                .iter_mut()
                .filter(|c| c.group == special)
                .enumerate()
            {
                let ratio = self
                    .outputs
                    .iter()
                    .find(|o| o.0 == c.monitor)
                    .map(|o| o.2 / o.1)
                    .unwrap_or(2. / 3.);
                c.rect = Rect {
                    x: left + i as f64 * w,
                    y,
                    w: w - self.gap,
                    h: w * ratio,
                };
                c.visible = true;
            }
            self.headers[special].0 = Rect {
                x: left,
                y: bottom + gap,
                w: special_width,
                h,
            };
        }
        let shelf_width = (right - start).max(1.);
        let capacity = (shelf_width / 168.).floor().max(1.) as usize;
        self.shelf.capacity = capacity;
        self.shelf.offset = self
            .shelf
            .offset
            .min(self.panels.len().saturating_sub(capacity));
        self.shelf.headers.clear();
        self.shelf.captions.clear();
        self.shelf.minis.clear();
        self.shelf.backgrounds.clear();
        self.shelf.title = Rect {
            x: start,
            y: bottom + gap,
            w: shelf_width,
            h,
        };
        let card_w = (shelf_width / capacity as f64).min(220.);
        let image_h = 82.;
        let card_y = y + h + gap;
        self.shelf.area = Rect {
            x: start,
            y: bottom + gap,
            w: shelf_width,
            h: card_y + image_h + h + gap - (bottom + gap),
        };
        for g in self.shelf.offset..(self.shelf.offset + capacity).min(self.panels.len()) {
            let x = start + (g - self.shelf.offset) as f64 * card_w;
            let body = Rect {
                x,
                y: card_y,
                w: (card_w - gap).max(1.),
                h: image_h,
            };
            let parked = self.parked.contains(&g) && self.expanded != Some(g);
            let grid = self.grid_specs[g];
            let ratio = self.outputs[g].2 / self.outputs[g].1;
            let w = (body.w / grid.columns as f64).min(body.h / (grid.rows as f64 * ratio));
            let bounds = Rect {
                x: body.x + (body.w - w * grid.columns as f64) / 2.,
                y: body.y,
                w: w * grid.columns as f64,
                h: w * ratio * grid.rows as f64,
            };
            // Align names with the painted miniature, not its padded card slot.
            let title = Rect {
                x: bounds.x,
                y: bounds.y - gap - h,
                w: bounds.w,
                h,
            };
            self.cards.insert(g, bounds);
            self.shelf.headers.insert(g, title);
            self.shelf.captions.insert(
                g,
                Rect {
                    x: bounds.x,
                    y: card_y + image_h + gap,
                    w: body.x + body.w - bounds.x,
                    h,
                },
            );
            if parked {
                self.headers[g] = (title, self.grid_names[g].clone());
            }
            self.shelf
                .backgrounds
                .push((g, bounds, grid.columns, grid.rows));
            for c in self.cells.iter_mut().filter(|c| c.group == g) {
                let rect = Rect {
                    x: bounds.x + c.x as f64 * w,
                    y: bounds.y + c.y as f64 * w * ratio,
                    w,
                    h: w * ratio,
                };
                if parked {
                    c.rect = rect;
                    c.visible = true;
                } else if c.occupied || c.shown {
                    let mut mini = c.clone();
                    mini.rect = rect;
                    mini.label.clear();
                    self.shelf.minis.push(mini);
                }
            }
        }
    }
    pub(super) fn shelf_header_at(&self) -> Option<usize> {
        self.shelf
            .headers
            .iter()
            .find(|(_, r)| r.contains(self.pointer.0, self.pointer.1))
            .map(|(g, _)| *g)
    }
    pub(super) fn reveal_shelf(&mut self, g: usize) {
        let cap = self.shelf.capacity.max(1);
        if g < self.shelf.offset {
            self.shelf.offset = g;
        } else if g >= self.shelf.offset + cap {
            self.shelf.offset = g + 1 - cap;
        }
    }
    pub(super) fn return_live(&mut self) {
        self.expanded = None;
        self.selected = self
            .cells
            .iter()
            .position(|c| c.group == self.focused_panel && c.shown)
            .or_else(|| {
                self.cells
                    .iter()
                    .position(|c| c.group == self.focused_panel)
            })
            .unwrap_or(self.selected);
        self.reflow();
    }
    pub(super) fn select_card(&mut self, g: usize) {
        self.expanded = self.parked.contains(&g).then_some(g);
        self.selected = self
            .cells
            .iter()
            .position(|c| c.group == g && c.shown)
            .or_else(|| self.cells.iter().position(|c| c.group == g))
            .unwrap_or(self.selected);
        self.reveal_shelf(g);
        self.reflow();
    }
    pub fn overview_scroll(&mut self, delta: f64) {
        guarded(|| {
            if delta != 0. && self.shelf.area.contains(self.pointer.0, self.pointer.1) {
                self.page_shelf(delta > 0.);
            }
        });
    }
    fn page_shelf(&mut self, next: bool) {
        let n = self.shelf.capacity.max(1);
        self.shelf.offset = if next {
            self.shelf.offset.saturating_add(n)
        } else {
            self.shelf.offset.saturating_sub(n)
        };
        self.reflow();
    }
    pub fn overview_pick_grid(&mut self, response: &str) {
        guarded(|| {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(response) {
                if let Some(id) = value["slot"].as_u64() {
                    if let Some(g) = self.group_slots.iter().position(|s| u64::from(*s) == id) {
                        self.reveal_shelf(g);
                        self.reflow();
                    }
                }
            }
        });
    }
    pub(super) fn shelf_key(&mut self, key: u32) -> Option<String> {
        if self.region.mode != 0 {
            return None;
        }
        match key {
            0xff55 => self.page_shelf(false),
            0xff56 => self.page_shelf(true),
            110 => self.rename = Some(("grid:new".into(), String::new())),
            0xffff => {
                let g = self
                    .mapping
                    .chosen()
                    .or_else(|| self.shelf_header_at())
                    .or_else(|| self.card_at())?;
                return Some(format!(
                    "@catalog:{}",
                    crate::grid_args("grid delete", &self.group_slots[g].to_string(), "")
                ));
            }
            _ => return None,
        }
        Some(String::new())
    }
    pub(super) fn shelf_labels(&self, convert: impl Fn(Rect) -> ffi::UIRect) -> Vec<ffi::UILabel> {
        let mut labels = vec![ffi::UILabel {
            text: format!(
                "Grids  {}–{}/{} · PgUp/PgDn · n: new",
                self.shelf.offset + 1,
                (self.shelf.offset + self.shelf.capacity).min(self.panels.len()),
                self.panels.len()
            ),
            rect: convert(self.shelf.title),
            colour: 0,
            band: false,
        }];
        for (g, rect) in &self.shelf.headers {
            if !self.parked.contains(g) || self.expanded == Some(*g) {
                labels.push(ffi::UILabel {
                    text: self.grid_names[*g].clone(),
                    rect: convert(*rect),
                    colour: 0,
                    band: false,
                });
            }
            let status = if self.parked.contains(g) {
                if self.parked_attached.contains(g) {
                    "parked"
                } else {
                    "disconnected"
                }
            } else {
                &self.panel_names[*g]
            };
            labels.push(ffi::UILabel {
                text: format!("{status} · {} windows", self.panel_counts[*g]),
                rect: convert(self.shelf.captions[g]),
                colour: 0,
                band: false,
            });
        }
        labels
    }
    pub(super) fn shelf_frame_cells(
        &self,
        convert: impl Fn(Rect) -> ffi::UIRect,
    ) -> Vec<ffi::UICell> {
        self.shelf
            .minis
            .iter()
            .map(|c| ffi::UICell {
                key: c.key.clone(),
                monitor: c.monitor,
                rect: convert(c.rect),
                label: String::new(),
                secondary: String::new(),
                colour: 0,
                selected: false,
                hovered: false,
                occupied: c.occupied,
                shown: c.shown,
                dimmed: false,
            })
            .collect()
    }
    pub(super) fn shelf_frame_backdrops(
        &self,
        convert: impl Fn(Rect) -> ffi::UIRect,
    ) -> Vec<ffi::UIBackdropPanel> {
        self.shelf
            .backgrounds
            .iter()
            .map(|(g, r, columns, rows)| ffi::UIBackdropPanel {
                monitor: self.outputs[*g].0,
                rect: convert(*r),
                columns: *columns,
                rows: *rows,
            })
            .collect()
    }
    pub(super) fn shelf_snapshot(&self, s: f64, ox: f64, oy: f64) -> serde_json::Value {
        let cards:Vec<_>=self.cards.iter().map(|(g,r)|{
            let h=self.shelf.headers[g];
            serde_json::json!({"slot":self.group_slots[*g],"name":self.grid_names[*g],"host":if self.parked.contains(g){None}else{Some(self.panel_names[*g].clone())},"windows":self.panel_counts[*g],"rect":[r.x*s+ox,r.y*s+oy,r.w*s,r.h*s],"header":[h.x*s+ox,h.y*s+oy,h.w*s,h.h*s]})
        }).collect();
        serde_json::json!({"offset":self.shelf.offset,"total":self.panels.len(),"cards":cards})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn shelf() -> Box<Overview> {
        let core = Core::default();
        let outputs: Vec<_> = (0..18)
            .map(|i| {
                let mut o = super::super::tests::output(i, 8934., 6682.);
                o.width = 2256.;
                o.height = 1504.;
                o.slot = if i < 10 {
                    i
                } else {
                    crate::grid::VIRTUAL_GRID | (16384 + i)
                };
                o.workspace = core
                    .slot_grid(o.slot, "")
                    .id(Cell { x: 0, y: 0 }, o.slot)
                    .unwrap();
                o.parked = i > 0;
                o.backing = 0;
                o
            })
            .collect();
        core.overview_new(&outputs, &[], &[], 0.)
    }
    #[test]
    fn pages_reach_every_grid_and_expand_lazily() {
        let mut ui = shelf();
        assert!(ui.cells.len() < 18 * 64);
        assert!(ui.cards.len() < 18);
        ui.reveal_shelf(17);
        ui.reflow();
        assert!(ui.cards.contains_key(&17));
        assert!(!ui.materialized.contains(&17));
        ui.select_card(17);
        assert!(ui.materialized.contains(&17));
        assert_eq!(ui.cells.iter().filter(|c| c.group == 17).count(), 64);
        assert_eq!(ui.overview_key(0xff1b, false, 1000.), "");
        assert!(ui.expanded.is_none() && ui.closing.is_none());
        assert_eq!(ui.cells[ui.selected].group, 0);
        ui.select_card(0);
        assert!(ui.expanded.is_none());
    }
    #[test]
    fn laptop_shelf_fits_and_physical_scaled_hit_testing_agrees() {
        let ui = shelf();
        for scale in [1., 1.5, 2.] {
            let layout = ui.scaled_viewport(2256. * scale, 1504. * scale, scale);
            let frame = layout.frame(2256. * scale, 1504. * scale, 1000.);
            for r in frame
                .cells
                .iter()
                .map(|c| &c.rect)
                .chain(frame.labels.iter().map(|l| &l.rect))
            {
                assert!(
                    r.x >= 0.
                        && r.y >= 0.
                        && r.x + r.width <= 2256. * scale + 0.01
                        && r.y + r.height <= 1504. * scale + 0.01
                );
            }
            let (s, ox, oy) = layout.transform(2256. * scale, 1504. * scale);
            let (&g, r) = layout.shelf.headers.iter().next().unwrap();
            for (group, bounds, _, _) in &layout.shelf.backgrounds {
                let header = layout.shelf.headers[group];
                assert!((header.x - bounds.x).abs() < 1e-6);
                assert!((header.y + header.h + layout.header_gap - bounds.y).abs() < 1e-6);
            }

            let physical = ((r.x + r.w / 2.) * s + ox, (r.y + r.h / 2.) * s + oy);
            let mut logical = ui.for_viewport(2256., 1504.);
            let (s, ox, oy) = logical.transform(2256., 1504.);
            logical.pointer = ((physical.0 / scale - ox) / s, (physical.1 / scale - oy) / s);
            assert_eq!(logical.shelf_header_at(), Some(g));
        }
    }
    #[test]
    fn keyboard_chooser_reserves_its_help_and_cannot_interrupt_a_region() {
        let mut ui = shelf();
        ui.overview_key(103, false, 1000.);
        let frame = ui.overview_frame(2256., 1504., 1000.);
        assert!(frame.labels.iter().any(|l| l.text.contains("Enter: bind")));
        assert!(frame
            .labels
            .iter()
            .all(|l| l.rect.y >= 0. && l.rect.y + l.rect.height <= 1504.));
        ui.overview_key(0xff1b, false, 1000.);
        ui.overview_key(118, false, 1000.);
        assert_ne!(ui.region.mode, 0);
        ui.overview_key(103, false, 1000.);
        assert!(ui.mapping.chosen().is_none());
    }
    #[test]
    fn catalogue_editor_and_delete_use_encoded_deferred_actions() {
        let mut ui = shelf();
        ui.overview_key(110, false, 1000.);
        for c in "a \"new\" grid".chars() {
            ui.overview_key(c as u32, false, 1000.);
        }
        let command = ui.overview_key(0xff0d, false, 1000.);
        let args: Vec<String> =
            serde_json::from_str(command.strip_prefix("@catalog:grid create ").unwrap()).unwrap();
        assert_eq!(args, ["a \"new\" grid"]);
        let r = ui.shelf.headers[&0];
        ui.pointer = (r.x + r.w / 2., r.y + r.h / 2.);
        assert_eq!(
            ui.shelf_key(0xffff).unwrap(),
            "@catalog:grid delete [\"0\"]"
        );
    }
}
