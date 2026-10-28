//! Header/card gestures choose mapping commands, never mutate native workspaces.
use super::*;
#[derive(Clone, Default)]
pub(super) struct State {
    pressed: Option<usize>,
    origin: (f64, f64),
    active: bool,
    target: Option<usize>,
    choose: bool,
    chosen: usize,
    host: usize,
    pub until: f64,
    now: f64,
    pulse: Option<(usize, usize, u32, u32)>,
    pulse_names: std::collections::BTreeMap<u32, String>,
}
impl State {
    pub(super) fn chosen(&self) -> Option<usize> {
        self.choose.then_some(self.chosen)
    }
    pub(super) fn now(&self) -> f64 {
        self.now
    }
    pub(super) fn clock(&mut self, now: f64) {
        self.now = now;
    }
}
impl Overview {
    pub(super) fn header_at(&self) -> Option<usize> {
        self.shelf_header_at().or_else(|| {
            self.headers
                .iter()
                .take(self.panels.len())
                .enumerate()
                .find(|(g, (r, _))| {
                    self.header_visible(*g) && r.contains(self.pointer.0, self.pointer.1)
                })
                .map(|(g, _)| g)
        })
    }
    pub(super) fn mapping_press(&mut self) -> bool {
        let group = self
            .header_at()
            .or_else(|| self.card_at().filter(|_| self.hit_window(None).is_none()));
        if let Some(group) = group {
            self.region = region::State::default();
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
            self.mapping.target = self
                .header_at()
                .or_else(|| {
                    self.cells
                        .iter()
                        .find(|c| {
                            c.visible
                                && c.group < self.panels.len()
                                && !self.parked.contains(&c.group)
                                && c.rect.contains(self.pointer.0, self.pointer.1)
                        })
                        .map(|c| c.group)
                })
                .filter(|g| !self.parked.contains(g));
        }
    }
    pub(super) fn mapping_command(&mut self, source: usize, target: usize) -> String {
        if source == target || self.parked.contains(&target) {
            return String::new();
        }
        if let Some(i) = self
            .cells
            .iter()
            .position(|c| c.group == source && c.shown)
            .or_else(|| self.cells.iter().position(|c| c.group == source))
        {
            self.selected = i;
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
            self.select_card(source);
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
            self.mapping.chosen = self.cells[self.selected]
                .group
                .min(self.panels.len().saturating_sub(1));
            self.mapping.host = self.focused_panel;
            return Some(String::new());
        }
        if self.mapping.choose {
            if key == 0xffff {
                return None;
            }
            match key {
                0xff1b => self.mapping.choose = false,
                0xff0d => {
                    self.mapping.choose = false;
                    return Some(self.mapping_command(self.mapping.chosen, self.mapping.host));
                }
                0xff09 | 0xfe20 => {
                    let live: Vec<_> = (0..self.panels.len())
                        .filter(|g| !self.parked.contains(g))
                        .collect();
                    if let Some(i) = live.iter().position(|g| *g == self.mapping.host) {
                        self.mapping.host = live[(i + if self.shift || key == 0xfe20 {
                            live.len() - 1
                        } else {
                            1
                        }) % live.len()];
                    }
                }
                104 | 106 | 107 | 108 | 0xff51 | 0xff52 | 0xff53 | 0xff54 => {
                    let n = self.panels.len();
                    let back = matches!(key, 104 | 107 | 0xff51 | 0xff52);
                    self.mapping.chosen = (self.mapping.chosen + if back { n - 1 } else { 1 }) % n;
                    self.reveal_shelf(self.mapping.chosen);
                    self.reflow();
                }
                _ => {}
            }
            return Some(String::new());
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
            let group = self.mapping.chosen;
            if let Some(rect) = self.shelf.headers.get(&group) {
                result.push(ffi::UIRegionRect {
                    key: "mapping-pick".into(),
                    rect: convert(*rect),
                    kind: 1,
                });
            }
        }
        if self.mapping.choose {
            result.push(ffi::UIRegionRect {
                key: "mapping-host".into(),
                rect: convert(self.headers[self.mapping.host].0),
                kind: 2,
            });
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
    pub(super) fn mapping_footer(&self) -> Rect {
        Rect {
            x: 24.,
            y: self.shelf.area.y + self.shelf.area.h + self.header_gap,
            w: 950.,
            h: self.header_height,
        }
    }
    pub(super) fn mapping_labels(
        &self,
        convert: impl Fn(Rect) -> ffi::UIRect,
    ) -> Vec<ffi::UILabel> {
        let mut labels = vec![];
        if self.mapping.choose {
            labels.push(ffi::UILabel {
                text: format!(
                    "{} → {} · hjkl: grid · Tab: display · Enter: bind · Esc: cancel",
                    self.grid_names[self.mapping.chosen], self.panel_names[self.mapping.host]
                ),
                rect: convert(self.mapping_footer()),
                colour: 0,
                band: true,
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chooser_accepts_iso_left_tab_from_a_real_shift_tab_key() {
        let mut ui = Core::default().overview_new(
            &[
                super::super::tests::output(0, 0., 0.),
                super::super::tests::output(1, 1280., 0.),
                super::super::tests::output(2, 2560., 0.),
            ],
            &[],
            &[],
            0.,
        );
        ui.mapping_key(103, 1000.);
        assert_eq!(ui.mapping.host, 0);
        ui.mapping_key(0xfe20, 1000.);
        assert_eq!(ui.mapping.host, 2);
        ui.mapping_key(0xff09, 1000.);
        assert_eq!(ui.mapping.host, 0);
    }
}
