//! Nearest physical neighbours; guides describe drag destinations, not moves.
use super::{ffi, Overview, Rect};
use std::collections::BTreeMap;
impl Overview {
    pub(super) fn guides(&self, convert: impl Fn(Rect) -> ffi::UIRect) -> Vec<ffi::UIGuide> {
        if self.expanded.is_some() {
            return vec![];
        }
        let mut result = vec![];
        for (g, a) in self
            .panels
            .iter()
            .enumerate()
            .filter(|(g, _)| !self.parked.contains(g))
        {
            for vertical in [false, true] {
                let next = self
                    .panels
                    .iter()
                    .enumerate()
                    .filter(|(h, b)| {
                        if *h == g || self.parked.contains(h) {
                            return false;
                        }
                        if vertical {
                            b.y + 1e-6 >= a.y + a.h
                                && b.x < a.x + a.w - 1e-6
                                && b.x + b.w > a.x + 1e-6
                        } else {
                            b.x + 1e-6 >= a.x + a.w
                                && b.y < a.y + a.h - 1e-6
                                && b.y + b.h > a.y + 1e-6
                        }
                    })
                    .min_by(|(_, b), (_, c)| {
                        let distance = |p: &Rect| {
                            if vertical {
                                p.y - a.y - a.h
                            } else {
                                p.x - a.x - a.w
                            }
                        };
                        distance(b).total_cmp(&distance(c))
                    });
                let Some((h, _)) = next else {
                    continue;
                };
                let edge = |group, end: bool| {
                    let mut rows = BTreeMap::<u32, &super::CellView>::new();
                    for c in self.cells.iter().filter(|c| c.visible && c.group == group) {
                        let index = if vertical { c.x } else { c.y };
                        let value = |c: &super::CellView| if vertical { c.y } else { c.x };
                        if rows.get(&index).is_none_or(|old| {
                            if end {
                                value(c) > value(old)
                            } else {
                                value(c) < value(old)
                            }
                        }) {
                            rows.insert(index, c);
                        }
                    }
                    rows
                };
                let from = edge(g, true);
                let to = edge(h, false);
                for (index, a) in from {
                    let Some(b) = to.get(&index) else {
                        continue;
                    };
                    let point = |x, y| convert(Rect { x, y, w: 0., h: 0. });
                    let (start, end) = if vertical {
                        (
                            point(a.rect.x + a.rect.w / 2., a.rect.y + a.rect.h),
                            point(b.rect.x + b.rect.w / 2., b.rect.y),
                        )
                    } else {
                        (
                            point(a.rect.x + a.rect.w, a.rect.y + a.rect.h / 2.),
                            point(b.rect.x, b.rect.y + b.rect.h / 2.),
                        )
                    };
                    result.push(ffi::UIGuide {
                        from_monitor: a.monitor,
                        to_monitor: b.monitor,
                        index,
                        vertical,
                        start,
                        end,
                    });
                }
            }
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::super::tests::output;
    use crate::{ffi, theme::Theme, Core};
    #[test]
    fn adjacent_unequal_panels_match_indices_without_skipping_middle() {
        let core = Core {
            theme: Theme {
                fit: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut mid = output(1, 1280., 0.);
        mid.width = 1920.;
        mid.height = 1080.;
        let mut right = output(2, 3200., 0.);
        right.width = 1024.;
        right.height = 768.;
        let bottom = output(3, 0., 1080.);
        let ui = core.overview_new(&[output(0, 0., 0.), mid, right, bottom], &[], &[], 0.);
        let f = ui.overview_frame(2256., 1504., 1000.);
        assert_eq!(f.guides.len(), 24);
        for guide in &f.guides {
            assert!(guide.index < 8);
            assert!(matches!(
                (guide.from_monitor, guide.to_monitor, guide.vertical),
                (0, 1, false) | (1, 2, false) | (0, 3, true)
            ));
            let axis = |c: &ffi::UICell| {
                // Default 8x8 numbering, stride ten and slot=output id.
                let cell = (c.key.parse::<u32>().unwrap() - c.monitor - 1) / 10;
                if guide.vertical {
                    cell % 8
                } else {
                    cell / 8
                }
            };
            let cells: Vec<_> = f
                .cells
                .iter()
                .filter(|c| c.monitor == guide.from_monitor && axis(c) == guide.index)
                .collect();
            if guide.vertical {
                assert!(
                    (guide.start.y
                        - cells
                            .iter()
                            .map(|c| c.rect.y + c.rect.height)
                            .fold(0., f64::max))
                    .abs()
                        < 1e-6
                );
                assert!(guide.end.y > guide.start.y);
            } else {
                assert!(
                    (guide.start.x
                        - cells
                            .iter()
                            .map(|c| c.rect.x + c.rect.width)
                            .fold(0., f64::max))
                    .abs()
                        < 1e-6
                );
                assert!(guide.end.x > guide.start.x);
            }
        }
        let single = core.overview_new(&[output(0, 0., 0.)], &[], &[], 0.);
        assert!(single.overview_frame(2256., 1504., 1000.).guides.is_empty());
    }
}
