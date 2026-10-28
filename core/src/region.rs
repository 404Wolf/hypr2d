//! Cell-block planning: snapshot first, then apply an injective workspace map.
use crate::{drop_script::quote, ffi, grid::Cell, guarded, Core};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub source_slot: u32,
    pub target_slot: u32,
    pub source_monitor: u32,
    pub target_monitor: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub target_x: i32,
    pub target_y: i32,
    pub swap: bool,
}
impl Core {
    pub(crate) fn region_map(&self, r: &Request) -> Result<BTreeMap<u32, u32>, String> {
        let a = self.slot_grid(r.source_slot, "");
        let b = self.slot_grid(r.target_slot, "");
        plan(a, b, r)
    }
}
pub(crate) fn plan(
    a: crate::grid::Grid,
    b: crate::grid::Grid,
    r: &Request,
) -> Result<BTreeMap<u32, u32>, String> {
    if r.width == 0
        || r.height == 0
        || r.width > a.columns
        || r.height > a.rows
        || r.width > b.columns
        || r.height > b.rows
        || r.x.checked_add(r.width).is_none_or(|v| v > a.columns)
        || r.y.checked_add(r.height).is_none_or(|v| v > a.rows)
    {
        return Err("block dimensions do not fit the grids".into());
    }
    if !b.wrap
        && (r.target_x < 0
            || r.target_y < 0
            || i64::from(r.target_x) + i64::from(r.width) > i64::from(b.columns)
            || i64::from(r.target_y) + i64::from(r.height) > i64::from(b.rows))
    {
        return Err("destination block is outside the grid".into());
    }
    let mut map = BTreeMap::new();
    let mut destinations = BTreeSet::new();
    for dy in 0..r.height {
        for dx in 0..r.width {
            let from = a.id(
                Cell {
                    x: r.x + dx,
                    y: r.y + dy,
                },
                r.source_slot,
            )?;
            let to = b.id(
                Cell {
                    x: (i64::from(r.target_x) + i64::from(dx)).rem_euclid(i64::from(b.columns))
                        as u32,
                    y: (i64::from(r.target_y) + i64::from(dy)).rem_euclid(i64::from(b.rows)) as u32,
                },
                r.target_slot,
            )?;
            if !destinations.insert(to) {
                return Err("non-injective block destination".into());
            }
            map.insert(from, to);
        }
    }
    if r.swap {
        if map.iter().all(|(a, b)| a == b) {
            return Ok(BTreeMap::new());
        }
        if map.keys().any(|k| destinations.contains(k)) {
            return Err("overlapping block swaps are ambiguous".into());
        }
        for (a, b) in map.clone() {
            map.insert(b, a);
        }
    }
    Ok(map)
}
impl Core {
    pub fn region_prepare(
        &self,
        input: &str,
        windows: &[ffi::UIWindow],
        monitors: &[ffi::Monitor],
    ) -> ffi::RegionBatch {
        guarded(|| {
            let result = (|| {
                let request: Request = serde_json::from_str(input).map_err(|e| e.to_string())?;
                if !self.known_grids().contains(&request.source_slot)
                    || !self.known_grids().contains(&request.target_slot)
                {
                    return Err("unknown grid".into());
                }
                let mapping = self.region_map(&request)?;
                let source = monitors
                    .iter()
                    .find(|m| m.id == request.source_monitor)
                    .ok_or("source display disappeared")?;
                let target = monitors
                    .iter()
                    .find(|m| m.id == request.target_monitor)
                    .ok_or("target display disappeared")?;
                let mut moves = vec![];
                for w in windows {
                    let Some(to) = w
                        .workspace
                        .parse::<u32>()
                        .ok()
                        .and_then(|id| mapping.get(&id))
                    else {
                        continue;
                    };
                    if w.workspace == to.to_string() {
                        continue;
                    }
                    let output = if self
                        .slot_grid(request.source_slot, "")
                        .cell(
                            w.workspace.parse().map_err(|_| "invalid workspace")?,
                            request.source_slot,
                        )
                        .is_some()
                    {
                        target
                    } else {
                        source
                    };
                    let current = monitors
                        .iter()
                        .find(|m| m.id == w.monitor)
                        .ok_or("source window display disappeared")?;
                    moves.push(Move {
                        address: w.id.clone(),
                        from: w.workspace.clone(),
                        to: to.to_string(),
                        monitor: output.id,
                        old_monitor: current.id,
                        x: w.x / current.width * output.width,
                        y: w.y / current.height * output.height,
                        old_x: w.x,
                        old_y: w.y,
                        floating: w.floating,
                    });
                }
                let mut targets = BTreeMap::new();
                for id in mapping.keys().chain(mapping.values()) {
                    let monitor = if self
                        .slot_grid(request.source_slot, "")
                        .cell(*id, request.source_slot)
                        .is_some()
                    {
                        source.id
                    } else {
                        target.id
                    };
                    targets.insert(*id, monitor);
                }
                let forward = if moves.is_empty() {
                    String::new()
                } else {
                    script(&moves, false)
                };
                let inverse = script(&moves, true);
                Ok(ffi::RegionBatch {
                    targets: targets.into_iter().map(|(w, m)| placement(w, m)).collect(),
                    script: forward,
                    undo_script: inverse,
                    workspaces: mapping
                        .keys()
                        .chain(mapping.values())
                        .map(u32::to_string)
                        .collect(),
                    error: String::new(),
                })
            })();
            result.unwrap_or_else(|error: String| ffi::RegionBatch {
                error,
                ..Default::default()
            })
        })
    }
}
struct Move {
    address: String,
    from: String,
    to: String,
    monitor: u32,
    old_monitor: u32,
    x: f64,
    y: f64,
    old_x: f64,
    old_y: f64,
    floating: bool,
}
fn script(moves: &[Move], undo: bool) -> String {
    // Lua is a native dispatcher adapter: all membership and coordinates are
    // planned above. Preflight every lookup before any public move can run.
    let mut s=crate::drop_script::MONITOR_LOOKUP.to_owned()+"local function dispatch(d) local r=hl.dispatch(d); assert(r.ok,r.error) end; local plan={}; local by_address={}; ";
    for m in moves {
        let (from, to, mon, x, y) = if undo {
            (&m.to, &m.from, m.old_monitor, m.old_x, m.old_y)
        } else {
            (&m.from, &m.to, m.monitor, m.x, m.y)
        };
        s+=&format!("do local w=assert(hl.get_window({}),'region window missing'); assert(w.mapped and w.workspace and w.workspace.addressable_name=={},'region source changed'); local p={{window=w,source={},dest={},monitor={},x={},y={},floating={},rollback_x=w.at.x,rollback_y=w.at.y}}; plan[#plan+1]=p; by_address[w.address]=p end; ",quote(&format!("address:{}",m.address)),quote(from),quote(from),quote(to),mon,x,y,m.floating);
    }
    s+="for _,p in ipairs(plan) do local g=p.window.group; if g then assert(not g.locked,'locked group'); for _,w in ipairs(g.members) do assert(by_address[w.address] and by_address[w.address].dest==p.dest,'group crosses block boundary') end end end; local saved={}; local focused=hl.get_active_monitor(); for _,m in ipairs(hl.get_monitors()) do saved[#saved+1]={monitor=m,workspace=m.active_workspace,special=m.active_special_workspace} end; local applied={}; local success,failure=pcall(function() dispatch(hl.dsp.group.lock({action='set'})); local done={}; for _,p in ipairs(plan) do if not done[p.window.address] then local g=p.window.group; if g then for _,w in ipairs(g.members) do done[w.address]=true end else done[p.window.address]=true end; dispatch(hl.dsp.focus({monitor=monitor_by_id(p.monitor)})); dispatch(hl.dsp.window.move({window=p.window,workspace=p.dest,follow=false})); applied[#applied+1]=p; end end; for _,p in ipairs(plan) do if p.floating then local m=monitor_by_id(p.monitor); assert(m,'target display missing'); dispatch(hl.dsp.window.move({window=p.window,x=m.x+p.x,y=m.y+p.y,relative=false})) end end end); if not success then for i=#applied,1,-1 do local p=applied[i]; pcall(function() dispatch(hl.dsp.window.move({window=p.window,workspace=p.source,follow=false})) end) end; for _,p in ipairs(plan) do if p.floating then pcall(function() dispatch(hl.dsp.window.move({window=p.window,x=p.rollback_x,y=p.rollback_y,relative=false})) end) end end end; dispatch(hl.dsp.group.lock({action='unset'})); for _,s in ipairs(saved) do local m=s.monitor; if m and m.enabled then dispatch(hl.dsp.focus({monitor=m})); if s.workspace then dispatch(hl.dsp.focus({workspace=s.workspace})) end; local wanted=s.special; if wanted and wanted.is_empty then wanted=nil end; if m.active_special_workspace~=wanted then if m.active_special_workspace then dispatch(hl.dsp.workspace.toggle_special(m.active_special_workspace.addressable_name:sub(9))) end; if wanted then dispatch(hl.dsp.workspace.toggle_special(wanted.addressable_name:sub(9))) end end end end; if focused then dispatch(hl.dsp.focus({monitor=focused})) end; if not success then error(failure) end; ";
    s
}
impl Default for ffi::RegionBatch {
    fn default() -> Self {
        Self {
            script: String::new(),
            undo_script: String::new(),
            workspaces: vec![],
            targets: vec![],
            error: "region planning failed".into(),
        }
    }
}

fn placement(workspace: u32, monitor: u32) -> ffi::Move {
    ffi::Move {
        workspace,
        monitor,
        x: 0,
        y: 0,
        vertical: false,
        positive: false,
        carry: false,
        all: true,
    }
}
pub(crate) struct Undo {
    script: String,
    targets: Vec<(u32, u32)>,
    workspaces: Vec<String>,
}
impl Core {
    pub fn region_record(&mut self, batch: &ffi::RegionBatch) {
        guarded(|| {
            if batch.undo_script.is_empty() {
                return;
            }
            self.region_history.push(Undo {
                script: batch.undo_script.clone(),
                targets: batch
                    .targets
                    .iter()
                    .map(|t| (t.workspace, t.monitor))
                    .collect(),
                workspaces: batch.workspaces.clone(),
            });
            if self.region_history.len() > 20 {
                self.region_history.remove(0);
            }
        });
    }
    pub fn region_inverse(&self) -> ffi::RegionBatch {
        guarded(|| {
            self.region_history
                .last()
                .map(|u| ffi::RegionBatch {
                    script: u.script.clone(),
                    undo_script: String::new(),
                    targets: u.targets.iter().map(|(w, m)| placement(*w, *m)).collect(),
                    workspaces: u.workspaces.clone(),
                    error: String::new(),
                })
                .unwrap_or_else(|| ffi::RegionBatch {
                    error: "No region operation to undo".into(),
                    ..Default::default()
                })
        })
    }
    pub fn region_forget(&mut self, all: bool) {
        guarded(|| {
            if all {
                self.region_history.clear();
            } else {
                self.region_history.pop();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r() -> Request {
        Request {
            source_slot: 0,
            target_slot: 1,
            source_monitor: 0,
            target_monitor: 1,
            x: 1,
            y: 2,
            width: 2,
            height: 2,
            target_x: 3,
            target_y: 4,
            swap: false,
        }
    }
    #[test]
    fn cell_blocks_preserve_offsets_merge_swap_wrap_and_refuse() {
        let core = Core::default();
        let p = r();
        let m = core.region_map(&p).unwrap();
        assert_eq!(m.len(), 4);
        assert_eq!(m[&171], 352);
        assert_eq!(m[&261], 442);
        let mut p = p;
        p.swap = true;
        assert_eq!(core.region_map(&p).unwrap().len(), 8);
        p.target_slot = 0;
        p.target_x = 2;
        p.target_y = 2;
        assert!(core.region_map(&p).is_err());
        p.swap = false;
        p.target_x = 7;
        p.target_y = 4;
        let m = core.region_map(&p).unwrap();
        assert!(m.values().any(|w| *w == 321));
        let mut c = core;
        c.grids.insert(
            "0".into(),
            crate::grid::Grid {
                wrap: false,
                ..Default::default()
            },
        );
        assert!(c.region_map(&p).is_err());
    }
}
