//! WebSocket messages in the proxy log. After a `101 Switching Protocols`
//! the proxy copies bytes both ways as before; with the log on it also reads
//! the frames as they pass and keeps the messages, which the entry holds as
//! `_webSocketMessages` (the field Chrome DevTools writes and imports).
//!
//! The reader only looks: every byte goes on unchanged and in order. It
//! unmasks client frames, joins fragments, and inflates `permessage-deflate`
//! messages, keeping the inflate context between messages unless the
//! extension says `*_no_context_takeover`.

use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use flate2::{Decompress, FlushDecompress};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{HarLog, HarRecord};
use crate::scripts::bodies::{Budget, Reservation};
use crate::scripts::lua_api::base64_encode;

/// Data kept of one message.
pub const MESSAGE_LIMIT: usize = 64 << 10;
/// Messages kept of one connection; later ones are counted only.
pub const MESSAGES_LIMIT: usize = 5000;

/// One message, as Chrome writes it in a HAR file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WsMessage {
    /// `send` (from the client) or `receive`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Seconds since 1970, with milliseconds.
    pub time: f64,
    /// 1 text, 2 binary, 8 close, 9 ping, 10 pong.
    pub opcode: u8,
    /// Text as it is; binary as base64.
    pub data: String,
    /// Bytes of the whole message after inflating.
    #[serde(rename = "_size")]
    pub size: u64,
    #[serde(rename = "_truncated", skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// A message read from the stream, before it gets its direction and time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub opcode: u8,
    pub data: Vec<u8>,
    pub size: u64,
    pub truncated: bool,
}

/// `permessage-deflate` for one direction.
struct Inflate {
    d: Decompress,
    /// Reset after each message.
    no_takeover: bool,
    broken: bool,
}

enum State {
    /// Bytes of a frame header so far.
    Header(Vec<u8>),
    Payload { left: u64, mask: Option<[u8; 4]>, at: usize, control: bool, fin: bool },
}

/// The message being read: data frames until one has FIN.
struct Message {
    opcode: u8,
    compressed: bool,
    data: Vec<u8>,
    size: u64,
    truncated: bool,
}

/// Reads frames from one direction of a WebSocket connection.
pub struct Parser {
    state: State,
    message: Option<Message>,
    control: Option<(u8, Vec<u8>)>,
    inflate: Option<Inflate>,
    /// The stream did not look like WebSocket frames: stop reading it.
    lost: bool,
}

impl Parser {
    /// `deflate`: the extension was agreed; `no_takeover` for this direction.
    pub fn new(deflate: bool, no_takeover: bool) -> Self {
        Self {
            state: State::Header(Vec::with_capacity(14)),
            message: None,
            control: None,
            inflate: deflate.then(|| Inflate { d: Decompress::new(false), no_takeover, broken: false }),
            lost: false,
        }
    }

    /// Read `data`; returns the messages it completed.
    pub fn feed(&mut self, mut data: &[u8]) -> Vec<Read> {
        let mut out = vec![];
        while !data.is_empty() && !self.lost {
            match &mut self.state {
                State::Header(buf) => {
                    buf.push(data[0]);
                    data = &data[1..];
                    if let Some(next) = self.header() {
                        self.state = next;
                        // A frame with no payload ends here.
                        if let State::Payload { left: 0, .. } = self.state {
                            self.end_frame(&mut out);
                        }
                    }
                }
                State::Payload { left, mask, at, .. } => {
                    let n = (*left).min(data.len() as u64) as usize;
                    let mut chunk = data[..n].to_vec();
                    if let Some(m) = mask {
                        for b in chunk.iter_mut() {
                            *b ^= m[*at % 4];
                            *at += 1;
                        }
                    }
                    *left -= n as u64;
                    data = &data[n..];
                    let done = *left == 0;
                    self.payload(&chunk);
                    if done {
                        self.end_frame(&mut out);
                    }
                }
            }
        }
        out
    }

    /// The header in the buffer, once it is whole.
    fn header(&mut self) -> Option<State> {
        let State::Header(buf) = &self.state else { return None };
        if buf.len() < 2 {
            return None;
        }
        let (b0, b1) = (buf[0], buf[1]);
        let len7 = (b1 & 0x7f) as usize;
        let masked = b1 & 0x80 != 0;
        let ext = match len7 {
            126 => 2,
            127 => 8,
            _ => 0,
        };
        let need = 2 + ext + if masked { 4 } else { 0 };
        if buf.len() < need {
            return None;
        }
        let len = match ext {
            2 => u16::from_be_bytes([buf[2], buf[3]]) as u64,
            8 => u64::from_be_bytes(buf[2..10].try_into().expect("8 bytes")),
            _ => len7 as u64,
        };
        let mask = masked.then(|| buf[2 + ext..need].try_into().expect("4 bytes"));
        let fin = b0 & 0x80 != 0;
        let rsv1 = b0 & 0x40 != 0;
        let opcode = b0 & 0x0f;
        let control = opcode >= 8;
        if control {
            if len > 125 || !fin {
                self.lost = true;
                return None;
            }
            self.control = Some((opcode, Vec::with_capacity(len as usize)));
        } else if opcode == 0 {
            if self.message.is_none() {
                self.lost = true;
                return None;
            }
        } else if opcode == 1 || opcode == 2 {
            let compressed = rsv1 && self.inflate.is_some();
            self.message = Some(Message { opcode, compressed, data: vec![], size: 0, truncated: false });
        } else {
            self.lost = true;
            return None;
        }
        Some(State::Payload { left: len, mask, at: 0, control, fin })
    }

    fn payload(&mut self, chunk: &[u8]) {
        let State::Payload { control, .. } = self.state else { return };
        if control {
            if let Some((_, data)) = self.control.as_mut() {
                data.extend_from_slice(chunk);
            }
            return;
        }
        let Some(m) = self.message.as_mut() else { return };
        if m.compressed {
            match self.inflate.as_mut() {
                Some(inf) if !inf.broken => {
                    if inflate(&mut inf.d, chunk, m).is_err() {
                        inf.broken = true;
                    }
                }
                _ => {}
            }
        } else {
            keep(m, chunk);
        }
    }

    fn end_frame(&mut self, out: &mut Vec<Read>) {
        let State::Payload { control, fin, .. } = self.state else { return };
        self.state = State::Header(Vec::with_capacity(14));
        if control {
            if let Some((opcode, data)) = self.control.take() {
                let size = data.len() as u64;
                out.push(Read { opcode, data, size, truncated: false });
            }
            return;
        }
        if !fin {
            return;
        }
        let Some(mut m) = self.message.take() else { return };
        if m.compressed
            && let Some(inf) = self.inflate.as_mut()
        {
            if !inf.broken && inflate(&mut inf.d, &[0, 0, 0xff, 0xff], &mut m).is_err() {
                inf.broken = true;
            }
            if inf.broken {
                m.data = b"(could not inflate this message)".to_vec();
                m.opcode = 1;
            } else if inf.no_takeover {
                inf.d.reset(false);
            }
        }
        out.push(Read { opcode: m.opcode, data: m.data, size: m.size, truncated: m.truncated });
    }
}

fn keep(m: &mut Message, bytes: &[u8]) {
    m.size += bytes.len() as u64;
    let room = MESSAGE_LIMIT.saturating_sub(m.data.len());
    m.data.extend_from_slice(&bytes[..room.min(bytes.len())]);
    if bytes.len() > room {
        m.truncated = true;
    }
}

/// Inflate `input` into the message, all of it: the context must see every
/// byte even when the message keeps only the first ones.
fn inflate(d: &mut Decompress, mut input: &[u8], m: &mut Message) -> Result<(), flate2::DecompressError> {
    let mut out = [0u8; 16 * 1024];
    loop {
        let (before_in, before_out) = (d.total_in(), d.total_out());
        d.decompress(input, &mut out, FlushDecompress::Sync)?;
        let used = (d.total_in() - before_in) as usize;
        let made = (d.total_out() - before_out) as usize;
        keep(m, &out[..made]);
        input = &input[used..];
        if (input.is_empty() && made < out.len()) || (used == 0 && made == 0) {
            return Ok(());
        }
    }
}

/// `permessage-deflate` from the server's `Sec-WebSocket-Extensions`: agreed,
/// and the no-context-takeover flags for client and server messages.
pub fn deflate_of(extensions: Option<&str>) -> (bool, bool, bool) {
    let Some(ext) = extensions else { return (false, false, false) };
    for offer in ext.split(',') {
        let mut parts = offer.split(';').map(|p| p.trim().to_ascii_lowercase());
        if parts.next().as_deref() != Some("permessage-deflate") {
            continue;
        }
        let params: Vec<String> = parts.collect();
        let has = |name: &str| params.iter().any(|p| p == name || p.starts_with(&format!("{name}=")));
        return (true, has("client_no_context_takeover"), has("server_no_context_takeover"));
    }
    (false, false, false)
}

#[derive(Default)]
struct TapState {
    messages: Vec<WsMessage>,
    dropped: u64,
    held: Option<Reservation>,
    record: Option<(Arc<HarLog>, HarRecord)>,
    started: bool,
    closed: bool,
}

/// The messages of one connection, shared by the copy loops and the code
/// that writes the entry. The entry is written when both sides closed and
/// the record is attached, whichever comes last.
#[derive(Clone)]
pub struct WsTap {
    state: Arc<Mutex<TapState>>,
    budget: Arc<Budget>,
    opened: Arc<Mutex<Option<Instant>>>,
}

impl WsTap {
    pub fn new(budget: Arc<Budget>) -> Self {
        Self { state: Arc::default(), budget, opened: Arc::default() }
    }

    /// The upgrade went through and the copy loops use this tap.
    pub fn mark_started(&self) {
        self.state.lock().unwrap().started = true;
        *self.opened.lock().unwrap() = Some(Instant::now());
    }

    pub fn started(&self) -> bool {
        self.state.lock().unwrap().started
    }

    /// Give the tap the record of the `101` answer; it writes it at close.
    pub fn attach(&self, log: Arc<HarLog>, record: HarRecord) {
        log.send_open(&record);
        let mut s = self.state.lock().unwrap();
        if s.closed {
            drop(s);
            self.write(log, record);
        } else {
            s.record = Some((log, record));
        }
    }

    fn push(&self, kind: &'static str, read: Read) {
        let mut s = self.state.lock().unwrap();
        if s.messages.len() >= MESSAGES_LIMIT {
            s.dropped += 1;
            return;
        }
        let n = read.data.len();
        let reserved = match s.held.as_mut() {
            Some(r) => r.grow(n),
            None => self.budget.try_reserve(n).map(|r| s.held = Some(r)).is_some(),
        };
        if !reserved {
            s.dropped += 1;
            return;
        }
        let text = read.opcode == 1 || read.opcode == 8;
        let data = if read.opcode == 8 && read.data.len() >= 2 {
            let code = u16::from_be_bytes([read.data[0], read.data[1]]);
            format!("{code} {}", String::from_utf8_lossy(&read.data[2..])).trim_end().to_string()
        } else if text || read.opcode >= 9 {
            String::from_utf8_lossy(&read.data).into_owned()
        } else {
            base64_encode(&read.data)
        };
        let message = WsMessage { kind, time: now_seconds(), opcode: read.opcode, data, size: read.size, truncated: read.truncated };
        if let Some((log, record)) = &s.record {
            log.send_message(record.id, &message);
        }
        s.messages.push(message);
    }

    fn close(&self) {
        let mut s = self.state.lock().unwrap();
        s.closed = true;
        if let Some((log, record)) = s.record.take() {
            drop(s);
            self.write(log, record);
        }
    }

    fn write(&self, log: Arc<HarLog>, mut record: HarRecord) {
        let mut s = self.state.lock().unwrap();
        record.ws_messages = Some(std::mem::take(&mut s.messages));
        record.ws_dropped = s.dropped;
        record.held.extend(s.held.take().map(Arc::new));
        drop(s);
        if let Some(at) = *self.opened.lock().unwrap() {
            record.receive_ms = at.elapsed().as_millis() as u64;
        }
        log.record(record);
    }
}

fn now_seconds() -> f64 {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    ms as f64 / 1000.0
}

/// Copy both ways until both sides close, reading the frames into `tap`.
/// `extensions` is the server's `Sec-WebSocket-Extensions`.
pub async fn pump<C, S>(client: C, server: S, tap: WsTap, extensions: Option<String>)
where
    C: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (deflate, client_no_takeover, server_no_takeover) = deflate_of(extensions.as_deref());
    let (cr, cw) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);
    let up = copy_reading(cr, sw, Parser::new(deflate, client_no_takeover), &tap, "send");
    let down = copy_reading(sr, cw, Parser::new(deflate, server_no_takeover), &tap, "receive");
    tokio::join!(up, down);
    tap.close();
}

async fn copy_reading<R, W>(mut from: R, mut to: W, mut parser: Parser, tap: &WsTap, kind: &'static str)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
        for read in parser.feed(&buf[..n]) {
            tap.push(kind, read);
        }
    }
    let _ = to.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame as a client (with `mask`) or a server writes it.
    fn frame(fin: bool, rsv1: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
        let mut out = vec![(if fin { 0x80 } else { 0 }) | (if rsv1 { 0x40 } else { 0 }) | opcode];
        let m = if mask.is_some() { 0x80 } else { 0 };
        match payload.len() {
            n if n < 126 => out.push(m | n as u8),
            n if n < 65536 => {
                out.push(m | 126);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                out.push(m | 127);
                out.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        match mask {
            Some(k) => {
                out.extend_from_slice(&k);
                out.extend(payload.iter().enumerate().map(|(i, b)| b ^ k[i % 4]));
            }
            None => out.extend_from_slice(payload),
        }
        out
    }

    fn texts(reads: &[Read]) -> Vec<(u8, String)> {
        reads.iter().map(|r| (r.opcode, String::from_utf8_lossy(&r.data).into_owned())).collect()
    }

    #[test]
    fn masked_frames_split_anywhere_are_read() {
        let mut bytes = frame(true, false, 1, b"hello", Some([1, 2, 3, 4]));
        bytes.extend(frame(true, false, 9, b"p", Some([9, 9, 9, 9])));
        let mut p = Parser::new(false, false);
        let mut got = vec![];
        for b in &bytes {
            got.extend(p.feed(std::slice::from_ref(b)));
        }
        assert_eq!(texts(&got), vec![(1, "hello".into()), (9, "p".into())]);
    }

    #[test]
    fn fragments_are_joined_and_a_control_frame_may_come_between() {
        let mut bytes = frame(false, false, 1, b"hel", None);
        bytes.extend(frame(true, false, 10, b"", None));
        bytes.extend(frame(true, false, 0, b"lo", None));
        let got = Parser::new(false, false).feed(&bytes);
        assert_eq!(texts(&got), vec![(10, String::new()), (1, "hello".into())]);
    }

    #[test]
    fn long_lengths_and_the_message_limit() {
        let big = vec![b'x'; MESSAGE_LIMIT + 10];
        let got = Parser::new(false, false).feed(&frame(true, false, 2, &big, None));
        assert_eq!((got[0].data.len(), got[0].size, got[0].truncated), (MESSAGE_LIMIT, big.len() as u64, true));
        let got = Parser::new(false, false).feed(&frame(true, false, 1, &[b'a'; 300], None));
        assert_eq!(got[0].size, 300);
    }

    fn deflated(msgs: &[&[u8]], takeover: bool) -> Vec<Vec<u8>> {
        let mut c = flate2::Compress::new(flate2::Compression::default(), false);
        msgs.iter()
            .map(|m| {
                if !takeover {
                    c.reset();
                }
                let mut out = Vec::with_capacity(m.len() + 64);
                c.compress_vec(m, &mut out, flate2::FlushCompress::Sync).unwrap();
                assert!(out.ends_with(&[0, 0, 0xff, 0xff]));
                out.truncate(out.len() - 4);
                out
            })
            .collect()
    }

    #[test]
    fn permessage_deflate_with_and_without_context_takeover() {
        for takeover in [true, false] {
            let parts = deflated(&[b"{\"a\":1}", b"{\"a\":1}{\"a\":1}"], takeover);
            let mut bytes = vec![];
            for p in &parts {
                bytes.extend(frame(true, true, 1, p, Some([5, 6, 7, 8])));
            }
            let got = Parser::new(true, !takeover).feed(&bytes);
            assert_eq!(texts(&got), vec![(1, "{\"a\":1}".into()), (1, "{\"a\":1}{\"a\":1}".into())], "takeover {takeover}");
        }
    }

    #[test]
    fn not_a_websocket_stream_stops_the_reader() {
        let mut p = Parser::new(false, false);
        assert!(p.feed(b"\x83\x00GET / HTTP/1.1").is_empty());
        assert!(p.lost);
    }

    #[test]
    fn the_extension_header_is_read() {
        assert_eq!(deflate_of(Some("permessage-deflate; client_max_window_bits=15")), (true, false, false));
        assert_eq!(deflate_of(Some("x-foo, permessage-deflate; server_no_context_takeover")), (true, false, true));
        assert_eq!(deflate_of(Some("permessage-deflate; client_no_context_takeover")), (true, true, false));
        assert_eq!(deflate_of(None), (false, false, false));
    }
}
