mod animation;
mod assignment;
mod doctor;
mod drop_script;
mod extension;
pub mod grid;
mod labels;
mod overview;
mod region;
mod slots;
mod theme;
mod transport;
mod ui;
use overview::probe_report;
use ui::Overview;
mod pinch;
pub mod swipe;
mod visual;
use grid::{Cell, Direction, Grid};
use serde_json::json;
use std::collections::HashMap;

#[cxx::bridge(namespace = "hypr2d")]
mod ffi {
    struct MonitorIdentity {
        id: u32,
        name: String,
        make: String,
        model: String,
        serial: String,
        x: f64,
        y: f64,
    }
    struct Monitor {
        id: u32,
        slot: u32,
        name: String,
        workspace: u32,
        focused: bool,
        offset_x: f64,
        offset_y: f64,
        width: f64,
        height: f64,
    }
    struct UnplugPlan {
        focus: u32,
        workspaces: Vec<Move>,
        notification: String,
    }
    struct SpecialInfo {
        name: String,
        app: String,
    }
    struct Window {
        workspace: u32,
        monitor: u32,
        address: String,
        title: String,
        app_id: String,
        urgent: bool,
    }
    struct DoctorWorkspace {
        key: String,
        reported: u32,
        usable: bool,
        targets: Vec<String>,
    }
    struct DoctorWindow {
        id: String,
        workspace: String,
        target_workspace: String,
        ghost_workspace: String,
        target: String,
        target_space: bool,
        ghost_space: bool,
        locked: bool,
        group: String,
        members: Vec<String>,
        current: String,
    }
    struct DoctorPlan {
        rebuild: Vec<String>,
        dissolve: Vec<String>,
        error: String,
    }
    struct Move {
        monitor: u32,
        workspace: u32,
        x: u32,
        y: u32,
        vertical: bool,
        positive: bool,
        carry: bool,
        all: bool,
    }
    struct Reply {
        moves: Vec<Move>,
        output: String,
        error: String,
    }
    enum SwipePhase {
        Begin,
        Update,
        End,
        Tick,
        Cancel,
    }
    struct SwipeInput {
        phase: SwipePhase,
        fingers: u32,
        dx: f64,
        dy: f64,
        now: f64,
        cancelled: bool,
    }
    struct SwipeView {
        workspace: u32,
        x: f64,
        y: f64,
    }
    struct SwipeFrame {
        views: Vec<SwipeView>,
        claimed: bool,
        active: bool,
        settling: bool,
        finish_after_ms: f64,
        done: bool,
        monitor: u32,
        current: u32,
        neighbor: u32,
        current_x: f64,
        current_y: f64,
        neighbor_x: f64,
        neighbor_y: f64,
        commit: u32,
        cell_x: u32,
        cell_y: u32,
    }
    struct ProbeSpecial {
        monitor: u32,
        name: String,
    }
    struct ProbeCell {
        monitor: u32,
        workspace: u32,
        special: String,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        capture_width: u32,
        capture_height: u32,
    }
    struct UIRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }
    struct UIOutput {
        id: u32,
        slot: u32,
        parked: bool,
        backing: u32,
        name: String,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        workspace: u32,
        focused: bool,
    }
    struct UISpecial {
        monitor: u32,
        name: String,
        shown: bool,
    }
    struct UIWindow {
        id: String,
        workspace: String,
        monitor: u32,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        floating: bool,
        title: String,
        group_size: u32,
    }
    struct UIStyle {
        accent: u32,
        background_darken: f64,
        memory_budget_mib: u32,
        background: u32,
        cell: u32,
        tile: u32,
        tile_alpha: f64,
        border: u32,
        hover: u32,
        current: u32,
        target: u32,
        label: u32,
        font: String,
        font_size: u32,
        rounding: u32,
        gap: u32,
    }
    struct ExtensionAction {
        token: u64,
        window: String,
        workspace: String,
        monitor: u32,
    }
    #[derive(Default)]
    struct ExtensionBatch {
        changed: bool,
        actions: Vec<ExtensionAction>,
    }
    struct UICell {
        secondary: String,
        key: String,
        monitor: u32,
        rect: UIRect,
        label: String,
        colour: u32,
        selected: bool,
        hovered: bool,
        occupied: bool,
        dimmed: bool,
        shown: bool,
    }
    struct UILabel {
        rect: UIRect,
        text: String,
        colour: u32,
        band: bool,
    }
    struct UIRegionRect {
        key: String,
        rect: UIRect,
        kind: u8,
    }
    struct UIDrop {
        operation: String,
        whole_group: bool,
        window: String,
        swap: String,
        stack: String,
        workspace: String,
        monitor: u32,
        x: f64,
        y: f64,
        floating: bool,
        activate: String,
    }
    #[derive(Default)]
    struct UIGhost {
        visible: bool,
        key: String,
        rect: UIRect,
        uv: UIRect,
    }
    struct UIWindowView {
        id: String,
        workspace: String,
        rect: UIRect,
        floating: bool,
        target: bool,
    }
    struct UIBackdropPanel {
        monitor: u32,
        rect: UIRect,
        columns: u32,
        rows: u32,
    }
    struct UIGuide {
        from_monitor: u32,
        to_monitor: u32,
        index: u32,
        vertical: bool,
        start: UIRect,
        end: UIRect,
    }
    struct RegionBatch {
        script: String,
        undo_script: String,
        workspaces: Vec<String>,
        targets: Vec<Move>,
        error: String,
    }
    struct UIFrame {
        guides: Vec<UIGuide>,
        region_rects: Vec<UIRegionRect>,
        region_ghosts: Vec<UIGhost>,
        region_drag: bool,
        ghost_count: u32,
        backdrops: Vec<UIBackdropPanel>,
        ghost: UIGhost,
        windows: Vec<UIWindowView>,
        target: String,
        drop_mode: String,
        refused: String,
        alive: bool,
        animating: bool,
        alpha: f64,
        cells: Vec<UICell>,
        labels: Vec<UILabel>,
    }
    extern "Rust" {
        type Overview;
        fn extension_start(self: &mut Core, path: &str) -> i32;
        fn extension_stop(self: &mut Core);
        fn extension_drain(self: &mut Core, monitors: &[Monitor]) -> ExtensionBatch;
        fn extension_result(self: &mut Core, token: u64, error: &str);
        fn extension_event(self: &mut Core, event: &str, workspace: &str, window: &str);
        fn ui_header_width(self: &mut Core, text: &str, width: f64);
        fn ui_font_height(self: &mut Core, height: f64);
        fn region_record(self: &mut Core, batch: &RegionBatch);
        fn region_inverse(self: &Core) -> RegionBatch;
        fn region_forget(self: &mut Core, all: bool);
        fn region_prepare(
            self: &Core,
            input: &str,
            windows: &[UIWindow],
            monitors: &[Monitor],
        ) -> RegionBatch;
        fn grid_animation(self: &Core, monitor: &Monitor, target: u32) -> String;
        fn grid_checkpoint(self: &Core) -> String;
        fn grid_rollback(self: &mut Core, snapshot: &str) -> String;
        fn grid_change(self: &mut Core, monitors: &[Monitor], input: &str) -> Reply;
        fn slots_init(self: &mut Core) -> String;
        fn rename(self: &mut Core, input: &str) -> String;
        fn ui_output_label(self: &Core, output: &UIOutput) -> String;
        fn specials_observe(self: &mut Core, windows: &[SpecialInfo]);
        fn slots_update(self: &mut Core, monitors: &[MonitorIdentity]) -> String;
        fn monitor_slot(self: &Core, name: &str) -> u32;
        fn monitor_observe(self: &mut Core, monitors: &[Monitor]);
        fn unplug(self: &mut Core, id: u32, monitors: &[Monitor], windows: &[Window])
            -> UnplugPlan;
        fn restore_workspace(self: &Core, monitor: &Monitor) -> u32;
        fn owns_workspace(self: &Core, monitor: &Monitor, workspace: u32) -> bool;
        fn parked_panels(
            self: &mut Core,
            outputs: &[UIOutput],
            windows: &[UIWindow],
        ) -> Vec<UIOutput>;
        fn ui_style(self: &Core) -> UIStyle;
        fn backdrop_size(self: &Core, width: f64, height: f64, total_area: f64) -> UIRect;
        fn overview_hover_clock(self: &mut Overview, now: f64);
        fn overview_modifiers(self: &mut Overview, shift: bool);
        fn overview_input_modifiers(self: &mut Overview, shift: bool, alt: bool, control: bool);
        fn overview_interactive(self: &Overview, now: f64) -> bool;
        fn overview_presented(self: &mut Overview, now: f64);
        fn overview_refuse(self: &mut Overview, window: &str, now: f64);
        fn overview_press(self: &mut Overview, width: f64, height: f64, x: f64, y: f64);
        fn overview_release(self: &mut Overview, width: f64, height: f64, x: f64, y: f64)
            -> UIDrop;
        fn overview_reconcile(self: &mut Overview, replacement: Box<Overview>) -> Vec<String>;
        fn overview_windows(self: &mut Overview, windows: &[UIWindow]) -> Vec<String>;
        fn overview_frame_scaled(
            self: &Overview,
            width: f64,
            height: f64,
            scale: f64,
            now: f64,
        ) -> UIFrame;
        fn overview_state_scaled(
            self: &Overview,
            width: f64,
            height: f64,
            scale: f64,
            now: f64,
        ) -> String;
        fn overview_state(self: &Overview, width: f64, height: f64, now: f64) -> String;
        fn overview_new(
            self: &Core,
            outputs: &[UIOutput],
            specials: &[UISpecial],
            windows: &[UIWindow],
            now: f64,
        ) -> Box<Overview>;
        fn overview_captures(self: &Overview, viewports: &[UIRect]) -> Vec<ProbeCell>;
        fn overview_frame(self: &Overview, width: f64, height: f64, now: f64) -> UIFrame;
        fn overview_pointer(
            self: &mut Overview,
            width: f64,
            height: f64,
            x: f64,
            y: f64,
            click: bool,
        ) -> String;
        fn overview_key(self: &mut Overview, key: u32, shift: bool, now: f64) -> String;
        fn overview_parked(self: &mut Overview);
        fn overview_close(self: &mut Overview, now: f64);

        fn probe_plan(
            self: &Core,
            monitors: &[Monitor],
            specials: &[ProbeSpecial],
            full: bool,
        ) -> Vec<ProbeCell>;
        fn probe_report(
            cells: &[ProbeCell],
            times: &[f64],
            active: bool,
            deleted: bool,
            restored: u32,
            renderer: &str,
        ) -> String;
        type Core;
        fn new_core() -> Box<Core>;
        fn drop_script(drop: &UIDrop, source: &str, x: f64, y: f64) -> String;
        fn doctor(workspaces: &[DoctorWorkspace], windows: &[DoctorWindow]) -> String;
        fn doctor_ok(workspaces: &[DoctorWorkspace], windows: &[DoctorWindow]) -> bool;
        fn doctor_plan(workspaces: &[DoctorWorkspace], windows: &[DoctorWindow]) -> DoctorPlan;
        fn doctor_repair_script(windows: &[String]) -> String;
        fn doctor_result(
            state: &str,
            error: &str,
            rebuilt: &[String],
            dissolved: &[String],
        ) -> String;
        fn pinch_event(self: &mut Core, phase: u8, fingers: u32, scale: f64, open: bool) -> u8;
        fn pinch_active(self: &Core) -> bool;
        fn swipe(self: &mut Core, monitors: &[Monitor], input: SwipeInput) -> SwipeFrame;
        fn texture_bytes(width: u32, height: u32, mipmaps: bool) -> u64;
        fn grid_args(operation: &str, a: &str, b: &str) -> String;
        fn spike_round_trip(value: u32) -> u32;
        fn command(self: &mut Core, monitors: &[Monitor], windows: &[Window], input: &str)
            -> Reply;
    }
}
pub struct Core {
    region_history: Vec<region::Undo>,
    ui_font_height: Option<f64>,
    ui_header_widths: HashMap<String, f64>,
    labels: labels::Labels,
    positions: HashMap<u32, (usize, usize)>,
    special_apps: std::collections::BTreeMap<String, Vec<String>>,
    slots: slots::Slots,
    assignments: assignment::Assignment,
    physical_slots: HashMap<String, u32>,
    connected_slots: HashMap<String, u32>,
    remembered_cells: HashMap<u32, u32>,
    parked_counts: HashMap<u32, usize>,
    extensions: extension::Extensions,
    extension_grids: HashMap<u32, Grid>,
    extension_slots: HashMap<u32, u32>,
    theme: theme::Theme,
    grids: HashMap<String, Grid>,
    stride: u32,
    pinch: pinch::Pinch,
    pinch_cancel_scale: f64,
    swipe_enabled: bool,
    swipe_config: swipe::Config,
    gesture: Option<swipe::Swipe>,
}
impl Default for Core {
    fn default() -> Self {
        Self {
            extensions: extension::Extensions::default(),
            extension_grids: HashMap::new(),
            extension_slots: HashMap::new(),
            region_history: Vec::new(),
            ui_font_height: None,
            ui_header_widths: HashMap::new(),
            labels: labels::Labels::default(),
            positions: HashMap::new(),
            special_apps: Default::default(),
            slots: slots::Slots::default(),
            assignments: assignment::Assignment::default(),
            physical_slots: HashMap::new(),
            connected_slots: HashMap::new(),
            remembered_cells: HashMap::new(),
            parked_counts: HashMap::new(),
            theme: theme::Theme::default(),
            grids: HashMap::new(),
            stride: 10,
            pinch: pinch::Pinch::default(),
            pinch_cancel_scale: 0.08,
            swipe_enabled: true,
            swipe_config: swipe::Config::default(),
            gesture: None,
        }
    }
}
fn texture_bytes(mut width: u32, mut height: u32, mipmaps: bool) -> u64 {
    guarded(|| {
        let mut bytes = 0;
        loop {
            bytes += u64::from(width) * u64::from(height) * 4;
            if !mipmaps || width <= 1 && height <= 1 {
                return bytes;
            }
            width = (width / 2).max(1);
            height = (height / 2).max(1);
        }
    })
}
fn guarded<T: Default>(operation: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(value) => value,
        Err(payload) => {
            std::mem::forget(payload);
            let _ = std::io::Write::write_all(
                &mut std::io::stderr(),
                b"hypr2d: contained Rust panic; operation ignored\n",
            );
            T::default()
        }
    }
}
fn drop_script(drop: &ffi::UIDrop, source: &str, x: f64, y: f64) -> String {
    drop_script::script(drop, source, x, y)
}
fn doctor(workspaces: &[ffi::DoctorWorkspace], windows: &[ffi::DoctorWindow]) -> String {
    doctor::report(workspaces, windows)
}
fn doctor_ok(workspaces: &[ffi::DoctorWorkspace], windows: &[ffi::DoctorWindow]) -> bool {
    guarded(|| doctor::analyze(workspaces, windows).errors.is_empty())
}
fn doctor_plan(
    workspaces: &[ffi::DoctorWorkspace],
    windows: &[ffi::DoctorWindow],
) -> ffi::DoctorPlan {
    guarded(|| doctor::plan(workspaces, windows))
}
fn doctor_repair_script(windows: &[String]) -> String {
    guarded(|| doctor::repair_script(windows))
}
fn doctor_result(state: &str, error: &str, rebuilt: &[String], dissolved: &[String]) -> String {
    guarded(|| {
        json!({"state":state,"error":error,"rebuilt":rebuilt,"dissolved":dissolved}).to_string()
    })
}
fn new_core() -> Box<Core> {
    guarded(|| Box::new(Core::default()))
}
fn spike_round_trip(value: u32) -> u32 {
    guarded(|| value.checked_add(1).unwrap_or(0))
}
impl Default for ffi::Reply {
    fn default() -> Self {
        Self {
            moves: Vec::new(),
            output: String::new(),
            error: "Rust operation failed".into(),
        }
    }
}
impl Core {
    fn grid(&self, m: &ffi::Monitor) -> Grid {
        self.slot_grid(m.slot, &m.name)
    }
    fn slot_grid(&self, slot: u32, name: &str) -> Grid {
        self.grids
            .get(&slot.to_string())
            .or_else(|| {
                self.slots
                    .aliases(slot)
                    .find_map(|alias| self.grids.get(alias))
            })
            .or_else(|| {
                self.connected_slots
                    .is_empty()
                    .then(|| self.grids.get(name))
                    .flatten()
            })
            .copied()
            .unwrap_or(Grid {
                stride: self.stride,
                ..Grid::default()
            })
    }

    fn pinch_event(&mut self, phase: u8, fingers: u32, scale: f64, open: bool) -> u8 {
        guarded(|| {
            let (claimed, close) =
                self.pinch
                    .event(phase, fingers, scale, open, self.pinch_cancel_scale);
            u8::from(claimed) | (u8::from(close) << 1)
        })
    }
    fn pinch_active(&self) -> bool {
        guarded(|| self.pinch.active())
    }
    fn command(
        &mut self,
        monitors: &[ffi::Monitor],
        windows: &[ffi::Window],
        input: &str,
    ) -> ffi::Reply {
        guarded(|| match self.plan(monitors, windows, input) {
            Ok(r) => r,
            Err(error) => ffi::Reply {
                error,
                ..Default::default()
            },
        })
    }
    fn plan(
        &mut self,
        monitors: &[ffi::Monitor],
        windows: &[ffi::Window],
        input: &str,
    ) -> Result<ffi::Reply, String> {
        if let Some(value) = input.strip_prefix("pinch-config ") {
            let scale: f64 = value.parse().map_err(|_| "invalid pinch cancelScale")?;
            if !scale.is_finite() || scale <= 0.0 || scale >= 1.0 {
                return Err("pinch cancelScale must be finite and between 0 and 1".into());
            }
            self.pinch_cancel_scale = scale;
            return Ok(ffi::Reply {
                output: "ok".into(),
                error: String::new(),
                moves: vec![],
            });
        }
        if let Some(json) = input.strip_prefix("overview-config ") {
            self.theme = theme::Theme::parse(json)?;
            return Ok(ffi::Reply {
                moves: vec![],
                output: "ok".into(),
                error: String::new(),
            });
        }
        let args: Vec<_> = input.split_whitespace().collect();
        if let ["grid-restore", mode] = args.as_slice() {
            self.assignments.restore_keep = match *mode {
                "auto" => false,
                "keep" => true,
                _ => return Err("restore must be auto or keep".into()),
            };
            return Ok(ffi::Reply {
                error: String::new(),
                output: "ok".into(),
                moves: vec![],
            });
        }
        let mut reply = ffi::Reply {
            moves: vec![],
            output: "ok".into(),
            error: String::new(),
        };
        if matches!(args.as_slice(), ["state"] | ["state", "-j"]) {
            reply.output = json!({"pinch_config":{"cancel_scale":self.pinch_cancel_scale},"version":1,"special_names":self.labels.names(),"specials":self.special_labels(),"parked":self.parked_snapshot(windows),"extensions":self.extensions.snapshot(),"swipe_enabled":self.swipe_enabled,"overview_config":self.theme,"swipe":self.gesture.as_ref().map(|g|g.status()),"swipe_config":self.swipe_config,"monitors":monitors.iter().map(|m| {
                let g = self.grid(m);
                json!({"id":m.id,"slot":m.slot,"name":m.name,"label":self.monitor_label(&m.name),"workspace":m.workspace,"focused":m.focused,"grid":g,"cell":g.cell(m.workspace,m.slot),"offset":[m.offset_x,m.offset_y],
                    "cells":(0..g.rows).flat_map(|y|(0..g.columns).map(move |x|Cell{x,y})).filter_map(|c|g.id(c,m.slot).ok().map(|id|json!({"x":c.x,"y":c.y,"workspace":id,"windows":windows.iter().filter(|w|w.workspace==id).map(|w|json!({"address":w.address,"title":w.title,"app_id":w.app_id,"urgent":w.urgent})).collect::<Vec<_>>()}))).collect::<Vec<_>>()})
            }).collect::<Vec<_>>()}).to_string();
            return Ok(reply);
        }
        if let ["configure", name, columns, rows, wrap, stride] = args.as_slice() {
            let matched = monitors
                .iter()
                .find(|m| m.name == *name || m.slot.to_string() == *name);
            let id = matched.map_or(0, |m| m.id);
            let configured_slot = self
                .physical_slots
                .get(*name)
                .copied()
                .or_else(|| name.parse::<u32>().ok())
                .or_else(|| matched.map(|m| m.slot));
            let g = Grid {
                columns: number(columns)?,
                rows: number(rows)?,
                wrap: wrap.parse().map_err(|_| "wrap must be true/false")?,
                stride: number(stride)?,
            }
            .validate(
                configured_slot
                    .unwrap_or(monitors.iter().find(|m| m.id == id).map_or(0, |m| m.slot)),
            )?;
            if monitors.iter().any(|other| other.slot >= g.stride) {
                return Err("stride must exceed every monitor slot".into());
            }
            // Stride is shared: distinct monitor residues must never collide.
            for config in self.grids.values() {
                Grid {
                    stride: g.stride,
                    ..*config
                }
                .validate(0)?;
            }
            for config in self.grids.values_mut() {
                config.stride = g.stride;
            }
            self.stride = g.stride;
            self.grids.insert(
                configured_slot
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| (*name).to_string()),
                g,
            );
            if let Some(m) = matched {
                self.extension_grids.insert(m.id, g);
                self.extension_slots.insert(m.id, m.slot);
            }
            return Ok(reply);
        }
        if let ["swipe-enabled", enabled] = args.as_slice() {
            self.swipe_enabled = enabled.parse().map_err(|_| "enabled must be true/false")?;
            self.gesture = None;
            return Ok(reply);
        }
        if args.first() == Some(&"swipe-config") {
            if args.len() != 6 && args.len() != 11 && args.len() != 12 {
                return Err("swipe-config requires FINGERS DISTANCE VELOCITY NATURAL LOCK [DIAGONAL_WIDTH HYSTERESIS HYSTERESIS_DISTANCE PROJECTION_DECEL SPRING_STIFFNESS [SENSITIVITY]]".into());
            }
            let mut config = swipe::Config {
                fingers: number(args[1])?,
                distance: args[2].parse().map_err(|_| "invalid distance")?,
                velocity: args[3].parse().map_err(|_| "invalid velocity")?,
                natural: args[4].parse().map_err(|_| "natural must be true/false")?,
                axis_lock: args[5].parse().map_err(|_| "invalid axis lock")?,
                ..Default::default()
            };
            if args.len() >= 11 {
                config.diagonal_width = args[6].parse().map_err(|_| "invalid diagonal width")?;
                config.hysteresis = args[7].parse().map_err(|_| "invalid hysteresis")?;
                config.hysteresis_distance =
                    args[8].parse().map_err(|_| "invalid hysteresis distance")?;
                config.projection_decel =
                    args[9].parse().map_err(|_| "invalid projection decel")?;
                config.spring_stiffness =
                    args[10].parse().map_err(|_| "invalid spring stiffness")?;
            }
            if args.len() == 12 {
                config.sensitivity = args[11].parse().map_err(|_| "invalid sensitivity")?;
            }
            self.swipe_config = config.validate()?;
            return Ok(reply);
        }
        let focused = monitors
            .iter()
            .find(|m| m.focused)
            .ok_or("no focused monitor")?;
        let (selected, target, dir, carry, all) = match args.as_slice() {
            [op @ ("move" | "carry"), direction] => (vec![focused],None,Some(Direction::parse(direction)?),*op=="carry",false),
            ["move", direction, "monitor", name] => (vec![select_monitor(monitors,name)?],None,Some(Direction::parse(direction)?),false,false),
            ["move", direction, "all"] => (monitors.iter().collect(),None,Some(Direction::parse(direction)?),false,true),
            ["goto", x, y] => (vec![focused],Some(Cell{x:number(x)?,y:number(y)?}),None,false,false),
            ["goto", x, y, monitor] => (vec![select_monitor(monitors,monitor)?],Some(Cell{x:number(x)?,y:number(y)?}),None,false,false),
            _ => return Err("expected move DIR [all], carry DIR, goto X Y [MONITOR], state, or configure MONITOR COLS ROWS WRAP STRIDE".into())
        };
        for m in selected {
            let g = self.grid(m).validate(m.slot)?;
            let current = g.cell(m.workspace, m.slot);
            let c = if let Some(c) = target {
                c
            } else {
                let c =
                    current.ok_or("active workspace is outside this monitor's grid; use goto")?;
                let direction = dir.ok_or("missing direction")?;
                let mut next = g.step(c, direction).unwrap_or(c);
                for _ in 0..g.rows * g.columns {
                    if !self.extensions.skip(m.id, next.y) {
                        break;
                    }
                    let Some(step) = g.step(next, direction) else {
                        next = c;
                        break;
                    };
                    next = step;
                }
                if self.extensions.skip(m.id, next.y) {
                    c
                } else {
                    next
                }
            };
            let d = dir.unwrap_or_else(|| {
                let old = current.unwrap_or(c);
                if c.y != old.y {
                    if c.y > old.y {
                        Direction::Down
                    } else {
                        Direction::Up
                    }
                } else if c.x > old.x {
                    Direction::Right
                } else {
                    Direction::Left
                }
            });
            reply.moves.push(ffi::Move {
                monitor: m.id,
                workspace: g.id(c, m.slot)?,
                x: c.x,
                y: c.y,
                vertical: d.vertical(),
                positive: d.positive(),
                carry,
                all,
            });
        }
        Ok(reply)
    }
}
fn number(s: &str) -> Result<u32, String> {
    s.parse()
        .map_err(|_| "expected a non-negative integer".into())
}
fn select_monitor<'a>(ms: &'a [ffi::Monitor], s: &str) -> Result<&'a ffi::Monitor, String> {
    ms.iter()
        .find(|m| m.name == s || m.id.to_string() == s)
        .ok_or_else(|| "unknown monitor".into())
}
impl Default for ffi::SwipeFrame {
    fn default() -> Self {
        Self {
            views: vec![],
            claimed: false,
            active: false,
            settling: false,
            finish_after_ms: 0.,
            done: true,
            monitor: 0,
            current: 0,
            neighbor: 0,
            current_x: 0.,
            current_y: 0.,
            neighbor_x: 0.,
            neighbor_y: 0.,
            commit: 0,
            cell_x: 0,
            cell_y: 0,
        }
    }
}
impl Core {
    fn swipe(&mut self, monitors: &[ffi::Monitor], input: ffi::SwipeInput) -> ffi::SwipeFrame {
        let ffi::SwipeInput {
            phase,
            fingers,
            dx,
            dy,
            now,
            cancelled,
        } = input;
        guarded(|| {
            if phase == ffi::SwipePhase::Cancel {
                self.gesture = None;
                return ffi::SwipeFrame::default();
            }
            if phase == ffi::SwipePhase::Begin {
                if !self.swipe_enabled {
                    return ffi::SwipeFrame::default();
                }
                if fingers != self.swipe_config.fingers {
                    return ffi::SwipeFrame {
                        done: false,
                        ..Default::default()
                    };
                }
                let Some(m) = monitors.iter().find(|m| m.focused) else {
                    return ffi::SwipeFrame::default();
                };
                self.gesture = swipe::Swipe::new(
                    m.slot,
                    m.workspace,
                    self.grid(m),
                    [m.width, m.height],
                    self.swipe_config,
                    now,
                );
                if let Some(g) = &mut self.gesture {
                    g.monitor = m.id;
                }
            }
            let Some(mut gesture) = self.gesture.take() else {
                return ffi::SwipeFrame::default();
            };
            if !monitors.iter().any(|m| m.id == gesture.monitor) {
                return ffi::SwipeFrame::default();
            }
            let mut commit = None;
            match phase {
                ffi::SwipePhase::Update => gesture.update([dx, dy], now),
                ffi::SwipePhase::End => commit = gesture.end(now, cancelled),
                ffi::SwipePhase::Tick => gesture.tick(now),
                _ => {}
            }
            let (current, neighbor) = gesture.offsets();
            let result = ffi::SwipeFrame {
                views: gesture
                    .views()
                    .into_iter()
                    .map(|(workspace, p)| ffi::SwipeView {
                        workspace,
                        x: p[0],
                        y: p[1],
                    })
                    .collect(),
                claimed: true,
                active: !gesture.finished,
                settling: gesture.settling(),
                finish_after_ms: if phase == ffi::SwipePhase::End && !gesture.finished {
                    gesture.deadline() + 1.
                } else {
                    0.
                },
                done: gesture.finished,
                monitor: gesture.monitor,
                current: gesture.origin,
                neighbor: gesture.neighbor_id(),
                current_x: current[0],
                current_y: current[1],
                neighbor_x: neighbor[0],
                neighbor_y: neighbor[1],
                commit: commit.map_or(0, |c| c.0),
                cell_x: commit.map_or(0, |c| c.1.x),
                cell_y: commit.map_or(0, |c| c.1.y),
            };
            if !gesture.finished {
                self.gesture = Some(gesture);
            }
            result
        })
    }
}

impl Core {
    fn ui_header_width(&mut self, text: &str, width: f64) {
        guarded(|| {
            if width.is_finite() && width > 0. {
                self.ui_header_widths.insert(text.into(), width);
            }
        })
    }
    fn ui_font_height(&mut self, height: f64) {
        guarded(|| {
            if height.is_finite() && (1.0..=128.0).contains(&height) {
                self.ui_font_height = Some(height);
                self.ui_header_widths.clear();
            }
        })
    }
    fn backdrop_size(&self, width: f64, height: f64, total_area: f64) -> ffi::UIRect {
        guarded(|| {
            let factor = visual::budget_scale(
                total_area,
                self.theme.memory_budget_mib as f64 * 1024. * 1024. * 0.25,
            );
            ffi::UIRect {
                x: 0.,
                y: 0.,
                width: (width * factor).floor().max(1.),
                height: (height * factor).floor().max(1.),
            }
        })
    }
    fn ui_style(&self) -> ffi::UIStyle {
        guarded(|| {
            let t = &self.theme;
            let c = |s: &str| u32::from_str_radix(&s[1..], 16).unwrap_or(0);
            ffi::UIStyle {
                accent: c(&t.accent),
                background_darken: t.background_darken,
                memory_budget_mib: t.memory_budget_mib,
                background: c(&t.background),
                cell: c(&t.cell),
                tile: c(&t.tile[..7]),
                tile_alpha: u8::from_str_radix(&t.tile[7..], 16).unwrap_or(0) as f64 / 255.,
                border: c(&t.border),
                hover: c(&t.hover),
                current: c(&t.current),
                target: c(&t.target),
                label: c(&t.label),
                font: t.font.clone(),
                font_size: t.font_size,
                rounding: t.rounding,
                gap: t.gap,
            }
        })
    }
}
impl Default for ffi::UIStyle {
    fn default() -> Self {
        Self {
            accent: 0x9CBEFF,
            background_darken: 0.35,
            memory_budget_mib: 128,
            background: 0,
            cell: 0x303030,
            tile: 0xFFFFFF,
            tile_alpha: 26. / 255.,
            border: 0x353535,
            hover: 0x4A4A4A,
            current: 0x82AAFF,
            target: 0xFFCB6B,
            label: 0xEEFFFF,
            font: "sans-serif".into(),
            font_size: 16,
            rounding: 3,
            gap: 0,
        }
    }
}

pub fn grid_args(operation: &str, a: &str, b: &str) -> String {
    guarded(|| {
        format!(
            "{operation} {}",
            if b.is_empty() {
                json!([a])
            } else {
                json!([a, b])
            }
        )
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn pinch_config_is_validated_atomically() {
        let mut core = Core::default();
        assert!(core.command(&[], &[], "pinch-config 0.12").error.is_empty());
        for bad in ["0", "-0.1", "1", "NaN", "inf", "text"] {
            assert!(!core
                .command(&[], &[], &format!("pinch-config {bad}"))
                .error
                .is_empty());
            assert_eq!(core.pinch_cancel_scale, 0.12);
        }
    }
    #[test]
    fn swipe_gain_config_preserves_legacy_forms_and_rejects_invalid_atomically() {
        let mut core = Core::default();
        let extended = "swipe-config 3 120 0.8 true 5 15 8 12 0.0006 576";
        assert!(core
            .command(&[], &[], &format!("{extended} 1.3"))
            .error
            .is_empty());
        assert_eq!(core.swipe_config.sensitivity, 1.3);
        assert!(!core
            .command(&[], &[], &format!("{extended} 0"))
            .error
            .is_empty());
        assert_eq!(core.swipe_config.sensitivity, 1.3);
        assert!(core.command(&[], &[], extended).error.is_empty());
        assert_eq!(core.swipe_config.sensitivity, 1.);
        assert!(core
            .command(&[], &[], "swipe-config 3 120 0.8 true 5")
            .error
            .is_empty());
        assert_eq!(core.swipe_config.sensitivity, 1.);
    }
    use super::*;
    fn monitor(id: u32, ws: u32) -> ffi::Monitor {
        ffi::Monitor {
            id,
            slot: id,
            name: format!("m{id}"),
            workspace: ws,
            focused: id == 0,
            offset_x: 0.,
            offset_y: 0.,
            width: 1280.,
            height: 720.,
        }
    }
    #[test]
    fn rename_preserves_grid_settings_and_parking_remembers_own_cell() {
        let mut core = Core::default();
        let slot = core
            .slots
            .resolve("Dock-A", "Make", "Model", "serial", &[])
            .unwrap();
        let mut dock = monitor(3, 92);
        dock.slot = slot;
        dock.name = "Dock-A".into();
        dock.focused = true;
        assert!(core
            .command(
                std::slice::from_ref(&dock),
                &[],
                "configure Dock-A 4 6 true 10"
            )
            .error
            .is_empty());
        assert_eq!(
            core.slots
                .resolve("Dock-B", "Make", "Model", "serial", &[])
                .unwrap(),
            slot
        );
        dock.name = "Dock-B".into();
        assert_eq!(core.grid(&dock).columns, 4);
        let mut live = monitor(0, 11);
        live.focused = false;
        core.monitor_observe(&[live, dock]);
        let mut live = monitor(0, 92);
        live.focused = false;
        let mut dock = monitor(3, 92);
        dock.slot = slot;
        dock.focused = true;
        let plan = core.unplug(3, &[live, dock], &[]);
        assert_eq!(plan.focus, 0);
        assert_eq!(
            plan.workspaces[0].workspace, 11,
            "a parked workspace must not replace the host's own current cell"
        );
    }
    #[test]
    fn diagonal_commands_carry_and_all_use_stable_slots() {
        let mut core = Core::default();
        let mut a = monitor(3, 1);
        a.slot = 0;
        a.focused = true;
        let mut b = monitor(9, 3);
        b.slot = 2;
        let all = core.command(&[a, b], &[], "move up-left all");
        assert!(all.error.is_empty(), "{}", all.error);
        assert_eq!(
            all.moves
                .iter()
                .map(|m| (m.monitor, m.workspace, m.x, m.y, m.all))
                .collect::<Vec<_>>(),
            vec![(3, 631, 7, 7, true), (9, 633, 7, 7, true)]
        );
        let mut a = monitor(3, 1);
        a.slot = 0;
        a.focused = true;
        let carry = core.command(&[a], &[], "carry down-right");
        assert!(carry.error.is_empty());
        assert!(carry.moves[0].carry);
        assert_eq!(carry.moves[0].workspace, 91);
    }
    #[test]
    fn numbering_slot_is_independent_of_native_monitor_id() {
        let mut core = Core::default();
        let mut m = monitor(2, 8);
        m.slot = 7;
        m.focused = true;
        let reply = core.command(&[m], &[], "move right");
        assert!(reply.error.is_empty());
        assert_eq!(reply.moves[0].monitor, 2);
        assert_eq!(reply.moves[0].workspace, 18);
    }
    #[test]
    fn panic_and_round_trip() {
        assert_eq!(guarded::<u32>(|| panic!("test")), 0u32);
        assert_eq!(spike_round_trip(41), 42);
        assert_eq!(spike_round_trip(u32::MAX), 0);
    }
    #[test]
    fn all_and_carry_and_invalid_commands() {
        let mut c = Core::default();
        let ms = [monitor(0, 1), monitor(1, 92)];
        let r = c.command(&ms, &[], "move left all");
        assert!(r.error.is_empty());
        assert_eq!(
            r.moves.iter().map(|m| m.workspace).collect::<Vec<_>>(),
            vec![71, 82]
        );
        let target = c.command(&ms, &[], "move down monitor m1");
        assert!(target.error.is_empty());
        assert_eq!(target.moves[0].monitor, 1);
        assert_eq!(target.moves[0].workspace, 172);
        let r = c.command(&ms, &[], "carry down");
        assert!(r.moves[0].carry);
        assert_eq!(r.moves[0].workspace, 81);
        for cmd in [
            "move bad",
            "goto 8 0",
            "goto -1 0",
            "move left huh",
            "configure m0 0 8 true 10",
        ] {
            assert!(!c.command(&ms, &[], cmd).error.is_empty(), "{cmd}");
        }
        let bad = [monitor(0, 2), monitor(1, 92)];
        assert!(!c.command(&bad, &[], "move left all").error.is_empty());
    }
}
