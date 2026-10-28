//! A completed overview drop restores monitor/workspace focus before its marker.
//! Replay known occupancy deltas without an IPC round trip behind render work.
//! Ambiguous, external, metadata, config and topology changes always fetch afresh.
use super::{Snapshot, Window};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Updates {
    moves: Vec<(String, String)>,
    finished: Option<(String, String)>,
    blocked: bool,
}
fn address(raw: &str) -> Option<String> {
    let raw = raw.strip_prefix("0x").unwrap_or(raw);
    (!raw.is_empty() && raw.len() <= 16 && raw.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("0x{}", raw.to_ascii_lowercase()))
}
impl Updates {
    pub fn observe(&mut self, event: &str, data: &str) {
        if event == "movewindowv2" {
            let mut parts = data.splitn(3, ',');
            let parsed = parts
                .next()
                .and_then(address)
                .zip(parts.next())
                .zip(parts.next());
            if let Some(((address, id), name)) = parsed {
                self.moves.push((
                    address,
                    if name.starts_with("special:") {
                        name.into()
                    } else {
                        id.into()
                    },
                ));
            } else {
                self.blocked = true;
            }
        } else if event == "hypr2d" && data.starts_with("drop>>") {
            self.finished = data[6..]
                .split_once(',')
                .and_then(|(raw, target)| address(raw).map(|a| (a, target.into())));
        } else if !matches!(
            event,
            "movewindow"
                | "createworkspace"
                | "createworkspacev2"
                | "destroyworkspace"
                | "destroyworkspacev2"
        ) {
            self.blocked = true;
        }
    }
    pub fn apply(&self, state: &mut Snapshot) -> bool {
        if self.blocked
            || self.moves.is_empty()
            || !self
                .finished
                .as_ref()
                .is_some_and(|last| self.moves.contains(last))
        {
            return false;
        }
        let known: HashMap<_, Window> = state
            .monitors
            .iter()
            .flat_map(|m| &m.cells)
            .flat_map(|c| &c.windows)
            .map(|w| (w.address.clone(), w.clone()))
            .collect();
        // Only each client's final destination contributes to occupancy.
        let moves: std::collections::BTreeMap<_, _> =
            self.moves.iter().map(|(a, t)| (a, t)).collect();
        // Validate the whole burst before changing the cached snapshot.
        let mut destinations = Vec::new();
        for (address, target) in &moves {
            if !known.contains_key(*address) {
                return false;
            }
            if target.starts_with("special:") {
                destinations.push(None);
                continue;
            }
            let Ok(id) = target.parse::<u32>() else {
                return false;
            };
            let Some(destination) = state.monitors.iter().enumerate().find_map(|(mi, m)| {
                m.cells
                    .iter()
                    .position(|c| c.workspace == id)
                    .map(|ci| (mi, ci))
            }) else {
                return false;
            };
            destinations.push(Some(destination));
        }
        for ((address, target), destination) in moves.iter().zip(destinations) {
            for cell in state.monitors.iter_mut().flat_map(|m| &mut m.cells) {
                cell.windows.retain(|w| &w.address != *address);
            }
            if let Some((mi, ci)) = destination {
                state.monitors[mi].cells[ci]
                    .windows
                    .push(known[*address].clone());
            } else {
                if !state.specials.is_array() {
                    state.specials = serde_json::json!([]);
                }
                let entries = state
                    .specials
                    .as_array_mut()
                    .expect("special array initialized");
                if let Some(entry) = entries
                    .iter_mut()
                    .find(|s| s["name"].as_str() == Some(target.as_str()))
                {
                    entry["windows"] =
                        serde_json::json!(entry["windows"].as_u64().unwrap_or(0) + 1);
                } else {
                    let app = &known[*address].app_id;
                    let name = target.strip_prefix("special:").unwrap_or(target);
                    let label = state
                        .special_names
                        .get(name)
                        .map(String::as_str)
                        .unwrap_or_else(|| {
                            if app.is_empty() {
                                name
                            } else {
                                app.rsplit('.').next().unwrap_or(app)
                            }
                        });
                    entries.push(
                        serde_json::json!({"name":target,"label":label,"app":app,"windows":1}),
                    );
                }
            }
        }
        true
    }
}
