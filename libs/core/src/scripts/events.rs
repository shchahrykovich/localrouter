//! Event streams (ADR 07, change 4, section 3): split a server-sent events or
//! NDJSON body into events as bytes arrive, and write a changed event back in
//! the same format.
//!
//! Server-sent events: an event ends at a blank line; lines end in LF, CRLF or
//! CR; `:` starts a comment; `data` lines are joined with LF. NDJSON and
//! JSONL: one event per line.

use bytes::{Bytes, BytesMut};

/// The two formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Sse,
    Lines,
}

impl Format {
    /// The format of an `events` body, from its Content-Type.
    pub fn of(content_type: Option<&str>) -> Option<Self> {
        let essence = content_type?.split(';').next()?.trim().to_ascii_lowercase();
        match essence.as_str() {
            "text/event-stream" => Some(Format::Sse),
            "application/x-ndjson" | "application/jsonl" | "application/ndjson" | "application/x-jsonlines" => {
                Some(Format::Lines)
            }
            _ => None,
        }
    }
}

/// One event. For NDJSON only `data` is set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Event {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
    pub retry: Option<String>,
    /// The bytes as they arrived, sent on when a script changes nothing.
    pub raw: Bytes,
    /// A block of only comments or blank lines: scripts never see it, but
    /// its bytes go on.
    pub comment: bool,
}

impl Event {
    /// The event written back in its format, ending with its separator.
    pub fn encode(&self, format: Format) -> Bytes {
        match format {
            Format::Lines => {
                let mut s = self.data.replace(['\r', '\n'], " ");
                s.push('\n');
                Bytes::from(s)
            }
            Format::Sse => {
                let mut s = String::new();
                if let Some(id) = &self.id {
                    s.push_str(&format!("id: {}\n", one_line(id)));
                }
                if let Some(event) = &self.event {
                    s.push_str(&format!("event: {}\n", one_line(event)));
                }
                if let Some(retry) = &self.retry {
                    s.push_str(&format!("retry: {}\n", one_line(retry)));
                }
                for line in self.data.split('\n') {
                    s.push_str(&format!("data: {}\n", line.trim_end_matches('\r')));
                }
                s.push('\n');
                Bytes::from(s)
            }
        }
    }
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

/// Splits bytes into events. Bytes that do not yet end an event wait in the
/// buffer; the caller decides what to do with a buffer that grows too large.
pub struct Splitter {
    format: Format,
    buf: BytesMut,
    /// Where the scan for the end of the event may start again.
    scanned: usize,
}

impl Splitter {
    pub fn new(format: Format) -> Self {
        Self { format, buf: BytesMut::new(), scanned: 0 }
    }

    /// Bytes waiting for the end of their event.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// Add bytes and return every event they complete.
    pub fn push(&mut self, data: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(data);
        let mut out = vec![];
        while let Some(end) = self.find_end() {
            let raw = self.buf.split_to(end).freeze();
            self.scanned = 0;
            if let Some(event) = parse(self.format, &raw) {
                out.push(event);
            } else if !raw.is_empty() {
                // A block of comments only: no event, but its bytes go on.
                out.push(Event { raw, comment: true, ..Event::default() });
            }
        }
        out
    }

    /// The bytes left at the end of the stream, as a last event if they hold one.
    pub fn finish(&mut self) -> Option<Event> {
        let raw = std::mem::take(&mut self.buf).freeze();
        if raw.is_empty() {
            return None;
        }
        Some(parse(self.format, &raw).unwrap_or(Event { raw, comment: true, ..Event::default() }))
    }

    /// Take the bytes waiting, without parsing (an event that is too large).
    pub fn take_pending(&mut self) -> Bytes {
        self.scanned = 0;
        std::mem::take(&mut self.buf).freeze()
    }

    /// The index just after the separator that ends the first event.
    fn find_end(&mut self) -> Option<usize> {
        let b = &self.buf[..];
        let from = self.scanned;
        // Nothing new that could end a line: no need to scan again.
        if !b[from..].iter().any(|&c| c == b'\n' || c == b'\r') {
            self.scanned = b.len();
            return None;
        }
        self.scanned = b.len();
        match self.format {
            Format::Lines => b.iter().position(|&c| c == b'\n').map(|i| i + 1),
            Format::Sse => {
                // A blank line: two line ends in a row (LF, CRLF or CR each).
                let mut i = 0;
                let mut line_empty = true;
                while i < b.len() {
                    let c = b[i];
                    if c != b'\n' && c != b'\r' {
                        line_empty = false;
                        i += 1;
                        continue;
                    }
                    let len = if c == b'\r' && b.get(i + 1) == Some(&b'\n') {
                        2
                    } else if c == b'\r' && i + 1 == b.len() {
                        // A CR at the end may be the first half of CRLF.
                        self.scanned = i;
                        return None;
                    } else {
                        1
                    };
                    if line_empty && i > 0 {
                        return Some(i + len);
                    }
                    line_empty = true;
                    i += len;
                }
                None
            }
        }
    }
}

/// Parse one event's bytes. `None` for a block with no field.
fn parse(format: Format, raw: &Bytes) -> Option<Event> {
    let text = String::from_utf8_lossy(raw);
    match format {
        Format::Lines => {
            let line = text.trim_end_matches(['\n', '\r']);
            if line.trim().is_empty() {
                return None;
            }
            Some(Event { data: line.to_string(), raw: raw.clone(), ..Event::default() })
        }
        Format::Sse => {
            let mut event = Event { raw: raw.clone(), ..Event::default() };
            let mut data: Vec<&str> = vec![];
            let mut any = false;
            for line in text.split('\n').flat_map(|l| l.split('\r')) {
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                let (field, value) = match line.split_once(':') {
                    Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                    None => (line, ""),
                };
                match field {
                    "data" => {
                        data.push(value);
                        any = true;
                    }
                    "event" => {
                        event.event = Some(value.to_string());
                        any = true;
                    }
                    "id" => {
                        event.id = Some(value.to_string());
                        any = true;
                    }
                    "retry" => {
                        event.retry = Some(value.to_string());
                        any = true;
                    }
                    _ => {}
                }
            }
            event.data = data.join("\n");
            any.then_some(event)
        }
    }
}

/// `sse.parse(s)`: every event of a whole server-sent events body.
pub fn parse_all(format: Format, body: &[u8]) -> Vec<Event> {
    let mut s = Splitter::new(format);
    let mut out = s.push(body);
    out.extend(s.finish());
    out.retain(|e| !e.comment);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(events: &[Event]) -> Vec<&str> {
        events.iter().filter(|e| !e.comment).map(|e| e.data.as_str()).collect()
    }

    // T21
    #[test]
    fn sse_events_end_at_a_blank_line_with_any_line_end() {
        for sep in ["\n", "\r\n", "\r"] {
            let body = format!("event: a{sep}data: 1{sep}data: 2{sep}{sep}: comment{sep}{sep}id: 7{sep}data: x{sep}{sep}");
            let events = parse_all(Format::Sse, body.as_bytes());
            assert_eq!(data(&events), ["1\n2", "x"], "{sep:?}");
            assert_eq!(events[0].event.as_deref(), Some("a"));
            assert_eq!(events[1].id.as_deref(), Some("7"));
        }
    }

    // T21
    #[test]
    fn an_event_split_across_parts_is_one_event() {
        let mut s = Splitter::new(Format::Sse);
        assert!(s.push(b"data: hel").is_empty());
        assert!(s.push(b"lo\r").is_empty());
        let got = s.push(b"\n\r\ndata: next\n");
        assert_eq!(data(&got), ["hello"]);
        assert_eq!(got[0].raw, Bytes::from_static(b"data: hello\r\n\r\n"));
        assert_eq!(s.pending(), b"data: next\n".len());
        assert_eq!(data(&s.push(b"\n")), ["next"]);
    }

    // T21: NDJSON, a line split across two network parts.
    #[test]
    fn ndjson_is_one_event_per_line() {
        let mut s = Splitter::new(Format::Lines);
        assert!(s.push(b"{\"a\":").is_empty());
        let got = s.push(b"1}\n{\"b\":2}\n{\"c\"");
        assert_eq!(data(&got), ["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(s.finish().unwrap().data, "{\"c\"");
    }

    #[test]
    fn encode_writes_the_format_back() {
        let e = Event { event: Some("delta".into()), data: "a\nb".into(), id: Some("1".into()), ..Event::default() };
        assert_eq!(e.encode(Format::Sse), Bytes::from_static(b"id: 1\nevent: delta\ndata: a\ndata: b\n\n"));
        let back = parse_all(Format::Sse, &e.encode(Format::Sse));
        assert_eq!(back[0].data, "a\nb");
        let line = Event { data: "{\"x\":1}".into(), ..Event::default() };
        assert_eq!(line.encode(Format::Lines), Bytes::from_static(b"{\"x\":1}\n"));
    }

    #[test]
    fn format_comes_from_the_content_type() {
        assert_eq!(Format::of(Some("text/event-stream; charset=utf-8")), Some(Format::Sse));
        assert_eq!(Format::of(Some("application/x-ndjson")), Some(Format::Lines));
        assert_eq!(Format::of(Some("application/json")), None);
        assert_eq!(Format::of(None), None);
    }
}
