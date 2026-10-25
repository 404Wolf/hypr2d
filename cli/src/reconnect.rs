//! Retry only unavailable connections; healthy sessions block on socket events.
use crate::{waybar, Args};
use std::env;
use std::io::{self, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};
#[derive(Default)]
pub struct Backoff(u32);
impl Backoff {
    pub fn next(&mut self) -> Duration {
        let delay = Duration::from_millis((100u64 << self.0.min(5)).min(2000));
        self.0 = self.0.saturating_add(1);
        delay
    }
    pub fn reset(&mut self) {
        self.0 = 0;
    }
}
fn valid(signature: &str) -> bool {
    !signature.is_empty()
        && signature
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}
fn candidates(explicit: Option<&str>) -> Vec<PathBuf> {
    let Some(runtime) = env::var_os("XDG_RUNTIME_DIR") else {
        return vec![];
    };
    let root = PathBuf::from(runtime).join("hypr");
    if let Some(sig) = explicit {
        return if valid(sig) {
            vec![root.join(sig)]
        } else {
            vec![]
        };
    }
    let mut paths = Vec::new();
    if let Ok(sig) = env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        if valid(&sig) {
            paths.push(root.join(sig));
        }
    }
    let mut discovered: Vec<_> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|d| {
            valid(&d.file_name().to_string_lossy()) && d.path().join(".socket2.sock").exists()
        })
        .collect();
    discovered.sort_by_key(|d| std::cmp::Reverse(d.metadata().and_then(|m| m.modified()).ok()));
    for d in discovered {
        if !paths.contains(&d.path()) {
            paths.push(d.path());
        }
    }
    paths
}
pub fn run(args: &Args, mut out: impl Write) -> io::Result<()> {
    let mut backoff = Backoff::default();
    let mut last = String::new();
    waybar::emit(
        &mut out,
        &mut last,
        waybar::unavailable("Waiting for Hyprland / hypr2d; reconnecting"),
    )?;
    loop {
        for path in candidates(args.instance.as_deref()) {
            let Ok(socket) = UnixStream::connect(path.join(".socket2.sock")) else {
                continue;
            };
            let Ok(reader) = socket.try_clone() else {
                continue;
            };
            let started = Instant::now();
            // Subscription precedes snapshot, including every reconnect.
            let result = waybar::stream(
                BufReader::new(reader),
                || {
                    serde_json::from_str(&crate::request(&path, "state -j")?)
                        .map_err(|e| e.to_string())
                },
                &mut out,
                args.monitor.as_deref(),
                args.radius_x.or(args.radius).unwrap_or(4),
                args.radius_y.or(args.radius).unwrap_or(1),
                &args.colors,
            );
            last.clear(); // stream wrote records using its own deduplication state.
                          // Wake and retire a blocked reader on output failure or disconnect.
            let _ = socket.shutdown(Shutdown::Both);
            if let Err(e) = result {
                if e.kind() == io::ErrorKind::BrokenPipe {
                    return Err(e);
                }
            }
            if started.elapsed() >= Duration::from_secs(2) {
                backoff.reset();
            }
            break;
        }
        waybar::emit(
            &mut out,
            &mut last,
            waybar::unavailable("Waiting for Hyprland / hypr2d; reconnecting"),
        )?;
        std::thread::sleep(backoff.next());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_is_bounded_and_resettable() {
        let mut b = Backoff::default();
        assert_eq!(
            (0..8).map(|_| b.next().as_millis()).collect::<Vec<_>>(),
            [100, 200, 400, 800, 1600, 2000, 2000, 2000]
        );
        b.reset();
        assert_eq!(b.next(), Duration::from_millis(100));
        assert!(!valid("../live"));
        assert!(valid("rev_123-456"));
    }
}
