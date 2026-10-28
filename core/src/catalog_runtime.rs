//! Catalogue policy over snapshots; only the adapter mutates compositor objects.
use crate::{
    catalog::Catalog,
    ffi,
    grid::{virtual_base, Cell, Grid},
    Core,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
struct Checkpoint {
    catalog: Catalog,
    assignments: crate::assignment::Assignment,
    cells: std::collections::HashMap<u32, u32>,
}

impl Core {
    pub(crate) fn load_grid_config(&mut self, config: &Value) -> Result<(), String> {
        if let Some(stride) = config["grid"].get("stride") {
            self.stride = stride
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or("invalid grid stride")?;
        }
        if let Some(grids) = config["grid"].get("monitors").and_then(|v| v.as_object()) {
            for (name, value) in grids {
                let mut grid: crate::grid::Grid =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                grid.stride = self.stride;
                let slot = name
                    .parse::<u32>()
                    .ok()
                    .or_else(|| {
                        self.slots
                            .known()
                            .into_iter()
                            .find(|s| self.slots.aliases(*s).any(|n| n == name))
                    })
                    .unwrap_or(0);
                grid.validate(slot)?;
                self.grids.insert(name.clone(), grid);
            }
        }
        Ok(())
    }
    pub fn catalog_workspaces(&mut self, ids: &[u32]) {
        crate::guarded(|| {
            self.known_workspaces = ids.iter().copied().collect();
        });
    }
    pub(crate) fn grid_name(&self, id: u32) -> String {
        if let Some(name) = self.catalog.entries.get(&id).and_then(|e| e.name.clone()) {
            return name;
        }
        let owner = self
            .catalog
            .entries
            .get(&id)
            .and_then(|e| e.owner)
            .unwrap_or(id);
        let fallback = self
            .slots
            .aliases(owner)
            .next()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("grid {id}"));
        self.slot_label(owner, &fallback)
    }
    pub(crate) fn grid_disconnected(&self, id: u32) -> bool {
        let entry = self.catalog.entries.get(&id);
        let owner = entry.and_then(|e| e.owner);
        if entry.is_some() && owner.is_none() {
            return false;
        }
        !self
            .physical_slots
            .values()
            .any(|s| *s == owner.unwrap_or(id))
    }
    pub(crate) fn catalog_rename(&mut self, source: &str, name: &str) -> Result<(), String> {
        let id = self.resolve_grid(source)?;
        if self
            .known_grids()
            .iter()
            .any(|s| *s != id && self.grid_name(*s).to_lowercase() == name.trim().to_lowercase())
        {
            return Err("grid name already exists".into());
        }
        self.catalog.rename(id, name)
    }
    pub(crate) fn known_grids(&self) -> BTreeSet<u32> {
        if !self.catalog.initialized() {
            self.slots.known()
        } else {
            self.catalog.known()
        }
    }
    pub(crate) fn resolve_grid(&self, name: &str) -> Result<u32, String> {
        if let Ok(id) = name.parse::<u32>() {
            if self.known_grids().contains(&id) {
                return Ok(id);
            }
        }
        let found: Vec<_> = self
            .known_grids()
            .into_iter()
            .filter(|id| self.grid_name(*id) == name)
            .collect();
        match found.as_slice() {
            [id] => Ok(*id),
            [] => Err("unknown grid".into()),
            _ => Err("ambiguous grid name; use its ID".into()),
        }
    }
    pub(crate) fn legacy_grids(&self) -> Vec<(u32, Grid)> {
        self.slots
            .known()
            .into_iter()
            .filter(|s| virtual_base(*s).is_none())
            .filter_map(|s| {
                let g = self.slot_grid(s, "");
                g.validate(s).ok().map(|_| (s, g))
            })
            .collect()
    }
    pub(crate) fn ensure_output_grid(&mut self, host: u32) -> Result<(), String> {
        if self.catalog.defaults.contains_key(&host) {
            return Ok(());
        }
        let grid = self.slot_grid(host, "");
        if virtual_base(host).is_none() && grid.validate(host).is_ok() {
            self.catalog.ensure_legacy(host)?;
        } else {
            let legacy = self.legacy_grids();
            let ids = self.known_workspaces.iter().copied().collect::<Vec<_>>();
            self.catalog.create_owned(host, grid, &legacy, &ids)?;
        }
        let default = *self
            .catalog
            .defaults
            .get(&host)
            .ok_or("output has no default grid")?;
        self.assignments.seed_default(host, default)
    }
    pub(crate) fn detached_grids(&self) -> Vec<(u32, String)> {
        self.known_grids()
            .into_iter()
            .filter(|id| !self.connected_slots.values().any(|s| s == id))
            .filter(|id| self.catalog.entries.contains_key(id) || self.assignments.known(*id))
            .map(|id| (id, self.grid_name(id)))
            .collect()
    }
    pub(crate) fn catalog_snapshot(
        &self,
        monitors: &[ffi::Monitor],
        windows: &[ffi::Window],
    ) -> Value {
        Value::Array(self.known_grids().into_iter().map(|id|{
            let grid=self.slot_grid(id,"");let host=monitors.iter().find(|m|m.slot==id);
            let workspace=host.map(|m|m.workspace).or_else(||self.remembered_cells.get(&id).copied()).unwrap_or_else(||grid.id(Cell{x:0,y:0},id).unwrap_or(0));
            json!({"slot":id,"name":self.grid_name(id),"grid":grid,"workspace":workspace,"monitor":host.map(|m|m.name.clone()),"monitor_label":host.map(|m|self.monitor_label(&m.name)),"windows":windows.iter().filter(|w|grid.cell(w.workspace,id).is_some()).count()})
        }).collect())
    }
    fn catalog_checkpoint(&self) -> Checkpoint {
        Checkpoint {
            catalog: self.catalog.clone(),
            assignments: self.assignments.clone(),
            cells: self.remembered_cells.clone(),
        }
    }
    fn catalog_save_cells(&mut self, before: Checkpoint) -> Result<(), String> {
        if let Err(e) = self.assignments.save() {
            self.catalog = before.catalog;
            self.assignments = before.assignments;
            self.remembered_cells = before.cells;
            if let Err(rollback) = self.catalog.save() {
                return Err(format!("{e}; catalogue rollback failed: {rollback}"));
            }
            return Err(e);
        }
        Ok(())
    }
    pub(crate) fn catalog_command(
        &mut self,
        monitors: &[ffi::Monitor],
        windows: &[ffi::Window],
        input: &str,
    ) -> Result<ffi::Reply, String> {
        let body = input.strip_prefix("grid ").ok_or("expected grid command")?;
        let (op, tail) = body.split_once(' ').unwrap_or((body, ""));
        let args: Vec<String> = if tail.trim_start().starts_with('[') {
            serde_json::from_str(tail).map_err(|e| e.to_string())?
        } else {
            tail.split_whitespace().map(str::to_owned).collect()
        };
        let mut output = "ok".to_owned();
        match (op, args.as_slice()) {
            ("list", []) => output = self.catalog_snapshot(monitors, windows).to_string(),
            ("create", [name]) | ("create", [name, _]) => {
                if self
                    .known_grids()
                    .iter()
                    .any(|s| self.grid_name(*s).to_lowercase() == name.trim().to_lowercase())
                {
                    return Err("grid name already exists".into());
                }
                let mut grid = monitors
                    .iter()
                    .find(|m| m.focused)
                    .map(|m| self.grid(m))
                    .unwrap_or(Grid {
                        stride: self.stride,
                        ..Default::default()
                    });
                if let Some(options) = args.get(1) {
                    let v: Value = serde_json::from_str(options).map_err(|e| e.to_string())?;
                    for key in v
                        .as_object()
                        .ok_or("grid options must be an object")?
                        .keys()
                    {
                        if !matches!(key.as_str(), "columns" | "rows" | "wrap") {
                            return Err("unknown grid option".into());
                        }
                    }
                    if let Some(n) = v.get("columns") {
                        grid.columns = n
                            .as_u64()
                            .and_then(|n| n.try_into().ok())
                            .ok_or("invalid columns")?;
                    }
                    if let Some(n) = v.get("rows") {
                        grid.rows = n
                            .as_u64()
                            .and_then(|n| n.try_into().ok())
                            .ok_or("invalid rows")?;
                    }
                    if let Some(v) = v.get("wrap") {
                        grid.wrap = v.as_bool().ok_or("wrap must be boolean")?;
                    }
                }
                let legacy = self.legacy_grids();
                let ids = self
                    .known_workspaces
                    .iter()
                    .copied()
                    .chain(windows.iter().map(|w| w.workspace))
                    .collect::<Vec<_>>();
                let before = self.catalog_checkpoint();
                let id = self.catalog.create(name, grid, &legacy, &ids)?;
                let origin = grid.id(Cell { x: 0, y: 0 }, id)?;
                self.remembered_cells.insert(id, origin);
                self.assignments.cells.insert(id, origin);
                self.catalog_save_cells(before)?;
                output = json!({"slot":id,"name":self.grid_name(id),"grid":grid}).to_string();
            }
            ("rename", [source, name]) => self.catalog_rename(source, name)?,
            ("delete", [source]) => {
                let id = self.resolve_grid(source)?;
                let grid = self.slot_grid(id, "");
                if self.connected_slots.values().any(|s| *s == id)
                    || monitors
                        .iter()
                        .any(|m| grid.cell(m.workspace, id).is_some())
                {
                    return Err("only parked, inactive grids can be deleted".into());
                }
                if windows.iter().any(|w| grid.cell(w.workspace, id).is_some()) {
                    return Err("grid contains windows; move them before deleting it".into());
                }
                let before = self.catalog_checkpoint();
                self.catalog.remove(id)?;
                self.remembered_cells.remove(&id);
                self.assignments.cells.remove(&id);
                self.catalog_save_cells(before)?;
                self.region_history.clear();
            }
            _ => {
                return Err(
                    "expected grid list/create NAME [OPTIONS]/rename GRID NAME/delete GRID".into(),
                )
            }
        }
        Ok(ffi::Reply {
            moves: vec![],
            output,
            error: String::new(),
        })
    }
    pub(crate) fn catalog_open(&mut self, path: std::path::PathBuf) -> Result<(), String> {
        self.catalog = Catalog::open(path)?;
        for (slot, grid) in self.legacy_grids() {
            if self.catalog.rejects_legacy(slot, grid) {
                return Err(
                    "legacy grid configuration overlaps a reserved named-grid block".into(),
                );
            }
        }
        for slot in self.slots.known() {
            if self.assignments.known(slot) {
                self.ensure_output_grid(slot)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observe(core: &mut Core) -> ffi::Monitor {
        assert!(core
            .slots_update(&[ffi::MonitorIdentity {
                id: 0,
                name: "eDP-1".into(),
                make: "VM".into(),
                model: "Panel".into(),
                serial: "one".into(),
                x: 8934.,
                y: 6682.
            }])
            .is_empty());
        ffi::Monitor {
            id: 0,
            slot: core.monitor_slot("eDP-1"),
            name: "eDP-1".into(),
            workspace: 1,
            focused: true,
            offset_x: 0.,
            offset_y: 0.,
            width: 2256.,
            height: 1504.,
        }
    }
    #[test]
    fn deleting_default_grids_does_not_resurrect_them_or_reassign_a_live_grid() {
        let mut c = Core::default();
        let identity = |name: &str, id| ffi::MonitorIdentity {
            id,
            name: name.into(),
            make: "VM".into(),
            model: name.into(),
            serial: name.into(),
            x: 0.,
            y: 0.,
        };
        let update = |c: &mut Core, both| {
            let mut ids = vec![identity("RIGHT", 1)];
            if both {
                ids.push(identity("eDP-1", 0));
            }
            assert!(c.slots_update(&ids).is_empty());
        };
        update(&mut c, true);
        let a: Value = serde_json::from_str(&c.command(&[], &[], "grid create a").output).unwrap();
        let a = a["slot"].as_u64().unwrap() as u32;
        let b: Value = serde_json::from_str(&c.command(&[], &[], "grid create b").output).unwrap();
        let b = b["slot"].as_u64().unwrap() as u32;
        let monitors = |c: &Core| {
            ["eDP-1", "RIGHT"]
                .into_iter()
                .enumerate()
                .map(|(id, name)| {
                    let mut m = ffi::Monitor {
                        id: id as u32,
                        slot: c.monitor_slot(name),
                        name: name.into(),
                        workspace: 0,
                        focused: id == 0,
                        offset_x: 0.,
                        offset_y: 0.,
                        width: 2256.,
                        height: 1504.,
                    };
                    m.workspace = c.restore_workspace(&m);
                    m
                })
                .collect::<Vec<_>>()
        };
        assert!(c
            .grid_change(&monitors(&c), "adopt a eDP-1")
            .error
            .is_empty());
        assert!(c
            .grid_change(&monitors(&c), "adopt b RIGHT")
            .error
            .is_empty());
        assert!(c
            .command(&monitors(&c), &[], "grid delete 0")
            .error
            .is_empty());
        assert!(c
            .command(&monitors(&c), &[], "grid delete 1")
            .error
            .is_empty());
        update(&mut c, false);
        assert!(c.command(&[], &[], "grid delete a").error.is_empty());
        update(&mut c, true);
        let fresh = c.monitor_slot("eDP-1");
        assert_ne!(fresh, a);
        assert_eq!(c.monitor_slot("RIGHT"), b);
        assert_eq!(c.catalog.entries[&fresh].owner, Some(0));
        assert!(!c.known_grids().contains(&0) && !c.known_grids().contains(&1));
        c.connected_slots.clear();
        update(&mut c, true);
        assert_eq!(c.monitor_slot("eDP-1"), fresh);
        assert_eq!(c.monitor_slot("RIGHT"), b);
    }
    #[test]
    fn dimensions_load_before_restoring_a_live_nine_by_nine_cell() {
        let mut core = Core::default();
        core.load_grid_config(
            &json!({"grid":{"stride":10,"monitors":{"eDP-1":{"columns":9,"rows":9,"wrap":true}}}}),
        )
        .unwrap();
        let mut m = observe(&mut core);
        m.workspace = 801;
        core.remembered_cells.insert(0, 801);
        assert_eq!(core.restore_workspace(&m), 801);
        assert_eq!(
            core.slot_grid(0, "eDP-1").cell(801, 0),
            Some(Cell { x: 8, y: 8 })
        );
    }
    #[test]
    fn named_grids_create_adopt_rename_and_delete_without_renumbering_legacy_content() {
        let mut c = Core::default();
        let m = observe(&mut c);
        c.catalog_workspaces(&[(1 << 30) + 7]);
        let created = c.command(
            &[m],
            &[],
            r#"grid create ["work","{\"columns\":9,\"rows\":9}"]"#,
        );
        assert!(created.error.is_empty(), "{}", created.error);
        let value: Value = serde_json::from_str(&created.output).unwrap();
        let id = value["slot"].as_u64().unwrap() as u32;
        assert_ne!(virtual_base(id), Some(1 << 30));
        let g = c.slot_grid(id, "");
        assert_eq!((g.columns, g.rows), (9, 9));
        let m = observe(&mut c);
        assert!(c
            .grid_change(&[m], r#"adopt ["work","eDP-1"]"#)
            .error
            .is_empty());
        assert_eq!(c.monitor_slot("eDP-1"), id);
        let m = observe(&mut c);
        assert!(!c.command(&[m], &[], "grid delete work").error.is_empty());
        let m = observe(&mut c);
        assert!(c.grid_change(&[m], "adopt 0").error.is_empty());
        let m = observe(&mut c);
        assert!(c
            .command(&[m], &[], r#"grid rename ["work","writing space"]"#)
            .error
            .is_empty());
        assert_eq!(c.resolve_grid("writing space").unwrap(), id);
        let w = ffi::Window {
            workspace: g.id(Cell { x: 2, y: 3 }, id).unwrap(),
            monitor: 0,
            address: "0xabc".into(),
            title: "safe".into(),
            app_id: "kitty".into(),
            urgent: false,
        };
        let m = observe(&mut c);
        assert!(!c
            .command(&[m], &[w], r#"grid delete ["writing space"]"#)
            .error
            .is_empty());
        let m = observe(&mut c);
        assert!(c
            .command(&[m], &[], r#"grid delete ["writing space"]"#)
            .error
            .is_empty());
        assert!(c.resolve_grid(&id.to_string()).is_err());
        let m = observe(&mut c);
        let recreated = c.command(&[m], &[], r#"grid create ["writing space"]"#);
        assert!(recreated.error.is_empty());
        assert_ne!(
            serde_json::from_str::<Value>(&recreated.output).unwrap()["slot"],
            id
        );
        assert_eq!(c.slot_grid(0, "").id(Cell { x: 7, y: 7 }, 0).unwrap(), 631);
    }
    #[test]
    fn catalogue_creation_rolls_back_if_current_cell_persistence_fails() {
        let root =
            std::env::temp_dir().join(format!("hypr2d-catalog-runtime-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut c = Core::default();
        observe(&mut c);
        c.catalog_open(root.join("catalog.json")).unwrap();
        c.assignments =
            crate::assignment::Assignment::open(root.join("blocked/grids.json"), false).unwrap();
        std::fs::write(root.join("blocked"), "not a directory").unwrap();
        let before = c.catalog.entries.len();
        let result = c.command(&[], &[], "grid create failure");
        assert!(!result.error.is_empty());
        assert_eq!(c.catalog.entries.len(), before);
        assert!(!c
            .known_grids()
            .into_iter()
            .any(|id| virtual_base(id).is_some()));
        assert_eq!(
            Catalog::open(root.join("catalog.json"))
                .unwrap()
                .entries
                .len(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
