//! T12 (byte copy cases): TCP routes against a real echo server.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use localrouter_core::logs::{LogEntry, RequestLog};
use localrouter_core::tcp::{self, TcpRouteInfo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

/// Echo server that answers after the client half-closes, like a request/response protocol.
async fn echo_after_eof() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut all = vec![];
                s.read_to_end(&mut all).await.unwrap();
                s.write_all(&all).await.unwrap();
                s.write_all(b"!").await.unwrap();
            });
        }
    });
    addr
}

/// A front listener that hands each connection to `tcp::serve`.
async fn front(target: SocketAddr, log: Arc<RequestLog>, cancel: CancellationToken) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let info = TcpRouteInfo { host: "db.shop".into(), listen_port: addr.port(), target };
    tokio::spawn(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            tokio::spawn(tcp::serve(s, info.clone(), log.clone(), cancel.clone()));
        }
    });
    addr
}

async fn wait_for_log(log: &RequestLog) -> LogEntry {
    for _ in 0..100 {
        if let Some(e) = log.recent(None, 1).pop() {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no log entry");
}

#[tokio::test]
async fn bytes_flow_both_ways_and_half_close_reaches_the_target() {
    let up = echo_after_eof().await;
    let log = Arc::new(RequestLog::new(10));
    let addr = front(up, log.clone(), CancellationToken::new()).await;

    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"ping").await.unwrap();
    c.shutdown().await.unwrap(); // half-close: the server only answers after EOF
    let mut back = vec![];
    c.read_to_end(&mut back).await.unwrap();
    assert_eq!(back, b"ping!");

    match wait_for_log(&log).await {
        LogEntry::Tcp { host, bytes_in, bytes_out, failed, .. } => {
            assert_eq!(host, "db.shop");
            assert_eq!((bytes_in, bytes_out, failed), (4, 5, false));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn closed_target_closes_the_client_and_logs_failed() {
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let log = Arc::new(RequestLog::new(10));
    let addr = front(closed, log.clone(), CancellationToken::new()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(3), c.read(&mut buf)).await.unwrap().unwrap_or(0);
    assert_eq!(n, 0, "client connection closed");
    assert!(matches!(wait_for_log(&log).await, LogEntry::Tcp { failed: true, .. }));
}

// I18
#[tokio::test]
async fn cancel_closes_open_connections() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let up = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (_held, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let log = Arc::new(RequestLog::new(10));
    let cancel = CancellationToken::new();
    let addr = front(up, log.clone(), cancel.clone()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"x").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(2), c.read(&mut buf)).await.expect("closed in time").unwrap_or(0);
    assert_eq!(n, 0);
    assert!(matches!(wait_for_log(&log).await, LogEntry::Tcp { bytes_in: 1, .. }));
}
