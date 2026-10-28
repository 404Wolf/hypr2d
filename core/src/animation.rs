//! Direction policy for native workspace switches, including activation.
use crate::{ffi, guarded, Core};
impl Core {
    pub fn grid_animation(&self, monitor: &ffi::Monitor, target: u32) -> String {
        guarded(|| {
            let g = self.grid(monitor);
            let (Some(old), Some(new)) = (
                g.cell(monitor.workspace, monitor.slot),
                g.cell(target, monitor.slot),
            ) else {
                return String::new();
            };
            let shortest = |a: u32, b: u32, size: u32| {
                let mut d = b as i32 - a as i32;
                let n = size as i32;
                if g.wrap {
                    if 2 * d > n {
                        d -= n;
                    } else if 2 * d < -n {
                        d += n;
                    }
                }
                d
            };
            let dx = shortest(old.x, new.x, g.columns);
            let dy = shortest(old.y, new.y, g.rows);
            if dx == 0 && dy == 0 {
                return String::new();
            }
            if dy != 0 && dy.abs() >= dx.abs() {
                format!("slidevert {}", if dy > 0 { "bottom" } else { "top" })
            } else {
                format!("slide {}", if dx > 0 { "right" } else { "left" })
            }
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_focus_uses_grid_axes_and_shortest_wrap() {
        let core = Core::default();
        let m = ffi::Monitor {
            id: 0,
            slot: 0,
            name: "eDP-1".into(),
            workspace: 1,
            focused: true,
            offset_x: 0.,
            offset_y: 0.,
            width: 2256.,
            height: 1504.,
        };
        for (target, style) in [
            (81, "slidevert bottom"),
            (561, "slidevert top"),
            (11, "slide right"),
            (71, "slide left"),
            (91, "slidevert bottom"),
        ] {
            assert_eq!(core.grid_animation(&m, target), style);
        }
        assert!(
            core.grid_animation(&m, 2).is_empty(),
            "another grid has native defaults"
        );
        let m = ffi::Monitor {
            workspace: 561,
            ..m
        };
        assert_eq!(core.grid_animation(&m, 1), "slidevert bottom");
    }
}
