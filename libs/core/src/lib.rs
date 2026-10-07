//! Shared library for LocalRouter: everything that can be tested without a
//! running daemon. The daemon (`apps/daemon`) wires these parts to sockets and
//! files; the CLI (`apps/cli`) only uses the API types.

pub mod api;
pub mod config;
pub mod folder;
pub mod forward;
pub mod har;
pub mod help;
pub mod inspect;
pub mod instance;
pub mod logs;
pub mod paths;
pub mod phone;
pub mod proxy;
pub mod routes;
pub mod scripts;
pub mod tcp;
pub mod tls;
pub mod upstream;

/// The top-level domain every route lives under.
pub const TLD: &str = "localhost";
