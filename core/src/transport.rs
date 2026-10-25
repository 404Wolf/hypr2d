//! Event-driven bounded transport. Only owned bytes cross the worker boundary.
use hypr2d_client::MAX_LINE;
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read, Write},
    os::fd::{AsRawFd, RawFd},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};
pub enum Incoming {
    Line(u64, Vec<u8>),
    Closed(u64),
}
enum Outgoing {
    Send(u64, Vec<u8>),
    Stop,
}
pub struct Server {
    pub incoming: Receiver<Incoming>,
    notify: UnixStream,
    wake: UnixStream,
    out: Sender<Outgoing>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
}
struct Peer {
    stream: UnixStream,
    rx: Vec<u8>,
    tx: VecDeque<u8>,
}
impl Server {
    pub fn start(path: PathBuf) -> io::Result<Self> {
        // Never unlink a pre-existing socket: a second load must fail safely.
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let (notify, mut main_notify) = UnixStream::pair()?;
        notify.set_nonblocking(true)?;
        main_notify.set_nonblocking(true)?;
        let (wake, worker_wake) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        worker_wake.set_nonblocking(true)?;
        let (tx, incoming) = mpsc::sync_channel(256);
        let (out, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("hypr2d-socket".into())
            .spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker(listener, worker_wake, rx, |message| {
                        if tx.try_send(message).is_ok() {
                            let _ = main_notify.write(&[1]);
                            true
                        } else {
                            false
                        }
                    })
                }));
            })?;
        Ok(Self {
            incoming,
            notify,
            wake,
            out,
            thread: Some(handle),
            path,
        })
    }
    pub fn dead(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| t.is_finished())
    }
    pub fn fd(&self) -> RawFd {
        self.notify.as_raw_fd()
    }
    pub fn drain(&mut self) -> Vec<Incoming> {
        let mut b = [0; 1024];
        while self.notify.read(&mut b).is_ok_and(|n| n > 0) {}
        self.incoming.try_iter().collect()
    }
    pub fn send(&mut self, id: u64, response: &hypr2d_client::Response) {
        if let Ok(mut bytes) = serde_json::to_vec(response) {
            if bytes.len() >= MAX_LINE {
                bytes = serde_json::to_vec(&hypr2d_client::Response::reply(
                    response.id.unwrap_or(0),
                    Err("response exceeds 64 KiB; reduce annotation state".into()),
                ))
                .unwrap_or_default();
            }
            bytes.push(b'\n');
            let _ = self.out.send(Outgoing::Send(id, bytes));
            let _ = self.wake.write(&[1]);
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.out.send(Outgoing::Stop);
        let _ = self.wake.write(&[1]);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}
fn worker(
    listener: UnixListener,
    mut wake: UnixStream,
    out: Receiver<Outgoing>,
    mut incoming: impl FnMut(Incoming) -> bool,
) {
    let mut peers: BTreeMap<u64, Peer> = BTreeMap::new();
    let mut next = 1;
    loop {
        let ids: Vec<_> = peers.keys().copied().collect();
        let mut fds = vec![
            libc::pollfd {
                fd: listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        for id in &ids {
            let p = &peers[id];
            fds.push(libc::pollfd {
                fd: p.stream.as_raw_fd(),
                events: libc::POLLIN | if p.tx.is_empty() { 0 } else { libc::POLLOUT },
                revents: 0,
            });
        }
        // The fd vector remains alive and unchanged across poll; no pointers escape.
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) } < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            } else {
                break;
            }
        }
        if fds[1].revents != 0 {
            let mut b = [0; 1024];
            while wake.read(&mut b).is_ok_and(|n| n > 0) {}
        }
        for message in out.try_iter() {
            match message {
                Outgoing::Stop => return,
                Outgoing::Send(id, b) => {
                    if let Some(p) = peers.get_mut(&id) {
                        if p.tx.len() + b.len() > 4 * MAX_LINE {
                            p.rx.resize(MAX_LINE + 1, 0)
                        } else {
                            p.tx.extend(b)
                        }
                    }
                }
            }
        }
        if fds[0].revents != 0 {
            while let Ok((stream, _)) = listener.accept() {
                if peers.len() >= 64 {
                    continue;
                }
                if stream.set_nonblocking(true).is_err() {
                    continue;
                }
                peers.insert(
                    next,
                    Peer {
                        stream,
                        rx: vec![],
                        tx: VecDeque::new(),
                    },
                );
                next += 1;
            }
        }
        for (index, id) in ids.iter().enumerate() {
            let Some(p) = peers.get_mut(id) else { continue };
            let events = fds[index + 2].revents;
            let mut closed = p.rx.len() > MAX_LINE;
            if events & libc::POLLIN != 0 {
                let mut b = [0; 4096];
                loop {
                    match p.stream.read(&mut b) {
                        Ok(0) => {
                            closed = true;
                            break;
                        }
                        Ok(n) => {
                            p.rx.extend_from_slice(&b[..n]);
                            while let Some(end) = p.rx.iter().position(|b| *b == b'\n') {
                                if end >= MAX_LINE {
                                    closed = true;
                                    break;
                                }
                                let line = p.rx.drain(..=end).collect();
                                if !incoming(Incoming::Line(*id, line)) {
                                    return;
                                }
                            }
                            if closed || p.rx.len() > MAX_LINE {
                                closed = true;
                                break;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            closed = true;
                            break;
                        }
                    }
                }
            }
            if events & libc::POLLOUT != 0 && !p.tx.is_empty() {
                let (bytes, _) = p.tx.as_slices();
                match p.stream.write(bytes) {
                    Ok(n) => {
                        p.tx.drain(..n);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => closed = true,
                }
            }
            if events & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                closed = true
            }
            if closed {
                peers.remove(id);
                if !incoming(Incoming::Closed(*id)) {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_frames_disconnect_and_shutdown_cleanup() {
        let path = std::env::temp_dir().join(format!("hypr2d-server-{}.sock", std::process::id()));
        let mut server = Server::start(path.clone()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut c = UnixStream::connect(&path).unwrap();
        c.write_all(b"{\"v\":1}").unwrap();
        c.write_all(b"\n").unwrap();
        let mut fd = libc::pollfd {
            fd: server.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut fd, 1, 2000) }, 1);
        let messages = server.drain();
        assert!(matches!(&messages[0],Incoming::Line(1,b) if b==b"{\"v\":1}\n"));
        drop(c);
        assert_eq!(unsafe { libc::poll(&mut fd, 1, 2000) }, 1);
        assert!(matches!(server.drain()[0], Incoming::Closed(1)));
        let mut c = UnixStream::connect(&path).unwrap();
        c.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        c.write_all(b"last\n").unwrap();
        assert_eq!(unsafe { libc::poll(&mut fd, 1, 2000) }, 1);
        assert!(matches!(server.drain()[0], Incoming::Line(2, _)));
        drop(server);
        assert!(!path.exists());
        assert_eq!(c.read(&mut [0; 1]).unwrap(), 0);
    }
}
