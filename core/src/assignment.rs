//! Grid ownership is independent of stable output identities.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Assignment {
    version: u32,
    preferred: BTreeMap<u32, u32>,
    last: BTreeMap<u32, u32>,
    pub cells: BTreeMap<u32, u32>,
    #[serde(skip)]
    pub restore_keep: bool,
    #[serde(skip)]
    path: Option<PathBuf>,
}
impl Assignment {
    pub fn open(path: PathBuf, keep: bool) -> Result<Self, String> {
        let mut a = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice::<Self>(&b)
                .map_err(|e| format!("invalid grid assignments: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self {
                version: 1,
                ..Self::default()
            },
            Err(e) => return Err(e.to_string()),
        };
        if a.version != 1 {
            return Err("unsupported grid assignment version".into());
        }
        a.path = Some(path);
        a.restore_keep = keep;
        Ok(a)
    }
    pub fn known(&self, slot: u32) -> bool {
        self.preferred.contains_key(&slot)
            || self.preferred.values().any(|g| *g == slot)
            || self.last.values().any(|g| *g == slot)
    }
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path.parent().ok_or("invalid assignment path")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, bytes)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| e.to_string())
    }
    pub fn connect(
        &mut self,
        physical: &BTreeMap<String, u32>,
        old: &std::collections::HashMap<String, u32>,
        known: &BTreeSet<u32>,
    ) -> Result<std::collections::HashMap<String, u32>, String> {
        let mut result = std::collections::HashMap::new();
        let mut used = BTreeSet::new();
        // Existing live displays always retain their grids; a reconnect never steals.
        for (name, host) in physical {
            if let Some(grid) = old.get(name) {
                if !used.insert(*grid) {
                    return Err("duplicate live grid mapping".into());
                }
                result.insert(name.clone(), *grid);
                self.last.insert(*host, *grid);
            }
        }
        for (name, host) in physical {
            if result.contains_key(name) {
                continue;
            }
            let preferred = *self.preferred.entry(*host).or_insert(*host);
            let candidate = if self.restore_keep {
                self.last.get(host).copied().unwrap_or(preferred)
            } else {
                preferred
            };
            let grid = [Some(candidate), Some(*host)]
                .into_iter()
                .flatten()
                .chain(known.iter().copied())
                .find(|g| known.contains(g) && !used.contains(g))
                .ok_or("no free grid for connected monitor")?;
            used.insert(grid);
            result.insert(name.clone(), grid);
            self.last.insert(*host, grid);
        }
        self.save()?;
        Ok(result)
    }
    pub fn assign(
        &mut self,
        physical: &std::collections::HashMap<String, u32>,
        active: &mut std::collections::HashMap<String, u32>,
        name: &str,
        grid: u32,
    ) -> Result<(), String> {
        let host = *physical.get(name).ok_or("monitor is not connected")?;
        if active.iter().any(|(n, g)| n != name && *g == grid) {
            return Err("grid is already live; use swap-grids".into());
        }
        active.insert(name.into(), grid);
        self.preferred.insert(host, grid);
        self.last.insert(host, grid);
        self.save()
    }
    pub fn swap(
        &mut self,
        physical: &std::collections::HashMap<String, u32>,
        active: &mut std::collections::HashMap<String, u32>,
        a: &str,
        b: &str,
    ) -> Result<(), String> {
        let ga = *active.get(a).ok_or("first monitor is not connected")?;
        let gb = *active.get(b).ok_or("second monitor is not connected")?;
        for (name, grid) in [(a, gb), (b, ga)] {
            let host = *physical.get(name).ok_or("unknown physical identity")?;
            active.insert(name.into(), grid);
            self.preferred.insert(host, grid);
            self.last.insert(host, grid);
        }
        self.save()
    }
}
impl crate::Core {
    pub fn grid_checkpoint(&self) -> String {
        crate::guarded(|| {
            serde_json::json!({"assignments":self.assignments,"active":self.connected_slots})
                .to_string()
        })
    }
    pub fn grid_rollback(&mut self, snapshot: &str) -> String {
        crate::guarded(|| {
            let result = (|| {
                let value: serde_json::Value =
                    serde_json::from_str(snapshot).map_err(|e| e.to_string())?;
                let mut assignments: Assignment =
                    serde_json::from_value(value["assignments"].clone())
                        .map_err(|e| e.to_string())?;
                let active =
                    serde_json::from_value(value["active"].clone()).map_err(|e| e.to_string())?;
                assignments.path = self.assignments.path.clone();
                assignments.restore_keep = self.assignments.restore_keep;
                assignments.save()?;
                self.assignments = assignments;
                self.connected_slots = active;
                Ok::<(), String>(())
            })();
            result.err().unwrap_or_default()
        })
    }
    pub fn grid_change(
        &mut self,
        monitors: &[crate::ffi::Monitor],
        input: &str,
    ) -> crate::ffi::Reply {
        crate::guarded(|| {
            let operation = (|| {
                let args: Vec<_> = input.split_whitespace().collect();
                let select = |s: &str| {
                    monitors
                        .iter()
                        .find(|m| m.name == s || m.id.to_string() == s)
                        .ok_or("unknown monitor")
                };
                let backup = self.assignments.clone();
                let old = self.connected_slots.clone();
                let result = match args.as_slice() {
                    ["adopt", slot] | ["adopt", slot, _] => {
                        let slot: u32 = slot.parse().map_err(|_| "invalid grid slot")?;
                        if self.slots.aliases(slot).next().is_none() {
                            return Err("unknown grid slot".into());
                        }
                        let monitor = if args.len() == 3 {
                            select(args[2])?
                        } else {
                            monitors
                                .iter()
                                .find(|m| m.focused)
                                .ok_or("no focused monitor")?
                        };
                        self.assignments.assign(
                            &self.physical_slots,
                            &mut self.connected_slots,
                            &monitor.name,
                            slot,
                        )
                    }
                    ["swap-grids", a, b] => {
                        let a = select(a)?;
                        let b = select(b)?;
                        self.assignments.swap(
                            &self.physical_slots,
                            &mut self.connected_slots,
                            &a.name,
                            &b.name,
                        )
                    }
                    _ => Err("expected adopt SLOT [MONITOR] or swap-grids A B".into()),
                };
                if let Err(e) = result {
                    self.assignments = backup;
                    self.connected_slots = old;
                    return Err(e);
                }
                self.region_history.clear();
                Ok(crate::ffi::Reply {
                    output: "ok".into(),
                    error: String::new(),
                    moves: vec![],
                })
            })();
            operation.unwrap_or_else(|error: String| crate::ffi::Reply {
                error,
                ..Default::default()
            })
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adoption_swapping_persistence_and_restore_policy() {
        let root = std::env::temp_dir().join(format!("hypr2d-assignment-{}", std::process::id()));
        let path = root.join("grids.json");
        let mut a = Assignment::open(path.clone(), false).unwrap();
        let hosts = BTreeMap::from([("A".into(), 0), ("B".into(), 1), ("C".into(), 2)]);
        let known = BTreeSet::from([0, 1, 2]);
        let mut active = a.connect(&hosts, &Default::default(), &known).unwrap();
        let p = hosts.clone().into_iter().collect();
        a.swap(&p, &mut active, "A", "B").unwrap();
        a.swap(&p, &mut active, "B", "C").unwrap();
        assert_eq!((active["A"], active["B"], active["C"]), (1, 2, 0));
        let mut reopened = Assignment::open(path.clone(), false).unwrap();
        assert_eq!(
            reopened
                .connect(&hosts, &Default::default(), &known)
                .unwrap(),
            active
        );
        assert!(a.assign(&p, &mut active, "A", 2).is_err());
        // Conflicting reconnect must never take a grid away from a live display.

        let live = std::collections::HashMap::from([("A".into(), 1)]);
        a.preferred.insert(1, 1);
        a.last.insert(1, 2);
        a.restore_keep = false;
        let pair = BTreeMap::from([("A".into(), 0), ("B".into(), 1)]);
        let result = a.connect(&pair, &live, &known).unwrap();
        assert_eq!(result["A"], 1);
        assert_ne!(result["B"], 1);

        a.last.insert(1, 2);
        a.preferred.insert(1, 0);
        a.restore_keep = true;
        assert_eq!(a.connect(&pair, &live, &known).unwrap()["B"], 2);
        a.restore_keep = false;
        assert_eq!(a.connect(&pair, &live, &known).unwrap()["B"], 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
