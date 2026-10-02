//! Bodies in the proxy log: the network task copies the bytes that pass, up
//! to a limit, and the entry is written when the response body ends.
//!
//! The copy never changes or delays traffic: each frame goes on as it came,
//! and a copy that would pass [`BODY_LIMIT`] or the memory budget is cut,
//! not waited for. Decoding (gzip, br, …) happens later, on the writer
//! thread.

use std::error::Error;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::Instant;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use hyper::{Request, Response};

use super::websocket::WsTap;
use super::{HarLog, HarRecord};
use crate::proxy::Body;
use crate::scripts::bodies::{Budget, Reservation};

/// Raw bytes kept of one request or response body.
pub const BODY_LIMIT: usize = 1 << 20;
/// Bytes kept after decoding gzip, deflate, br or zstd.
pub const DECODED_LIMIT: usize = 4 << 20;
/// Every copy in memory together: bodies on their way and records in the
/// writer's queue.
pub const BUDGET: usize = 256 << 20;

/// What was copied of one body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Copied {
    /// The first bytes, at most [`BODY_LIMIT`].
    pub data: Vec<u8>,
    /// Every byte that passed, kept or not.
    pub size: u64,
    /// Fewer bytes were kept than passed: the limit or the budget.
    pub truncated: bool,
}

#[derive(Debug, Default)]
struct CopyState {
    copied: Copied,
    held: Option<Reservation>,
}

/// One body's copy, shared by the body that fills it and the code that
/// writes the entry.
#[derive(Debug, Clone)]
pub struct BodyCopy {
    state: Arc<Mutex<CopyState>>,
    budget: Arc<Budget>,
}

impl BodyCopy {
    pub fn new(budget: Arc<Budget>) -> Self {
        Self { state: Arc::default(), budget }
    }

    /// Count `bytes` and keep what fits.
    pub fn push(&self, bytes: &[u8]) {
        let mut s = self.state.lock().unwrap();
        s.copied.size += bytes.len() as u64;
        if s.copied.truncated || bytes.is_empty() {
            return;
        }
        let take = (BODY_LIMIT - s.copied.data.len()).min(bytes.len());
        let reserved = take > 0
            && match s.held.as_mut() {
                Some(r) => r.grow(take),
                None => self.budget.try_reserve(take).map(|r| s.held = Some(r)).is_some(),
            };
        if !reserved {
            s.copied.truncated = true;
            return;
        }
        s.copied.data.extend_from_slice(&bytes[..take]);
        if take < bytes.len() {
            s.copied.truncated = true;
        }
    }

    /// The copy so far, and its reservation, which the record keeps until it
    /// is written.
    pub fn take(&self) -> (Copied, Option<Reservation>) {
        let mut s = self.state.lock().unwrap();
        (std::mem::take(&mut s.copied), s.held.take())
    }
}

/// A body that passes every frame on and copies its data.
pub struct TeeBody {
    inner: Body,
    copy: BodyCopy,
}

impl hyper::body::Body for TeeBody {
    type Data = Bytes;
    type Error = Box<dyn Error + Send + Sync>;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let frame = ready!(Pin::new(&mut self.inner).poll_frame(cx));
        if let Some(Ok(f)) = &frame
            && let Some(data) = f.data_ref()
        {
            self.copy.push(data);
        }
        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// What the entry still needs when the response body ends.
struct Finish {
    log: Arc<HarLog>,
    record: HarRecord,
    request: Option<BodyCopy>,
    /// When the response headers were ready.
    headers_at: Instant,
}

impl Finish {
    fn done(mut self, response: &BodyCopy, error: Option<String>) {
        if let Some(copy) = &self.request {
            let (copied, held) = copy.take();
            self.record.request_body = Some(copied);
            self.record.held.extend(held.map(Arc::new));
        }
        let (copied, held) = response.take();
        self.record.response_body = Some(copied);
        self.record.held.extend(held.map(Arc::new));
        self.record.receive_ms = self.headers_at.elapsed().as_millis() as u64;
        if self.record.body_error.is_none() {
            self.record.body_error = error;
        }
        self.log.record(self.record);
    }
}

/// The response body the client gets: it copies the data and writes the
/// entry when it ends, fails, or is dropped (the client went away).
pub struct RecordingBody {
    inner: Body,
    copy: BodyCopy,
    finish: Option<Finish>,
}

impl RecordingBody {
    fn end(&mut self, error: Option<String>) {
        if let Some(f) = self.finish.take() {
            f.done(&self.copy, error);
        }
    }
}

impl hyper::body::Body for RecordingBody {
    type Data = Bytes;
    type Error = Box<dyn Error + Send + Sync>;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let frame = ready!(Pin::new(&mut self.inner).poll_frame(cx));
        match &frame {
            Some(Ok(f)) => {
                if let Some(data) = f.data_ref() {
                    self.copy.push(data);
                }
                if self.inner.is_end_stream() {
                    self.end(None);
                }
            }
            Some(Err(e)) => {
                let why = e.to_string();
                self.end(Some(why));
            }
            None => self.end(None),
        }
        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for RecordingBody {
    fn drop(&mut self) {
        let unfinished = !self.inner.is_end_stream();
        self.end(unfinished.then(|| "the client closed the connection before the body ended".to_string()));
    }
}

/// One request the log follows: its record, the copy of its body, and for
/// an upgrade the tap the WebSocket copy loops fill.
pub struct Recording {
    pub record: HarRecord,
    pub request: Option<BodyCopy>,
    pub tap: Option<WsTap>,
}

impl HarLog {
    /// Follow a request: copy its body, and give an upgrade a [`WsTap`] in
    /// its extensions, where the code that sends it finds it.
    pub fn start_recording(&self, record: HarRecord, req: Request<Body>) -> (Request<Body>, Recording) {
        let (mut req, request) = self.copy_request(req);
        let tap = crate::proxy::is_upgrade(req.headers()).then(|| {
            let tap = WsTap::new(self.budget.clone());
            req.extensions_mut().insert(tap.clone());
            tap
        });
        (req, Recording { record, request, tap })
    }

    /// The response the client gets, for a request [`Self::start_recording`]
    /// followed. The caller has finished the record with the response
    /// headers. A WebSocket's entry is written when it closes; any other
    /// entry when the response body ends.
    pub fn end_recording(self: &Arc<Self>, rec: Recording, resp: Response<Body>) -> Response<Body> {
        if resp.status() == hyper::StatusCode::SWITCHING_PROTOCOLS
            && let Some(tap) = rec.tap.filter(WsTap::started)
        {
            tap.attach(self.clone(), rec.record);
            return resp;
        }
        self.finish(rec.record, rec.request, resp)
    }

    /// Copy the request body as it is read. `None` when the log is off.
    pub fn copy_request(&self, req: Request<Body>) -> (Request<Body>, Option<BodyCopy>) {
        if !self.enabled() {
            return (req, None);
        }
        let copy = BodyCopy::new(self.budget.clone());
        let req = req.map(|inner| TeeBody { inner, copy: copy.clone() }.boxed_unsync());
        (req, Some(copy))
    }

    /// The response the client gets. Its entry is written when the body
    /// ends; a body that never ends is written when the client goes away.
    pub fn finish(self: &Arc<Self>, record: HarRecord, request: Option<BodyCopy>, resp: Response<Body>) -> Response<Body> {
        let streams = !resp.headers().contains_key(hyper::header::CONTENT_LENGTH);
        if streams {
            self.send_open(&record);
        }
        let copy = BodyCopy::new(self.budget.clone());
        let finish = Finish { log: self.clone(), record, request, headers_at: Instant::now() };
        resp.map(|inner| RecordingBody { inner, copy, finish: Some(finish) }.boxed_unsync())
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::{Full, StreamBody};

    use super::*;

    fn body(chunks: Vec<&'static [u8]>) -> Body {
        let frames = chunks.into_iter().map(|c| Ok::<_, Box<dyn Error + Send + Sync>>(Frame::data(Bytes::from_static(c))));
        StreamBody::new(futures_util::stream::iter(frames)).boxed_unsync()
    }

    #[test]
    fn a_copy_keeps_the_limit_and_counts_every_byte() {
        let budget = Budget::new(BUDGET);
        let c = BodyCopy::new(budget.clone());
        c.push(b"abc");
        let big = vec![7u8; BODY_LIMIT];
        c.push(&big);
        let (copied, held) = c.take();
        assert_eq!(copied.size, 3 + BODY_LIMIT as u64);
        assert_eq!(copied.data.len(), BODY_LIMIT);
        assert!(copied.truncated);
        assert_eq!(budget.used(), BODY_LIMIT, "the kept bytes are reserved");
        drop(held);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn a_copy_stops_at_the_budget_and_never_waits() {
        let budget = Budget::new(4);
        let c = BodyCopy::new(budget.clone());
        c.push(b"ab");
        c.push(b"cdef");
        let (copied, _held) = c.take();
        assert_eq!((copied.data, copied.size, copied.truncated), (b"ab".to_vec(), 6, true));
    }

    #[tokio::test]
    async fn a_tee_body_passes_every_frame_on() {
        let copy = BodyCopy::new(Budget::new(BUDGET));
        let tee = TeeBody { inner: body(vec![b"hello ", b"world"]), copy: copy.clone() };
        let got = tee.collect().await.unwrap().to_bytes();
        assert_eq!(&got[..], b"hello world");
        assert_eq!(copy.take().0.data, b"hello world");
        let empty = TeeBody { inner: Full::new(Bytes::new()).map_err(|n| match n {}).boxed_unsync(), copy: copy.clone() };
        assert!(hyper::body::Body::is_end_stream(&empty));
    }
}
