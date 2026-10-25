mod reconnect;
mod waybar;
use std::env;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const USAGE:&str="hypr2d [--instance SIGNATURE] state\nhypr2d [--instance SIGNATURE] move DIR [all] [--monitor NAME]\nhypr2d [--instance SIGNATURE] carry DIR\nhypr2d [--instance SIGNATURE] goto X Y [MONITOR]\nhypr2d [--instance SIGNATURE] waybar [--monitor NAME] [--radius-x 0..4] [--radius-y 0..4] [--current-color #RRGGBB] [--occupied-color #RRGGBB] [--empty-color #RRGGBB] [--urgent-color #RRGGBB]";
#[derive(Default)]
struct Args {
    instance: Option<String>,
    monitor: Option<String>,
    radius: Option<u32>,
    radius_x: Option<u32>,
    radius_y: Option<u32>,
    colors: waybar::Colors,
    words: Vec<String>,
}
impl Args {
    fn parse(input: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = Self::default();
        let mut it = input.into_iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--instance" => args.instance = Some(it.next().ok_or("missing instance")?),
                "--monitor" => args.monitor = Some(it.next().ok_or("missing monitor")?),
                "--current-color" | "--occupied-color" | "--empty-color" | "--urgent-color" => {
                    let value = it.next().ok_or("missing color")?;
                    if value.len() != 7
                        || !value.starts_with('#')
                        || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err("color must be #RRGGBB".into());
                    }
                    match arg.as_str() {
                        "--current-color" => args.colors.current = Some(value),
                        "--occupied-color" => args.colors.occupied = Some(value),
                        "--empty-color" => args.colors.empty = Some(value),
                        _ => args.colors.urgent = Some(value),
                    }
                }
                "--radius" | "--radius-x" | "--radius-y" => {
                    let n = it
                        .next()
                        .ok_or("missing radius")?
                        .parse::<u32>()
                        .map_err(|_| "invalid radius")?;
                    if n > 4 {
                        return Err("radius must be 0..4".into());
                    }
                    match arg.as_str() {
                        "--radius-x" => args.radius_x = Some(n),
                        "--radius-y" => args.radius_y = Some(n),
                        _ => args.radius = Some(n),
                    }
                }
                _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
                _ => args.words.push(arg),
            }
        }
        Ok(args)
    }
    fn command(&self) -> Result<String, String> {
        if self.radius.is_some() || self.radius_x.is_some() || self.radius_y.is_some() {
            return Err("--radius is only valid for waybar".into());
        }
        if self.colors.current.is_some()
            || self.colors.occupied.is_some()
            || self.colors.empty.is_some()
            || self.colors.urgent.is_some()
        {
            return Err("color options are only valid for waybar".into());
        }
        let words: Vec<_> = self.words.iter().map(String::as_str).collect();
        let result = match words.as_slice() {
            ["state"] if self.monitor.is_none() => "state -j".into(),
            ["move", d] => {
                if let Some(m) = &self.monitor {
                    format!("move {d} monitor {m}")
                } else {
                    format!("move {d}")
                }
            }
            ["move", d, "all"] if self.monitor.is_none() => format!("move {d} all"),
            ["carry", d] if self.monitor.is_none() => format!("carry {d}"),
            ["goto", x, y] => format!(
                "goto {x} {y}{}",
                self.monitor
                    .as_ref()
                    .map_or(String::new(), |m| format!(" {m}"))
            ),
            ["goto", x, y, m] if self.monitor.is_none() => format!("goto {x} {y} {m}"),
            _ => return Err(USAGE.into()),
        };
        Ok(result)
    }
}
fn instance_path(signature: Option<String>) -> Result<PathBuf, String> {
    let signature = signature
        .or_else(|| env::var("HYPRLAND_INSTANCE_SIGNATURE").ok())
        .ok_or("set HYPRLAND_INSTANCE_SIGNATURE or pass --instance SIGNATURE")?;
    if signature.is_empty()
        || !signature
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("invalid instance signature".into());
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime).join("hypr").join(signature))
}
fn request(path: &std::path::Path, command: &str) -> Result<String, String> {
    let mut socket = UnixStream::connect(path.join(".socket.sock"))
        .map_err(|e| format!("connect to selected Hyprland: {e}"))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    socket
        .write_all(format!("j/hypr2d {command}").as_bytes())
        .map_err(|e| e.to_string())?;
    socket
        .shutdown(Shutdown::Write)
        .map_err(|e| e.to_string())?;
    let mut response = String::new();
    socket
        .read_to_string(&mut response)
        .map_err(|e| e.to_string())?;
    if response.starts_with("error:")
        || response.trim().is_empty()
        || response.starts_with("unknown request")
    {
        return Err(response);
    }
    Ok(response)
}
fn run() -> Result<(), String> {
    let raw: Vec<_> = env::args().skip(1).collect();
    if raw.is_empty() || raw.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let args = Args::parse(raw)?;
    if args.words.as_slice() == ["waybar"] {
        reconnect::run(&args, io::stdout().lock()).map_err(|e| e.to_string())?;
    } else {
        let path = instance_path(args.instance.clone())?;
        let response = request(&path, &args.command()?)?;
        if response.trim() != "ok" && !response.trim_start().starts_with('{') {
            return Err(response);
        }
        println!("{}", response.trim_end());
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("hypr2d: {error}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn diagonal_commands_are_forwarded_without_rewriting() {
        for words in [vec!["move", "up-left", "all"], vec!["carry", "down-right"]] {
            let args = super::Args::parse(words.iter().map(|s| s.to_string())).unwrap();
            assert_eq!(args.command().unwrap(), words.join(" "));
        }
    }

    use super::*;
    #[test]
    fn minimap_color_arguments_are_validated_and_scoped() {
        for flag in [
            "--current-color",
            "--occupied-color",
            "--empty-color",
            "--urgent-color",
        ] {
            assert!(Args::parse(["waybar", flag, "#9CbEFF"].map(str::to_owned)).is_ok());
            for bad in ["red", "#abcd", "#GG0000", "#000000\""] {
                assert!(Args::parse(["waybar", flag, bad].map(str::to_owned)).is_err());
            }
            assert!(Args::parse(["waybar", flag].map(str::to_owned)).is_err());
            assert!(Args::parse(["state", flag, "#A8A8A8"].map(str::to_owned))
                .unwrap()
                .command()
                .is_err());
        }
    }
    #[test]
    fn explicit_monitor_moves_are_atomic_commands() {
        let args = Args::parse(["move", "right", "--monitor", "DP-2"].map(str::to_owned)).unwrap();
        assert_eq!(args.command().unwrap(), "move right monitor DP-2");
        assert!(Args::parse(["waybar", "--radius", "9"].map(str::to_owned)).is_err());
        assert!(
            Args::parse(["move", "up", "all", "--monitor", "DP-2"].map(str::to_owned))
                .unwrap()
                .command()
                .is_err()
        );
    }
}
