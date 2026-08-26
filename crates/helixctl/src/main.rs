use clap::{Parser, Subcommand};
use helix_auth::{OpaqueToken, TokenDomain};
use helix_config::{ConfigOverrides, RuntimeConfig};
use helix_core::unix_timestamp_ms;
use helix_state::{
    BackupOutcome, BootstrapInstallOutcome, BootstrapTokenHash, DatabaseSet, MetricsDatabaseReader,
    PragmaReport, StateDatabaseReader,
};
use std::{
    error::Error,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant},
};

type DynError = Box<dyn Error + Send + Sync>;
const SETUP_TOKEN_LIFETIME_MS: i64 = 15 * 60 * 1_000;
const MAX_PROBE_RESPONSE_BYTES: usize = 64 * 1024;
const PROBE_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const PROBE_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Parser)]
#[command(name = "helixctl", version, about = "Administrative CLI for Helix")]
struct Args {
    /// Read configuration from this TOML file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Override the trusted state directory.
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a one-time owner-setup token that expires after 15 minutes.
    SetupToken,
    /// Show local schema and durability status.
    Status,
    /// Wait for daemon liveness, critical-state API, and compiled UI readiness.
    Ready {
        /// Stop retrying after this many seconds.
        #[arg(
            long,
            default_value_t = 20,
            value_parser = clap::value_parser!(u64).range(1..=120)
        )]
        timeout_seconds: u64,
    },
    /// Validate SQLite configuration and integrity.
    Doctor {
        /// Run SQLite's full integrity check instead of quick_check.
        #[arg(long)]
        full: bool,
    },
    /// Create a consistent, verified state database snapshot.
    BackupState {
        #[arg(value_name = "DESTINATION")]
        destination: PathBuf,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("helixctl: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), DynError> {
    let args = Args::parse();
    let config = RuntimeConfig::load(
        args.config.as_deref(),
        ConfigOverrides {
            data_dir: args.data_dir,
            ..ConfigOverrides::default()
        },
    )?;

    match args.command {
        Command::SetupToken => {
            ensure_setup_token_unprivileged()?;
            install_setup_token(&config.data_dir, &mut io::stdout())?;
        }
        Command::Status => {
            require_initialized(&config.data_dir)?;
            print_status(&config.data_dir)?;
        }
        Command::Ready { timeout_seconds } => {
            wait_until_ready(config.listen, Duration::from_secs(timeout_seconds))?;
            println!("Helix readiness passed: liveness, critical-state API, and compiled UI");
        }
        Command::Doctor { full } => {
            require_initialized(&config.data_dir)?;
            run_doctor(&config.data_dir, full)?;
        }
        Command::BackupState { destination } => {
            require_initialized(&config.data_dir)?;
            let outcome = backup_state(&config.data_dir, &destination)?;
            println!(
                "Verified state snapshot written to {}",
                destination.display()
            );
            if let BackupOutcome::PublishedWithResidue { temporary_path } = outcome {
                eprintln!(
                    "helixctl: the snapshot is verified and published, but temporary cleanup could not finish; remove {} after confirming the destination remains readable",
                    temporary_path.display()
                );
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ProbeExpectation {
    path: &'static str,
    status: u16,
    content_type: Option<&'static str>,
    body: ProbeBody,
}

#[derive(Clone, Copy)]
enum ProbeBody {
    Empty,
    NonEmpty,
}

const READINESS_PROBES: [ProbeExpectation; 3] = [
    ProbeExpectation {
        path: "/healthz",
        status: 204,
        content_type: None,
        body: ProbeBody::Empty,
    },
    ProbeExpectation {
        path: "/api/v1/setup/status",
        status: 200,
        content_type: Some("application/json"),
        body: ProbeBody::NonEmpty,
    },
    ProbeExpectation {
        path: "/",
        status: 200,
        content_type: Some("text/html"),
        body: ProbeBody::NonEmpty,
    },
];

fn wait_until_ready(address: SocketAddr, timeout: Duration) -> Result<(), io::Error> {
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "readiness timeout is too large",
        )
    })?;
    let mut last_error = None;

    loop {
        let now = Instant::now();
        if now >= deadline {
            let detail = last_error.map_or_else(
                || "the readiness deadline elapsed".to_owned(),
                |error: io::Error| error.to_string(),
            );
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Helix did not become ready at {address}: {detail}"),
            ));
        }

        match probe_readiness(address, deadline) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            continue;
        }
        thread::sleep(PROBE_RETRY_INTERVAL.min(remaining));
    }
}

fn probe_readiness(address: SocketAddr, deadline: Instant) -> Result<(), io::Error> {
    for expectation in READINESS_PROBES {
        let now = Instant::now();
        let endpoint_deadline = now
            .checked_add(PROBE_ENDPOINT_TIMEOUT)
            .map_or(deadline, |candidate| candidate.min(deadline));
        remaining_before(endpoint_deadline, expectation.path, "connecting")?;
        probe_endpoint(address, endpoint_deadline, expectation)?;
    }
    Ok(())
}

fn probe_endpoint(
    address: SocketAddr,
    deadline: Instant,
    expectation: ProbeExpectation,
) -> Result<(), io::Error> {
    let connect_timeout = remaining_before(deadline, expectation.path, "connecting")?;
    let mut stream = TcpStream::connect_timeout(&address, connect_timeout)
        .map_err(|error| map_socket_timeout(error, expectation.path, "connecting"))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        expectation.path, address
    );
    write_before_deadline(&mut stream, request.as_bytes(), deadline, expectation.path)?;

    let mut response = Vec::with_capacity(4 * 1024);
    let mut buffer = [0_u8; 8 * 1024];
    while response.len() <= MAX_PROBE_RESPONSE_BYTES {
        let remaining = remaining_before(deadline, expectation.path, "reading the response")?;
        stream.set_read_timeout(Some(remaining))?;
        let capacity = (MAX_PROBE_RESPONSE_BYTES + 1 - response.len()).min(buffer.len());
        let read = stream
            .read(&mut buffer[..capacity])
            .map_err(|error| map_socket_timeout(error, expectation.path, "reading the response"))?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..read]);
    }
    if response.len() > MAX_PROBE_RESPONSE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} readiness response exceeded the size limit",
                expectation.path
            ),
        ));
    }
    validate_probe_response(&response, expectation)
}

fn write_before_deadline(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
    path: &str,
) -> Result<(), io::Error> {
    while !bytes.is_empty() {
        let remaining = remaining_before(deadline, path, "writing the request")?;
        stream.set_write_timeout(Some(remaining))?;
        let written = stream
            .write(bytes)
            .map_err(|error| map_socket_timeout(error, path, "writing the request"))?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("{path} readiness request could not be written"),
            ));
        }
        bytes = &bytes[written..];
    }

    let remaining = remaining_before(deadline, path, "flushing the request")?;
    stream.set_write_timeout(Some(remaining))?;
    stream
        .flush()
        .map_err(|error| map_socket_timeout(error, path, "flushing the request"))
}

fn remaining_before(deadline: Instant, path: &str, operation: &str) -> Result<Duration, io::Error> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(readiness_timeout(path, operation))
    } else {
        Ok(remaining)
    }
}

fn map_socket_timeout(error: io::Error, path: &str, operation: &str) -> io::Error {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        readiness_timeout(path, operation)
    } else {
        error
    }
}

fn readiness_timeout(path: &str, operation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{path} readiness deadline elapsed while {operation}"),
    )
}

fn validate_probe_response(
    response: &[u8],
    expectation: ProbeExpectation,
) -> Result<(), io::Error> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed HTTP response"))?;
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 HTTP headers"))?;
    let mut lines = headers.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP status line"))?;
    let mut status_parts = status_line.split_whitespace();
    let protocol = status_parts.next().unwrap_or_default();
    let status = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP status line"))?;
    if !matches!(protocol, "HTTP/1.0" | "HTTP/1.1") || status != expectation.status {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} returned HTTP {status}, expected {}",
                expectation.path, expectation.status
            ),
        ));
    }

    let mut content_type = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed HTTP header"))?;
        if name.eq_ignore_ascii_case("content-type") {
            if content_type.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate Content-Type header",
                ));
            }
            content_type = Some(value.trim());
        }
    }
    if let Some(expected) = expectation.content_type
        && !content_type.is_some_and(|actual| {
            actual
                .split(';')
                .next()
                .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case(expected))
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} returned an unexpected Content-Type", expectation.path),
        ));
    }

    let body = &response[header_end + 4..];
    match expectation.body {
        ProbeBody::Empty if !body.is_empty() => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} returned an unexpected response body", expectation.path),
        )),
        ProbeBody::NonEmpty if body.is_empty() => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} returned an empty response body", expectation.path),
        )),
        ProbeBody::Empty | ProbeBody::NonEmpty => Ok(()),
    }
}

fn install_setup_token(data_dir: &Path, output: &mut impl io::Write) -> Result<(), DynError> {
    let databases = DatabaseSet::open_for_daemon(data_dir)?;
    let token = OpaqueToken::generate()?;
    let token_hash = token.verification_hash(TokenDomain::Bootstrap);
    let bootstrap_hash = BootstrapTokenHash::from_digest(*token_hash.as_bytes());
    let now_unix_ms = i64::try_from(unix_timestamp_ms()).unwrap_or(i64::MAX);
    let expires_at_unix_ms = now_unix_ms
        .checked_add(SETUP_TOKEN_LIFETIME_MS)
        .ok_or_else(|| io::Error::other("the system clock is outside the supported range"))?;

    match databases.state().replace_bootstrap_token(
        &bootstrap_hash,
        now_unix_ms,
        expires_at_unix_ms,
    )? {
        BootstrapInstallOutcome::Installed { .. } => {
            let encoded = token.encode();
            writeln!(output, "Owner setup token (shown once):")?;
            writeln!(output, "{}", encoded.expose_secret())?;
            writeln!(
                output,
                "Open Helix on this server and paste the token within 15 minutes."
            )?;
            writeln!(
                output,
                "Running this command again invalidates the previous setup token."
            )?;
        }
        BootstrapInstallOutcome::OwnerAlreadyExists => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "an owner account already exists; setup tokens are permanently disabled",
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_setup_token_unprivileged() -> Result<(), io::Error> {
    reject_setup_token_root(rustix::process::geteuid().is_root())
}

#[cfg(not(target_os = "linux"))]
fn ensure_setup_token_unprivileged() -> Result<(), io::Error> {
    reject_setup_token_root(false)
}

fn reject_setup_token_root(is_root: bool) -> Result<(), io::Error> {
    if is_root {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "setup-token refuses to run as Linux root; run it as the dedicated helix account",
        ))
    } else {
        Ok(())
    }
}

fn require_initialized(data_dir: &Path) -> Result<(), io::Error> {
    let state_path = data_dir.join("state").join("helix-state.db");
    if state_path.is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "no Helix installation was found at {}; start helixd or run setup first",
                data_dir.display()
            ),
        ))
    }
}

fn print_status(data_dir: &Path) -> Result<(), DynError> {
    let state_database = StateDatabaseReader::open(data_dir)?;
    let state = state_database.pragma_report()?;
    println!("Helix installation: {}", state_database.installation_id());
    println!("State schema:       {}", state.user_version);
    match MetricsDatabaseReader::open(data_dir) {
        Ok(metrics_database) => {
            println!("Metrics schema:     {}", metrics_database.schema_version()?);
            println!("Metrics status:     readable");
        }
        Err(_) => {
            println!("Metrics schema:     unavailable");
            println!("Metrics status:     unavailable (read-only inspection failed)");
        }
    }
    Ok(())
}

fn run_doctor(data_dir: &Path, full: bool) -> Result<(), DynError> {
    let state_database = StateDatabaseReader::open(data_dir)?;
    if full {
        state_database.full_integrity_check()?;
    } else {
        state_database.quick_integrity_check()?;
    }
    let metrics_database = MetricsDatabaseReader::open(data_dir)?;
    metrics_database.quick_integrity_check()?;
    print_pragma("state", &state_database.pragma_report()?);
    print_pragma("metrics", &metrics_database.pragma_report()?);
    println!(
        "Integrity: state {} check passed; metrics quick check passed",
        if full { "full" } else { "quick" }
    );
    Ok(())
}

fn backup_state(data_dir: &Path, destination: &Path) -> Result<BackupOutcome, DynError> {
    Ok(StateDatabaseReader::open(data_dir)?.backup_to(destination)?)
}

fn print_pragma(name: &str, report: &PragmaReport) {
    println!(
        "{name}: journal={}, synchronous={}, foreign_keys={}, busy_timeout={}ms, schema={}",
        report.journal_mode,
        report.synchronous,
        if report.foreign_keys { "on" } else { "off" },
        report.busy_timeout_ms,
        report.user_version
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    fn private_test_directory(description: &str) -> tempfile::TempDir {
        let directory =
            tempfile::tempdir().unwrap_or_else(|error| panic!("{description}: {error}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
                .unwrap_or_else(|error| panic!("secure {description}: {error}"));
        }
        directory
    }

    #[test]
    fn readiness_response_validation_is_strict_and_bounded() {
        let liveness = READINESS_PROBES[0];
        validate_probe_response(
            b"HTTP/1.1 204 No Content\r\nCache-Control: no-store\r\n\r\n",
            liveness,
        )
        .expect("minimal liveness response");
        assert!(
            validate_probe_response(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}",
                liveness,
            )
            .is_err()
        );

        let setup = READINESS_PROBES[1];
        validate_probe_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\n\r\n{}",
            setup,
        )
        .expect("critical-state API response");
        assert!(
            validate_probe_response(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n{}",
                setup,
            )
            .is_err()
        );

        let ui = READINESS_PROBES[2];
        validate_probe_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<!doctype html>",
            ui,
        )
        .expect("compiled UI response");
        assert!(
            validate_probe_response(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n", ui,)
                .is_err()
        );
    }

    #[test]
    fn readiness_probe_enforces_an_absolute_deadline_against_trickle_responses() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind trickle server");
        let address = listener.local_addr().expect("trickle server address");
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept readiness probe");
            let mut request = [0_u8; 512];
            let _ = stream.read(&mut request);
            for _ in 0..80 {
                if server_stop.load(Ordering::Relaxed) || stream.write_all(b"x").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
        });

        let timeout = Duration::from_millis(250);
        let started = Instant::now();
        let deadline = started.checked_add(timeout).expect("test deadline");
        let error = probe_endpoint(address, deadline, READINESS_PROBES[0])
            .expect_err("trickle response must not extend the deadline");
        let elapsed = started.elapsed();
        stop.store(true, Ordering::Relaxed);
        server.join().expect("trickle server");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            elapsed < Duration::from_millis(750),
            "probe exceeded its absolute deadline: {elapsed:?}"
        );
    }

    #[test]
    fn setup_token_initializes_state_and_never_echoes_a_replaced_token() {
        let temp = private_test_directory("temporary directory");
        let mut first_output = Vec::new();
        install_setup_token(temp.path(), &mut first_output).expect("first setup token");
        let first_text = String::from_utf8(first_output).expect("UTF-8 output");
        let first_token = first_text.lines().nth(1).expect("first token").to_owned();
        assert_eq!(first_token.len(), helix_auth::OPAQUE_TOKEN_ENCODED_LEN);

        let mut second_output = Vec::new();
        install_setup_token(temp.path(), &mut second_output).expect("replacement setup token");
        let second_text = String::from_utf8(second_output).expect("UTF-8 output");
        let second_token = second_text.lines().nth(1).expect("second token");

        assert_ne!(first_token, second_token);
        assert!(!second_text.contains(&first_token));
    }

    #[test]
    fn setup_token_root_policy_is_explicit() {
        assert_eq!(
            reject_setup_token_root(true)
                .expect_err("root must be rejected")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        reject_setup_token_root(false).expect("unprivileged user is allowed");
    }

    #[test]
    fn state_backup_does_not_open_or_recover_broken_metrics() {
        let temp = private_test_directory("temporary directory");
        drop(DatabaseSet::open_for_daemon(temp.path()).expect("initialize databases"));
        let metrics_path = temp.path().join("metrics").join("helix-metrics.db");
        fs::write(&metrics_path, b"not a SQLite database").expect("break metrics database");
        let metrics_before = fs::read(&metrics_path).expect("read broken metrics");
        let destination = temp.path().join("state-backup.db");

        backup_state(temp.path(), &destination).expect("back up state only");

        assert!(destination.is_file());
        assert_eq!(
            fs::read(metrics_path).expect("read metrics after backup"),
            metrics_before
        );
    }
}
