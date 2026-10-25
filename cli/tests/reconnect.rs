//! Real CLI process + disposable Unix sockets: never contact the live runtime.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
struct Server {
    dir: PathBuf,
    ready: Arc<AtomicBool>,
    cell: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    events: UnixListener,
}
impl Server {
    fn new(root: &Path, name: &str, ready: bool) -> Self {
        let dir = root.join("hypr").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let ipc = UnixListener::bind(dir.join(".socket.sock")).unwrap();
        ipc.set_nonblocking(true).unwrap();
        let events = UnixListener::bind(dir.join(".socket2.sock")).unwrap();
        events.set_nonblocking(true).unwrap();
        let ready = Arc::new(AtomicBool::new(ready));
        let cell = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (r, c, n, s) = (ready.clone(), cell.clone(), calls.clone(), stop.clone());
        let label = name.to_string();
        let handle = thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match ipc.accept() {
                    Ok((mut sock, _)) => {
                        sock.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                        let mut request = String::new();
                        sock.read_to_string(&mut request).unwrap();
                        assert_eq!(request, "j/hypr2d state -j");
                        n.fetch_add(1, Ordering::SeqCst);
                        let response = if r.load(Ordering::SeqCst) {
                            json!({"version":1,"monitors":[{"id":0,"name":label,"focused":true,"workspace":1,"grid":{"columns":2,"rows":1,"wrap":true},"cell":{"x":c.load(Ordering::SeqCst),"y":0},"cells":[{"x":0,"y":0,"workspace":1,"windows":[]},{"x":1,"y":0,"workspace":11,"windows":[]}]}]}).to_string()
                        } else {
                            "unknown request".into()
                        };
                        let _ = sock.write_all(response.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("fake IPC: {e}"),
                }
            }
        });
        Self {
            dir,
            ready,
            cell,
            calls,
            stop,
            handle: Some(handle),
            events,
        }
    }
    fn accept(&self) -> UnixStream {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match self.events.accept() {
                Ok((s, _)) => return s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "socket2 resubscription"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        }
    }
    fn replace_socket2(&mut self) {
        std::fs::remove_file(self.dir.join(".socket2.sock")).unwrap();
        self.events = UnixListener::bind(self.dir.join(".socket2.sock")).unwrap();
        self.events.set_nonblocking(true).unwrap();
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.take().unwrap().join().unwrap();
        for name in [".socket.sock", ".socket2.sock"] {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
    }
}
struct Process(Child, PathBuf);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}
fn record(rx: &mpsc::Receiver<Value>, test: impl Fn(&Value) -> bool) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    loop {
        let v = rx
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .expect("minimap line");
        if test(&v) {
            return v;
        }
    }
}
#[test]
fn startup_unready_reload_restart_and_socket_replacement_keep_one_process_alive() {
    let root = std::env::temp_dir().join(format!(
        "hypr2d-reconnect-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_hypr2d"))
        .arg("waybar")
        .env("XDG_RUNTIME_DIR", &root)
        .env("HYPRLAND_INSTANCE_SIGNATURE", "stale_instance")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut process = Process(child, root.clone());
    let stdout = process.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if tx.send(serde_json::from_str(&line).unwrap()).is_err() {
                break;
            }
        }
    });
    record(&rx, |v| v["alt"] == "unavailable");
    assert!(process.0.try_wait().unwrap().is_none());
    let a = Server::new(&root, "instance_a", false);
    let mut events = a.accept();
    // No ready event: snapshot retry alone must recover the late-loaded plugin.
    thread::sleep(Duration::from_millis(150));
    a.ready.store(true, Ordering::SeqCst);
    record(&rx, |v| {
        v["alt"] == "0,0" && v["tooltip"].as_str().unwrap().contains("instance_a")
    });
    let count = a.calls.load(Ordering::SeqCst);
    thread::sleep(Duration::from_millis(350));
    assert_eq!(
        a.calls.load(Ordering::SeqCst),
        count,
        "healthy snapshots must never poll"
    );
    a.cell.store(1, Ordering::SeqCst);
    events.write_all(b"movewindowv2>>a,11,11\n").unwrap();
    record(&rx, |v| v["alt"] == "1,0");
    a.ready.store(false, Ordering::SeqCst);
    events.write_all(b"hypr2d>>unloaded\n").unwrap();
    record(&rx, |v| v["alt"] == "unavailable");
    thread::sleep(Duration::from_millis(150));
    a.ready.store(true, Ordering::SeqCst);
    record(&rx, |v| v["alt"] == "1,0");
    let _ = events.shutdown(std::net::Shutdown::Both);
    drop(a);
    record(&rx, |v| v["alt"] == "unavailable");
    let mut b = Server::new(&root, "instance_b", true);
    let mut new_events = b.accept();
    record(&rx, |v| {
        v["alt"] == "0,0" && v["tooltip"].as_str().unwrap().contains("instance_b")
    });
    // Replace socket2 at the same instance path, then prove fresh events work.
    b.replace_socket2();
    let _ = new_events.shutdown(std::net::Shutdown::Both);
    new_events = b.accept();
    b.cell.store(1, Ordering::SeqCst);
    new_events.write_all(b"workspace>>11\n").unwrap();
    record(&rx, |v| {
        v["alt"] == "1,0" && v["tooltip"].as_str().unwrap().contains("instance_b")
    });
    assert!(process.0.try_wait().unwrap().is_none());
    drop(new_events);
    drop(events);
    drop(b);
    drop(process);
    reader.join().unwrap();
}
