pub mod grid;
mod overview;
mod ui;
use overview::probe_report;
use ui::Overview;
pub mod swipe;
use grid::{Cell, Direction, Grid};
use serde_json::json;
use std::collections::HashMap;

#[cxx::bridge(namespace = "hypr2d")]
mod ffi {
    struct Monitor {
        id: u32,
        name: String,
        workspace: u32,
        focused: bool,
        offset_x: f64,
        offset_y: f64,
        width: f64,
        height: f64,
    }
    struct Window {
        workspace: u32,
        address: String,
        title: String,
        app_id: String,
        urgent: bool,
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
    struct SwipeFrame {
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
    }
    struct UICell {
        key: String,
        monitor: u32,
        rect: UIRect,
        label: String,
        selected: bool,
        hovered: bool,
        occupied: bool,
        shown: bool,
    }
    struct UILabel {
        rect: UIRect,
        text: String,
    }
    struct UIDrop {
        window: String,
        swap: String,
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
    struct UIFrame {
        ghost: UIGhost,
        windows: Vec<UIWindowView>,
        target: String,
        alive: bool,
        animating: bool,
        alpha: f64,
        cells: Vec<UICell>,
        labels: Vec<UILabel>,
    }
    extern "Rust" {
        type Overview;
        fn overview_press(self: &mut Overview, width: f64, height: f64, x: f64, y: f64);
        fn overview_release(self: &mut Overview, width: f64, height: f64, x: f64, y: f64)
            -> UIDrop;
        fn overview_reconcile(self: &mut Overview, replacement: Box<Overview>) -> Vec<String>;
        fn overview_windows(self: &mut Overview, windows: &[UIWindow]) -> Vec<String>;
        fn overview_state(self: &Overview, width: f64, height: f64, now: f64) -> String;
        fn overview_new(
            self: &Core,
            outputs: &[UIOutput],
            specials: &[UISpecial],
            windows: &[UIWindow],
            now: f64,
        ) -> Box<Overview>;
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
        fn swipe(self: &mut Core, monitors: &[Monitor], input: SwipeInput) -> SwipeFrame;
        fn spike_round_trip(value: u32) -> u32;
        fn command(self: &mut Core, monitors: &[Monitor], windows: &[Window], input: &str)
            -> Reply;
    }
}
pub struct Core {
    grids: HashMap<String, Grid>,
    stride: u32,
    swipe_config: swipe::Config,
    gesture: Option<swipe::Swipe>,
}
impl Default for Core {
    fn default() -> Self {
        Self {
            grids: HashMap::new(),
            stride: 10,
            swipe_config: swipe::Config::default(),
            gesture: None,
        }
    }
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
        self.grids
            .get(m.name.as_str())
            .or_else(|| self.grids.get(&m.id.to_string()))
            .copied()
            .unwrap_or(Grid {
                stride: self.stride,
                ..Grid::default()
            })
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
        let args: Vec<_> = input.split_whitespace().collect();
        let mut reply = ffi::Reply {
            moves: vec![],
            output: "ok".into(),
            error: String::new(),
        };
        if matches!(args.as_slice(), ["state"] | ["state", "-j"]) {
            reply.output = json!({"version":1,"swipe":self.gesture.as_ref().map(|g|g.status()),"swipe_config":self.swipe_config,"monitors":monitors.iter().map(|m| {
                let g = self.grid(m);
                json!({"id":m.id,"name":m.name,"workspace":m.workspace,"focused":m.focused,"grid":g,"cell":g.cell(m.workspace,m.id),"offset":[m.offset_x,m.offset_y],
                    "cells":(0..g.rows).flat_map(|y|(0..g.columns).map(move |x|Cell{x,y})).filter_map(|c|g.id(c,m.id).ok().map(|id|json!({"x":c.x,"y":c.y,"workspace":id,"windows":windows.iter().filter(|w|w.workspace==id).map(|w|json!({"address":w.address,"title":w.title,"app_id":w.app_id,"urgent":w.urgent})).collect::<Vec<_>>()}))).collect::<Vec<_>>()})
            }).collect::<Vec<_>>()}).to_string();
            return Ok(reply);
        }
        if let ["configure", name, columns, rows, wrap, stride] = args.as_slice() {
            let id = monitors
                .iter()
                .find(|m| m.name == *name || m.id.to_string() == *name)
                .map_or(0, |m| m.id);
            let g = Grid {
                columns: number(columns)?,
                rows: number(rows)?,
                wrap: wrap.parse().map_err(|_| "wrap must be true/false")?,
                stride: number(stride)?,
            }
            .validate(id)?;
            if monitors.iter().any(|other| other.id >= g.stride) {
                return Err("stride must exceed every monitor id".into());
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
            self.grids.insert((*name).to_string(), g);
            return Ok(reply);
        }
        if let ["swipe-config", fingers, distance, velocity, natural, axis_lock] = args.as_slice() {
            let config = swipe::Config {
                fingers: number(fingers)?,
                distance: distance.parse().map_err(|_| "invalid distance")?,
                velocity: velocity.parse().map_err(|_| "invalid velocity")?,
                natural: natural.parse().map_err(|_| "natural must be true/false")?,
                axis_lock: axis_lock.parse().map_err(|_| "invalid axis lock")?,
            }
            .validate()?;
            self.swipe_config = config;
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
            let g = self.grid(m).validate(m.id)?;
            let current = g.cell(m.workspace, m.id);
            let c = if let Some(c) = target {
                c
            } else {
                let c =
                    current.ok_or("active workspace is outside this monitor's grid; use goto")?;
                g.step(c, dir.ok_or("missing direction")?).unwrap_or(c)
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
                workspace: g.id(c, m.id)?,
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
                    m.id,
                    m.workspace,
                    self.grid(m),
                    [m.width, m.height],
                    self.swipe_config,
                    now,
                );
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
                claimed: true,
                active: !gesture.finished,
                settling: gesture.settling(),
                finish_after_ms: if phase == ffi::SwipePhase::End && !gesture.finished {
                    swipe::MAX_SETTLE_MS + 1.
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

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(id: u32, ws: u32) -> ffi::Monitor {
        ffi::Monitor {
            id,
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
