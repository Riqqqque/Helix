mod http_server;

use clap::Parser;
use helix_api::ApiState;
use helix_config::{ConfigOverrides, RuntimeConfig};
use helix_core::DatabaseStatus;
use helix_state::{DatabaseSet, MetricsOpenOutcome, StateDatabase, StateError};
use helix_system::HostSampler;
use http_server::{MAX_CONCURRENT_CONNECTIONS, REQUEST_HEADER_TIMEOUT};
use std::{
    error::Error,
    future::Future,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process,
    sync::Arc,
    time::Duration,
};
use tokio::{sync::watch, time::Instant};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

type DynError = Box<dyn Error + Send + Sync>;
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Parser)]
#[command(name = "helixd", version, about = "Helix local control-plane daemon")]
struct Args {
    /// Read configuration from this TOML file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Override the configured listen address.
    #[arg(long, value_name = "ADDRESS")]
    listen: Option<SocketAddr>,
    /// Override the trusted state directory.
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,
    /// Override the compiled frontend directory.
    #[arg(long, value_name = "PATH")]
    web_root: Option<PathBuf>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), DynError> {
    init_tracing()?;
    let args = Args::parse();
    ensure_unprivileged_execution()?;
    let config = RuntimeConfig::load(
        args.config.as_deref(),
        ConfigOverrides {
            listen: args.listen,
            data_dir: args.data_dir,
            web_root: args.web_root,
        },
    )?;

    let databases = Arc::new(DatabaseSet::open_for_daemon(&config.data_dir)?);
    // Recovery validation must be the first state operation after opening the
    // durability domains. In particular, do not prune or build API state
    // before the full integrity check required after an unclean shutdown.
    let now_unix_ms = i64::try_from(helix_core::unix_timestamp_ms()).unwrap_or(i64::MAX);
    let (was_unclean, repaired_session_rows, pruned_audit_rows) =
        prepare_runtime_state(databases.state(), now_unix_ms)?;
    if was_unclean {
        warn!("an unclean Helix shutdown was detected; the full state integrity check passed");
    }
    if repaired_session_rows > 0 {
        info!(
            repaired_session_rows,
            "repaired one bounded batch of excess session rows"
        );
    }
    if pruned_audit_rows > 0 {
        info!(
            pruned_audit_rows,
            "pruned one bounded batch of retained authentication audit events"
        );
    }
    let metrics_database = match databases.metrics_outcome() {
        MetricsOpenOutcome::Healthy => DatabaseStatus::Ok,
        MetricsOpenOutcome::Recovered { forensic_path } => {
            warn!(
                forensic_file = %safe_file_name(forensic_path),
                "Pulse metrics were corrupt; the original was preserved and a clean metrics database was created"
            );
            DatabaseStatus::Recovered
        }
        MetricsOpenOutcome::Unavailable { reason } => {
            warn!(reason = ?reason, "Pulse metrics are unavailable; critical state remains online");
            DatabaseStatus::Unavailable
        }
    };

    if !config.web_root.join("index.html").is_file() {
        warn!("compiled frontend index is missing; API and health routes remain available");
    }

    let api_state =
        ApiState::initialize(HostSampler::new(), metrics_database, Arc::clone(&databases)).await?;
    let blocking_tasks = api_state.blocking_task_tracker();
    let app = helix_api::router(api_state, config.web_root)?;
    let listener = tokio::net::TcpListener::bind(config.listen).await?;

    info!(
        listen = %config.listen,
        maximum_connections = MAX_CONCURRENT_CONNECTIONS,
        request_header_timeout_seconds = REQUEST_HEADER_TIMEOUT.as_secs(),
        version = helix_core::VERSION,
        "Helix is ready"
    );

    let (shutdown_started_tx, mut shutdown_started_rx) = watch::channel(None);
    let announce_shutdown = async move {
        shutdown_signal().await;
        shutdown_started_tx.send_replace(Some(Instant::now()));
    };
    let server = http_server::serve(listener, app, announce_shutdown);
    let mut server = Box::pin(std::future::IntoFuture::into_future(server));
    let shutdown_started_at = tokio::select! {
        biased;
        result = wait_for_shutdown_start(&mut shutdown_started_rx) => result?,
        result = &mut server => {
            result?;
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP server stopped before a shutdown signal",
            ).into());
        }
    };
    let shutdown_deadline = shutdown_started_at + GRACEFUL_SHUTDOWN_TIMEOUT;
    match drain_and_mark_clean(
        &mut server,
        blocking_tasks.wait_idle(),
        shutdown_deadline,
        databases.state(),
    )
    .await
    {
        Ok(()) => {}
        Err(ShutdownCompletionError::Http(error)) => return Err(error.into()),
        Err(ShutdownCompletionError::Marker(error)) => return Err(error.into()),
        Err(ShutdownCompletionError::Deadline) => {
            tracing::error!(
                timeout_seconds = GRACEFUL_SHUTDOWN_TIMEOUT.as_secs(),
                "shutdown drain deadline expired; forcing an unclean process exit"
            );
            process::exit(1);
        }
    }
    info!("Helix stopped cleanly");
    Ok(())
}

async fn wait_for_shutdown_start(
    receiver: &mut watch::Receiver<Option<Instant>>,
) -> Result<Instant, io::Error> {
    loop {
        if let Some(started_at) = *receiver.borrow_and_update() {
            return Ok(started_at);
        }
        receiver.changed().await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "shutdown signal task ended without announcing shutdown",
            )
        })?;
    }
}

trait CleanShutdownState {
    type Error;

    fn mark_clean_shutdown(&self) -> Result<(), Self::Error>;
}

impl CleanShutdownState for StateDatabase {
    type Error = StateError;

    fn mark_clean_shutdown(&self) -> Result<(), Self::Error> {
        StateDatabase::mark_clean_shutdown(self)
    }
}

#[derive(Debug)]
enum ShutdownCompletionError<E> {
    Http(io::Error),
    Deadline,
    Marker(E),
}

async fn drain_and_mark_clean<H, B, S>(
    http_drain: H,
    blocking_drain: B,
    deadline: Instant,
    state: &S,
) -> Result<(), ShutdownCompletionError<S::Error>>
where
    H: Future<Output = io::Result<()>>,
    B: Future<Output = ()>,
    S: CleanShutdownState,
{
    tokio::time::timeout_at(deadline, http_drain)
        .await
        .map_err(|_| ShutdownCompletionError::Deadline)?
        .map_err(ShutdownCompletionError::Http)?;
    tokio::time::timeout_at(deadline, blocking_drain)
        .await
        .map_err(|_| ShutdownCompletionError::Deadline)?;
    state
        .mark_clean_shutdown()
        .map_err(ShutdownCompletionError::Marker)
}

trait RuntimeState {
    type Error;

    fn begin_runtime(&self) -> Result<bool, Self::Error>;
    fn prune_stale_sessions(&self, now_unix_ms: i64) -> Result<usize, Self::Error>;
    fn maintain_session_row_limit(&self) -> Result<usize, Self::Error>;
    fn maintain_authentication_audit_retention(
        &self,
        now_unix_ms: i64,
    ) -> Result<usize, Self::Error>;
}

impl RuntimeState for StateDatabase {
    type Error = StateError;

    fn begin_runtime(&self) -> Result<bool, Self::Error> {
        StateDatabase::begin_runtime(self)
    }

    fn prune_stale_sessions(&self, now_unix_ms: i64) -> Result<usize, Self::Error> {
        StateDatabase::prune_stale_sessions(self, now_unix_ms)
    }

    fn maintain_session_row_limit(&self) -> Result<usize, Self::Error> {
        StateDatabase::maintain_session_row_limit(self)
    }

    fn maintain_authentication_audit_retention(
        &self,
        now_unix_ms: i64,
    ) -> Result<usize, Self::Error> {
        StateDatabase::maintain_authentication_audit_retention(self, now_unix_ms)
    }
}

fn prepare_runtime_state<S>(state: &S, now_unix_ms: i64) -> Result<(bool, usize, usize), S::Error>
where
    S: RuntimeState,
{
    let was_unclean = state.begin_runtime()?;
    state.prune_stale_sessions(now_unix_ms)?;
    let repaired_session_rows = state.maintain_session_row_limit()?;
    let pruned_audit_rows = state.maintain_authentication_audit_retention(now_unix_ms)?;
    Ok((was_unclean, repaired_session_rows, pruned_audit_rows))
}

#[cfg(target_os = "linux")]
fn ensure_unprivileged_execution() -> Result<(), io::Error> {
    reject_root_execution(rustix::process::geteuid().is_root())
}

#[cfg(not(target_os = "linux"))]
fn ensure_unprivileged_execution() -> Result<(), io::Error> {
    reject_root_execution(false)
}

fn reject_root_execution(is_root: bool) -> Result<(), io::Error> {
    if is_root {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "helixd refuses to run as Linux root; use a dedicated unprivileged service account",
        ))
    } else {
        Ok(())
    }
}

fn init_tracing() -> Result<(), DynError> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("helixd=info,helix_api=info"));
    if std::env::var("HELIX_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .json()
            .try_init()?;
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .compact()
            .try_init()?;
    }
    Ok(())
}

fn safe_file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "preserved-metrics-database".to_owned())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(%error, "could not install SIGTERM handler; waiting for Ctrl+C");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::error!(%error, "Ctrl+C handler failed");
            }
        }
        _ = terminate.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "Ctrl+C handler failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        future,
    };

    struct RecordingRuntimeState {
        calls: RefCell<Vec<(&'static str, i64)>>,
        begin_result: Result<bool, &'static str>,
    }

    struct RecordingCleanShutdownState {
        marks: Cell<usize>,
    }

    impl CleanShutdownState for RecordingCleanShutdownState {
        type Error = &'static str;

        fn mark_clean_shutdown(&self) -> Result<(), Self::Error> {
            self.marks.set(self.marks.get() + 1);
            Ok(())
        }
    }

    impl RuntimeState for RecordingRuntimeState {
        type Error = &'static str;

        fn begin_runtime(&self) -> Result<bool, Self::Error> {
            self.calls.borrow_mut().push(("begin", 0));
            self.begin_result
        }

        fn prune_stale_sessions(&self, now_unix_ms: i64) -> Result<usize, Self::Error> {
            self.calls.borrow_mut().push(("prune", now_unix_ms));
            Ok(0)
        }

        fn maintain_session_row_limit(&self) -> Result<usize, Self::Error> {
            self.calls.borrow_mut().push(("session-limit", 0));
            Ok(7)
        }

        fn maintain_authentication_audit_retention(
            &self,
            now_unix_ms: i64,
        ) -> Result<usize, Self::Error> {
            self.calls
                .borrow_mut()
                .push(("audit-retention", now_unix_ms));
            Ok(5)
        }
    }

    #[test]
    fn root_policy_rejects_root_and_accepts_unprivileged_users() {
        assert_eq!(
            reject_root_execution(true)
                .expect_err("root must be rejected")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        reject_root_execution(false).expect("unprivileged user is allowed");
    }

    #[test]
    fn recovery_validation_precedes_startup_cleanup() {
        let state = RecordingRuntimeState {
            calls: RefCell::new(Vec::new()),
            begin_result: Ok(true),
        };

        assert_eq!(
            prepare_runtime_state(&state, 42).expect("prepare runtime"),
            (true, 7, 5)
        );
        assert_eq!(
            *state.calls.borrow(),
            [
                ("begin", 0),
                ("prune", 42),
                ("session-limit", 0),
                ("audit-retention", 42)
            ]
        );
    }

    #[test]
    fn failed_recovery_validation_prevents_startup_cleanup() {
        let state = RecordingRuntimeState {
            calls: RefCell::new(Vec::new()),
            begin_result: Err("integrity failed"),
        };

        assert_eq!(
            prepare_runtime_state(&state, 42).expect_err("startup must fail closed"),
            "integrity failed"
        );
        assert_eq!(*state.calls.borrow(), [("begin", 0)]);
    }

    #[tokio::test]
    async fn complete_http_and_blocking_drains_write_one_clean_marker() {
        let state = RecordingCleanShutdownState {
            marks: Cell::new(0),
        };

        drain_and_mark_clean(
            future::ready(Ok(())),
            future::ready(()),
            Instant::now() + Duration::from_secs(1),
            &state,
        )
        .await
        .expect("complete shutdown");
        assert_eq!(state.marks.get(), 1);
    }

    #[tokio::test]
    async fn drain_deadline_never_writes_a_clean_marker() {
        for (http_drain, blocking_drain) in [(true, false), (false, true)] {
            let state = RecordingCleanShutdownState {
                marks: Cell::new(0),
            };
            let http = async move {
                if http_drain {
                    future::pending::<io::Result<()>>().await
                } else {
                    Ok(())
                }
            };
            let blocking = async move {
                if blocking_drain {
                    future::pending::<()>().await;
                }
            };

            assert!(matches!(
                drain_and_mark_clean(
                    http,
                    blocking,
                    Instant::now() + Duration::from_millis(25),
                    &state,
                )
                .await,
                Err(ShutdownCompletionError::Deadline)
            ));
            assert_eq!(state.marks.get(), 0);
        }
    }
}
