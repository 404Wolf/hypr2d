//! Marquee, block pickup and keyboard controls share one cell-index plan.
use super::*;
#[derive(Clone, Default)]
pub(super) struct State {
    // 0=none, 1=draft, 2=selected/extending, 3=picked up.
    pub mode: u8,
    anchor: (usize, u32, u32),
    end: (u32, u32),
    target: (usize, i32, i32),
    origin: (f64, f64),
    grab: (i32, i32),
    moved: bool,
    keyboard: bool,
}
impl Overview {
    pub fn overview_input_modifiers(&mut self, shift: bool, alt: bool, control: bool) {
        guarded(|| {
            self.shift = shift;
            self.alt = alt;
            self.control = control;
        });
    }
    fn region_contains(&self, c: &CellView) -> bool {
        let r = &self.region;
        c.group == r.anchor.0
            && c.x >= r.anchor.1.min(r.end.0)
            && c.x <= r.anchor.1.max(r.end.0)
            && c.y >= r.anchor.2.min(r.end.1)
            && c.y <= r.anchor.2.max(r.end.1)
    }
    pub(super) fn region_press(&mut self) -> bool {
        let Some(i) = self.hover else { return false };
        let c = &self.cells[i];
        if c.group >= self.panels.len() {
            return false;
        }
        if self.region.mode == 2 && self.region_contains(c) && !self.control {
            self.region.mode = 3;
            self.region.keyboard = false;
            self.region.grab = (
                c.x as i32 - self.region.anchor.1.min(self.region.end.0) as i32,
                c.y as i32 - self.region.anchor.2.min(self.region.end.1) as i32,
            );
            self.region.target = (
                c.group,
                c.x as i32 - self.region.grab.0,
                c.y as i32 - self.region.grab.1,
            );
            self.drag = None;
            return true;
        }
        if self.control || self.hit_window(None).is_none() {
            self.region = State {
                mode: 1,
                anchor: (c.group, c.x, c.y),
                end: (c.x, c.y),
                origin: self.pointer,
                ..Default::default()
            };
            self.pressed = Some(i);
            self.drag = None;
            return true;
        }
        self.region = State::default();
        false
    }
    pub(super) fn region_motion(&mut self) {
        if self.region.mode == 1 {
            if (self.pointer.0 - self.region.origin.0).hypot(self.pointer.1 - self.region.origin.1)
                > 2.
            {
                self.region.moved = true;
            }
            if let Some(c) = self
                .hover
                .and_then(|i| self.cells.get(i))
                .filter(|c| c.group == self.region.anchor.0)
            {
                self.region.end = (c.x, c.y);
            }
        } else if self.region.mode == 3 && !self.region.keyboard {
            if let Some(c) = self
                .hover
                .and_then(|i| self.cells.get(i))
                .filter(|c| c.group < self.panels.len())
            {
                self.region.target = (
                    c.group,
                    c.x as i32 - self.region.grab.0,
                    c.y as i32 - self.region.grab.1,
                );
            }
        }
    }
    fn region_request(&self) -> Option<crate::region::Request> {
        let r = &self.region;
        let a = r.anchor.0;
        let b = r.target.0;
        Some(crate::region::Request {
            source_slot: *self.group_slots.get(a)?,
            target_slot: *self.group_slots.get(b)?,
            source_monitor: *self.backing.get(&self.outputs.get(a)?.0)?,
            target_monitor: *self.backing.get(&self.outputs.get(b)?.0)?,
            x: r.anchor.1.min(r.end.0),
            y: r.anchor.2.min(r.end.1),
            width: r.anchor.1.abs_diff(r.end.0) + 1,
            height: r.anchor.2.abs_diff(r.end.1) + 1,
            target_x: r.target.1,
            target_y: r.target.2,
            swap: self.alt,
        })
    }
    fn region_drop(&mut self) -> ffi::UIDrop {
        let Some(request) = self.region_request() else {
            return ffi::UIDrop::default();
        };
        let valid = crate::region::plan(
            self.grid_specs[self.region.anchor.0],
            self.grid_specs[self.region.target.0],
            &request,
        );
        if valid.is_err() {
            self.overview_refuse("region", self.started + 1000.);
            self.region.mode = 2;
            return ffi::UIDrop::default();
        }
        self.region = State::default();
        self.pressed = None;
        ffi::UIDrop {
            operation: format!(
                "@region:{}",
                serde_json::to_string(&request).unwrap_or_default()
            ),
            ..Default::default()
        }
    }
    pub(super) fn region_release(&mut self) -> Option<ffi::UIDrop> {
        if self.region.mode == 1 {
            let moved = self.region.moved || self.control;
            self.region.mode = if moved { 2 } else { 0 };
            if moved {
                self.pressed = None;
                return Some(ffi::UIDrop::default());
            }
        } else if self.region.mode == 3 && !self.region.keyboard {
            return Some(self.region_drop());
        }
        None
    }
    pub(super) fn region_key(&mut self, key: u32) -> Option<String> {
        if self.control && key == 122 {
            self.region = State::default();
            return Some("@undo".into());
        }
        if key == 118 {
            let c = &self.cells[self.selected];
            if c.group < self.panels.len() {
                self.region = State {
                    mode: 2,
                    anchor: (c.group, c.x, c.y),
                    end: (c.x, c.y),
                    keyboard: true,
                    ..Default::default()
                };
            }
            return Some(String::new());
        }
        if key == 0xff1b && self.region.mode != 0 {
            self.region = State::default();
            self.pressed = None;
            self.cancelled_release = true;
            return Some(String::new());
        }
        if self.region.mode == 0 {
            return None;
        }
        if matches!(key, 0xff09 | 0xfe20) && self.region.mode == 3 {
            let count = self.panels.len();
            if count > 0 {
                self.region.target.0 = if self.shift || key == 0xfe20 {
                    (self.region.target.0 + count - 1) % count
                } else {
                    (self.region.target.0 + 1) % count
                };
                let group = self.region.target.0;
                self.expanded = self.parked.contains(&group).then_some(group);
                self.reflow();
            }
            return Some(String::new());
        }
        if key == 109 && self.region.mode == 2 {
            let r = &mut self.region;
            r.mode = 3;
            r.keyboard = true;
            r.target = (
                r.anchor.0,
                r.anchor.1.min(r.end.0) as i32,
                r.anchor.2.min(r.end.1) as i32,
            );
            return Some(String::new());
        }
        if key == 0xff0d && self.region.mode == 3 {
            return Some(self.region_drop().operation);
        }
        let delta = match key {
            104 | 0xff51 => (-1, 0),
            108 | 0xff53 => (1, 0),
            107 | 0xff52 => (0, -1),
            106 | 0xff54 => (0, 1),
            _ => return None,
        };
        let r = &mut self.region;
        if r.mode == 3 {
            r.target.1 += delta.0;
            r.target.2 += delta.1;
        } else {
            let g = self.grid_specs[r.anchor.0];
            r.end = (
                (r.end.0 as i32 + delta.0).clamp(0, g.columns as i32 - 1) as u32,
                (r.end.1 as i32 + delta.1).clamp(0, g.rows as i32 - 1) as u32,
            );
        }
        Some(String::new())
    }
    pub(super) fn region_frame(
        &self,
        convert: impl Fn(Rect) -> ffi::UIRect,
    ) -> (Vec<ffi::UIRegionRect>, Vec<ffi::UIGhost>) {
        if self.region.mode == 0 {
            return (vec![], vec![]);
        }
        let mut rects = vec![];
        let mut ghosts = vec![];
        for c in self
            .cells
            .iter()
            .filter(|c| c.visible && self.region_contains(c))
        {
            rects.push(ffi::UIRegionRect {
                key: c.key.clone(),
                rect: convert(c.rect),
                kind: if self.refused == "region" && self.refused_until > self.mapping.now() {
                    3
                } else {
                    1
                },
            });
        }
        if self.region.mode == 3 {
            if let Some(r) = self.region_request() {
                let mapping = crate::region::plan(
                    self.grid_specs[self.region.anchor.0],
                    self.grid_specs[self.region.target.0],
                    &r,
                );
                if let Ok(map) = mapping {
                    for (from, to) in map.iter().filter(|(from, _)| {
                        self.grid_specs[self.region.anchor.0]
                            .cell(**from, r.source_slot)
                            .is_some()
                    }) {
                        if let Some(c) = self
                            .cells
                            .iter()
                            .find(|c| c.visible && c.key == to.to_string())
                        {
                            rects.push(ffi::UIRegionRect {
                                key: c.key.clone(),
                                rect: convert(c.rect),
                                kind: 2,
                            });
                            ghosts.push(ffi::UIGhost {
                                visible: true,
                                key: from.to_string(),
                                rect: convert(c.rect),
                                uv: ffi::UIRect {
                                    x: 0.,
                                    y: 0.,
                                    width: 1.,
                                    height: 1.,
                                },
                            });
                        }
                    }
                } else {
                    for c in self
                        .cells
                        .iter()
                        .filter(|c| c.visible && c.group == self.region.target.0)
                    {
                        rects.push(ffi::UIRegionRect {
                            key: c.key.clone(),
                            rect: convert(c.rect),
                            kind: 3,
                        });
                    }
                }
            }
        }
        (rects, ghosts)
    }
}
