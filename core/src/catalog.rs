//! Named grids have stable identities; output identity remains in `slots.json`.
use crate::grid::{virtual_base, Grid, GRID_BLOCK, VIRTUAL_GRID};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// None preserves the legacy owner-derived display name until renamed.
    pub name: Option<String>,
    pub owner: Option<u32>,
    /// Legacy grids retain existing configure/alias behavior until explicitly set.
    pub config: Option<Grid>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Catalog {
    version: u32,
    pub entries: BTreeMap<u32, Entry>,
    retired: BTreeSet<u32>,
    pub defaults: BTreeMap<u32, u32>,
    pub bindings_seeded: bool,
    #[serde(skip)]
    path: Option<PathBuf>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            entries: BTreeMap::new(),
            retired: BTreeSet::new(),
            defaults: BTreeMap::new(),
            bindings_seeded: false,
            path: None,
        }
    }
}
fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 96 || name.chars().any(char::is_control) {
        return Err("grid names need 1..96 printable characters".into());
    }
    Ok(name.to_owned())
}
/// Test a legacy grid's arithmetic progression without enumerating its cells.
pub fn overlaps_legacy(base: u32, slot: u32, grid: Grid) -> bool {
    if virtual_base(slot).is_some() || grid.stride == 0 || grid.columns == 0 || grid.rows == 0 {
        return false;
    }
    let start = u64::from(slot) + 1;
    let end = u64::from(base) + u64::from(GRID_BLOCK) - 1;
    let first = if u64::from(base) > start {
        (u64::from(base) - start).div_ceil(u64::from(grid.stride))
    } else {
        0
    };
    first < u64::from(grid.columns) * u64::from(grid.rows)
        && start + first * u64::from(grid.stride) <= end
}
impl Catalog {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let mut c: Self = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("invalid grid catalogue: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.to_string()),
        };
        if c.version != 1 || c.entries.keys().any(|id| c.retired.contains(id)) {
            return Err("invalid grid catalogue version or recycled identity".into());
        }
        let mut names = BTreeSet::new();
        for (id, entry) in &c.entries {
            if let Some(grid) = entry.config {
                grid.validate(*id)?;
            }
            if let Some(name) = &entry.name {
                valid_name(name)?;
                if !names.insert(name.to_lowercase()) {
                    return Err("duplicate grid name".into());
                }
            }
            if *id >= VIRTUAL_GRID && (virtual_base(*id).is_none() || entry.config.is_none()) {
                return Err("invalid named grid identity or missing dimensions".into());
            }
        }
        c.path = Some(path);
        Ok(c)
    }
    pub fn known(&self) -> BTreeSet<u32> {
        self.entries.keys().copied().collect()
    }
    pub fn rejects_legacy(&self, slot: u32, grid: Grid) -> bool {
        self.entries
            .keys()
            .chain(&self.retired)
            .filter_map(|s| virtual_base(*s))
            .any(|b| overlaps_legacy(b, slot, grid))
    }
    pub fn create(
        &mut self,
        name: &str,
        grid: Grid,
        legacy: &[(u32, Grid)],
        workspace_ids: &[u32],
    ) -> Result<u32, String> {
        let name = valid_name(name)?;
        self.unique(&name, None)?;
        let id = (16384..=32767)
            .map(|b| VIRTUAL_GRID | b)
            .find(|id| {
                let base = virtual_base(*id).unwrap_or(0);
                !self.entries.contains_key(id)
                    && !self.retired.contains(id)
                    && !legacy.iter().any(|(s, g)| overlaps_legacy(base, *s, *g))
                    && !workspace_ids.iter().any(|w| {
                        *w >= base && u64::from(*w) < u64::from(base) + u64::from(GRID_BLOCK)
                    })
            })
            .ok_or("no collision-free named-grid workspace block available")?;
        grid.validate(id)?;
        self.entries.insert(
            id,
            Entry {
                name: Some(name),
                owner: None,
                config: Some(grid),
            },
        );
        if let Err(e) = self.save() {
            self.entries.remove(&id);
            return Err(e);
        }
        Ok(id)
    }
    fn unique(&self, name: &str, except: Option<u32>) -> Result<(), String> {
        if self.entries.iter().any(|(id, e)| {
            Some(*id) != except
                && e.name
                    .as_ref()
                    .is_some_and(|n| n.to_lowercase() == name.to_lowercase())
        }) {
            Err("grid name already exists".into())
        } else {
            Ok(())
        }
    }
    pub fn rename(&mut self, id: u32, name: &str) -> Result<(), String> {
        let name = valid_name(name)?;
        self.unique(&name, Some(id))?;
        let entry = self.entries.get_mut(&id).ok_or("unknown grid")?;
        let old = entry.name.replace(name);
        if let Err(e) = self.save() {
            self.entries.get_mut(&id).ok_or("grid disappeared")?.name = old;
            return Err(e);
        }
        Ok(())
    }
    /// Metadata only: the compositor adapter must first reject live/nonempty grids.
    pub fn remove(&mut self, id: u32) -> Result<(), String> {
        let old = self.entries.remove(&id).ok_or("unknown grid")?;
        self.retired.insert(id);
        if let Err(e) = self.save() {
            self.retired.remove(&id);
            self.entries.insert(id, old);
            return Err(e);
        }
        Ok(())
    }
    pub fn ensure_legacy(&mut self, slot: u32) -> Result<bool, String> {
        if self.entries.contains_key(&slot) || self.retired.contains(&slot) {
            return Ok(false);
        }
        if slot >= VIRTUAL_GRID {
            return Err("legacy identity overlaps named-grid namespace".into());
        }
        self.entries.insert(
            slot,
            Entry {
                name: None,
                owner: Some(slot),
                config: None,
            },
        );
        self.defaults.insert(slot, slot);
        if let Err(e) = self.save() {
            self.entries.remove(&slot);
            self.defaults.remove(&slot);
            return Err(e);
        }
        Ok(true)
    }
    pub fn save(&self) -> Result<(), String> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path.parent().ok_or("catalogue path needs a parent")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp = path.with_extension(format!("tmp.{}.{nonce}", std::process::id()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| e.to_string())?;
            file.write_all(&serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(tmp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Cell;
    #[test]
    fn catalogue_persists_independent_names_without_recycling_or_legacy_collisions() {
        let dir = std::env::temp_dir().join(format!("hypr2d-catalog-{}", std::process::id()));
        let path = dir.join("catalog.json");
        let mut c = Catalog::open(path.clone()).unwrap();
        c.ensure_legacy(0).unwrap();
        c.ensure_legacy(1).unwrap();
        let g = Grid {
            columns: 9,
            rows: 9,
            ..Default::default()
        };
        let large_stride = Grid {
            columns: 2,
            rows: 1,
            stride: 1 << 30,
            ..Default::default()
        };
        let a = c.create("work", g, &[(0, large_stride)], &[]).unwrap();
        assert_ne!(virtual_base(a), Some(1 << 30));
        let b = c
            .create("research", g, &[(0, Grid::default())], &[])
            .unwrap();
        assert_ne!(a, b);
        assert!(c.create("WORK", g, &[], &[]).is_err());
        c.rename(a, "writing").unwrap();
        c.remove(b).unwrap();
        let mut reopened = Catalog::open(path.clone()).unwrap();
        assert_eq!(reopened.entries[&a].name.as_deref(), Some("writing"));
        let next = reopened.create("new", g, &[], &[]).unwrap();
        assert_ne!(next, b);
        for id in [a, next] {
            for y in 0..9 {
                for x in 0..9 {
                    let cell = Cell { x, y };
                    let ws = g.id(cell, id).unwrap();
                    assert_eq!(g.cell(ws, id), Some(cell));
                    assert!(g.cell(ws, 0).is_none());
                }
            }
        }
        let base = virtual_base(next).unwrap();
        assert!(reopened.rejects_legacy(
            0,
            Grid {
                columns: 2,
                rows: 1,
                stride: base,
                ..Default::default()
            }
        ));
        assert!(!reopened.ensure_legacy(b).unwrap());
        let mut many = BTreeSet::new();
        for i in 0..16 {
            let id = reopened
                .create(&format!("project {i}"), g, &[], &[])
                .unwrap();
            assert_ne!(id, b);
            assert!(many.insert(id));
        }
        assert_eq!(many.len(), 16); // independent of two physical/default grids
        let reserved = virtual_base(VIRTUAL_GRID | 16384).unwrap();
        let mut fresh = Catalog::default();
        assert_ne!(
            virtual_base(
                fresh
                    .create("avoid external workspace", g, &[], &[reserved + 9])
                    .unwrap()
            ),
            Some(reserved)
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn virtual_maximum_grid_fits_signed_workspace_ids_and_rejects_other_blocks() {
        let g = Grid {
            columns: 256,
            rows: 256,
            ..Default::default()
        };
        let id = VIRTUAL_GRID | 32767;
        g.validate(id).unwrap();
        assert_eq!(g.id(Cell { x: 255, y: 255 }, id).unwrap(), i32::MAX as u32);
        assert!(g.cell(0, id).is_none());
        assert!(g.cell(1 << 30, id).is_none());
        assert!(g.validate(VIRTUAL_GRID | 32768).is_err());
        assert_eq!(Grid::default().id(Cell { x: 7, y: 7 }, 0).unwrap(), 631);
    }
}
