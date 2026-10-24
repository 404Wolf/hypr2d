use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::io::{self, BufRead, Write};

#[derive(Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub monitors: Vec<Monitor>,
}
#[derive(Deserialize)]
pub struct Monitor {
    pub id: u32,
    pub name: String,
    pub focused: bool,
    pub workspace: u32,
    pub grid: Grid,
    pub cell: Option<Cell>,
    pub cells: Vec<GridCell>,
}
#[derive(Deserialize)]
pub struct Grid {
    pub columns: u32,
    pub rows: u32,
    pub wrap: bool,
}
#[derive(Clone, Copy, Deserialize)]
pub struct Cell {
    pub x: u32,
    pub y: u32,
}
#[derive(Deserialize)]
pub struct GridCell {
    pub x: u32,
    pub y: u32,
    pub workspace: u32,
    pub windows: Vec<Window>,
}
#[derive(Deserialize)]
pub struct Window {
    pub title: String,
    pub app_id: String,
    pub urgent: bool,
}
#[derive(Debug, Serialize, PartialEq)]
pub struct Record {
    pub text: String,
    pub tooltip: String,
    pub alt: String,
    pub class: Vec<String>,
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn unavailable(message: &str) -> Record {
    Record {
        text: "×".into(),
        tooltip: escape(message),
        alt: "unavailable".into(),
        class: vec!["unavailable".into()],
    }
}
pub fn render(snapshot: &Snapshot, selected: Option<&str>, radius: u32) -> Record {
    if snapshot.version != 1 {
        return unavailable("Unsupported hypr2d state version");
    }
    let monitor = match selected {
        Some(name) => snapshot
            .monitors
            .iter()
            .find(|m| m.name == name || m.id.to_string() == name),
        None => snapshot
            .monitors
            .iter()
            .find(|m| m.focused)
            .or_else(|| snapshot.monitors.first()),
    };
    let Some(m) = monitor else {
        return unavailable("Selected monitor is not connected");
    };
    let Some(current) = m.cell else {
        return Record {
            text: "?".into(),
            tooltip: format!(
                "{}: workspace {} is outside its grid; use hypr2d goto",
                escape(&m.name),
                m.workspace
            ),
            alt: "outside-grid".into(),
            class: vec!["outside-grid".into()],
        };
    };
    if m.grid.columns == 0 || m.grid.rows == 0 {
        return unavailable("Invalid grid dimensions");
    }
    let cells: HashMap<_, _> = m.cells.iter().map(|c| ((c.x, c.y), c)).collect();
    let width = (2 * radius + 1).min(m.grid.columns);
    let height = (2 * radius + 1).min(m.grid.rows);
    let mut classes = BTreeSet::new();
    let mut text = String::from("<span font_family=\"monospace\">");
    for row in 0..height {
        if row > 0 {
            text.push('\n');
        }
        for col in 0..width {
            if col > 0 {
                text.push(' ');
            }
            let x = i64::from(current.x) + i64::from(col) - i64::from(width / 2);
            let y = i64::from(current.y) + i64::from(row) - i64::from(height / 2);
            if !m.grid.wrap
                && (x < 0 || y < 0 || x >= m.grid.columns.into() || y >= m.grid.rows.into())
            {
                text.push(' ');
                continue;
            }
            let xy = (
                x.rem_euclid(m.grid.columns.into()) as u32,
                y.rem_euclid(m.grid.rows.into()) as u32,
            );
            let Some(c) = cells.get(&xy) else {
                text.push(' ');
                continue;
            };
            let active = xy == (current.x, current.y);
            let urgent = c.windows.iter().any(|w| w.urgent);
            let occupied = !c.windows.is_empty();
            classes.insert(if occupied { "occupied" } else { "empty" });
            if active {
                classes.insert("current");
            }
            if urgent {
                classes.insert("urgent");
            }
            let (glyph, color) = if urgent {
                ("!", "#e06c75")
            } else if active {
                ("●", "#61afef")
            } else if occupied {
                ("◉", "#98c379")
            } else {
                ("·", "#4b5263")
            };
            text.push_str(&format!("<span foreground=\"{color}\">{glyph}</span>"));
        }
    }
    text.push_str("</span>");
    let mut tooltip = format!(
        "<b>{}</b> · ({}, {}) · workspace {}\n● current · ◉ occupied · · empty · ! urgent",
        escape(&m.name),
        current.x,
        current.y,
        m.workspace
    );
    for c in &m.cells {
        if c.windows.is_empty() {
            continue;
        }
        tooltip.push_str(&format!(
            "\n\n<b>({}, {}) · workspace {}</b>",
            c.x, c.y, c.workspace
        ));
        for w in &c.windows {
            tooltip.push_str(&format!(
                "\n{}{} — {}",
                if w.urgent { "! " } else { "" },
                escape(&w.title),
                escape(&w.app_id)
            ));
        }
    }
    Record {
        text,
        tooltip,
        alt: format!("{},{}", current.x, current.y),
        class: classes.into_iter().map(str::to_owned).collect(),
    }
}
fn relevant(event: &str) -> bool {
    [
        "hypr2d",
        "workspace",
        "workspacev2",
        "focusedmon",
        "focusedmonv2",
        "monitoradded",
        "monitoraddedv2",
        "monitorremoved",
        "monitorremovedv2",
        "openwindow",
        "closewindow",
        "movewindow",
        "movewindowv2",
        "windowtitle",
        "windowtitlev2",
        "activewindow",
        "activewindowv2",
        "urgent",
        "activespecial",
        "activespecialv2",
    ]
    .contains(&event)
}
fn emit(out: &mut impl Write, last: &mut String, record: Record) -> io::Result<()> {
    let line = serde_json::to_string(&record)?;
    if line != *last {
        writeln!(out, "{line}")?;
        out.flush()?;
        *last = line;
    }
    Ok(())
}
fn refresh(
    fetch: &mut impl FnMut() -> Result<Snapshot, String>,
    out: &mut impl Write,
    last: &mut String,
    monitor: Option<&str>,
    radius: u32,
) -> io::Result<()> {
    let record = match fetch() {
        Ok(state) => render(&state, monitor, radius),
        Err(_) => unavailable("hypr2d unavailable; waiting for compositor events"),
    };
    emit(out, last, record)
}
// The only loop blocks on socket2. Fetch once after subscription, then only on
// relevant events. Identical rendered records are not emitted twice.
pub fn stream(
    mut events: impl BufRead,
    mut fetch: impl FnMut() -> Result<Snapshot, String>,
    mut out: impl Write,
    monitor: Option<&str>,
    radius: u32,
) -> io::Result<()> {
    let mut last = String::new();
    let mut loaded = true;
    refresh(&mut fetch, &mut out, &mut last, monitor, radius)?;
    let mut line = String::new();
    loop {
        line.clear();
        if events.read_line(&mut line)? == 0 {
            break;
        }
        let Some((event, data)) = line.trim_end().split_once(">>") else {
            continue;
        };
        if event == "hypr2d" && data == "unloaded" {
            loaded = false;
            emit(
                &mut out,
                &mut last,
                unavailable("hypr2d plugin unavailable; waiting for load"),
            )?;
            continue;
        }
        if event == "hypr2d" && data == "ready" {
            loaded = true;
        }
        if loaded && relevant(event) {
            refresh(&mut fetch, &mut out, &mut last, monitor, radius)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Snapshot {
        serde_json::from_str(r#"{"version":1,"monitors":[{"id":0,"name":"DP<&","focused":true,"workspace":1,"grid":{"columns":2,"rows":1,"wrap":true},"cell":{"x":0,"y":0},"cells":[{"x":0,"y":0,"workspace":1,"windows":[]},{"x":1,"y":0,"workspace":11,"windows":[{"title":"<b>unsafe & title</b>","app_id":"app<","urgent":true}]}]}]}"#).unwrap()
    }
    #[test]
    fn markup_states_and_small_grid_are_correct() {
        let r = render(&fixture(), None, 1);
        for class in ["current", "empty", "occupied", "urgent"] {
            assert!(r.class.iter().any(|c| c == class));
        }
        assert!(r.tooltip.contains("&lt;b&gt;unsafe &amp; title&lt;/b&gt;"));
        assert!(!r.tooltip.contains("<b>unsafe"));
        assert_eq!(r.text.matches("foreground=").count(), 2);
        assert!(render(&fixture(), Some("missing"), 1)
            .class
            .contains(&"unavailable".into()));
    }
    #[test]
    fn stream_is_event_driven_and_deduplicated() {
        let mut calls = 0;
        let mut out = Vec::new();
        stream(
            io::Cursor::new("unknown>>anything\nworkspace>>11\nwindowtitle>>abc\n"),
            || {
                calls += 1;
                Ok(fixture())
            },
            &mut out,
            None,
            1,
        )
        .unwrap();
        assert_eq!(calls, 3); // initial snapshot + two relevant events
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), 1);
    }
    #[test]
    fn unload_waits_for_ready_without_querying() {
        let mut calls = 0;
        let mut out = Vec::new();
        stream(
            io::Cursor::new("hypr2d>>unloaded\nworkspace>>11\nwindowtitle>>abc\nhypr2d>>ready\n"),
            || {
                calls += 1;
                Ok(fixture())
            },
            &mut out,
            None,
            1,
        )
        .unwrap();
        assert_eq!(calls, 2);
        let output = String::from_utf8(out).unwrap();
        assert_eq!(output.lines().count(), 3);
        assert!(output.contains("unavailable"));
    }
    #[test]
    fn transient_fetch_failure_recovers_on_an_event() {
        let mut calls = 0;
        let mut out = Vec::new();
        stream(
            io::Cursor::new("workspace>>11\n"),
            || {
                calls += 1;
                if calls == 1 {
                    Err("temporary IPC error".into())
                } else {
                    Ok(fixture())
                }
            },
            &mut out,
            None,
            1,
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), 2);
    }
    #[test]
    fn non_wrapping_edges_and_outside_grid() {
        let mut s = fixture();
        s.monitors[0].grid.wrap = false;
        let r = render(&s, None, 1);
        assert_eq!(r.text.matches("foreground=").count(), 1);
        s.monitors[0].cell = None;
        assert!(render(&s, None, 1).class.contains(&"outside-grid".into()));
    }
}
