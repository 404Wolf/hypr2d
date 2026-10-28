//! Presentation names are independent of workspace numbering and output identity.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Saved {
    version: u32,
    monitors: BTreeMap<u32, String>,
    specials: BTreeMap<String, String>,
    last_position: BTreeMap<u32, String>,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            version: 1,
            monitors: BTreeMap::new(),
            specials: BTreeMap::new(),
            last_position: BTreeMap::new(),
        }
    }
}
#[derive(Default)]
pub(crate) struct Labels {
    saved: Saved,
    configured_monitors: BTreeMap<String, String>,
    configured_specials: BTreeMap<String, String>,
    position: bool,
    path: Option<PathBuf>,
}
fn special(name: &str) -> &str {
    name.strip_prefix("special:").unwrap_or(name)
}
fn valid(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > 96 || value.chars().any(char::is_control)
    {
        Err("labels must contain 1..96 printable characters".into())
    } else {
        Ok(())
    }
}
impl Labels {
    pub fn open(path: PathBuf, config: &serde_json::Value) -> Result<Self, String> {
        let saved: Saved = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("invalid label state: {e}"))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(e) => return Err(e.to_string()),
        };
        if saved.version != 1 {
            return Err("unsupported label state version".into());
        }
        let configured_monitors: BTreeMap<String, String> = serde_json::from_value(
            config["monitors"]
                .get("labels")
                .cloned()
                .unwrap_or(serde_json::json!({})),
        )
        .map_err(|e| e.to_string())?;
        let names: BTreeMap<String, String> = serde_json::from_value(
            config["specials"]
                .get("names")
                .cloned()
                .unwrap_or(serde_json::json!({})),
        )
        .map_err(|e| e.to_string())?;
        let configured_specials = names
            .into_iter()
            .map(|(k, v)| (special(&k).to_owned(), v))
            .collect::<BTreeMap<_, _>>();
        for label in configured_monitors
            .values()
            .chain(configured_specials.values())
            .chain(saved.monitors.values())
            .chain(saved.specials.values())
            .chain(saved.last_position.values())
        {
            valid(label)?;
        }
        let position = match config["monitors"]
            .get("autoLabel")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
        {
            "position" => true,
            "none" => false,
            _ => return Err("monitors.autoLabel must be none or position".into()),
        };
        Ok(Self {
            saved,
            configured_monitors,
            configured_specials,
            position,
            path: Some(path),
        })
    }
    fn configured_monitor(&self, keys: &[String]) -> Option<&str> {
        keys.iter()
            .find_map(|key| self.configured_monitors.get(key).map(String::as_str))
    }
    pub fn monitor(
        &self,
        slot: u32,
        keys: &[String],
        fallback: &str,
        position: Option<(usize, usize)>,
    ) -> String {
        self.configured_monitor(keys)
            .map(str::to_owned)
            .or_else(|| self.saved.monitors.get(&slot).cloned())
            .or_else(|| {
                self.position
                    .then(|| {
                        position
                            .map(|(i, n)| Self::position_label(i, n))
                            .or_else(|| self.saved.last_position.get(&slot).cloned())
                    })
                    .flatten()
            })
            .unwrap_or_else(|| fallback.to_owned())
    }
    pub(crate) fn names(&self) -> BTreeMap<String, String> {
        let mut names = self.saved.specials.clone();
        names.extend(self.configured_specials.clone());
        names
    }
    pub fn special(&self, name: &str) -> Option<&str> {
        let name = special(name);
        self.configured_specials
            .get(name)
            .or_else(|| self.saved.specials.get(name))
            .map(String::as_str)
    }
    pub fn position_label(index: usize, count: usize) -> String {
        if count <= 1 {
            "main".into()
        } else if index == 0 {
            "left".into()
        } else if index + 1 == count {
            "right".into()
        } else if count == 3 {
            "middle".into()
        } else {
            format!("middle {index}")
        }
    }
    pub fn remember_position(&mut self, positions: &[(u32, usize, usize)]) -> Result<(), String> {
        if !self.position {
            return Ok(());
        }
        let old = self.saved.clone();
        let mut changed = false;
        for (slot, index, count) in positions {
            let label = Self::position_label(*index, *count);
            if self.saved.last_position.get(slot) != Some(&label) {
                self.saved.last_position.insert(*slot, label);
                changed = true;
            }
        }
        if changed {
            if let Err(e) = self.save() {
                self.saved = old;
                return Err(e);
            }
        }
        Ok(())
    }
    pub fn rename(&mut self, target: &str, value: &str, keys: &[String]) -> Result<(), String> {
        let value = value.trim();
        if !value.is_empty() {
            valid(value)?;
        }
        let old = self.saved.clone();
        let table = if let Some(name) = target.strip_prefix("special:") {
            if self.configured_specials.contains_key(name) {
                return Err("this special label is set by configuration".into());
            }
            (&mut self.saved.specials, name.to_owned())
        } else if let Some(slot) = target.strip_prefix("monitor:") {
            let slot: u32 = slot.parse().map_err(|_| "invalid monitor label target")?;
            if self.configured_monitor(keys).is_some() {
                return Err("this monitor label is set by configuration".into());
            }
            if value.is_empty() {
                self.saved.monitors.remove(&slot);
            } else {
                self.saved.monitors.insert(slot, value.into());
            }
            if let Err(e) = self.save() {
                self.saved = old;
                return Err(e);
            }
            return Ok(());
        } else {
            return Err("unknown label target".into());
        };
        if value.is_empty() {
            table.0.remove(&table.1);
        } else {
            table.0.insert(table.1, value.into());
        }
        if let Err(e) = self.save() {
            self.saved = old;
            return Err(e);
        }
        Ok(())
    }
    fn save(&self) -> Result<(), String> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path.parent().ok_or("label state has no parent")?;
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
            file.write_all(&serde_json::to_vec_pretty(&self.saved).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

impl crate::Core {
    pub(crate) fn slot_label(&self, slot: u32, fallback: &str) -> String {
        self.labels.monitor(
            slot,
            &self.slots.label_keys(slot, fallback),
            fallback,
            self.positions.get(&slot).copied(),
        )
    }
    pub(crate) fn monitor_label(&self, name: &str) -> String {
        self.physical_slots
            .get(name)
            .map(|s| self.slot_label(*s, name))
            .unwrap_or_else(|| name.to_owned())
    }
    pub fn ui_output_label(&self, output: &crate::ffi::UIOutput) -> String {
        crate::guarded(|| {
            if output.parked && self.catalog.entries.contains_key(&output.slot) {
                self.grid_name(output.slot)
            } else if output.parked {
                self.slot_label(output.slot, &output.name)
            } else {
                self.monitor_label(&output.name)
            }
        })
    }
    pub fn specials_observe(&mut self, windows: &[crate::ffi::SpecialInfo]) {
        crate::guarded(|| {
            self.special_apps.clear();
            for w in windows {
                self.special_apps
                    .entry(w.name.clone())
                    .or_default()
                    .push(w.app.clone());
            }
        });
    }
    pub(crate) fn special_labels(&self) -> serde_json::Value {
        serde_json::Value::Array(self.special_apps.iter().map(|(name,apps)|serde_json::json!({"name":name,"label":self.labels.special(name).map(str::to_owned).unwrap_or_else(||crate::ui::special_label(name,apps.first().map(|a|crate::ui::short_app(a)).unwrap_or_default())),"app":apps.first(),"windows":apps.len()})).collect())
    }
    pub fn rename(&mut self, input: &str) -> String {
        crate::guarded(|| {
            let result = (|| {
                let args: Vec<String> = serde_json::from_str(input).map_err(|e| e.to_string())?;
                let [target, value] = args.as_slice() else {
                    return Err("expected label target and text".into());
                };
                if let Some(id) = target.strip_prefix("grid:") {
                    return self.catalog_rename(id, value);
                }
                let keys = target
                    .strip_prefix("monitor:")
                    .and_then(|s| s.parse().ok())
                    .map(|s| self.slots.label_keys(s, ""))
                    .unwrap_or_default();
                self.labels.rename(target, value, &keys)
            })();
            result.err().unwrap_or_default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dock_description_aliases_position_and_multiword_dispatch_arguments() {
        let path =
            std::env::temp_dir().join(format!("hypr2d-label-identity-{}.json", std::process::id()));
        let mut core = crate::Core { labels:Labels::open(path.clone(),&serde_json::json!({"monitors":{"autoLabel":"position","labels":{"eDP-1":"main laptop","Vendor Panel serial-a":"desk"}}})).unwrap(),..Default::default() };
        let identity = |name: &str, x: f64| crate::ffi::MonitorIdentity {
            id: 0,
            name: name.into(),
            make: "Vendor".into(),
            model: "Panel".into(),
            serial: if name == "eDP-1" {
                "internal"
            } else {
                "serial-a"
            }
            .into(),
            x,
            y: 6682.,
        };
        assert!(core
            .slots_update(&[identity("eDP-1", 8934.), identity("DP-1", 0.)])
            .is_empty());
        let dock = core.monitor_slot("DP-1");
        assert!(core.slots_update(&[identity("eDP-1", 8934.)]).is_empty());
        assert!(core
            .slots_update(&[identity("eDP-1", 8934.), identity("DP-9", 0.)])
            .is_empty());
        assert_eq!(core.monitor_slot("DP-9"), dock);
        assert_eq!(core.monitor_label("DP-9"), "desk");
        let monitor = |id, name: &str, slot| crate::ffi::Monitor {
            id,
            slot,
            name: name.into(),
            workspace: slot + 1,
            focused: id == 0,
            offset_x: 0.,
            offset_y: 0.,
            width: 2256.,
            height: 1504.,
        };
        let ms = [monitor(0, "eDP-1", 0), monitor(1, "DP-9", dock)];
        let command = crate::grid_args("swap-grids", "main laptop", "desk");
        assert!(core.grid_change(&ms, &command).error.is_empty());
        assert_eq!(core.monitor_slot("eDP-1"), dock);
        let encoded = crate::grid_args("adopt", "0", "a \"quoted\" name");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(encoded.split_once(' ').unwrap().1).unwrap()[1],
            "a \"quoted\" name"
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn configured_runtime_position_names_and_atomic_persistence() {
        let dir = std::env::temp_dir().join(format!("hypr2d-labels-{}", std::process::id()));
        let path = dir.join("labels.json");
        let cfg = serde_json::json!({"monitors":{"labels":{"eDP-1":"main","Dell Panel SERIAL":"desk"},"autoLabel":"position"},"specials":{"names":{"3":"chatting"}}});
        let mut labels = Labels::open(path.clone(), &cfg).unwrap();
        assert_eq!(labels.special("special:3"), Some("chatting"));
        assert!(labels.rename("special:3", "ignored", &[]).is_err());
        assert!(labels
            .rename("monitor:0", "ignored", &["eDP-1".into()])
            .is_err());
        labels
            .rename("monitor:1", "work", &["DP-1".into()])
            .unwrap();
        labels.rename("special:5", "research", &[]).unwrap();
        labels
            .remember_position(&[(0, 0, 3), (1, 1, 3), (2, 2, 3)])
            .unwrap();
        let mut labels = Labels::open(path.clone(), &cfg).unwrap();
        assert_eq!(
            labels.monitor(1, &["DP-9".into()], "DP-9", Some((2, 3))),
            "work"
        );
        assert_eq!(
            labels.monitor(
                1,
                &["DP-9".into(), "Dell Panel SERIAL".into()],
                "DP-9",
                None
            ),
            "desk"
        );
        assert_eq!(labels.monitor(2, &[], "DP-2", None), "right");
        assert_eq!(labels.special("special:5"), Some("research"));
        labels.rename("special:5", "", &[]).unwrap();
        assert_eq!(labels.special("5"), None);
        assert!(labels.rename("special:5", "bad\nlabel", &[]).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
