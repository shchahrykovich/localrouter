//! The socket API server: newline-delimited JSON on `daemon.sock` (mode 0600).

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use localrouter_core::api::{
    ApiError, Empty, ErrorCode, Event, FindFreePortParams, GetLogsParams, GetProxyParams, HelloParams, HostParams, IdParams, Request,
    Response, SetConfigParams, SetScriptRuleParams, SubscribeLogsParams, SubscribeLogsResult,
};
use localrouter_core::routes::Route;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use crate::daemon::Daemon;

/// Largest request line accepted (routes are small; this stops a runaway client).
const MAX_LINE: usize = 1024 * 1024;

pub fn bind(daemon: &Daemon) -> anyhow::Result<UnixListener> {
    let path = daemon.paths.socket();
    // We hold the instance lock, so an existing socket file is stale.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn serve(daemon: Arc<Daemon>, listener: UnixListener) {
    loop {
        let stream = tokio::select! {
            r = listener.accept() => match r {
                Ok((s, _)) => s,
                Err(e) => { tracing::warn!("socket accept failed: {e}"); continue; }
            },
            _ = daemon.shutdown.cancelled() => return,
        };
        tokio::spawn(connection(daemon.clone(), stream));
    }
}

async fn connection(daemon: Arc<Daemon>, stream: UnixStream) {
    let (read, write) = stream.into_split();
    let write = Arc::new(Mutex::new(write));
    let mut lines = BufReader::new(read);
    let mut line = String::new();
    loop {
        line.clear();
        match lines.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) if line.len() > MAX_LINE => {
                let _ = send(&write, &Response::err(0, ApiError::new(ErrorCode::InvalidRequest, "request too large"))).await;
                return;
            }
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let reply = Response::err(0, ApiError::new(ErrorCode::InvalidRequest, format!("not a request: {e}")));
                if send(&write, &reply).await.is_err() {
                    return;
                }
                continue;
            }
        };
        let id = request.id;
        let reply = match dispatch(&daemon, request, &write).await {
            Ok(value) => Response { id, result: Some(value), error: None },
            Err(e) => Response::err(id, e),
        };
        if send(&write, &reply).await.is_err() {
            return;
        }
    }
}

async fn send(write: &Mutex<OwnedWriteHalf>, value: &impl serde::Serialize) -> std::io::Result<()> {
    let mut text = serde_json::to_string(value).map_err(std::io::Error::other)?;
    text.push('\n');
    write.lock().await.write_all(text.as_bytes()).await
}

fn params<T: DeserializeOwned + Default>(value: serde_json::Value) -> Result<T, ApiError> {
    if value.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(value).map_err(|e| ApiError::new(ErrorCode::InvalidRequest, format!("bad params: {e}")))
}

fn required<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, ApiError> {
    serde_json::from_value(value).map_err(|e| ApiError::new(ErrorCode::InvalidRequest, format!("bad params: {e}")))
}

fn json(value: impl serde::Serialize) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(value).map_err(|e| ApiError::new(ErrorCode::Io, e.to_string()))
}

async fn dispatch(daemon: &Arc<Daemon>, req: Request, write: &Arc<Mutex<OwnedWriteHalf>>) -> Result<serde_json::Value, ApiError> {
    match req.method.as_str() {
        "hello" => json(daemon.hello(required::<HelloParams>(req.params)?)?),
        "status" => {
            let _: Empty = params(req.params)?;
            json(daemon.status().await)
        }
        "register_route" => json(daemon.register_route(required::<Route>(req.params)?).await?),
        "unregister_route" => json(daemon.unregister_route(required::<HostParams>(req.params)?).await?),
        "list_routes" => json(daemon.list_routes().await),
        "find_free_port" => json(daemon.find_free_port(params::<FindFreePortParams>(req.params)?)?),
        "get_logs" => json(daemon.get_logs(params::<GetLogsParams>(req.params)?)),
        "subscribe_logs" => {
            let p: SubscribeLogsParams = params(req.params)?;
            let mut rx = daemon.log.subscribe();
            let write = write.clone();
            let shutdown = daemon.shutdown.clone();
            tokio::spawn(async move {
                loop {
                    let entry = tokio::select! {
                        r = rx.recv() => match r {
                            Ok(e) => e,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => return,
                        },
                        _ = shutdown.cancelled() => return,
                    };
                    if p.host.as_deref().is_some_and(|h| !entry.matches_host(h)) {
                        continue;
                    }
                    if send(&write, &Event::Log { entry }).await.is_err() {
                        return;
                    }
                }
            });
            json(SubscribeLogsResult { subscribed: true })
        }
        "get_config" => json(daemon.get_config()),
        "set_config" => json(daemon.set_config(params::<SetConfigParams>(req.params)?).await?),
        "reset_ca" => json(daemon.reset_ca().await?),
        "get_proxy" => {
            json(daemon.get_proxy(params::<GetProxyParams>(req.params)?).await?)
        }
        "reset_inspect_ca" => {
            let _: Empty = params(req.params)?;
            json(daemon.reset_inspect_ca().await?)
        }
        "set_script_rule" => json(daemon.set_script_rule(required::<SetScriptRuleParams>(req.params)?).await?),
        "remove_script_rule" => json(daemon.remove_script_rule(required::<IdParams>(req.params)?).await?),
        "list_script_rules" => {
            let _: Empty = params(req.params)?;
            json(daemon.list_script_rules())
        }
        other => Err(ApiError::new(ErrorCode::UnknownMethod, format!("unknown method {other}"))),
    }
}
