use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
pub struct Snapshot {
    pub version: u32,
    #[serde(default)]
    pub parked: serde_json::Value,
    #[serde(default)]
    pub extensions: serde_json::Value,
    #[serde(default)]
    pub overview_config: serde_json::Value,
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
    #[serde(default)]
    pub address: String,
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
        text: "<span size=\"4096\" foreground=\"#4A4A4A\">●</span>".into(),
        tooltip: escape(message),
        alt: "unavailable".into(),
        class: vec!["unavailable".into()],
    }
}
#[derive(Default)]
pub struct Colors {
    pub current: Option<String>,
    pub occupied: Option<String>,
    pub empty: Option<String>,
    pub urgent: Option<String>,
}
pub fn render(
    snapshot: &Snapshot,
    selected: Option<&str>,
    radius_x: u32,
    radius_y: u32,
    colors: &Colors,
) -> Record {
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
    let width = 2 * radius_x + 1;
    let height = 2 * radius_y + 1;
    let mut classes = BTreeSet::new();
    let mut text =
        String::from("<span font_family=\"monospace\" size=\"4096\" line_height=\"0.75\">");
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
                ("●", colors.urgent.as_deref().unwrap_or("#F07178"))
            } else if active {
                (
                    "●",
                    colors
                        .current
                        .as_deref()
                        .or(snapshot.overview_config["current"].as_str())
                        .unwrap_or("#82AAFF"),
                )
            } else if occupied {
                (
                    "●",
                    colors
                        .occupied
                        .as_deref()
                        .or(snapshot.overview_config["label"].as_str())
                        .unwrap_or("#EEFFFF"),
                )
            } else {
                (
                    "●",
                    colors
                        .empty
                        .as_deref()
                        .or(snapshot.overview_config["hover"].as_str())
                        .unwrap_or("#4A4A4A"),
                )
            };
            // State colours stay consistent; extension metadata remains in the
            // tooltip instead of expanding the fixed 9×3 dot geometry.
            text.push_str(&format!("<span foreground=\"{color}\">{glyph}</span>"));
        }
    }
    text.push_str("</span>");
    let mut tooltip = format!(
        "<b>{}</b> · ({}, {}) · workspace {}\n● current · ● occupied · ● empty · ● urgent",
        escape(&m.name),
        current.x,
        current.y,
        m.workspace
    );
    if let Some(parked) = snapshot.parked.as_array() {
        for grid in parked {
            tooltip.push_str(&format!(
                "\nParked: {} · {} windows (SUPER+ALT+Tab)",
                escape(grid["name"].as_str().unwrap_or("monitor")),
                grid["windows"]
            ));
        }
    }
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
    if let Some(items) = snapshot.extensions["annotations"].as_array() {
        for item in items {
            let t = &item["target"];
            let a = &item["annotation"];
            let relevant = match t["kind"].as_str() {
                Some("cell") => t["monitor"].as_u64() == Some(m.id.into()),
                Some("window") => m.cells.iter().any(|c| {
                    c.windows
                        .iter()
                        .any(|w| Some(w.address.as_str()) == t["address"].as_str())
                }),
                Some("special") => true,
                _ => false,
            };
            if relevant {
                classes.insert("annotated");
                let mut bits = vec![];
                for field in ["label", "badge", "tooltip"] {
                    if let Some(s) = a[field].as_str() {
                        bits.push(escape(s));
                    }
                }
                if a["dot"] == true {
                    bits.push("●".into());
                }
                let line = bits.join(" · ");
                if let Some(col) = a["colour"].as_str() {
                    tooltip.push_str(&format!(
                        "\n<span foreground=\"{}\">{line}</span>",
                        escape(col)
                    ));
                } else {
                    tooltip.push_str(&format!("\n{line}"));
                }
            }
        }
    }
    if let Some(items) = snapshot.extensions["regions"].as_array() {
        for item in items {
            let r = &item["region"];
            if r["monitor"].as_u64() == Some(m.id.into()) {
                tooltip.push_str(&format!(
                    "\n{} (rows {} + {})",
                    escape(r["title"].as_str().unwrap_or("")),
                    r["row"],
                    r["rows"]
                ));
            }
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
        "createworkspace",
        "createworkspacev2",
        "destroyworkspace",
        "destroyworkspacev2",
        "moveworkspace",
        "moveworkspacev2",
        "fullscreen",
        "changefloatingmode",
        "configreloaded",
        "activespecial",
        "activespecialv2",
    ]
    .contains(&event)
}
pub(crate) fn emit(out: &mut impl Write, last: &mut String, record: Record) -> io::Result<()> {
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
    radius_x: u32,
    radius_y: u32,
    colors: &Colors,
) -> io::Result<bool> {
    let (record, ready) = match fetch() {
        Ok(state) => {
            let record = render(&state, monitor, radius_x, radius_y, colors);
            let ready = record.alt != "unavailable";
            (record, ready)
        }
        Err(_) => (
            unavailable("Waiting for Hyprland / hypr2d; retrying"),
            false,
        ),
    };
    emit(out, last, record)?;
    Ok(ready)
}
// A blocking socket reader feeds readiness events. Idle means recv(), never a
// timeout or snapshot request. A fixed 16ms deadline coalesces each burst without
// allowing a continuous event stream to postpone a refresh indefinitely.
pub fn stream(
    mut events: impl BufRead + Send + 'static,
    mut fetch: impl FnMut() -> Result<Snapshot, String>,
    mut out: impl Write,
    monitor: Option<&str>,
    radius_x: u32,
    radius_y: u32,
    colors: &Colors,
) -> io::Result<()> {
    let (tx, rx) = mpsc::sync_channel(128);
    std::thread::spawn(move || loop {
        let mut line = String::new();
        match events.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if tx.send(Ok(line)).is_err() {
                    break;
                }
            }
            Err(e) => {
                let _ = tx.send(Err(e));
                break;
            }
        }
    });
    let mut last = String::new();
    let mut backoff = crate::reconnect::Backoff::default();
    let ready = refresh(
        &mut fetch, &mut out, &mut last, monitor, radius_x, radius_y, colors,
    )?;
    let mut loaded = ready;
    let mut retry = if ready {
        None
    } else {
        Some(Instant::now() + backoff.next())
    };
    let mut pending: Option<Instant> = None;
    loop {
        let now = Instant::now();
        if pending.is_some_and(|t| now >= t) || retry.is_some_and(|t| now >= t) {
            let ready = refresh(
                &mut fetch, &mut out, &mut last, monitor, radius_x, radius_y, colors,
            )?;
            pending = None;
            if ready {
                loaded = true;
                retry = None;
                backoff.reset();
            } else {
                loaded = false;
                retry = Some(Instant::now() + backoff.next());
            }
            continue;
        }
        let deadline = pending.into_iter().chain(retry).min();
        let next = if let Some(deadline) = deadline {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(line) => Some(line),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
            }
        } else {
            rx.recv().ok()
        };
        let Some(line) = next else {
            if pending.is_some() && loaded {
                refresh(
                    &mut fetch, &mut out, &mut last, monitor, radius_x, radius_y, colors,
                )?;
            }
            break;
        };
        let line = line?;
        let Some((event, data)) = line.trim_end().split_once(">>") else {
            continue;
        };
        if event == "hypr2d" && data == "unloaded" {
            loaded = false;
            pending = None;
            backoff.reset();
            retry = Some(Instant::now() + backoff.next());
            emit(
                &mut out,
                &mut last,
                unavailable("Waiting for Hyprland / hypr2d; retrying"),
            )?;
        } else {
            if event == "hypr2d" && data == "ready" {
                loaded = true;
            }
            if loaded && relevant(event) && pending.is_none() {
                pending = Some(Instant::now() + Duration::from_millis(16));
            }
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
    fn dedicated_colors_override_independently_and_preserve_fallbacks() {
        let mut state = fixture();
        state.overview_config =
            serde_json::json!({"current":"#123456","label":"#234567","hover":"#345678"});
        let defaults = Colors::default();
        let colors = Colors {
            current: Some("#9CBEFF".into()),
            occupied: Some("#A8A8A8".into()),
            ..Colors::default()
        };
        let default = render(&state, None, 1, 0, &defaults);
        assert!(default.text.contains("#123456"));
        assert!(default.text.contains("#F07178"));
        let custom = render(&state, None, 1, 0, &colors);
        assert!(custom.text.contains("#9CBEFF"));
        assert!(custom.text.contains("#F07178"));
        state.monitors[0].cells[1].windows[0].urgent = false;
        assert!(render(&state, None, 1, 0, &defaults)
            .text
            .contains("#234567"));
        assert!(render(&state, None, 1, 0, &colors).text.contains("#A8A8A8"));
        state.monitors[0].cells[1].windows.clear();
        assert!(render(&state, None, 1, 0, &colors).text.contains("#345678"));
        let colors = Colors {
            empty: Some("#456789".into()),
            urgent: Some("#567890".into()),
            ..colors
        };
        assert!(render(&state, None, 1, 0, &colors).text.contains("#456789"));
        state = fixture();
        assert!(render(&state, None, 1, 0, &colors).text.contains("#567890"));
    }
    #[test]
    fn nine_by_three_uniform_dots_and_all_occupancy_events() {
        let record = render(&fixture(), None, 4, 1, &Colors::default());
        assert_eq!(record.text.matches("foreground=").count(), 27);
        assert_eq!(record.text.matches('●').count(), 27);
        assert_eq!(record.text.matches('\n').count(), 2);
        assert!(record.text.contains("size=\"4096\" line_height=\"0.75\""));
        for event in [
            "openwindow",
            "closewindow",
            "movewindow",
            "movewindowv2",
            "workspace",
            "focusedmon",
            "createworkspace",
            "destroyworkspace",
            "moveworkspace",
            "fullscreen",
            "urgent",
            "hypr2d",
        ] {
            assert!(relevant(event), "missing {event}");
        }
    }
    #[test]
    fn moving_window_refreshes_once_for_a_socket_event_burst() {
        use std::os::unix::net::UnixStream;
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Barrier,
        };
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let moved = Arc::new(AtomicBool::new(false));
        let changed = moved.clone();
        let barrier = Arc::new(Barrier::new(2));
        let ready = barrier.clone();
        let sender = std::thread::spawn(move || {
            ready.wait();
            changed.store(true, Ordering::SeqCst);
            writer
                .write_all(
                    b"movewindowv2>>abc,1,1\nworkspace>>1\ncreateworkspace>>1\nfullscreen>>0\n",
                )
                .unwrap();
            std::thread::sleep(Duration::from_millis(40));
        });
        let mut calls = 0;
        let mut output = Vec::new();
        stream(
            io::BufReader::new(reader),
            || {
                calls += 1;
                let mut state = fixture();
                if moved.load(Ordering::SeqCst) {
                    let mut windows = std::mem::take(&mut state.monitors[0].cells[1].windows);
                    windows[0].urgent = false;
                    state.monitors[0].cells[0].windows = windows;
                }
                if calls == 1 {
                    barrier.wait();
                }
                Ok(state)
            },
            &mut output,
            None,
            4,
            1,
            &Colors::default(),
        )
        .unwrap();
        sender.join().unwrap();
        assert_eq!(calls, 2, "initial snapshot plus one frame-coalesced burst");
        let lines: Vec<_> = std::str::from_utf8(&output).unwrap().lines().collect();
        assert_eq!(lines.len(), 2);
        assert_ne!(lines[0], lines[1]);
        assert!(lines[1].contains("#4A4A4A"));
    }
    #[test]
    fn markup_states_and_small_grid_are_correct() {
        let r = render(&fixture(), None, 1, 1, &Colors::default());
        for class in ["current", "empty", "occupied", "urgent"] {
            assert!(r.class.iter().any(|c| c == class));
        }
        assert!(r.tooltip.contains("&lt;b&gt;unsafe &amp; title&lt;/b&gt;"));
        assert!(!r.tooltip.contains("<b>unsafe"));
        assert_eq!(r.text.matches("foreground=").count(), 9);
        assert!(
            render(&fixture(), Some("missing"), 1, 1, &Colors::default())
                .class
                .contains(&"unavailable".into())
        );
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
            1,
            &Colors::default(),
        )
        .unwrap();
        assert_eq!(calls, 2); // initial snapshot + one coalesced event burst
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
            1,
            &Colors::default(),
        )
        .unwrap();
        assert_eq!(calls, 2);
        let output = String::from_utf8(out).unwrap();
        assert_eq!(output.lines().count(), 3);
        assert!(output.contains("unavailable"));
    }
    #[test]
    fn transient_fetch_failure_recovers_on_a_ready_event() {
        let mut calls = 0;
        let mut out = Vec::new();
        stream(
            io::Cursor::new("hypr2d>>ready\n"),
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
            1,
            &Colors::default(),
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), 2);
    }
    #[test]
    fn non_wrapping_edges_and_outside_grid() {
        let mut s = fixture();
        s.monitors[0].grid.wrap = false;
        let r = render(&s, None, 1, 1, &Colors::default());
        assert_eq!(r.text.matches("foreground=").count(), 2);
        s.monitors[0].cell = None;
        assert!(render(&s, None, 1, 1, &Colors::default())
            .class
            .contains(&"outside-grid".into()));
    }
}
