//! Session-owned extension state and native action plans. Runs on compositor thread.
use crate::{
    ffi,
    grid::Cell,
    guarded,
    transport::{Incoming, Server},
    Core,
};
use hypr2d_client::{Annotation, Operation, Request, Response, Target, VERSION};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
pub struct Extensions {
    server: Option<Server>,
    sessions: BTreeMap<u64, Session>,
    pending: BTreeMap<u64, (u64, u64)>,
    next: u64,
}
#[derive(Default)]
struct Session {
    annotations: BTreeMap<String, (Target, Annotation)>,
    regions: BTreeMap<String, Region>,
    events: BTreeSet<String>,
}
#[derive(Clone, serde::Serialize)]
struct Region {
    monitor: u32,
    row: u32,
    rows: u32,
    title: String,
    skip: bool,
}
impl Extensions {
    pub fn snapshot(&self) -> Value {
        json!({"annotations":self.sessions.iter().flat_map(|(owner,s)|s.annotations.iter().map(move |(key,(target,annotation))|json!({"owner":owner,"key":key,"target":target,"annotation":annotation}))).collect::<Vec<_>>(),"regions":self.sessions.iter().flat_map(|(owner,s)|s.regions.iter().map(move |(key,r)|json!({"owner":owner,"key":key,"region":r}))).collect::<Vec<_>>()})
    }
    pub fn annotation(&self, target: &Target) -> Option<&Annotation> {
        self.sessions.values().rev().find_map(|s| {
            s.annotations
                .values()
                .rev()
                .find(|(t, _)| t == target)
                .map(|(_, a)| a)
        })
    }
    pub fn row_titles(&self, monitor: u32) -> Vec<(u32, String)> {
        self.sessions
            .values()
            .flat_map(|s| {
                s.regions
                    .values()
                    .filter(move |r| r.monitor == monitor)
                    .map(|r| (r.row, r.title.clone()))
            })
            .collect()
    }
    pub fn skip(&self, monitor: u32, y: u32) -> bool {
        self.sessions.values().any(|s| {
            s.regions
                .values()
                .any(|r| r.monitor == monitor && r.skip && (r.row..r.row + r.rows).contains(&y))
        })
    }
    fn send(&mut self, owner: u64, r: Response) {
        if let Some(s) = &mut self.server {
            s.send(owner, &r)
        }
    }
}
fn text(s: &str) -> bool {
    s.len() <= 512 && !s.chars().any(|c| c.is_control())
}
fn colour(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
}
impl Core {
    fn resolve(&self, ms: &[ffi::Monitor], target: &Target) -> Result<(String, u32), String> {
        match target {
            Target::Cell { monitor, x, y } => {
                let m = ms
                    .iter()
                    .find(|m| m.id == *monitor)
                    .ok_or("unknown monitor")?;
                Ok((
                    self.grid(m)
                        .id(Cell { x: *x, y: *y }, *monitor)?
                        .to_string(),
                    *monitor,
                ))
            }
            Target::Special { name }
                if name.starts_with("special:") && name.len() > 8 && text(name) =>
            {
                Ok((name.clone(), u32::MAX))
            }
            Target::Window { address }
                if address.starts_with("0x")
                    && address.len() <= 18
                    && u64::from_str_radix(&address[2..], 16).is_ok() =>
            {
                Ok((address.clone(), u32::MAX))
            }
            _ => Err("invalid target".into()),
        }
    }
    pub fn extension_start(&mut self, path: &str) -> i32 {
        guarded(|| {
            if self.extensions.server.is_some() {
                return Some(-1);
            }
            Some(match Server::start(path.into()) {
                Ok(s) => {
                    let fd = s.fd();
                    self.extensions.server = Some(s);
                    fd
                }
                Err(e) => {
                    eprintln!("hypr2d extension: {e}");
                    -1
                }
            })
        })
        .unwrap_or(-1)
    }
    pub fn extension_stop(&mut self) {
        guarded(|| {
            self.extensions = Extensions::default();
        });
    }
    pub fn extension_drain(&mut self, ms: &[ffi::Monitor]) -> ffi::ExtensionBatch {
        guarded(|| {
            self.extension_grids = ms.iter().map(|m| (m.id, self.grid(m))).collect();
            let incoming = self
                .extensions
                .server
                .as_mut()
                .map(|s| s.drain())
                .unwrap_or_default();
            let mut actions = vec![];
            let mut changed = false;
            for message in incoming {
                match message {
                    Incoming::Closed(owner) => {
                        changed |= self.extensions.sessions.remove(&owner).is_some();
                        self.extensions.pending.retain(|_, v| v.0 != owner);
                    }
                    Incoming::Line(owner, line) => {
                        let request = serde_json::from_slice::<Request>(&line);
                        let id = request.as_ref().map_or(0, |r| r.id);
                        let result = (|| {
                            let r = request.map_err(|e| format!("invalid request: {e}"))?;
                            if r.v != VERSION {
                                return Err("unsupported version; expected v=1".into());
                            }
                            self.extensions.sessions.entry(owner).or_default();
                            match r.operation {
                                Operation::Annotate {
                                    key,
                                    target,
                                    annotation,
                                } => {
                                    if key.is_empty() || !text(&key) {
                                        return Err("invalid key".into());
                                    }
                                    self.resolve(ms, &target)?;
                                    if [
                                        annotation.label.as_deref(),
                                        annotation.badge.as_deref(),
                                        annotation.tooltip.as_deref(),
                                    ]
                                    .into_iter()
                                    .flatten()
                                    .any(|s| !text(s))
                                        || annotation.colour.as_deref().is_some_and(|c| !colour(c))
                                    {
                                        return Err("invalid annotation text/colour".into());
                                    }
                                    let s = self
                                        .extensions
                                        .sessions
                                        .get_mut(&owner)
                                        .ok_or("session missing")?;
                                    if s.annotations.len() >= 256
                                        && !s.annotations.contains_key(&key)
                                    {
                                        return Err("annotation limit (256)".into());
                                    }
                                    s.annotations.insert(key, (target, annotation));
                                    changed = true;
                                }
                                Operation::Reserve {
                                    key,
                                    monitor,
                                    row,
                                    rows,
                                    title,
                                    skip,
                                } => {
                                    if key.is_empty() || !text(&key) || !text(&title) {
                                        return Err("invalid key/title".into());
                                    }
                                    let m = ms
                                        .iter()
                                        .find(|m| m.id == monitor)
                                        .ok_or("unknown monitor")?;
                                    let g = self.grid(m);
                                    if rows == 0
                                        || row.checked_add(rows).is_none_or(|end| end > g.rows)
                                        || skip && rows == g.rows
                                    {
                                        return Err("invalid reserved rows".into());
                                    }
                                    for (id, s) in &self.extensions.sessions {
                                        for (k, r) in &s.regions {
                                            if (*id != owner || *k != key)
                                                && r.monitor == monitor
                                                && row < r.row + r.rows
                                                && r.row < row + rows
                                            {
                                                return Err("reserved rows overlap".into());
                                            }
                                        }
                                    }
                                    // Keep at least one navigable row across all reservations.
                                    if skip
                                        && (0..g.rows).all(|y| {
                                            (row..row + rows).contains(&y)
                                                || self.extensions.sessions.iter().any(|(id, s)| {
                                                    s.regions.iter().any(|(k, r)| {
                                                        (*id != owner || *k != key)
                                                            && r.monitor == monitor
                                                            && r.skip
                                                            && (r.row..r.row + r.rows).contains(&y)
                                                    })
                                                })
                                        })
                                    {
                                        return Err("cannot skip every row".into());
                                    }
                                    let s = self
                                        .extensions
                                        .sessions
                                        .get_mut(&owner)
                                        .ok_or("session missing")?;
                                    if s.regions.len() >= 32 && !s.regions.contains_key(&key) {
                                        return Err("region limit (32)".into());
                                    }
                                    s.regions.insert(
                                        key,
                                        Region {
                                            monitor,
                                            row,
                                            rows,
                                            title,
                                            skip,
                                        },
                                    );
                                    changed = true;
                                }
                                Operation::Clear { key } => {
                                    let s = self
                                        .extensions
                                        .sessions
                                        .get_mut(&owner)
                                        .ok_or("session missing")?;
                                    changed |= s.annotations.remove(&key).is_some()
                                        | s.regions.remove(&key).is_some();
                                }
                                Operation::Subscribe { events } => {
                                    if events.iter().any(|s| s != "activate" && s != "drop") {
                                        return Err("events must be activate/drop".into());
                                    }
                                    self.extensions
                                        .sessions
                                        .get_mut(&owner)
                                        .ok_or("session missing")?
                                        .events = events.into_iter().collect();
                                }
                                Operation::Place { window, target } => {
                                    self.resolve(
                                        ms,
                                        &Target::Window {
                                            address: window.clone(),
                                        },
                                    )?;
                                    if matches!(target, Target::Window { .. }) {
                                        return Err("placement target must be cell/special".into());
                                    }
                                    let (workspace, monitor) = self.resolve(ms, &target)?;
                                    if self.extensions.pending.len() >= 128 {
                                        return Err("placement queue full".into());
                                    }
                                    self.extensions.next += 1;
                                    let token = self.extensions.next;
                                    self.extensions.pending.insert(token, (owner, id));
                                    actions.push(ffi::ExtensionAction {
                                        token,
                                        window,
                                        workspace,
                                        monitor,
                                    });
                                    return Ok(None);
                                }
                                Operation::State => return Ok(Some(self.extensions.snapshot())),
                            }
                            Ok(Some(json!({})))
                        })();
                        match result {
                            Ok(None) => {}
                            other => self.extensions.send(
                                owner,
                                Response::reply(id, other.map(|v| v.unwrap_or(Value::Null))),
                            ),
                        }
                    }
                }
            }
            if self.extensions.server.as_ref().is_some_and(|s| s.dead()) {
                self.extensions = Extensions::default();
                changed = true;
                actions.clear();
            }
            ffi::ExtensionBatch { changed, actions }
        })
    }
    pub fn extension_result(&mut self, token: u64, error: &str) {
        guarded(|| {
            if let Some((owner, id)) = self.extensions.pending.remove(&token) {
                self.extensions.send(
                    owner,
                    Response::reply(
                        id,
                        if error.is_empty() {
                            Ok(json!({}))
                        } else {
                            Err(error.into())
                        },
                    ),
                );
            }
        });
    }
    pub fn extension_event(&mut self, event: &str, workspace: &str, window: &str) {
        guarded(|| {
            let id = workspace.parse::<u32>().ok();
            let mut recipients = vec![];
            for (owner, s) in &self.extensions.sessions {
                if !s.events.contains(event) {
                    continue;
                }
                let owned: Vec<_> = s
                    .regions
                    .iter()
                    .filter(|(_, r)| {
                        id.is_some_and(|id| {
                            // Numbering uses the configured monitor's columns, not a hardcoded 8.
                            let g = self
                                .grids
                                .get(&r.monitor.to_string())
                                .copied()
                                .or_else(|| self.extension_grids.get(&r.monitor).copied())
                                .unwrap_or(crate::grid::Grid {
                                    stride: self.stride,
                                    ..Default::default()
                                });
                            let g = crate::grid::Grid {
                                stride: self.stride,
                                ..g
                            };
                            g.cell(id, r.monitor)
                                .is_some_and(|c| (r.row..r.row + r.rows).contains(&c.y))
                        })
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                if !owned.is_empty() {
                    recipients.push((*owner, owned))
                }
            }
            for (owner, regions) in recipients {
                self.extensions.send(
                    owner,
                    Response::event(
                        event,
                        json!({"workspace":workspace,"window":window,"regions":regions}),
                    ),
                );
            }
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_precedence_and_row_ownership() {
        let mut e = Extensions::default();
        let t = Target::Cell {
            monitor: 0,
            x: 0,
            y: 0,
        };
        for owner in [1, 2] {
            e.sessions.entry(owner).or_default().annotations.insert(
                "job".into(),
                (
                    t.clone(),
                    Annotation {
                        label: Some(owner.to_string()),
                        ..Default::default()
                    },
                ),
            );
        }
        assert_eq!(e.annotation(&t).unwrap().label.as_deref(), Some("2"));
        e.sessions.remove(&2);
        assert_eq!(e.annotation(&t).unwrap().label.as_deref(), Some("1"));
        e.sessions.get_mut(&1).unwrap().regions.insert(
            "tools".into(),
            Region {
                monitor: 0,
                row: 7,
                rows: 1,
                title: "Tools".into(),
                skip: true,
            },
        );
        assert!(e.skip(0, 7));
        assert!(!e.skip(1, 7));
        e.sessions.remove(&1);
        assert!(!e.skip(0, 7));
        assert!(e.annotation(&t).is_none());
    }
}
