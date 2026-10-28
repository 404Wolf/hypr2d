//! Header/card gestures choose mapping commands, never mutate native workspaces.
use super::*;
#[derive(Clone, Default)]
pub(super) struct State {
    pressed: Option<usize>,
    origin: (f64, f64),
    active: bool,
    target: Option<usize>,
    choose: bool,
    pub until: f64,
    now: f64,
    pulse: Option<(usize, usize, u32, u32)>,
    pulse_names: std::collections::BTreeMap<u32, String>,
}
impl State {
    pub(super) fn now(&self) -> f64 {
        self.now
    }
    pub(super) fn clock(&mut self, now: f64) {
        self.now = now;
    }
}
impl Overview {
    pub(super) fn header_at(&self) -> Option<usize> {
        self.headers
            .iter()
            .take(self.panels.len())
            .enumerate()
            .find(|(g, (r, _))| {
                self.header_visible(*g) && r.contains(self.pointer.0, self.pointer.1)
            })
            .map(|(g, _)| g)
    }
    pub(super) fn mapping_press(&mut self) -> bool {
        let group = self
            .header_at()
            .or_else(|| self.card_at().filter(|_| self.hit_window(None).is_none()));
        if let Some(group) = group {
            self.mapping.pressed = Some(group);
            self.mapping.origin = self.pointer;
            self.mapping.active = false;
            self.pressed = None;
            self.drag = None;
            return true;
        }
        false
    }
    pub(super) fn mapping_motion(&mut self) {
        if self.mapping.pressed.is_some() {
            if (self.pointer.0 - self.mapping.origin.0)
                .hypot(self.pointer.1 - self.mapping.origin.1)
                > 3.
            {
                self.mapping.active = true;
            }
            self.mapping.target = self.header_at().filter(|g| !self.parked.contains(g));
        }
    }
    pub(super) fn mapping_command(&mut self, source: usize, target: usize) -> String {
        if source == target || self.parked.contains(&target) {
            return String::new();
        }
        for g in [source, target] {
            self.mapping
                .pulse_names
                .insert(self.group_slots[g], self.grid_names[g].clone());
        }
        self.mapping.until = self.mapping.now + 180.;
        self.mapping.pulse = Some((
            source,
            target,
            self.group_slots[source],
            self.group_slots[target],
        ));
        if self.parked.contains(&source) {
            format!(
                "@adopt {} {}",
                self.group_slots[source], self.outputs[target].0
            )
        } else {
            format!(
                "@swap-grids {} {}",
                self.outputs[source].0, self.outputs[target].0
            )
        }
    }
    pub(super) fn mapping_release(&mut self) -> Option<ffi::UIDrop> {
        let source = self.mapping.pressed.take()?;
        let action = if self.mapping.active {
            self.mapping
                .target
                .and_then(|target| (!self.parked.contains(&target)).then_some(target))
                .map(|target| self.mapping_command(source, target))
                .unwrap_or_default()
        } else {
            if self.parked.contains(&source) {
                self.expanded = Some(source);
                self.reflow();
            }
            String::new()
        };
        self.mapping.active = false;
        self.mapping.target = None;
        Some(ffi::UIDrop {
            operation: action,
            ..Default::default()
        })
    }
    pub(super) fn mapping_key(&mut self, key: u32, now: f64) -> Option<String> {
        self.mapping.now = now;
        if key == 0xff1b && self.mapping.pressed.is_some() {
            self.mapping.pressed = None;
            self.mapping.active = false;
            self.mapping.target = None;
            return Some(String::new());
        }
        if key == 103 {
            self.mapping.choose = !self.mapping.choose;
            return Some(String::new());
        }
        if self.mapping.choose && key == 0xff1b {
            self.mapping.choose = false;
            return Some(String::new());
        }
        if self.mapping.choose && key == 0xff0d {
            self.mapping.choose = false;
            let source = self.cells[self.selected].group;
            let target = self.focused_panel;
            return Some(self.mapping_command(source, target));
        }
        None
    }
    pub(super) fn mapping_rects(
        &self,
        convert: impl Fn(Rect) -> ffi::UIRect,
    ) -> Vec<ffi::UIRegionRect> {
        let mut result = vec![];
        if let Some(target) = self.mapping.target.filter(|_| self.mapping.active) {
            result.push(ffi::UIRegionRect {
                key: "mapping-target".into(),
                rect: convert(self.headers[target].0),
                kind: 2,
            });
        }
        if self.mapping.choose {
            let group = self.cells[self.selected].group;
            if let Some((rect, _)) = self.headers.get(group) {
                result.push(ffi::UIRegionRect {
                    key: "mapping-pick".into(),
                    rect: convert(*rect),
                    kind: 2,
                });
            }
        }
        if self.mapping.now < self.mapping.until {
            if let Some((a, b, _, _)) = self.mapping.pulse {
                for g in [a, b] {
                    if let Some((rect, _)) = self.headers.get(g) {
                        result.push(ffi::UIRegionRect {
                            key: "mapping-swap".into(),
                            rect: convert(*rect),
                            kind: 4,
                        });
                    }
                }
            }
        }
        result
    }
    pub(super) fn mapping_labels(
        &self,
        convert: impl Fn(Rect) -> ffi::UIRect,
    ) -> Vec<ffi::UILabel> {
        let mut labels = vec![];
        if self.mapping.active {
            if let Some(g) = self.mapping.pressed {
                labels.push(ffi::UILabel {
                    text: self.grid_names[g].clone(),
                    rect: convert(Rect {
                        x: self.pointer.0 + 4.,
                        y: self.pointer.1 + 4.,
                        w: 100.,
                        h: self.header_height,
                    }),
                    colour: 0,
                    band: true,
                });
            }
        }
        if self.mapping.now < self.mapping.until {
            if let Some((a, b, a_slot, b_slot)) = self.mapping.pulse {
                let t = crate::visual::ease((self.mapping.now - self.mapping.until + 180.) / 180.);
                for (source, target, slot) in [(a, b, a_slot), (b, a, b_slot)] {
                    if let (Some((from, _)), Some((to, _))) =
                        (self.headers.get(source), self.headers.get(target))
                    {
                        labels.push(ffi::UILabel {
                            text: self
                                .mapping
                                .pulse_names
                                .get(&slot)
                                .cloned()
                                .unwrap_or_else(|| format!("grid {slot}")),
                            rect: convert(Rect {
                                x: from.x + (to.x - from.x) * t,
                                y: from.y + (to.y - from.y) * t,
                                w: 100.,
                                h: self.header_height,
                            }),
                            colour: 0,
                            band: true,
                        });
                    }
                }
            }
        }
        labels
    }
}
