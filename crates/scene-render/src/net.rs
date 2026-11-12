//! The wire between `scene-render encode --workers` and `scene-render serve`: authenticated JSON-RPC over TCP.
//!
//! A message is one line: 64 hexadecimal digits, a space and a JSON-RPC 2.0 object. The digits are the
//! HMAC-SHA256, under the link's key, of the sender's side (`C` for the coordinator, `W` for the worker), the
//! count of messages that side has sent before this one (eight bytes, big-endian) and the object's bytes: a message
//! that was altered, repeated, dropped, reordered or sent back to its sender does not verify, and the link ends.
//! A message whose object has a `payload` member (`{"bytes": N, "sha256": "…"}`) is followed by N raw bytes, a file;
//! the digest is in the authenticated object, and the receiver checks it as it writes the file.
//!
//! The key starts as HMAC-SHA256(token, the protocol's name). The worker opens with a `challenge` carrying 32 random
//! bytes; the coordinator answers with `hello` and 32 of its own, both under that first key; from then on both use
//! HMAC-SHA256(first key, worker's bytes ‖ coordinator's bytes). A coordinator without the token cannot answer the
//! challenge, a worker without it cannot answer the hello, and a recorded session does not verify against new
//! random bytes. Nothing is encrypted: the link keeps strangers from giving a worker work and from changing what
//! passes, and does not hide it. On a network that others can read, run it through a tunnel.
//!
//! Everything read is untrusted until its line verifies, and bounded before it is read: a line holds at most
//! [`MAX_LINE`] bytes and a payload at most [`MAX_PAYLOAD`].

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sr_deliver::destinations::hmac;

/// The protocol's name and revision: both sides must speak the same.
pub const PROTOCOL: &str = "scene-render-worker/1";
/// The longest line read, mark and newline included.
pub const MAX_LINE: usize = 1 << 20;
/// The largest file sent after a message.
pub const MAX_PAYLOAD: u64 = 64 << 30;
/// The environment variable holding the shared token.
pub const TOKEN_ENV: &str = "SR_WORKER_TOKEN";
/// The environment variable holding the seconds a side waits for the other's next message (default 600).
pub const TIMEOUT_ENV: &str = "SR_WORKER_TIMEOUT";

/// Which end of the link this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// `encode --workers`.
    Coordinator,
    /// `serve`.
    Worker,
}

impl Side {
    fn mark(self) -> u8 {
        match self {
            Side::Coordinator => b'C',
            Side::Worker => b'W',
        }
    }
    fn other(self) -> Side {
        match self {
            Side::Coordinator => Side::Worker,
            Side::Worker => Side::Coordinator,
        }
    }
}

/// A file announced by a message, still to be read from the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// Its length.
    pub bytes: u64,
    /// Its SHA-256, hexadecimal.
    pub sha256: String,
}

/// Why a link ended or a message was refused.
#[derive(Debug)]
pub enum LinkError {
    /// The connection failed, closed or timed out.
    Io(std::io::Error),
    /// What arrived is not this protocol, does not verify, or is past a bound.
    Protocol(String),
    /// The other side answered a request with a JSON-RPC error.
    Refused(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Io(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                write!(f, "no message within the time allowed ({TIMEOUT_ENV})")
            }
            LinkError::Io(e) => write!(f, "{e}"),
            LinkError::Protocol(m) | LinkError::Refused(m) => f.write_str(m),
        }
    }
}

impl From<std::io::Error> for LinkError {
    fn from(e: std::io::Error) -> Self {
        LinkError::Io(e)
    }
}

fn protocol<T>(message: impl Into<String>) -> Result<T, LinkError> {
    Err(LinkError::Protocol(message.into()))
}

/// Lowercase hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The bytes of 64 hexadecimal digits.
pub fn unhex32(text: &str) -> Option<[u8; 32]> {
    let t = text.as_bytes();
    if t.len() != 64 {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0u8; 32];
    for (i, pair) in t.chunks(2).enumerate() {
        out[i] = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

/// True when `text` is a SHA-256 as this protocol writes it: 64 lowercase hexadecimal digits.
pub fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Compares two marks without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// 32 bytes nobody else can guess: the system's random source where there is one, else the process's own hashing
/// keys (which the standard library seeds from the system) over the time and the process.
pub fn random32() -> [u8; 32] {
    let mut out = [0u8; 32];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut out).is_ok() {
            return out;
        }
    }
    use std::hash::{BuildHasher, Hasher};
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut h = Sha256::new();
    for round in 0u64..8 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(now);
        hasher.write_u64(round);
        hasher.write_u32(std::process::id());
        h.update(hasher.finish().to_le_bytes());
    }
    out.copy_from_slice(&h.finalize());
    out
}

/// The shared token: the content of `file` when given, else of the environment variable [`TOKEN_ENV`]. Surrounding
/// white space is dropped; an empty token is none.
pub fn token(file: Option<&Path>) -> Result<String, String> {
    let text = match file {
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| format!("cannot read the token file {}: {e}", p.display()))?
        }
        None => std::env::var(TOKEN_ENV).unwrap_or_default(),
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(format!(
            "no token: both sides need the same secret, in the environment variable {TOKEN_ENV} or in a file named by --token-file"
        ));
    }
    Ok(text)
}

/// The seconds a side waits for the other's next message.
pub fn timeout() -> Duration {
    let seconds = std::env::var(TIMEOUT_ENV).ok().and_then(|s| s.parse::<u64>().ok()).filter(|&s| s > 0);
    Duration::from_secs(seconds.unwrap_or(600))
}

/// One end of an authenticated connection.
pub struct Link {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    side: Side,
    key: [u8; 32],
    sent: u64,
    received: u64,
}

impl Link {
    /// The link over `stream` for `side`, under the first key of `token`. [`Link::accept`] or [`Link::greet`]
    /// must follow before anything else is sent.
    pub fn new(stream: TcpStream, side: Side, token: &str) -> std::io::Result<Link> {
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(timeout()))?;
        stream.set_write_timeout(Some(timeout()))?;
        let key = hmac(token.as_bytes(), PROTOCOL.as_bytes());
        Ok(Link { reader: BufReader::new(stream.try_clone()?), writer: stream, side, key, sent: 0, received: 0 })
    }

    fn mark(&self, side: Side, count: u64, body: &[u8]) -> [u8; 32] {
        let mut message = Vec::with_capacity(9 + body.len());
        message.push(side.mark());
        message.extend_from_slice(&count.to_be_bytes());
        message.extend_from_slice(body);
        hmac(&self.key, &message)
    }

    /// Sends `message`, followed by the file `payload` when one is given (its length and digest are added to the
    /// message as `payload`).
    pub fn send(&mut self, mut message: Value, payload: Option<&Path>) -> Result<(), LinkError> {
        let mut file = None;
        if let Some(path) = payload {
            let mut f = std::fs::File::open(path)?;
            let (bytes, digest) = digest_of(&mut f)?;
            if bytes > MAX_PAYLOAD {
                return protocol(format!("{} is larger than a link carries ({MAX_PAYLOAD} bytes)", path.display()));
            }
            message["payload"] = json!({ "bytes": bytes, "sha256": digest });
            file = Some((std::fs::File::open(path)?, bytes));
        }
        let body = message.to_string();
        if body.len() + 66 > MAX_LINE {
            return protocol("a message is longer than a link carries");
        }
        let line = format!("{} {body}\n", hex(&self.mark(self.side, self.sent, body.as_bytes())));
        self.sent += 1;
        self.writer.write_all(line.as_bytes())?;
        if let Some((mut f, bytes)) = file {
            let copied = std::io::copy(&mut (&mut f).take(bytes), &mut self.writer)?;
            if copied != bytes {
                return protocol("a file changed while it was sent");
            }
        }
        self.writer.flush()?;
        Ok(())
    }

    /// The next message, verified. One that announces a file must be followed by [`Link::take`] or
    /// [`Link::skip`] before the next is read.
    pub fn recv(&mut self) -> Result<(Value, Option<Payload>), LinkError> {
        let mut line = Vec::new();
        let read = (&mut self.reader).take(MAX_LINE as u64 + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Err(LinkError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the other side closed")));
        }
        if line.last() != Some(&b'\n') {
            return protocol("a line is longer than a link carries, or was cut short");
        }
        line.pop();
        if line.len() < 66 || line[64] != b' ' {
            return protocol("what arrived is not a scene-render worker message");
        }
        let (mark, body) = (&line[..64], &line[65..]);
        let expected = hex(&self.mark(self.side.other(), self.received, body));
        if !same(mark, expected.as_bytes()) {
            return protocol(
                "a message does not verify: the two sides do not hold the same token, or the link was tampered with",
            );
        }
        self.received += 1;
        let message: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => return protocol(format!("a message is not JSON: {e}")),
        };
        let payload = match message.get("payload") {
            None | Some(Value::Null) => None,
            Some(p) => {
                let bytes = p.get("bytes").and_then(Value::as_u64);
                let sha256 = p.get("sha256").and_then(Value::as_str).filter(|s| is_digest(s));
                match (bytes, sha256) {
                    (Some(bytes), Some(sha256)) if bytes <= MAX_PAYLOAD => {
                        Some(Payload { bytes, sha256: sha256.to_string() })
                    }
                    _ => return protocol("a message announces a file without a usable length and digest"),
                }
            }
        };
        Ok((message, payload))
    }

    /// Writes the announced file to `to` and checks its digest; a file that does not match is removed.
    pub fn take(&mut self, payload: &Payload, to: &Path) -> Result<(), LinkError> {
        let mut out = std::fs::File::create(to)?;
        let mut h = Sha256::new();
        let mut left = payload.bytes;
        let mut buffer = vec![0u8; 1 << 16];
        while left > 0 {
            let want = left.min(buffer.len() as u64) as usize;
            let got = self.reader.read(&mut buffer[..want])?;
            if got == 0 {
                drop(out);
                let _ = std::fs::remove_file(to);
                return Err(LinkError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "a file was cut short",
                )));
            }
            h.update(&buffer[..got]);
            out.write_all(&buffer[..got])?;
            left -= got as u64;
        }
        out.flush()?;
        drop(out);
        if hex(&h.finalize()) != payload.sha256 {
            let _ = std::fs::remove_file(to);
            return protocol("a file does not match its digest");
        }
        Ok(())
    }

    /// Reads the announced file and drops it.
    pub fn skip(&mut self, payload: &Payload) -> Result<(), LinkError> {
        let copied = std::io::copy(&mut (&mut self.reader).take(payload.bytes), &mut std::io::sink())?;
        if copied != payload.bytes {
            return Err(LinkError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "a file was cut short")));
        }
        Ok(())
    }

    fn rekey(&mut self, worker: &[u8; 32], coordinator: &[u8; 32]) {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(worker);
        both[32..].copy_from_slice(coordinator);
        self.key = hmac(&self.key, &both);
    }

    /// The worker's opening: sends the challenge, reads the coordinator's `hello`, and returns its parameters and
    /// request id. The answer to it, and everything after, goes under the session key.
    pub fn accept(&mut self) -> Result<(Value, Value), LinkError> {
        let ours = random32();
        self.send(
            json!({ "jsonrpc": "2.0", "method": "challenge", "params": { "protocol": PROTOCOL, "nonce": hex(&ours) } }),
            None,
        )?;
        let (hello, payload) = self.recv()?;
        if payload.is_some() || hello.get("method").and_then(Value::as_str) != Some("hello") {
            return protocol("the coordinator did not open with hello");
        }
        let params = hello.get("params").cloned().unwrap_or(Value::Null);
        let echoed = params.get("challenge").and_then(Value::as_str).and_then(unhex32);
        let theirs = params.get("nonce").and_then(Value::as_str).and_then(unhex32);
        let (Some(echoed), Some(theirs)) = (echoed, theirs) else {
            return protocol("the coordinator's hello carries no challenge and nonce");
        };
        if !same(&echoed, &ours) {
            return protocol("the coordinator answered another challenge");
        }
        self.rekey(&ours, &theirs);
        Ok((params, hello.get("id").cloned().unwrap_or(Value::Null)))
    }

    /// The coordinator's opening: reads the challenge, sends `hello` with `about` (its version and the like) and
    /// returns the worker's answer. An answer that verifies proves the worker holds the token.
    pub fn greet(&mut self, about: Value) -> Result<Value, LinkError> {
        let (challenge, payload) = self.recv()?;
        if payload.is_some() || challenge.get("method").and_then(Value::as_str) != Some("challenge") {
            return protocol("the worker did not open with a challenge");
        }
        let params = challenge.get("params").cloned().unwrap_or(Value::Null);
        if params.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
            return protocol(format!(
                "the worker speaks {}, and this engine {PROTOCOL}",
                params.get("protocol").and_then(Value::as_str).unwrap_or("another protocol")
            ));
        }
        let Some(theirs) = params.get("nonce").and_then(Value::as_str).and_then(unhex32) else {
            return protocol("the worker's challenge carries no nonce");
        };
        let ours = random32();
        let mut hello = about;
        hello["protocol"] = json!(PROTOCOL);
        hello["challenge"] = json!(hex(&theirs));
        hello["nonce"] = json!(hex(&ours));
        self.send(json!({ "jsonrpc": "2.0", "id": 0, "method": "hello", "params": hello }), None)?;
        self.rekey(&theirs, &ours);
        let (answer, payload) = self.recv()?;
        if payload.is_some() {
            return protocol("the worker's hello carries a file");
        }
        result_of(answer)
    }

    /// Sends a request and returns the answer to it, with the file it announces still to be read. `notified` is
    /// given each notification that arrives first and may end the wait by returning an error.
    pub fn call(
        &mut self,
        method: &str,
        params: Value,
        payload: Option<&Path>,
        notified: &mut dyn FnMut(&str, &Value) -> Result<(), String>,
    ) -> Result<(Value, Option<Payload>), LinkError> {
        let id = self.sent;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }), payload)?;
        loop {
            let (message, file) = self.recv()?;
            if let Some(name) = message.get("method").and_then(Value::as_str) {
                if let Some(file) = &file {
                    self.skip(file)?;
                }
                notified(name, message.get("params").unwrap_or(&Value::Null)).map_err(LinkError::Refused)?;
                continue;
            }
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                return protocol("the worker answered another request");
            }
            if message.get("error").is_some() {
                if let Some(file) = &file {
                    self.skip(file)?;
                }
            }
            return Ok((result_of(message)?, file));
        }
    }

    /// Closes both directions: the other side's next read or write fails.
    pub fn close(&self) {
        let _ = self.writer.shutdown(std::net::Shutdown::Both);
    }

    /// The other end's address.
    pub fn peer(&self) -> String {
        self.writer.peer_addr().map_or_else(|_| "an unknown address".into(), |a| a.to_string())
    }
}

/// The result of a JSON-RPC answer, or its error as a refusal.
fn result_of(mut answer: Value) -> Result<Value, LinkError> {
    if let Some(error) = answer.get("error") {
        let message = error.get("message").and_then(Value::as_str).unwrap_or("the request failed");
        return Err(LinkError::Refused(message.to_string()));
    }
    match answer.get_mut("result") {
        Some(result) => Ok(result.take()),
        None => protocol("an answer has neither a result nor an error"),
    }
}

/// The length and SHA-256 (hexadecimal) of what `reader` holds.
pub fn digest_of(reader: &mut impl Read) -> std::io::Result<(u64, String)> {
    let mut h = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut bytes = 0u64;
    loop {
        let got = reader.read(&mut buffer)?;
        if got == 0 {
            break;
        }
        h.update(&buffer[..got]);
        bytes += got as u64;
    }
    Ok((bytes, hex(&h.finalize())))
}

/// A JSON-RPC answer carrying `result`.
pub fn answer(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC answer refusing a request.
pub fn refusal(id: &Value, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": message.into() } })
}

/// A notification.
pub fn notice(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn pair(token_a: &str, token_b: &str) -> (Link, Link) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
        let (server, _) = listener.accept().unwrap();
        (Link::new(server, Side::Worker, token_a).unwrap(), Link::new(client, Side::Coordinator, token_b).unwrap())
    }

    /// What each side got from the opening, and the two links.
    type Opened = (Result<(Value, Value), LinkError>, Result<Value, LinkError>, Link, Link);

    fn opened(token_a: &str, token_b: &str) -> Opened {
        let (mut worker, mut coordinator) = pair(token_a, token_b);
        let serving = std::thread::spawn(move || {
            let hello = worker.accept();
            if let Ok((_, id)) = &hello {
                worker.send(answer(id, json!({ "version": "w" })), None).unwrap();
            }
            (hello, worker)
        });
        let greeted = coordinator.greet(json!({ "version": "c" }));
        if greeted.is_err() {
            coordinator.close();
        }
        let (hello, worker) = serving.join().unwrap();
        (hello, greeted, worker, coordinator)
    }

    #[test]
    fn two_sides_with_one_token_open_a_link_and_pass_messages_and_files() {
        let (hello, greeted, mut worker, mut coordinator) = opened("secret", "secret");
        assert_eq!(hello.unwrap().0["version"], "c");
        assert_eq!(greeted.unwrap()["version"], "w");
        let dir = std::env::temp_dir().join(format!("sr-net-{}-{}", std::process::id(), hex(&random32()[..4])));
        std::fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("from"), dir.join("to"));
        std::fs::write(&from, vec![7u8; 200_000]).unwrap();
        let sending = std::thread::spawn(move || {
            let (request, file) = worker.recv().unwrap();
            assert_eq!(request["method"], "put");
            worker.take(&file.unwrap(), &to).unwrap();
            worker.send(notice("progress", json!({ "done": 1 })), None).unwrap();
            worker.send(answer(&request["id"], json!({ "ok": true })), None).unwrap();
            to
        });
        let mut seen = Vec::new();
        let (result, file) = coordinator
            .call("put", json!({}), Some(&from), &mut |name, params| {
                seen.push((name.to_string(), params.clone()));
                Ok(())
            })
            .unwrap();
        let to = sending.join().unwrap();
        assert_eq!((result["ok"].as_bool(), file), (Some(true), None));
        assert_eq!(seen, vec![("progress".to_string(), json!({ "done": 1 }))]);
        assert_eq!(std::fs::read(&to).unwrap(), std::fs::read(&from).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_side_with_another_token_is_refused_by_both() {
        let (hello, greeted, ..) = opened("secret", "another");
        // the coordinator cannot verify the challenge, and the worker never sees a hello that verifies
        assert!(matches!(greeted, Err(LinkError::Protocol(m)) if m.contains("does not verify")));
        assert!(hello.is_err());
    }

    #[test]
    fn a_message_that_is_changed_or_repeated_does_not_verify() {
        let (_, _, mut worker, coordinator) = opened("secret", "secret");
        let key_holder = Link { sent: 5, ..coordinator };
        let mut raw = key_holder.writer.try_clone().unwrap();
        let body = br#"{"jsonrpc":"2.0","id":9,"method":"open","params":{}}"#;
        // a line marked for another place in the sequence: a repeat of an earlier message looks like this
        let stale = hex(&key_holder.mark(Side::Coordinator, 0, body));
        raw.write_all(format!("{stale} {}\n", String::from_utf8_lossy(body)).as_bytes()).unwrap();
        assert!(matches!(worker.recv(), Err(LinkError::Protocol(m)) if m.contains("does not verify")));
        // and one whose content is not what was marked
        let right = hex(&key_holder.mark(Side::Coordinator, 1, body));
        raw.write_all(format!("{right} {}\n", String::from_utf8_lossy(body).replace("open", "oops")).as_bytes())
            .unwrap();
        assert!(matches!(worker.recv(), Err(LinkError::Protocol(m)) if m.contains("does not verify")));
    }

    #[test]
    fn digests_and_hex_read_back() {
        let d = hex(&Sha256::digest(b"abc"));
        assert!(is_digest(&d) && !is_digest(&d.to_uppercase()) && !is_digest("abc"));
        assert_eq!(hex(&unhex32(&d).unwrap()), d);
        assert_ne!(random32(), random32());
    }
}
