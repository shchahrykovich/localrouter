//! Socket client for the daemon. Holds no state (see ADR 01, change 5).

use std::path::Path;

use localrouter_core::api::{self, API_VERSION, ApiError, Event, HelloParams, HelloResult, Request, Response};
use localrouter_core::instance::Instance;
use localrouter_core::logs::LogEntry;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// Names this program's instance: "LocalRouter-dev is not running. Open
/// LocalRouter-dev.app or run localrouterd-dev." (ADR 04).
fn not_running() -> String {
    let i = Instance::of_this_program().unwrap_or_else(|_| Instance::release());
    format!("{i} is not running. Open {i}.app or run {}.", i.daemon_program())
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("{}", not_running())]
    NotRunning,
    #[error("{0}")]
    Api(ApiError),
    #[error("the daemon speaks API {daemon} but this {client} speaks {API_VERSION}; update the older one")]
    VersionMismatch { daemon: String, client: String },
    #[error("connection to the daemon failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("unexpected reply from the daemon: {0}")]
    Protocol(String),
}

pub struct Client {
    read: BufReader<OwnedReadHalf>,
    write: OwnedWriteHalf,
    next: u64,
    pub daemon_version: String,
}

impl Client {
    /// Connect and say hello. Fails when the major API versions differ (I14).
    pub async fn connect(socket: &Path, client: &str) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => ClientError::NotRunning,
            _ => ClientError::Io(e),
        })?;
        let (read, write) = stream.into_split();
        let mut c = Self { read: BufReader::new(read), write, next: 1, daemon_version: String::new() };
        let hello: HelloResult = c
            .call("hello", HelloParams { client: client.into(), api_version: API_VERSION.into() })
            .await
            .map_err(|e| match e {
                ClientError::Api(e) if e.code == api::ErrorCode::VersionMismatch => {
                    ClientError::VersionMismatch { daemon: "a different version".into(), client: client.into() }
                }
                other => other,
            })?;
        if api::api_major(&hello.api_version) != api::api_major(API_VERSION) {
            return Err(ClientError::VersionMismatch { daemon: hello.api_version, client: client.into() });
        }
        c.daemon_version = hello.daemon_version;
        Ok(c)
    }

    pub async fn call<T: DeserializeOwned>(&mut self, method: &str, params: impl Serialize) -> Result<T, ClientError> {
        let value = self.call_value(method, params).await?;
        serde_json::from_value(value).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub async fn call_value(&mut self, method: &str, params: impl Serialize) -> Result<serde_json::Value, ClientError> {
        let id = self.next;
        self.next += 1;
        let params = serde_json::to_value(params).map_err(|e| ClientError::Protocol(e.to_string()))?;
        let mut line = serde_json::to_string(&Request { id, method: method.into(), params })
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        line.push('\n');
        self.write.write_all(line.as_bytes()).await?;
        loop {
            let reply = self.next_line().await?;
            if reply.get("event").is_some() {
                continue;
            }
            let reply: Response = serde_json::from_value(reply).map_err(|e| ClientError::Protocol(e.to_string()))?;
            if reply.id != id {
                continue;
            }
            if let Some(e) = reply.error {
                return Err(ClientError::Api(e));
            }
            return Ok(reply.result.unwrap_or(serde_json::Value::Null));
        }
    }

    async fn next_line(&mut self) -> Result<serde_json::Value, ClientError> {
        let mut line = String::new();
        if self.read.read_line(&mut line).await? == 0 {
            return Err(ClientError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the daemon closed the connection")));
        }
        serde_json::from_str(&line).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// After `subscribe_logs`: the next log entry.
    pub async fn next_log(&mut self) -> Result<LogEntry, ClientError> {
        loop {
            let value = self.next_line().await?;
            if value.get("event").is_some() {
                let Event::Log { entry } = serde_json::from_value(value).map_err(|e| ClientError::Protocol(e.to_string()))?;
                return Ok(entry);
            }
        }
    }
}
