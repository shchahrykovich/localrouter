//! TCP byte copy for TCP routes: no TLS, no parsing. See ADR 01, change 7.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::sync::CancellationToken;

use crate::logs::{LogEntry, RequestLog, now_ms};
use crate::proxy::CONNECT_TIMEOUT;

/// Which route an accepted connection belongs to.
#[derive(Debug, Clone)]
pub struct TcpRouteInfo {
    pub host: String,
    pub listen_port: u16,
    pub target: SocketAddr,
}

/// Connect `client` to the route's target and copy bytes both ways until both
/// sides close, or until `cancel` fires (the route was removed, I18). Writes one
/// log entry when the connection ends.
pub async fn serve(mut client: TcpStream, route: TcpRouteInfo, log: Arc<RequestLog>, cancel: CancellationToken) {
    let start = Instant::now();
    let time_ms = now_ms();
    let _ = client.set_nodelay(true);
    let entry = |bytes_in, bytes_out, failed| LogEntry::Tcp {
        time_ms,
        host: route.host.clone(),
        listen_port: route.listen_port,
        bytes_in,
        bytes_out,
        duration_ms: start.elapsed().as_millis() as u64,
        failed,
    };

    let upstream = tokio::select! {
        r = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(route.target)) => r,
        _ = cancel.cancelled() => return,
    };
    let upstream = match upstream {
        Ok(Ok(s)) => s,
        _ => {
            let _ = client.shutdown().await;
            log.push(entry(0, 0, true));
            return;
        }
    };
    let _ = upstream.set_nodelay(true);

    let bytes_in = AtomicU64::new(0);
    let bytes_out = AtomicU64::new(0);
    let (client_r, client_w) = client.into_split();
    let (up_r, up_w) = upstream.into_split();
    tokio::select! {
        _ = async { tokio::join!(pipe(client_r, up_w, &bytes_in), pipe(up_r, client_w, &bytes_out)) } => {}
        _ = cancel.cancelled() => {}
    }
    log.push(entry(bytes_in.load(Ordering::Relaxed), bytes_out.load(Ordering::Relaxed), false));
}

/// Copy one direction. At end of input, half-close the other side.
async fn pipe(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, count: &AtomicU64) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                count.fetch_add(n as u64, Ordering::Relaxed);
            }
        }
    }
    let _ = to.shutdown().await;
}
