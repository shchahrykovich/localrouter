//! `localrouterd`: the LocalRouter daemon. One per user.
//!
//! Start-up order: lock, config, CA, routes, shared HTTP listeners, TCP route
//! listeners, socket. See ADR 01.

mod daemon;
mod listen;
mod lock;
mod logfile;
mod pidwatch;
mod socket;
mod store;
mod tcp_listen;

use std::process::ExitCode;

use localrouter_core::instance::Instance;
use localrouter_core::paths::Paths;
use localrouter_core::tls::{CaLoad, LocalCa};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::daemon::{DAEMON_VERSION, Daemon};

fn main() -> ExitCode {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("localrouterd {DAEMON_VERSION}");
        return ExitCode::SUCCESS;
    }
    // Before any folder is opened: a bad suffix must not create one (ADR 04, I3).
    let instance = match Instance::of_this_program() {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("localrouterd: {e}");
            return ExitCode::from(2);
        }
    };
    let paths = Paths::from_env(&instance);
    if let Some(problem) = paths.socket_path_problem() {
        eprintln!("localrouterd: {problem}");
        return ExitCode::from(2);
    }
    if let Err(e) = std::fs::create_dir_all(&paths.data) {
        eprintln!("localrouterd: cannot create {}: {e}", paths.data.display());
        return ExitCode::from(2);
    }
    let _lock = match lock::acquire(&paths.lock()) {
        Ok(lock) => lock,
        Err(lock::LockError::AlreadyRunning) => {
            eprintln!("localrouterd: already running for {}", paths.data.display());
            return ExitCode::from(3);
        }
        Err(lock::LockError::Io(e)) => {
            eprintln!("localrouterd: cannot lock {}: {e}", paths.lock().display());
            return ExitCode::from(2);
        }
    };
    init_logging(&paths);

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("localrouterd: {e}");
            return ExitCode::from(2);
        }
    };
    match runtime.block_on(run(paths, instance)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            eprintln!("localrouterd: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_logging(paths: &Paths) {
    let filter = EnvFilter::try_from_env("LOCALROUTER_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr = tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_target(false);
    let registry = tracing_subscriber::registry().with(filter).with(stderr);
    match logfile::RotatingFile::open(paths.daemon_log()) {
        Ok(file) => registry.with(tracing_subscriber::fmt::layer().with_writer(file).with_ansi(false).with_target(false)).init(),
        Err(e) => {
            registry.init();
            tracing::warn!("cannot open {}: {e}", paths.daemon_log().display());
        }
    }
}

async fn run(paths: Paths, instance: Instance) -> anyhow::Result<()> {
    tracing::info!("{} {DAEMON_VERSION} starting, data in {}", instance.daemon_program(), paths.data.display());
    let (pids, mut exited) = pidwatch::PidWatch::start()?;
    let (ca, ca_problem) = match LocalCa::load_or_create(&paths, &instance) {
        CaLoad::Ready(ca) => (Some(*ca), None),
        CaLoad::Broken(why) => {
            tracing::error!("HTTPS is off: {why}. Run `localrouter ca reset` to make a new CA.");
            (None, Some(why))
        }
    };
    let daemon = Daemon::load(paths, instance, pids, ca, ca_problem);
    daemon.start_http_listeners()?;
    daemon.start_saved_tcp_routes();
    let listener = socket::bind(&daemon)?;
    tokio::spawn(socket::serve(daemon.clone(), listener));

    let d = daemon.clone();
    tokio::spawn(async move {
        while let Some(pid) = exited.recv().await {
            d.remove_owned_by(pid).await;
        }
    });

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = term.recv() => {},
        _ = tokio::signal::ctrl_c() => {},
    }
    tracing::info!("stopping");
    daemon.shutdown.cancel();
    let _ = std::fs::remove_file(daemon.paths.socket());
    Ok(())
}
