//! Rendering spike only: fixed thumbnail budget and a normalized contact sheet.
//! This is deliberately not the M4 interaction/layout implementation.
use crate::{ffi, guarded, Core};
use serde_json::json;

impl Core {
    pub fn probe_plan(
        &self,
        monitors: &[ffi::Monitor],
        specials: &[ffi::ProbeSpecial],
        full: bool,
    ) -> Vec<ffi::ProbeCell> {
        guarded(|| {
            let mut cells = Vec::new();
            for (panel, m) in monitors.iter().enumerate() {
                let grid = self.grid(m);
                let count = if full { grid.columns * grid.rows } else { 4 };
                let panel_width = 0.96 / monitors.len().max(1) as f64;
                let width = panel_width / grid.columns as f64;
                let height = width * m.height / m.width;
                let mut push = |workspace, special: String, x, y| {
                    cells.push(ffi::ProbeCell {
                        monitor: m.id,
                        workspace,
                        special,
                        x: 0.02 + panel as f64 * panel_width + x as f64 * width,
                        y: 0.08 + y as f64 * height,
                        width: width * 0.94,
                        height: height * 0.94,
                        capture_width: 160,
                        capture_height: (160.0 * m.height / m.width).round().max(1.0) as u32,
                    });
                };
                for i in 0..count.min(grid.columns * grid.rows) {
                    let cell = crate::grid::Cell {
                        x: i % grid.columns,
                        y: i / grid.columns,
                    };
                    if let Ok(id) = grid.id(cell, m.slot) {
                        push(id, String::new(), cell.x, cell.y);
                    }
                }
                for (i, special) in specials.iter().filter(|s| s.monitor == m.id).enumerate() {
                    push(0, special.name.clone(), i as u32, grid.rows + 1);
                }
            }
            cells
        })
    }
}

pub fn probe_report(
    cells: &[ffi::ProbeCell],
    times: &[f64],
    active: bool,
    deleted: bool,
    restored: u32,
    renderer: &str,
) -> String {
    guarded(|| {
        let mut sorted = times.to_vec();
        sorted.sort_by(f64::total_cmp);
        json!({
            "active": active, "deleted_gl_objects": deleted, "restored_captures": restored,
            "renderer": renderer, "cells": cells.iter().zip(times).map(|(c,t)| json!({
                "monitor": c.monitor, "workspace": c.workspace, "special": c.special,
                "width": c.capture_width, "height": c.capture_height,
                "rgba8_bytes": u64::from(c.capture_width)*u64::from(c.capture_height)*4,
                "finish_ms": t,
            })).collect::<Vec<_>>(),
            "rgba8_bytes": cells.iter().map(|c| u64::from(c.capture_width)*u64::from(c.capture_height)*4).sum::<u64>(),
            "total_ms": times.iter().sum::<f64>(),
            "mean_ms": times.iter().sum::<f64>() / times.len().max(1) as f64,
            "median_ms": sorted.get(sorted.len()/2).copied().unwrap_or(0.0),
            "max_ms": sorted.last().copied().unwrap_or(0.0),
        }).to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_monitor_budget_and_special_strip() {
        let monitors = [0, 1].map(|id| ffi::Monitor {
            id,
            slot: id,
            name: id.to_string(),
            workspace: id + 1,
            focused: id == 0,
            offset_x: 0.0,
            offset_y: 0.0,
            width: 1280.0,
            height: 720.0,
        });
        let specials = [ffi::ProbeSpecial {
            monitor: 1,
            name: "special:test".into(),
        }];
        let plan = Core::default().probe_plan(&monitors, &specials, true);
        assert_eq!(plan.len(), 129);
        assert_eq!(plan[63].workspace, 631);
        assert_eq!(plan[127].workspace, 632);
        assert_eq!(plan[128].special, "special:test");
        assert!(plan[128].y > plan[127].y);
        assert!(plan
            .iter()
            .all(|c| c.capture_width == 160 && c.capture_height == 90 && c.x + c.width <= 1.0));
        let report: serde_json::Value = serde_json::from_str(&probe_report(
            &plan,
            &vec![1.0; 129],
            true,
            false,
            129,
            "test",
        ))
        .unwrap();
        assert_eq!(report["rgba8_bytes"], 7_430_400);
        assert_eq!(report["mean_ms"], 1.0);
    }
}
