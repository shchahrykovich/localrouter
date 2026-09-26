//! Shared library for LocalRouter: everything that can be tested without a
//! running daemon. The daemon (`apps/daemon`) wires these parts to sockets and
//! files; the CLI (`apps/cli`) only uses the API types.

pub mod api;
pub mod config;
pub mod help;
pub mod logs;
pub mod paths;
pub mod proxy;
pub mod routes;
pub mod tcp;
pub mod tls;

/// The top-level domain every route lives under.
pub const TLD: &str = "localhost";
