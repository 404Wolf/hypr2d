//! Persistent identity policy. Native monitor IDs remain IPC addresses only.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    slot: u32,
    names: BTreeSet<String>,
    identity: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Slots {
    version: u32,
    entries: Vec<Entry>,
    #[serde(skip)]
    path: Option<PathBuf>,
}
impl Default for Slots {
    fn default() -> Self {
        Self {
            version: 1,
            entries: vec![Entry {
                slot: 0,
                names: BTreeSet::from(["eDP-1".into()]),
                identity: String::new(),
            }],
            path: None,
        }
    }
}
impl Slots {
    pub fn open(path: PathBuf, pins: BTreeMap<String, u32>) -> Result<Self, String> {
        let mut slots = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Self>(&bytes)
                .map_err(|e| format!("invalid slots file: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.to_string()),
        };
        if slots.version != 1
            || slots.entries.iter().any(|e| e.names.is_empty())
            || slots
                .entries
                .iter()
                .map(|e| e.slot)
                .collect::<BTreeSet<_>>()
                .len()
                != slots.entries.len()
            || !slots
                .entries
                .iter()
                .any(|e| e.slot == 0 && e.names.contains("eDP-1"))
        {
            return Err("invalid slot registry; refusing to renumber workspaces".into());
        }
        let mut assigned = BTreeSet::new();
        for (name, slot) in &pins {
            if name.is_empty()
                || (*slot == 0 && name != "eDP-1")
                || (name == "eDP-1" && *slot != 0)
                || !assigned.insert(*slot)
            {
                return Err("monitorSlots must be unique; slot 0 is reserved for eDP-1".into());
            }
            if slots
                .entries
                .iter()
                .any(|e| e.slot == *slot && !e.names.contains(name))
            {
                return Err(format!("slot {slot} is already permanently assigned"));
            }
            if slots
                .entries
                .iter()
                .any(|e| e.names.contains(name) && e.slot != *slot)
            {
                return Err(format!(
                    "monitor {name} already has a different persisted slot"
                ));
            }
        }
        // Reserve configured slots even before their monitor is plugged in.
        for (name, slot) in &pins {
            if !slots.entries.iter().any(|e| e.names.contains(name)) {
                slots.entries.push(Entry {
                    slot: *slot,
                    names: BTreeSet::from([name.clone()]),
                    identity: String::new(),
                });
            }
        }
        slots.path = Some(path);
        slots.save()?;
        Ok(slots)
    }
    fn save(&self) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let Some(path) = &self.path else {
            return Ok(());
        };
        std::fs::create_dir_all(path.parent().ok_or("slot file needs a parent")?)
            .map_err(|e| e.to_string())?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| e.to_string())?;
            f.write_all(&serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(tmp);
        }
        result
    }
    pub fn known(&self) -> BTreeSet<u32> {
        self.entries.iter().map(|e| e.slot).collect()
    }
    pub fn aliases(&self, slot: u32) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(move |e| e.slot == slot)
            .flat_map(|e| e.names.iter().map(String::as_str))
    }
    pub fn resolve(
        &mut self,
        name: &str,
        make: &str,
        model: &str,
        serial: &str,
        live: &[u32],
    ) -> Result<u32, String> {
        let identity = if make.is_empty() || model.is_empty() || serial.is_empty() {
            String::new()
        } else {
            serde_json::to_string(&(make, model, serial)).map_err(|e| e.to_string())?
        };
        let existing = self
            .entries
            .iter()
            .position(|e| e.names.contains(name))
            .or_else(|| {
                if identity.is_empty() {
                    return None;
                }
                let mut candidates = self
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.identity == identity && !live.contains(&e.slot));
                let first = candidates.next().map(|(i, _)| i);
                if candidates.next().is_none() {
                    first
                } else {
                    None
                }
            });
        let index = if let Some(i) = existing {
            i
        } else {
            let used: BTreeSet<_> = self.entries.iter().map(|e| e.slot).collect();
            let slot = (1..u32::MAX)
                .find(|n| !used.contains(n))
                .ok_or("no monitor slots available")?;
            self.entries.push(Entry {
                slot,
                names: BTreeSet::new(),
                identity: String::new(),
            });
            self.entries.len() - 1
        };
        let entry = &mut self.entries[index];
        let changed = entry.names.insert(name.into()) || entry.identity != identity;
        if !identity.is_empty() {
            entry.identity = identity
        }
        let slot = entry.slot;
        if changed {
            self.save()?;
        }
        Ok(slot)
    }
}
impl crate::Core {
    pub fn slots_init(&mut self) -> String {
        crate::guarded(|| {
            let init = (|| {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .ok_or("HOME is not set")?;
                let state = std::env::var_os("XDG_STATE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".local/state"));
                let config = std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".config"));
                let pins = match std::fs::read(config.join("hypr/hypr2d.json")) {
                    Ok(bytes) => {
                        let value: serde_json::Value =
                            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                        serde_json::from_value(
                            value
                                .get("monitorSlots")
                                .cloned()
                                .unwrap_or(serde_json::json!({})),
                        )
                        .map_err(|e| e.to_string())?
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                    Err(e) => return Err(e.to_string()),
                };
                self.slots = Slots::open(state.join("hypr2d/slots.json"), pins)?;
                let config_value: serde_json::Value =
                    std::fs::read(config.join("hypr/hypr2d.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or_default();
                let keep = match config_value
                    .get("gridRestore")
                    .and_then(|v| v.as_str())
                    .unwrap_or("auto")
                {
                    "auto" => false,
                    "keep" => true,
                    _ => return Err("gridRestore must be auto or keep".into()),
                };
                self.assignments =
                    crate::assignment::Assignment::open(state.join("hypr2d/grids.json"), keep)?;
                self.remembered_cells = self
                    .assignments
                    .cells
                    .iter()
                    .map(|(a, b)| (*a, *b))
                    .collect();
                Ok::<_, String>(())
            })();
            init.err().unwrap_or_default()
        })
    }
    pub fn slots_update(&mut self, monitors: &[crate::ffi::MonitorIdentity]) -> String {
        crate::guarded(|| {
            let mut connected = std::collections::HashMap::new();
            let mut live = Vec::new();
            // Retain known live identities before considering serial fallbacks.
            for m in monitors {
                if let Some(slot) = self.physical_slots.get(&m.name) {
                    live.push(*slot);
                }
            }
            for m in monitors {
                match self
                    .slots
                    .resolve(&m.name, &m.make, &m.model, &m.serial, &live)
                {
                    Ok(slot) => {
                        if connected.values().any(|existing| *existing == slot) {
                            return format!("monitor identity alias shares live slot {slot}; configure distinct identities");
                        }
                        connected.insert(m.name.clone(), slot);
                        live.push(slot);
                    }
                    Err(e) => return e,
                }
            }
            let physical = connected.iter().map(|(n, s)| (n.clone(), *s)).collect();
            let old = self.connected_slots.clone();
            match self
                .assignments
                .connect(&physical, &old, &self.slots.known())
            {
                Ok(mapping) => self.connected_slots = mapping,
                Err(e) => return e,
            }
            self.physical_slots = connected;
            String::new()
        })
    }
    pub fn monitor_slot(&self, name: &str) -> u32 {
        crate::guarded(|| self.connected_slots.get(name).copied().unwrap_or(u32::MAX))
    }
}

impl Slots {
    fn detached(&self, connected: &std::collections::HashMap<String, u32>) -> Vec<(u32, String)> {
        self.entries
            .iter()
            .filter(|e| !connected.values().any(|s| *s == e.slot))
            .map(|e| (e.slot, e.names.iter().next().cloned().unwrap_or_default()))
            .collect()
    }
}
impl crate::Core {
    fn parked_grid(&self, slot: u32, name: &str) -> crate::grid::Grid {
        self.slot_grid(slot, name)
    }
    pub fn parked_snapshot(&self, windows: &[crate::ffi::Window]) -> serde_json::Value {
        serde_json::Value::Array(self.slots.detached(&self.connected_slots).into_iter().filter_map(|(slot,name)|{
            let grid=self.parked_grid(slot,&name);
            let occupied:Vec<_>=windows.iter().filter(|w|grid.cell(w.workspace,slot).is_some()).collect();
            if occupied.is_empty(){return None}
            Some(serde_json::json!({"slot":slot,"name":name,"windows":occupied.len(),"workspaces":occupied.iter().map(|w|w.workspace).collect::<BTreeSet<_>>()}))
        }).collect())
    }
    pub fn parked_panels(
        &mut self,
        outputs: &[crate::ffi::UIOutput],
        windows: &[crate::ffi::UIWindow],
    ) -> Vec<crate::ffi::UIOutput> {
        crate::guarded(|| {
            let detached: Vec<_> = self
                .slots
                .detached(&self.connected_slots)
                .into_iter()
                .filter(|(slot, name)| {
                    self.assignments.known(*slot)
                        || windows.iter().any(|w| {
                            w.workspace
                                .parse()
                                .ok()
                                .and_then(|id| self.slot_grid(*slot, name).cell(id, *slot))
                                .is_some()
                        })
                })
                .collect();
            self.parked_counts = detached
                .iter()
                .map(|(slot, name)| {
                    let grid = self.parked_grid(*slot, name);
                    (
                        *slot,
                        windows
                            .iter()
                            .filter(|w| {
                                w.workspace
                                    .parse()
                                    .ok()
                                    .and_then(|id| grid.cell(id, *slot))
                                    .is_some()
                            })
                            .count(),
                    )
                })
                .collect();
            detached
                .into_iter()
                .filter_map(|(slot, name)| {
                    let grid = self.parked_grid(slot, &name);
                    let window = windows.iter().find(|w| {
                        w.workspace
                            .parse()
                            .ok()
                            .and_then(|id| grid.cell(id, slot))
                            .is_some()
                    });
                    let backing = window
                        .and_then(|w| outputs.iter().find(|o| o.id == w.monitor))
                        .or_else(|| outputs.first())?;
                    Some(crate::ffi::UIOutput {
                        id: 0x80000000 | slot,
                        slot,
                        parked: true,
                        backing: backing.id,
                        name,
                        x: backing.x,
                        y: backing.y,
                        width: backing.width,
                        height: backing.height,
                        workspace: self
                            .remembered_cells
                            .get(&slot)
                            .copied()
                            .unwrap_or(slot + 1),
                        focused: false,
                    })
                })
                .collect()
        })
    }
    pub fn monitor_observe(&mut self, monitors: &[crate::ffi::Monitor]) {
        crate::guarded(|| {
            for m in monitors {
                if self.owns_workspace(m, m.workspace)
                    && self.remembered_cells.insert(m.slot, m.workspace) != Some(m.workspace)
                {
                    self.assignments.cells.insert(m.slot, m.workspace);
                    if let Err(e) = self.assignments.save() {
                        eprintln!("hypr2d grid current-cell persistence: {e}");
                    }
                }
            }
        });
    }
    pub fn unplug(
        &mut self,
        id: u32,
        monitors: &[crate::ffi::Monitor],
        windows: &[crate::ffi::Window],
    ) -> crate::ffi::UnplugPlan {
        crate::guarded(|| {
            let Some(unplugged) = monitors.iter().find(|m| m.id == id) else {
                return crate::ffi::UnplugPlan::default();
            };
            self.monitor_observe(monitors);
            let remaining: Vec<_> = monitors.iter().filter(|m| m.id != id).collect();
            let focus = remaining
                .iter()
                .find(|m| m.focused)
                .or(remaining.first())
                .map_or(u32::MAX, |m| m.id);
            let workspaces = remaining
                .iter()
                .map(|m| crate::ffi::Move {
                    monitor: m.id,
                    workspace: if self.owns_workspace(m, m.workspace) {
                        m.workspace
                    } else {
                        self.restore_workspace(m)
                    },
                    x: 0,
                    y: 0,
                    vertical: false,
                    positive: false,
                    carry: false,
                    all: true,
                })
                .collect();
            crate::ffi::UnplugPlan {
                focus,
                workspaces,
                notification: format!(
                    "{} unplugged: {} windows parked (SUPER+ALT+Tab)",
                    unplugged.name,
                    windows
                        .iter()
                        .filter(|w| w.monitor == id && self.owns_workspace(unplugged, w.workspace))
                        .count()
                ),
            }
        })
    }
    pub fn owns_workspace(&self, monitor: &crate::ffi::Monitor, workspace: u32) -> bool {
        crate::guarded(|| self.grid(monitor).cell(workspace, monitor.slot).is_some())
    }
    pub fn restore_workspace(&self, monitor: &crate::ffi::Monitor) -> u32 {
        crate::guarded(|| {
            self.remembered_cells
                .get(&monitor.slot)
                .copied()
                .filter(|w| self.owns_workspace(monitor, *w))
                .or_else(|| {
                    self.owns_workspace(monitor, monitor.workspace)
                        .then_some(monitor.workspace)
                })
                .unwrap_or_else(|| {
                    self.grid(monitor)
                        .id(crate::grid::Cell { x: 0, y: 0 }, monitor.slot)
                        .unwrap_or(0)
                })
        })
    }
}
impl Default for crate::ffi::UnplugPlan {
    fn default() -> Self {
        Self {
            focus: u32::MAX,
            workspaces: Vec::new(),
            notification: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_slots_survive_order_rename_and_restart() {
        let path = std::env::temp_dir()
            .join(format!("hypr2d-slots-{}", std::process::id()))
            .join("slots.json");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let mut s = Slots::open(path.clone(), BTreeMap::new()).unwrap();
        assert_eq!(
            s.resolve("DP-1", "Dell", "Panel", "serial-a", &[]).unwrap(),
            1
        );
        assert_eq!(
            s.resolve("eDP-1", "BOE", "Laptop", "internal", &[1])
                .unwrap(),
            0
        );
        assert_eq!(
            s.resolve("HDMI-A-1", "LG", "Panel", "serial-b", &[0, 1])
                .unwrap(),
            2
        );
        let mut s = Slots::open(path.clone(), BTreeMap::new()).unwrap();
        assert_eq!(
            s.resolve("HDMI-A-1", "LG", "Panel", "serial-b", &[])
                .unwrap(),
            2
        );
        assert_eq!(
            s.resolve("DP-9", "Dell", "Panel", "serial-a", &[2])
                .unwrap(),
            1
        );
        assert_eq!(s.resolve("HEADLESS-1", "", "", "", &[1, 2]).unwrap(), 3);
        assert_eq!(s.resolve("HEADLESS-2", "", "", "", &[1, 2]).unwrap(), 4);
        assert!(Slots::open(path.clone(), BTreeMap::from([("eDP-1".into(), 2)])).is_err());
        assert!(Slots::open(path.clone(), BTreeMap::from([("new".into(), 1)])).is_err());
        assert!(Slots::open(path.clone(), BTreeMap::from([("new".into(), 5)])).is_ok());
        std::fs::write(&path, "broken").unwrap();
        assert!(Slots::open(path.clone(), BTreeMap::new()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
