//! Drag decisions and window hit testing; the shim only applies validated plans.
use super::*;
#[derive(Clone)]
pub(super) struct Drag {
    pub(super) window: Window,
    origin: (f64, f64),
    grab: (f64, f64),
    pub(super) active: bool,
    rect: Rect,
    cell: Rect,
}
impl Overview {
    fn output_size(&self, monitor: u32) -> Option<(f64, f64)> {
        self.outputs
            .iter()
            .find(|o| o.0 == monitor)
            .map(|o| (o.1, o.2))
    }
    fn window_rect(&self, w: &Window) -> Option<Rect> {
        let cell = self
            .cells
            .iter()
            .find(|c| c.visible && c.key == w.workspace)?;
        let (width, height) = self.output_size(cell.monitor)?;
        Some(Rect {
            x: cell.rect.x + w.rect.x / width * cell.rect.w,
            y: cell.rect.y + w.rect.y / height * cell.rect.h,
            w: w.rect.w / width * cell.rect.w,
            h: w.rect.h / height * cell.rect.h,
        })
    }
    pub(super) fn visible_window_rect(&self, w: &Window) -> Option<Rect> {
        let r = self.window_rect(w)?;
        let c = &self
            .cells
            .iter()
            .find(|c| c.visible && c.key == w.workspace)?
            .rect;
        let x = r.x.max(c.x);
        let y = r.y.max(c.y);
        let right = (r.x + r.w).min(c.x + c.w);
        let bottom = (r.y + r.h).min(c.y + c.h);
        (right > x && bottom > y).then_some(Rect {
            x,
            y,
            w: right - x,
            h: bottom - y,
        })
    }
    pub(super) fn hit_window(&self, ignore: Option<&str>) -> Option<&Window> {
        let cell = self.cells.get(self.hover?)?;
        // The overlaid workspace caption is a move target, even when a
        // fullscreen window occupies every pixel below it.
        if ignore.is_some()
            && self.pointer.1
                < cell.rect.y
                    + if cell.key.starts_with("special:") {
                        (cell.rect.h * 0.35).min(10.0)
                    } else {
                        (cell.rect.h * 0.15).min(4.0)
                    }
        {
            return None;
        }
        self.windows
            .iter()
            .rev()
            .filter(|w| w.floating)
            .chain(self.windows.iter().rev().filter(|w| !w.floating))
            .find(|w| {
                w.workspace == cell.key
                    && Some(w.id.as_str()) != ignore
                    && self
                        .window_rect(w)
                        .is_some_and(|r| r.contains(self.pointer.0, self.pointer.1))
            })
    }
    pub(super) fn validate_drag(&mut self) {
        if self.drag.as_ref().is_some_and(|d| {
            !self
                .windows
                .iter()
                .any(|w| w.id == d.window.id && w.workspace == d.window.workspace)
        }) {
            self.drag = None;
            self.pressed = None;
            self.cancelled_release = true;
        }
    }
    pub(super) fn drag_motion(&mut self, scale: f64) {
        if let Some(d) = &mut self.drag {
            if ((self.pointer.0 - d.origin.0).hypot(self.pointer.1 - d.origin.1)) * scale >= 6.0 {
                d.active = true;
            }
        }
    }
    pub fn overview_press(&mut self, width: f64, height: f64, x: f64, y: f64) {
        guarded(|| {
            if self.rename.is_some() {
                return;
            }
            self.overview_pointer(width, height, x, y, false);
            if self.closing.is_some() {
                return;
            }
            self.cancelled_release = false;
            if !self.parked.is_empty() && self.live_tab.contains(self.pointer.0, self.pointer.1) {
                self.expanded = None;
                self.reflow();
                return;
            }
            if !self.parked.is_empty() && self.parked_tab.contains(self.pointer.0, self.pointer.1) {
                self.expanded = self.parked.iter().next().copied();
                self.reflow();
                return;
            }
            if self.mapping_press() {
                return;
            }
            if self.region_press() {
                return;
            }
            self.pressed = self.hover;
            self.pressed_card = self.card_at();
            self.drag = self.hit_window(None).cloned().and_then(|w| {
                let c = self.cells.get(self.hover?)?;
                let (size_x, size_y) = self.output_size(c.monitor)?;
                let grab = (
                    (self.pointer.0 - c.rect.x) / c.rect.w * size_x - w.rect.x,
                    (self.pointer.1 - c.rect.y) / c.rect.h * size_y - w.rect.y,
                );
                Some(Drag {
                    rect: self.visible_window_rect(&w)?,
                    cell: c.rect,
                    window: w,
                    origin: self.pointer,
                    grab,
                    active: false,
                })
            });
        });
    }
    pub fn overview_release(&mut self, width: f64, height: f64, x: f64, y: f64) -> ffi::UIDrop {
        guarded(|| {
            if self.rename.is_some() {
                return ffi::UIDrop::default();
            }
            self.overview_pointer(width, height, x, y, false);
            if let Some(drop) = self.mapping_release() {
                return drop;
            }
            if let Some(drop) = self.region_release() {
                return drop;
            }
            let pressed = self.pressed.take();
            let drag = self.drag.take();
            if self.cancelled_release || self.closing.is_some() {
                self.cancelled_release = false;
                return ffi::UIDrop::default();
            }
            if let Some(d) = drag.filter(|d| d.active) {
                let Some(c) = self.hover.and_then(|i| self.cells.get(i)) else {
                    return ffi::UIDrop::default();
                };
                let Some((size_x, size_y)) = self.output_size(c.monitor) else {
                    return ffi::UIDrop::default();
                };
                let swap = self
                    .hit_window(Some(&d.window.id))
                    .map(|w| w.id.clone())
                    .unwrap_or_default();
                return ffi::UIDrop {
                    operation: String::new(),
                    window: d.window.id,
                    stack: if self.alt || self.shift {
                        String::new()
                    } else {
                        swap.clone()
                    },
                    swap: if self.alt { swap } else { String::new() },
                    whole_group: self.shift,
                    workspace: c.key.clone(),
                    monitor: c.monitor,
                    x: ((self.pointer.0 - c.rect.x) / c.rect.w * size_x - d.grab.0),
                    y: ((self.pointer.1 - c.rect.y) / c.rect.h * size_y - d.grab.1),
                    floating: d.window.floating,
                    activate: String::new(),
                };
            }
            if self
                .pressed_card
                .take()
                .is_some_and(|g| self.card_at() == Some(g))
            {
                if let Some(g) = self.card_at() {
                    self.expanded = Some(g);
                    self.reflow();
                }
                return ffi::UIDrop::default();
            }
            if self.hover == pressed {
                if let Some(i) = self.hover {
                    self.selected = i;
                    return ffi::UIDrop {
                        activate: self.activate_selected(),
                        ..Default::default()
                    };
                }
            }
            ffi::UIDrop::default()
        })
    }
    pub(super) fn drag_frame(
        &self,
        convert: impl Fn(Rect) -> ffi::UIRect,
    ) -> (ffi::UIGhost, Vec<ffi::UIWindowView>, String) {
        let dragged = self.drag.as_ref().filter(|d| d.active);
        let mut target = dragged
            .and_then(|_| self.hover.and_then(|i| self.cells.get(i)))
            .map(|c| c.key.clone())
            .unwrap_or_default();
        let hovered = self
            .hit_window(dragged.map(|d| d.window.id.as_str()))
            .map(|w| w.id.as_str());
        if dragged.is_some() && hovered.is_some() && (!self.shift || self.alt) {
            target.clear();
        }
        let windows = self
            .windows
            .iter()
            .filter_map(|w| {
                self.visible_window_rect(w).map(|r| ffi::UIWindowView {
                    id: w.id.clone(),
                    workspace: w.workspace.clone(),
                    rect: convert(r),
                    floating: w.floating,
                    target: dragged.is_some()
                        && (!self.shift || self.alt)
                        && hovered == Some(w.id.as_str()),
                })
            })
            .collect();
        let ghost = dragged
            .map(|d| {
                let r = self.visible_window_rect(&d.window).unwrap_or(d.rect);
                let c = self
                    .cells
                    .iter()
                    .find(|c| c.visible && c.key == d.window.workspace)
                    .map(|c| &c.rect)
                    .unwrap_or(&d.cell);
                ffi::UIGhost {
                    visible: true,
                    key: d.window.workspace.clone(),
                    rect: convert(Rect {
                        x: self.pointer.0 - (d.origin.0 - r.x),
                        y: self.pointer.1 - (d.origin.1 - r.y),
                        ..r
                    }),
                    uv: ffi::UIRect {
                        x: (r.x - c.x) / c.w,
                        y: (r.y - c.y) / c.h,
                        width: r.w / c.w,
                        height: r.h / c.h,
                    },
                }
            })
            .unwrap_or_default();
        (ghost, windows, target)
    }
}
impl Default for ffi::UIDrop {
    fn default() -> Self {
        Self {
            operation: String::new(),
            whole_group: false,
            window: String::new(),
            swap: String::new(),
            stack: String::new(),
            workspace: String::new(),
            monitor: 0,
            x: 0.0,
            y: 0.0,
            floating: false,
            activate: String::new(),
        }
    }
}
impl Default for ffi::UIRect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(id: u32) -> ffi::UIOutput {
        ffi::UIOutput {
            id,
            slot: id,
            parked: false,
            backing: id,
            name: id.to_string(),
            x: 1280.0 * id as f64,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
            workspace: id + 1,
            focused: id == 0,
        }
    }
    fn window(id: &str, workspace: &str, monitor: u32) -> ffi::UIWindow {
        ffi::UIWindow {
            group_size: 1,
            id: id.into(),
            workspace: workspace.into(),
            monitor,
            x: 100.0,
            y: 100.0,
            width: 300.0,
            height: 200.0,
            floating: true,
            title: id.into(),
        }
    }
    fn model() -> Box<Overview> {
        Core {
            theme: crate::theme::Theme {
                fit: false,
                ..Default::default()
            },
            ..Default::default()
        }
        .overview_new(
            &[output(0), output(1)],
            &[ffi::UISpecial {
                monitor: 1,
                name: "special:test".into(),
                shown: false,
            }],
            &[window("a", "1", 0), window("b", "2", 1)],
            -200.0,
        )
    }
    fn press(ui: &mut Overview) {
        let frame = ui.overview_frame(1000.0, 700.0, 101.0);
        let r = &frame.windows[0].rect;
        let (x, y) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        ui.overview_press(1000.0, 700.0, x, y);
    }
    #[test]
    fn swap_badges_keep_original_grid_identity_and_expire_after_reconcile() {
        let mut ui = model();
        ui.mapping.clock(1000.);
        assert_eq!(ui.mapping_command(0, 1), "@swap-grids 0 1");
        let mut replacement = model();
        replacement.group_slots.reverse();
        ui.overview_reconcile(replacement);
        let frame = ui.overview_frame(1000., 700., 1000.);
        let start = frame
            .labels
            .iter()
            .find(|l| l.text == "grid 0")
            .unwrap()
            .rect
            .x;
        ui.overview_hover_clock(1090.);
        let frame = ui.overview_frame(1000., 700., 1090.);
        assert!(
            frame
                .labels
                .iter()
                .find(|l| l.text == "grid 0")
                .unwrap()
                .rect
                .x
                > start
        );
        ui.overview_hover_clock(1180.);
        assert!(!ui
            .overview_frame(1000., 700., 1180.)
            .labels
            .iter()
            .any(|l| l.text.starts_with("grid ")));
    }
    #[test]
    fn region_keyboard_cross_panel_alt_swap_and_escape_cancel() {
        let mut ui = model();
        assert!(ui.overview_key(118, false, 101.).is_empty());
        assert_eq!(ui.region.mode, 2);
        ui.overview_key(108, false, 101.);
        ui.overview_key(106, false, 101.);
        ui.overview_key(109, false, 101.);
        assert_eq!(ui.region.mode, 3);
        ui.overview_key(0xff09, false, 101.);
        ui.overview_input_modifiers(false, true, false);
        let action = ui.overview_key(0xff0d, false, 101.);
        let request: crate::region::Request =
            serde_json::from_str(action.strip_prefix("@region:").unwrap()).unwrap();
        assert_eq!(
            (
                request.width,
                request.height,
                request.source_slot,
                request.target_slot
            ),
            (2, 2, 0, 1)
        );
        assert!(request.swap);
        assert_eq!(ui.region.mode, 0);
        ui.overview_input_modifiers(false, false, false);
        ui.overview_key(118, false, 101.);
        ui.overview_key(0xff1b, false, 101.);
        assert_eq!(ui.region.mode, 0);
        assert!(ui.closing.is_none());
    }
    #[test]
    fn stack_and_ghost_use_window_hit_geometry() {
        let mut ui = model();
        press(&mut ui);
        let f = ui.overview_frame(1000.0, 700.0, 101.0);
        let r = &f.windows[1].rect;
        let (x, y) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        ui.overview_pointer(1000.0, 700.0, x, y, false);
        let frame = ui.overview_frame(1000.0, 700.0, 101.0);
        assert!(frame.ghost.visible);
        assert_eq!(frame.drop_mode, "stack");
        assert!(frame.target.is_empty());
        assert_eq!(frame.ghost.key, "1");
        assert!(frame.windows.iter().any(|w| w.id == "b" && w.target));
        let drop = ui.overview_release(1000.0, 700.0, x, y);
        assert_eq!(
            (
                drop.window.as_str(),
                drop.stack.as_str(),
                drop.workspace.as_str()
            ),
            ("a", "b", "2")
        );
    }
    #[test]
    fn alt_drop_swaps_and_refusal_flash_expires() {
        let mut ui = model();
        press(&mut ui);
        ui.overview_input_modifiers(false, true, false);
        let f = ui.overview_frame(1000., 700., 101.);
        let r = &f.windows[1].rect;
        let x = r.x + r.width / 2.;
        let y = r.y + r.height / 2.;
        ui.overview_pointer(1000., 700., x, y, false);
        assert_eq!(ui.overview_frame(1000., 700., 101.).drop_mode, "swap");
        let drop = ui.overview_release(1000., 700., x, y);
        assert_eq!(drop.swap, "b");
        assert!(drop.stack.is_empty());
        ui.overview_refuse("b", 101.);
        assert_eq!(
            ui.overview_frame(1000., 700., 350.).refused,
            "b",
            "feedback must survive a delayed first frame"
        );
        ui.overview_presented(350.);
        assert_eq!(ui.overview_frame(1000., 700., 529.).refused, "b");
        assert!(ui.overview_frame(1000., 700., 529.).animating);
        assert!(ui.overview_frame(1000., 700., 530.).refused.is_empty());
    }
    #[test]
    fn shift_changes_three_member_ghost_live_and_moves_whole_group() {
        let mut ui = model();
        ui.windows[0].group_size = 3;
        press(&mut ui);
        let f = ui.overview_frame(1000., 700., 101.);
        let r = &f.windows[1].rect;
        let (x, y) = (r.x + r.width / 2., r.y + r.height / 2.);
        ui.overview_pointer(1000., 700., x, y, false);
        assert_eq!(ui.overview_frame(1000., 700., 101.).ghost_count, 1);
        ui.overview_input_modifiers(true, false, false);
        let f = ui.overview_frame(1000., 700., 101.);
        assert_eq!(f.ghost_count, 3);
        assert!(!f.target.is_empty());
        assert!(!f.windows.iter().any(|w| w.target));
        ui.overview_input_modifiers(false, false, false);
        assert_eq!(ui.overview_frame(1000., 700., 101.).ghost_count, 1);
        ui.overview_input_modifiers(true, false, false);
        let d = ui.overview_release(1000., 700., x, y);
        assert!(d.whole_group);
        assert!(d.stack.is_empty() && d.swap.is_empty());
    }
    #[test]
    fn floating_drop_retains_grab_offset_on_other_monitor_and_special() {
        let mut ui = model();
        press(&mut ui);
        let f = ui.overview_frame(1000.0, 700.0, 101.0);
        let c = f.cells.iter().find(|c| c.key == "82").unwrap();
        let (x, y) = (
            c.rect.x + c.rect.width * 0.7,
            c.rect.y + c.rect.height * 0.5,
        );
        let drop = ui.overview_release(1000.0, 700.0, x, y);
        assert_eq!(drop.workspace, "82");
        assert_eq!(drop.monitor, 1);
        assert!(drop.floating);
        assert!((drop.x - 746.0).abs() < 0.001 && (drop.y - 260.0).abs() < 0.001);
        press(&mut ui);
        let f = ui.overview_frame(1000.0, 700.0, 101.0);
        let r = &f.cells.last().unwrap().rect;
        let (x, y) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert_eq!(
            ui.overview_release(1000.0, 700.0, x, y).workspace,
            "special:test"
        );
    }
    #[test]
    fn escape_cancels_drag_then_closes_and_source_disappearance_is_safe() {
        let mut ui = model();
        press(&mut ui);
        ui.overview_pointer(1000.0, 700.0, 500.0, 400.0, false);
        assert!(ui.overview_frame(1000.0, 700.0, 101.0).ghost.visible);
        ui.overview_key(0xff1b, false, 101.0);
        assert!(!ui.overview_frame(1000.0, 700.0, 102.0).ghost.visible);
        assert!(ui.overview_frame(1000.0, 700.0, 102.0).alive);
        assert!(ui
            .overview_release(1000.0, 700.0, 500.0, 400.0)
            .activate
            .is_empty());
        ui.overview_key(0xff1b, false, 103.0);
        assert!(!ui.overview_frame(1000.0, 700.0, 284.0).alive);
        let mut ui = model();
        press(&mut ui);
        ui.overview_windows(&[window("b", "2", 1)]);
        let d = ui.overview_release(1000.0, 700.0, 500.0, 400.0);
        assert!(d.window.is_empty() && d.activate.is_empty());
    }
    #[test]
    fn caption_is_a_cell_drop_target_above_a_full_window() {
        let mut ui = model();
        let mut full = window("full", "special:test", 1);
        full.x = 0.0;
        full.y = 0.0;
        full.width = 1280.0;
        full.height = 720.0;
        full.floating = false;
        ui.overview_windows(&[window("a", "1", 0), full]);
        press(&mut ui);
        let f = ui.overview_frame(1000.0, 700.0, 101.0);
        let r = &f.cells.last().unwrap().rect;
        let d = ui.overview_release(1000.0, 700.0, r.x + r.width / 2.0, r.y + 1.0);
        assert_eq!(d.workspace, "special:test");
        assert!(d.swap.is_empty());
    }
}
