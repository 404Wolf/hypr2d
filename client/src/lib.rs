//! Synchronous JSON-lines client. Keep the connection alive to retain its state.
use serde::{Deserialize, Serialize};
use std::{
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
};
pub const VERSION: u32 = 1;
pub const MAX_LINE: usize = 65536;
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Cell { monitor: u32, x: u32, y: u32 },
    Special { name: String },
    Window { address: String },
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Annotation {
    pub label: Option<String>,
    pub badge: Option<String>,
    pub dot: bool,
    pub colour: Option<String>,
    pub tooltip: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Annotate {
        key: String,
        target: Target,
        annotation: Annotation,
    },
    Reserve {
        key: String,
        monitor: u32,
        row: u32,
        rows: u32,
        title: String,
        #[serde(default)]
        skip: bool,
    },
    Clear {
        key: String,
    },
    Place {
        window: String,
        target: Target,
    },
    Subscribe {
        events: Vec<String>,
    },
    State,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    pub v: u32,
    pub id: u64,
    #[serde(flatten)]
    pub operation: Operation,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Response {
    pub v: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
}
impl Response {
    pub fn reply(id: u64, result: Result<serde_json::Value, String>) -> Self {
        let (ok, data, error) = match result {
            Ok(v) => (true, v, None),
            Err(e) => (false, serde_json::Value::Null, Some(e)),
        };
        Self {
            v: VERSION,
            id: Some(id),
            ok,
            error,
            event: None,
            data,
        }
    }
    pub fn event(name: &str, data: serde_json::Value) -> Self {
        Self {
            v: VERSION,
            id: None,
            ok: true,
            error: None,
            event: Some(name.into()),
            data,
        }
    }
}
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: u64,
    pending: std::collections::VecDeque<Response>,
}
impl Client {
    pub fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        let writer = UnixStream::connect(path)?;
        Ok(Self {
            reader: BufReader::new(writer.try_clone()?),
            writer,
            next: 1,
            pending: Default::default(),
        })
    }
    /// Connect to an explicitly selected compositor instance.
    pub fn instance(runtime: impl AsRef<Path>, signature: &str) -> io::Result<Self> {
        if signature.is_empty()
            || !signature
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid instance signature",
            ));
        }
        Self::connect(
            runtime
                .as_ref()
                .join("hypr")
                .join(signature)
                .join("hypr2d.sock"),
        )
    }
    fn read(&mut self) -> io::Result<Response> {
        let mut line = Vec::new();
        let n = (&mut self.reader)
            .take((MAX_LINE + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if n > MAX_LINE || line.last() != Some(&b'\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid response frame",
            ));
        }
        serde_json::from_slice(&line).map_err(io::Error::other)
    }
    pub fn request(&mut self, operation: Operation) -> io::Result<Response> {
        let id = self.next;
        self.next += 1;
        serde_json::to_writer(
            &mut self.writer,
            &Request {
                v: VERSION,
                id,
                operation,
            },
        )?;
        self.writer.write_all(b"\n")?;
        loop {
            let reply = self.read()?;
            if reply.id == Some(id) {
                return Ok(reply);
            }
            self.pending.push_back(reply);
            if self.pending.len() > 1024 {
                return Err(io::Error::other("event queue full"));
            }
        }
    }
    /// Blocks without polling. Events arriving during request() are preserved.
    pub fn next_event(&mut self) -> io::Result<Response> {
        if let Some(r) = self.pending.pop_front() {
            Ok(r)
        } else {
            self.read()
        }
    }
}
use std::io::Read;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_wire_round_trip_and_invalid_targets() {
        let raw = r##"{"v":1,"id":4,"op":"annotate","key":"job","target":{"kind":"cell","monitor":0,"x":2,"y":3},"annotation":{"badge":"2","colour":"#82AAFF"}}"##;
        let r: Request = serde_json::from_str(raw).unwrap();
        assert_eq!(r.id, 4);
        let encoded = serde_json::to_string(&r).unwrap();
        assert!(serde_json::from_str::<Request>(&encoded).is_ok());
        assert!(serde_json::from_str::<Request>(&raw.replace("\"x\":2", "\"x\":-1")).is_err());
        assert!(serde_json::from_str::<Request>(
            &raw.replace("\"kind\":\"cell\"", "\"kind\":\"wrong\"")
        )
        .is_err());
    }
    #[test]
    fn events_before_reply_are_preserved() {
        let path = std::env::temp_dir().join(format!("hypr2d-client-{}.sock", std::process::id()));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(s.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let req: Request = serde_json::from_str(&line).unwrap();
            for r in [
                Response::event("drop", serde_json::json!({})),
                Response::reply(req.id, Ok(serde_json::json!({}))),
            ] {
                serde_json::to_writer(&mut s, &r).unwrap();
                s.write_all(b"\n").unwrap();
            }
        });
        let mut c = Client::connect(&path).unwrap();
        assert!(c.request(Operation::State).unwrap().ok);
        assert_eq!(c.next_event().unwrap().event.as_deref(), Some("drop"));
        t.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
