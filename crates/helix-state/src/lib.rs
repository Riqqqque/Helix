//! SQLite durability domains and startup recovery primitives.

mod secrets;
mod security;

#[cfg(test)]
pub(crate) fn private_test_directory(description: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap_or_else(|error| panic!("{description}: {error}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|error| panic!("secure {description}: {error}"));
    }
    directory
}

use fs2::FileExt;
use helix_core::unix_timestamp_ms;
use rusqlite::{Connection, ErrorCode, MAIN_DB, OpenFlags, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};
use thiserror::Error;
use uuid::Uuid;

pub use secrets::{
    EncryptedSecretWrite, InstallMasterKeyInput, InstallMasterKeyOutcome, MasterKeyRecord,
    SecretRecordMetadata, StoredSecretRecord,
};
pub use security::{
    AuthenticatedSession, BootstrapInstallOutcome, BootstrapPreflightOutcome, BootstrapTokenHash,
    CredentialRecord, CsrfRequirement, CsrfTokenHash, LoginDelayState, OwnerClaimInput,
    OwnerClaimOutcome, OwnerClaimRejection, PasswordPhc, PasswordRehash,
    SessionAuthenticationInput, SessionAuthorization, SessionCreateInput, SessionCreateOutcome,
    SessionTokenHash, SetupStatus, UserStatus,
};

pub const STATE_SCHEMA_VERSION: i64 = 4;
pub const METRICS_SCHEMA_VERSION: i64 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const DAEMON_LEASE_FILE: &str = ".helixd.lock";
const MIGRATION_ALIAS_FORMAT: &str = "helix-migration-snapshot-v1";
const MAX_MIGRATION_ALIAS_BYTES: u64 = 512;
const MAX_LEGACY_MIGRATION_PARTIALS_PER_OPEN: usize = 16;
const METRICS_FORENSIC_FORMAT: &str = "helix-metrics-forensic-v1";
const METRICS_FORENSIC_STAGING_DIR: &str = ".helix-metrics-forensic-staging";
const METRICS_FORENSIC_MANIFEST: &str = "manifest.v1";
const METRICS_FORENSIC_MANIFEST_PARTIAL: &str = ".manifest.v1.partial";
const MAX_METRICS_FORENSIC_MANIFEST_BYTES: u64 = 512;
const PUBLISHED_BACKUP_CLEANUP_ATTEMPTS: usize = 3;

const STATE_MIGRATION_1: &str = r#"
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    name TEXT NOT NULL UNIQUE,
    applied_at_unix_ms INTEGER NOT NULL CHECK (applied_at_unix_ms >= 0)
) STRICT;

CREATE TABLE installation (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    id TEXT NOT NULL UNIQUE CHECK (length(id) = 36),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    clean_shutdown INTEGER NOT NULL DEFAULT 1 CHECK (clean_shutdown IN (0, 1)),
    last_started_at_unix_ms INTEGER,
    last_clean_shutdown_at_unix_ms INTEGER
) STRICT;

CREATE TABLE nodes (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    kind TEXT NOT NULL CHECK (kind IN ('local', 'remote')),
    display_name TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
) STRICT;

CREATE UNIQUE INDEX nodes_single_local_idx ON nodes (kind) WHERE kind = 'local';

CREATE TABLE operation_ledger (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    operation_kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'planned', 'staging', 'applying', 'verifying', 'completed',
        'rollback_pending', 'rolled_back', 'needs_intervention'
    )),
    target_id TEXT,
    intent_json TEXT NOT NULL CHECK (json_valid(intent_json)),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0),
    completed_at_unix_ms INTEGER,
    error_code TEXT,
    error_detail TEXT
) STRICT;

CREATE INDEX operation_ledger_status_idx
    ON operation_ledger (status, updated_at_unix_ms);
"#;

const METRICS_MIGRATION_1: &str = r#"
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    name TEXT NOT NULL UNIQUE,
    applied_at_unix_ms INTEGER NOT NULL CHECK (applied_at_unix_ms >= 0)
) STRICT;

CREATE TABLE metric_samples (
    id INTEGER PRIMARY KEY,
    node_id TEXT NOT NULL CHECK (length(node_id) = 36),
    metric TEXT NOT NULL,
    value REAL NOT NULL,
    unit TEXT NOT NULL,
    collected_at_unix_ms INTEGER NOT NULL CHECK (collected_at_unix_ms >= 0)
) STRICT;

CREATE INDEX metric_samples_lookup_idx
    ON metric_samples (node_id, metric, collected_at_unix_ms DESC);
"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackupOutcome {
    Published,
    PublishedWithResidue { temporary_path: PathBuf },
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("filesystem operation '{operation}' failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the operating system random source failed")]
    RandomSource,
    #[error("{database} integrity check failed: {details:?}")]
    Integrity {
        database: &'static str,
        details: Vec<String>,
    },
    #[error("SQLite connection policy could not be enforced: {details:?}")]
    PragmaMismatch { details: Vec<String> },
    #[error(
        "{database} schema version {found} is newer than this Helix build supports ({supported})"
    )]
    UnsupportedSchema {
        database: &'static str,
        found: i64,
        supported: i64,
    },
    #[error("database mutex was poisoned")]
    LockPoisoned,
    #[error("another helixd process already owns the daemon lease at {0}")]
    DaemonLeaseHeld(PathBuf),
    #[error("trusted data directory must not be a symbolic link: {0}")]
    DataDirectorySymlink(PathBuf),
    #[error("trusted data path is not a directory: {0}")]
    DataPathNotDirectory(PathBuf),
    #[error("trusted database path must not be a symbolic link: {0}")]
    DatabasePathSymlink(PathBuf),
    #[error("trusted database path is not a regular file: {0}")]
    DatabasePathNotFile(PathBuf),
    #[error("unsafe Unix permissions on {path}: found {found:#o}, expected exactly {expected:#o}")]
    UnsafePermissions {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[error("unsafe Unix owner on {path}: found uid {found}, expected uid {expected}")]
    UnsafeOwner {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[error("backup destination already exists: {0}")]
    BackupDestinationExists(PathBuf),
    #[error("invalid security input: {0}")]
    InvalidSecurityInput(&'static str),
    #[error("invalid secret-store input: {0}")]
    InvalidSecretInput(&'static str),
    #[error("secret record was not found")]
    SecretNotFound,
    #[error("secret revision conflict: expected {expected}, found {actual}")]
    SecretRevisionConflict { expected: i64, actual: i64 },
}

impl StateError {
    fn indicates_corruption(&self) -> bool {
        match self {
            Self::Integrity { .. } => true,
            Self::Sqlite(error) => matches!(
                error.sqlite_error_code(),
                Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
            ),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetricsOpenOutcome {
    Healthy,
    Recovered { forensic_path: PathBuf },
    Unavailable { reason: MetricsUnavailableReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricsUnavailableReason {
    StoragePolicy,
    UnsupportedSchema,
    Integrity,
    Io,
    RandomSource,
    Sqlite,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PragmaReport {
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: bool,
    pub busy_timeout_ms: i64,
    pub user_version: i64,
}

pub struct DatabaseSet {
    state: StateDatabase,
    metrics: Option<MetricsDatabase>,
    metrics_outcome: MetricsOpenOutcome,
    _daemon_lease: DaemonLease,
}

pub struct StateDatabase {
    connection: Mutex<Connection>,
    path: PathBuf,
    installation_id: String,
}

pub struct MetricsDatabase {
    connection: Mutex<Connection>,
    path: PathBuf,
}

pub struct StateDatabaseReader {
    connection: Connection,
    path: PathBuf,
    installation_id: String,
}

pub struct MetricsDatabaseReader {
    connection: Connection,
    path: PathBuf,
}

struct DaemonLease {
    _file: File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MigrationSnapshotAlias {
    source_schema_version: i64,
    target_schema_version: i64,
    source_identity: String,
    snapshot_identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MetricsForensicManifest {
    destination_name: String,
    wal_present: bool,
    shm_present: bool,
}

impl DatabaseSet {
    /// Open Helix's writable durability domains while holding the process-wide
    /// daemon lease. Administrative readers must use the read-only types.
    pub fn open_for_daemon(data_dir: &Path) -> Result<Self, StateError> {
        ensure_data_directory(data_dir)?;
        let daemon_lease = acquire_daemon_lease(data_dir)?;
        let state_dir = data_dir.join("state");
        let metrics_dir = data_dir.join("metrics");
        ensure_data_directory(&state_dir)?;
        let state = StateDatabase::open(&state_dir.join("helix-state.db"))?;
        let metrics_path = metrics_dir.join("helix-metrics.db");
        let metrics_result = (|| {
            ensure_data_directory(&metrics_dir)?;
            MetricsDatabase::open_resilient(&metrics_path)
        })();
        let (metrics, metrics_outcome) = match metrics_result {
            Ok((metrics, outcome)) => (Some(metrics), outcome),
            Err(error) => (
                None,
                MetricsOpenOutcome::Unavailable {
                    reason: MetricsUnavailableReason::from_error(&error),
                },
            ),
        };
        Ok(Self {
            state,
            metrics,
            metrics_outcome,
            _daemon_lease: daemon_lease,
        })
    }

    #[must_use]
    pub fn state(&self) -> &StateDatabase {
        &self.state
    }

    #[must_use]
    pub fn metrics(&self) -> Option<&MetricsDatabase> {
        self.metrics.as_ref()
    }

    #[must_use]
    pub fn metrics_outcome(&self) -> &MetricsOpenOutcome {
        &self.metrics_outcome
    }
}

impl MetricsUnavailableReason {
    fn from_error(error: &StateError) -> Self {
        match error {
            StateError::DataDirectorySymlink(_)
            | StateError::DataPathNotDirectory(_)
            | StateError::DatabasePathSymlink(_)
            | StateError::DatabasePathNotFile(_)
            | StateError::UnsafePermissions { .. }
            | StateError::UnsafeOwner { .. } => Self::StoragePolicy,
            StateError::UnsupportedSchema { .. } => Self::UnsupportedSchema,
            StateError::Integrity { .. } => Self::Integrity,
            StateError::Io { .. } => Self::Io,
            StateError::RandomSource => Self::RandomSource,
            _ => Self::Sqlite,
        }
    }
}

impl StateDatabaseReader {
    /// Open the critical state database without creating files, migrating, or
    /// changing persistent SQLite policy.
    pub fn open(data_dir: &Path) -> Result<Self, StateError> {
        let path = data_dir.join("state").join("helix-state.db");
        validate_database_artifacts(&path)?;
        let connection = open_read_only_connection(&path, 2)?;
        reject_newer_schema(&connection, "helix-state.db", STATE_SCHEMA_VERSION)?;
        let schema_version = pragma_i64(&connection, "user_version")?;
        validate_state_semantics(&connection, schema_version, "helix-state.db")?;
        let installation_id = connection.query_row(
            "SELECT id FROM installation WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(Self {
            connection,
            path,
            installation_id,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }

    pub fn schema_version(&self) -> Result<i64, StateError> {
        pragma_i64(&self.connection, "user_version")
    }

    pub fn pragma_report(&self) -> Result<PragmaReport, StateError> {
        read_pragma_report(&self.connection)
    }

    pub fn quick_integrity_check(&self) -> Result<(), StateError> {
        check_integrity(&self.connection, "helix-state.db", false)
    }

    pub fn full_integrity_check(&self) -> Result<(), StateError> {
        check_integrity(&self.connection, "helix-state.db", true)
    }

    pub fn backup_to(&self, destination: &Path) -> Result<BackupOutcome, StateError> {
        online_backup(&self.connection, destination, "helix-state.db")
    }
}

impl MetricsDatabaseReader {
    /// Open the disposable metrics domain without creating or recovering it.
    pub fn open(data_dir: &Path) -> Result<Self, StateError> {
        let path = data_dir.join("metrics").join("helix-metrics.db");
        validate_database_artifacts(&path)?;
        let connection = open_read_only_connection(&path, 1)?;
        reject_newer_schema(&connection, "helix-metrics.db", METRICS_SCHEMA_VERSION)?;
        Ok(Self { connection, path })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn schema_version(&self) -> Result<i64, StateError> {
        pragma_i64(&self.connection, "user_version")
    }

    pub fn pragma_report(&self) -> Result<PragmaReport, StateError> {
        read_pragma_report(&self.connection)
    }

    pub fn quick_integrity_check(&self) -> Result<(), StateError> {
        check_integrity(&self.connection, "helix-metrics.db", false)
    }
}

impl StateDatabase {
    fn open(path: &Path) -> Result<Self, StateError> {
        let existing_nonempty = metadata_if_exists(path, "inspect database before migration")?
            .is_some_and(|metadata| metadata.is_file() && metadata.len() > 0);
        prepare_writable_database(path)?;
        let mut connection = Connection::open(path)?;
        configure_state_connection(&connection)?;
        migrate_state(&mut connection, path, existing_nonempty)?;
        validate_state_semantics(&connection, STATE_SCHEMA_VERSION, "helix-state.db")?;
        validate_database_artifacts(path)?;
        check_integrity(&connection, "helix-state.db", false)?;
        let installation_id = ensure_installation(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            path: path.to_path_buf(),
            installation_id,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }

    pub fn schema_version(&self) -> Result<i64, StateError> {
        let connection = self.lock()?;
        pragma_i64(&connection, "user_version")
    }

    pub fn pragma_report(&self) -> Result<PragmaReport, StateError> {
        let connection = self.lock()?;
        read_pragma_report(&connection)
    }

    pub fn quick_integrity_check(&self) -> Result<(), StateError> {
        let connection = self.lock()?;
        check_integrity(&connection, "helix-state.db", false)
    }

    pub fn full_integrity_check(&self) -> Result<(), StateError> {
        let connection = self.lock()?;
        check_integrity(&connection, "helix-state.db", true)
    }

    /// Mark daemon startup and perform the stronger check required after an
    /// unclean exit. Returns `true` when an unclean exit was detected.
    pub fn begin_runtime(&self) -> Result<bool, StateError> {
        let mut connection = self.lock()?;
        let previous_clean = connection.query_row(
            "SELECT clean_shutdown FROM installation WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let was_unclean = previous_clean == 0;
        if was_unclean {
            check_integrity(&connection, "helix-state.db", true)?;
        }
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE installation
             SET clean_shutdown = 0, last_started_at_unix_ms = ?1
             WHERE singleton = 1",
            [timestamp_i64()],
        )?;
        transaction.commit()?;
        Ok(was_unclean)
    }

    pub fn mark_clean_shutdown(&self) -> Result<(), StateError> {
        self.lock()?.execute(
            "UPDATE installation
             SET clean_shutdown = 1, last_clean_shutdown_at_unix_ms = ?1
             WHERE singleton = 1",
            [timestamp_i64()],
        )?;
        Ok(())
    }

    /// Create and verify a consistent online snapshot. Existing destinations
    /// are never overwritten.
    pub fn backup_to(&self, destination: &Path) -> Result<BackupOutcome, StateError> {
        let connection = self.lock()?;
        online_backup(&connection, destination, "helix-state.db")
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, StateError> {
        self.connection.lock().map_err(|_| StateError::LockPoisoned)
    }
}

impl MetricsDatabase {
    fn open_resilient(path: &Path) -> Result<(Self, MetricsOpenOutcome), StateError> {
        let reconciled_forensic_path = reconcile_metrics_forensic_staging(path)?;
        match Self::open_once(path) {
            Ok(database) => Ok((
                database,
                reconciled_forensic_path.map_or(MetricsOpenOutcome::Healthy, |forensic_path| {
                    MetricsOpenOutcome::Recovered { forensic_path }
                }),
            )),
            Err(error) if path.exists() && error.indicates_corruption() => {
                let forensic_path = preserve_corrupt_database(path)?;
                let database = Self::open_once(path)?;
                Ok((database, MetricsOpenOutcome::Recovered { forensic_path }))
            }
            Err(error) => Err(error),
        }
    }

    fn open_once(path: &Path) -> Result<Self, StateError> {
        prepare_writable_database(path)?;
        let mut connection = Connection::open(path)?;
        configure_metrics_connection(&connection)?;
        migrate_metrics(&mut connection)?;
        validate_database_artifacts(path)?;
        check_integrity(&connection, "helix-metrics.db", false)?;
        Ok(Self {
            connection: Mutex::new(connection),
            path: path.to_path_buf(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn schema_version(&self) -> Result<i64, StateError> {
        let connection = self.lock()?;
        pragma_i64(&connection, "user_version")
    }

    pub fn pragma_report(&self) -> Result<PragmaReport, StateError> {
        let connection = self.lock()?;
        read_pragma_report(&connection)
    }

    pub fn quick_integrity_check(&self) -> Result<(), StateError> {
        let connection = self.lock()?;
        check_integrity(&connection, "helix-metrics.db", false)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, StateError> {
        self.connection.lock().map_err(|_| StateError::LockPoisoned)
    }
}

fn configure_state_connection(connection: &Connection) -> Result<(), StateError> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "wal_autocheckpoint", 1_000_i64)?;
    connection.pragma_update(None, "journal_size_limit", 16_777_216_i64)?;
    verify_pragmas(connection, 2)
}

fn configure_metrics_connection(connection: &Connection) -> Result<(), StateError> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "wal_autocheckpoint", 1_000_i64)?;
    connection.pragma_update(None, "journal_size_limit", 67_108_864_i64)?;
    verify_pragmas(connection, 1)
}

fn open_read_only_connection(
    path: &Path,
    expected_synchronous: i64,
) -> Result<Connection, StateError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(
        None,
        "synchronous",
        if expected_synchronous == 2 {
            "FULL"
        } else {
            "NORMAL"
        },
    )?;
    connection.pragma_update(None, "query_only", true)?;
    verify_pragmas(&connection, expected_synchronous)?;
    if pragma_i64(&connection, "query_only")? != 1 {
        return Err(StateError::PragmaMismatch {
            details: vec!["query_only=OFF".to_owned()],
        });
    }
    Ok(connection)
}

fn verify_pragmas(connection: &Connection, expected_synchronous: i64) -> Result<(), StateError> {
    let report = read_pragma_report(connection)?;
    let mut failures = Vec::new();
    if !report.journal_mode.eq_ignore_ascii_case("wal") {
        failures.push(format!("journal_mode={}", report.journal_mode));
    }
    if report.synchronous != expected_synchronous {
        failures.push(format!("synchronous={}", report.synchronous));
    }
    if !report.foreign_keys {
        failures.push("foreign_keys=OFF".to_owned());
    }
    if report.busy_timeout_ms != 5_000 {
        failures.push(format!("busy_timeout={}", report.busy_timeout_ms));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(StateError::PragmaMismatch { details: failures })
    }
}

fn read_pragma_report(connection: &Connection) -> Result<PragmaReport, StateError> {
    Ok(PragmaReport {
        journal_mode: connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?,
        synchronous: pragma_i64(connection, "synchronous")?,
        foreign_keys: pragma_i64(connection, "foreign_keys")? == 1,
        busy_timeout_ms: pragma_i64(connection, "busy_timeout")?,
        user_version: pragma_i64(connection, "user_version")?,
    })
}

fn pragma_i64(connection: &Connection, name: &str) -> Result<i64, StateError> {
    let sql = format!("PRAGMA {name}");
    Ok(connection.query_row(&sql, [], |row| row.get(0))?)
}

fn migrate_state(
    connection: &mut Connection,
    path: &Path,
    existing_nonempty: bool,
) -> Result<(), StateError> {
    let current = pragma_i64(connection, "user_version")?;
    if current > STATE_SCHEMA_VERSION {
        return Err(StateError::UnsupportedSchema {
            database: "helix-state.db",
            found: current,
            supported: STATE_SCHEMA_VERSION,
        });
    }
    if current < STATE_SCHEMA_VERSION && (current > 0 || existing_nonempty) {
        ensure_migration_snapshot(connection, path, current)?;
    }
    if current < 1 {
        apply_migration(connection, 1, "foundational-state", STATE_MIGRATION_1)?;
    }
    if current < 2 {
        security::migrate_security(connection)?;
    }
    if current < 3 {
        secrets::migrate_secrets(connection)?;
    }
    if current < 4 {
        security::migrate_audit_retention(connection)?;
    }
    Ok(())
}

fn ensure_migration_snapshot(
    connection: &Connection,
    path: &Path,
    source_schema_version: i64,
) -> Result<(), StateError> {
    let backup_dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("migration-backups");
    ensure_data_directory(&backup_dir)?;

    let source_identity = migration_source_identity(connection, path, source_schema_version)?;
    let pending = backup_dir.join(format!(
        ".helix-state-v{source_schema_version}-to-v{STATE_SCHEMA_VERSION}.pending.db"
    ));
    reconcile_migration_snapshot_staging(&pending, &backup_dir, source_schema_version)?;

    let alias_path = migration_alias_path(path, source_schema_version, &source_identity);
    if reuse_migration_snapshot_alias(
        &alias_path,
        &backup_dir,
        source_schema_version,
        &source_identity,
    )? {
        return Ok(());
    }

    let partial = with_suffix(&pending, ".partial");
    online_backup_with_temporary(connection, &pending, "helix-state.db", &partial)?;
    let snapshot_identity =
        publish_migration_snapshot(&pending, &backup_dir, source_schema_version)?;
    if migration_source_identity(connection, path, source_schema_version)? != source_identity {
        return Err(migration_alias_integrity(
            "migration source changed while its rollback snapshot was created",
        ));
    }
    publish_migration_snapshot_alias(
        &alias_path,
        &backup_dir,
        &MigrationSnapshotAlias {
            source_schema_version,
            target_schema_version: STATE_SCHEMA_VERSION,
            source_identity,
            snapshot_identity,
        },
    )
}

fn migration_source_identity(
    connection: &Connection,
    path: &Path,
    source_schema_version: i64,
) -> Result<String, StateError> {
    check_integrity(connection, "helix-state.db backup", true)?;
    validate_state_semantics(connection, source_schema_version, "helix-state.db backup")?;
    let (busy, log_frames, checkpointed_frames) =
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
    if busy != 0 || log_frames != checkpointed_frames {
        return Err(StateError::PragmaMismatch {
            details: vec![format!(
                "migration source checkpoint busy={busy}, log={log_frames}, checkpointed={checkpointed_frames}"
            )],
        });
    }
    migration_snapshot_identity(path)
}

fn reconcile_migration_snapshot_staging(
    pending: &Path,
    backup_dir: &Path,
    source_schema_version: i64,
) -> Result<(), StateError> {
    for path in [pending.to_path_buf(), with_suffix(pending, ".partial")] {
        if metadata_if_exists(&path, "inspect migration snapshot staging file")?.is_some() {
            reconcile_one_migration_partial(&path, backup_dir, source_schema_version)?;
        }
    }

    let pending_name = pending.file_name().and_then(|name| name.to_str()).ok_or(
        StateError::InvalidSecurityInput("migration snapshot path must be Unicode"),
    )?;
    let legacy_prefix = format!("{pending_name}.partial-");
    let entries = fs::read_dir(backup_dir).map_err(|source| StateError::Io {
        operation: "enumerate legacy migration snapshot partials",
        path: backup_dir.to_path_buf(),
        source,
    })?;
    let mut processed = 0_usize;
    for entry in entries {
        let entry = entry.map_err(|source| StateError::Io {
            operation: "read legacy migration snapshot partial entry",
            path: backup_dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(uuid) = name.strip_prefix(&legacy_prefix) else {
            continue;
        };
        if Uuid::parse_str(uuid).is_err() {
            continue;
        }
        if processed == MAX_LEGACY_MIGRATION_PARTIALS_PER_OPEN {
            return Err(StateError::Integrity {
                database: "helix-state.db backup",
                details: vec![format!(
                    "more than {MAX_LEGACY_MIGRATION_PARTIALS_PER_OPEN} legacy migration partials require reconciliation; retry startup"
                )],
            });
        }
        reconcile_one_migration_partial(&entry.path(), backup_dir, source_schema_version)?;
        processed += 1;
    }
    Ok(())
}

fn reconcile_one_migration_partial(
    path: &Path,
    backup_dir: &Path,
    source_schema_version: i64,
) -> Result<(), StateError> {
    match publish_migration_snapshot(path, backup_dir, source_schema_version) {
        Ok(_) => Ok(()),
        Err(error) if error.indicates_corruption() => {
            remove_migration_snapshot_artifacts(path, "remove invalid migration snapshot partial")
        }
        Err(error) => Err(error),
    }
}

fn publish_migration_snapshot(
    path: &Path,
    backup_dir: &Path,
    source_schema_version: i64,
) -> Result<String, StateError> {
    normalize_migration_snapshot(path)?;
    verify_migration_snapshot(path, source_schema_version)?;
    let identity = migration_snapshot_identity(path)?;
    let destination = migration_snapshot_path(backup_dir, source_schema_version, &identity);

    if metadata_if_exists(&destination, "inspect migration snapshot")?.is_some() {
        verify_migration_snapshot(&destination, source_schema_version)?;
        if migration_snapshot_identity(&destination)? != identity {
            return Err(StateError::Integrity {
                database: "helix-state.db backup",
                details: vec!["snapshot content does not match its identity".to_owned()],
            });
        }
    } else {
        fs::hard_link(path, &destination).map_err(|source| StateError::Io {
            operation: "publish content-addressed migration snapshot",
            path: destination.clone(),
            source,
        })?;
        validate_regular_file(&destination)?;
        sync_parent(&destination)?;
        verify_migration_snapshot(&destination, source_schema_version)?;
    }

    remove_migration_snapshot_artifacts(path, "remove published migration snapshot staging file")?;
    Ok(identity)
}

fn migration_snapshot_path(
    backup_dir: &Path,
    source_schema_version: i64,
    identity: &str,
) -> PathBuf {
    backup_dir.join(format!(
        "helix-state-v{source_schema_version}-to-v{STATE_SCHEMA_VERSION}-{identity}.db"
    ))
}

fn migration_alias_path(
    database_path: &Path,
    source_schema_version: i64,
    source_identity: &str,
) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".helix-state-v{source_schema_version}-to-v{STATE_SCHEMA_VERSION}-source-{source_identity}.snapshot-ref"
        ))
}

fn reuse_migration_snapshot_alias(
    alias_path: &Path,
    backup_dir: &Path,
    source_schema_version: i64,
    source_identity: &str,
) -> Result<bool, StateError> {
    let pending_alias = with_suffix(alias_path, ".pending");
    if metadata_if_exists(alias_path, "inspect migration snapshot alias")?.is_some() {
        let alias = read_migration_snapshot_alias(alias_path)?;
        validate_migration_snapshot_alias(&alias, source_schema_version, source_identity)?;
        verify_aliased_migration_snapshot(&alias, backup_dir)?;
        if metadata_if_exists(&pending_alias, "inspect pending migration snapshot alias")?.is_some()
        {
            remove_migration_alias_file(&pending_alias)?;
        }
        return Ok(true);
    }

    if metadata_if_exists(&pending_alias, "inspect pending migration snapshot alias")?.is_none() {
        return Ok(false);
    }
    let alias = match read_migration_snapshot_alias(&pending_alias) {
        Ok(alias) => alias,
        Err(error) if error.indicates_corruption() => {
            remove_migration_alias_file(&pending_alias)?;
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    validate_migration_snapshot_alias(&alias, source_schema_version, source_identity)?;
    verify_aliased_migration_snapshot(&alias, backup_dir)?;
    publish_pending_migration_alias(&pending_alias, alias_path, &alias)?;
    Ok(true)
}

fn publish_migration_snapshot_alias(
    alias_path: &Path,
    backup_dir: &Path,
    alias: &MigrationSnapshotAlias,
) -> Result<(), StateError> {
    verify_aliased_migration_snapshot(alias, backup_dir)?;
    if metadata_if_exists(alias_path, "inspect migration snapshot alias")?.is_some() {
        let existing = read_migration_snapshot_alias(alias_path)?;
        if existing != *alias {
            return Err(migration_alias_integrity(
                "existing alias does not match the current migration source",
            ));
        }
        return Ok(());
    }

    let pending_alias = with_suffix(alias_path, ".pending");
    if metadata_if_exists(&pending_alias, "inspect pending migration snapshot alias")?.is_some() {
        remove_migration_alias_file(&pending_alias)?;
    }
    create_private_file(&pending_alias)?;
    let contents = encode_migration_snapshot_alias(alias);
    let write_result = (|| -> Result<(), StateError> {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&pending_alias)
            .map_err(|source| StateError::Io {
                operation: "open pending migration snapshot alias",
                path: pending_alias.clone(),
                source,
            })?;
        file.write_all(contents.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|source| StateError::Io {
                operation: "write pending migration snapshot alias",
                path: pending_alias.clone(),
                source,
            })
    })();
    if let Err(error) = write_result {
        let _ = remove_migration_alias_file(&pending_alias);
        return Err(error);
    }
    let written = read_migration_snapshot_alias(&pending_alias)?;
    if written != *alias {
        return Err(migration_alias_integrity(
            "pending alias did not preserve its intended contents",
        ));
    }
    publish_pending_migration_alias(&pending_alias, alias_path, alias)
}

fn publish_pending_migration_alias(
    pending_alias: &Path,
    alias_path: &Path,
    expected: &MigrationSnapshotAlias,
) -> Result<(), StateError> {
    fs::hard_link(pending_alias, alias_path).map_err(|source| StateError::Io {
        operation: "publish migration snapshot alias",
        path: alias_path.to_path_buf(),
        source,
    })?;
    validate_regular_file(alias_path)?;
    if read_migration_snapshot_alias(alias_path)? != *expected {
        return Err(migration_alias_integrity(
            "published alias does not match its verified pending file",
        ));
    }
    sync_parent(alias_path)?;
    remove_migration_alias_file(pending_alias)
}

fn verify_aliased_migration_snapshot(
    alias: &MigrationSnapshotAlias,
    backup_dir: &Path,
) -> Result<(), StateError> {
    let snapshot = migration_snapshot_path(
        backup_dir,
        alias.source_schema_version,
        &alias.snapshot_identity,
    );
    verify_migration_snapshot(&snapshot, alias.source_schema_version)?;
    if migration_snapshot_identity(&snapshot)? != alias.snapshot_identity {
        return Err(migration_alias_integrity(
            "aliased snapshot content does not match its recorded identity",
        ));
    }
    Ok(())
}

fn validate_migration_snapshot_alias(
    alias: &MigrationSnapshotAlias,
    source_schema_version: i64,
    source_identity: &str,
) -> Result<(), StateError> {
    if alias.source_schema_version != source_schema_version
        || alias.target_schema_version != STATE_SCHEMA_VERSION
        || alias.source_identity != source_identity
        || !is_sha256_identity(&alias.snapshot_identity)
    {
        return Err(migration_alias_integrity(
            "alias binding does not match the current migration source",
        ));
    }
    Ok(())
}

fn encode_migration_snapshot_alias(alias: &MigrationSnapshotAlias) -> String {
    format!(
        "{MIGRATION_ALIAS_FORMAT}\nsource_schema_version={}\ntarget_schema_version={}\nsource_sha256={}\nsnapshot_sha256={}\n",
        alias.source_schema_version,
        alias.target_schema_version,
        alias.source_identity,
        alias.snapshot_identity
    )
}

fn read_migration_snapshot_alias(path: &Path) -> Result<MigrationSnapshotAlias, StateError> {
    validate_regular_file(path)?;
    let metadata = fs::metadata(path).map_err(|source| StateError::Io {
        operation: "inspect migration snapshot alias size",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_MIGRATION_ALIAS_BYTES {
        return Err(migration_alias_integrity("alias exceeds its size limit"));
    }
    let bytes = fs::read(path).map_err(|source| StateError::Io {
        operation: "read migration snapshot alias",
        path: path.to_path_buf(),
        source,
    })?;
    let contents = std::str::from_utf8(&bytes)
        .map_err(|_| migration_alias_integrity("alias is not valid UTF-8"))?;
    let mut lines = contents.lines();
    if lines.next() != Some(MIGRATION_ALIAS_FORMAT) {
        return Err(migration_alias_integrity("alias format marker is invalid"));
    }
    let source_schema_version = parse_migration_alias_i64(
        lines.next(),
        "source_schema_version=",
        "source schema version",
    )?;
    let target_schema_version = parse_migration_alias_i64(
        lines.next(),
        "target_schema_version=",
        "target schema version",
    )?;
    let source_identity = parse_migration_alias_identity(lines.next(), "source_sha256=")?;
    let snapshot_identity = parse_migration_alias_identity(lines.next(), "snapshot_sha256=")?;
    if lines.next().is_some() {
        return Err(migration_alias_integrity(
            "alias has unexpected extra fields",
        ));
    }
    Ok(MigrationSnapshotAlias {
        source_schema_version,
        target_schema_version,
        source_identity,
        snapshot_identity,
    })
}

fn parse_migration_alias_i64(
    line: Option<&str>,
    prefix: &str,
    field: &str,
) -> Result<i64, StateError> {
    line.and_then(|line| line.strip_prefix(prefix))
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| migration_alias_integrity(&format!("alias {field} is invalid")))
}

fn parse_migration_alias_identity(line: Option<&str>, prefix: &str) -> Result<String, StateError> {
    let identity = line
        .and_then(|line| line.strip_prefix(prefix))
        .filter(|identity| is_sha256_identity(identity))
        .ok_or_else(|| migration_alias_integrity("alias SHA-256 identity is invalid"))?;
    Ok(identity.to_owned())
}

fn is_sha256_identity(identity: &str) -> bool {
    identity.len() == 64
        && identity
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn migration_alias_integrity(detail: &str) -> StateError {
    StateError::Integrity {
        database: "helix-state.db backup",
        details: vec![detail.to_owned()],
    }
}

fn remove_migration_alias_file(path: &Path) -> Result<(), StateError> {
    validate_regular_file(path)?;
    fs::remove_file(path).map_err(|source| StateError::Io {
        operation: "remove pending migration snapshot alias",
        path: path.to_path_buf(),
        source,
    })?;
    sync_parent(path)
}

fn remove_migration_snapshot_artifacts(
    path: &Path,
    operation: &'static str,
) -> Result<(), StateError> {
    let mut artifacts = Vec::new();
    for artifact in [
        with_suffix(path, "-wal"),
        with_suffix(path, "-shm"),
        path.to_path_buf(),
    ] {
        if metadata_if_exists(&artifact, "inspect migration snapshot artifact")?.is_some() {
            validate_regular_file(&artifact)?;
            artifacts.push(artifact);
        }
    }
    for artifact in artifacts {
        fs::remove_file(&artifact).map_err(|source| StateError::Io {
            operation,
            path: artifact,
            source,
        })?;
    }
    sync_parent(path)
}

fn normalize_migration_snapshot(path: &Path) -> Result<(), StateError> {
    validate_database_artifacts(path)?;
    let snapshot = Connection::open(path)?;
    let journal_mode = snapshot.query_row("PRAGMA journal_mode=DELETE", [], |row| {
        row.get::<_, String>(0)
    })?;
    if !journal_mode.eq_ignore_ascii_case("delete") {
        return Err(StateError::PragmaMismatch {
            details: vec![format!("migration snapshot journal_mode={journal_mode}")],
        });
    }
    drop(snapshot);
    sync_file(path)?;
    validate_database_artifacts(path)
}

fn verify_migration_snapshot(path: &Path, expected_schema_version: i64) -> Result<(), StateError> {
    validate_database_artifacts(path)?;
    let snapshot = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    check_integrity(&snapshot, "helix-state.db backup", true)?;
    validate_state_semantics(&snapshot, expected_schema_version, "helix-state.db backup")
}

fn migration_snapshot_identity(path: &Path) -> Result<String, StateError> {
    validate_regular_file(path)?;
    let mut file = File::open(path).map_err(|source| StateError::Io {
        operation: "open migration snapshot for identity",
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| StateError::Io {
            operation: "read migration snapshot for identity",
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(encoded)
}

fn reject_newer_schema(
    connection: &Connection,
    database: &'static str,
    supported: i64,
) -> Result<(), StateError> {
    let found = pragma_i64(connection, "user_version")?;
    if found > supported {
        Err(StateError::UnsupportedSchema {
            database,
            found,
            supported,
        })
    } else {
        Ok(())
    }
}

fn migrate_metrics(connection: &mut Connection) -> Result<(), StateError> {
    let current = pragma_i64(connection, "user_version")?;
    if current > METRICS_SCHEMA_VERSION {
        return Err(StateError::UnsupportedSchema {
            database: "helix-metrics.db",
            found: current,
            supported: METRICS_SCHEMA_VERSION,
        });
    }
    if current < 1 {
        apply_migration(connection, 1, "foundational-metrics", METRICS_MIGRATION_1)?;
    }
    Ok(())
}

fn apply_migration(
    connection: &mut Connection,
    version: i64,
    name: &str,
    sql: &str,
) -> Result<(), StateError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(sql)?;
    #[cfg(test)]
    if FAIL_NEXT_MIGRATION_VERSION.with(|target| {
        let should_fail = target.get() == Some(version);
        if should_fail {
            target.set(None);
        }
        should_fail
    }) {
        return Err(StateError::Sqlite(rusqlite::Error::ExecuteReturnedResults));
    }
    transaction.execute(
        "INSERT INTO schema_migrations (version, name, applied_at_unix_ms)
         VALUES (?1, ?2, ?3)",
        params![version, name, timestamp_i64()],
    )?;
    transaction.pragma_update(None, "user_version", version)?;
    transaction.commit()?;
    Ok(())
}

fn ensure_installation(connection: &mut Connection) -> Result<String, StateError> {
    if let Some(id) = connection
        .query_row(
            "SELECT id FROM installation WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }

    let installation_id = random_uuid_v4()?.to_string();
    let node_id = random_uuid_v4()?.to_string();
    let timestamp = timestamp_i64();
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO installation (singleton, id, created_at_unix_ms, clean_shutdown)
         VALUES (1, ?1, ?2, 1)",
        params![installation_id, timestamp],
    )?;
    transaction.execute(
        "INSERT INTO nodes (id, kind, display_name, created_at_unix_ms)
         VALUES (?1, 'local', 'Local node', ?2)",
        params![node_id, timestamp],
    )?;
    transaction.commit()?;
    Ok(installation_id)
}

fn check_integrity(
    connection: &Connection,
    database: &'static str,
    full: bool,
) -> Result<(), StateError> {
    let pragma = if full {
        "PRAGMA integrity_check"
    } else {
        "PRAGMA quick_check"
    };
    let mut statement = connection.prepare(pragma)?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if messages.len() != 1 || !messages[0].eq_ignore_ascii_case("ok") {
        return Err(StateError::Integrity {
            database,
            details: messages,
        });
    }

    let mut foreign_key_statement = connection.prepare("PRAGMA foreign_key_check")?;
    let mut rows = foreign_key_statement.query([])?;
    let mut foreign_key_failures = Vec::new();
    while let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        let row_id: Option<i64> = row.get(1)?;
        foreign_key_failures.push(format!("table={table}, row={row_id:?}"));
    }
    if !foreign_key_failures.is_empty() {
        return Err(StateError::Integrity {
            database,
            details: foreign_key_failures,
        });
    }
    Ok(())
}

fn validate_state_semantics(
    connection: &Connection,
    expected_schema_version: i64,
    database: &'static str,
) -> Result<(), StateError> {
    let actual_schema_version = pragma_i64(connection, "user_version")?;
    let mut failures = Vec::new();
    if actual_schema_version != expected_schema_version {
        failures.push(format!(
            "snapshot user_version={actual_schema_version}, expected {expected_schema_version}"
        ));
    }
    if expected_schema_version == 0 {
        return if failures.is_empty() {
            Ok(())
        } else {
            Err(StateError::Integrity {
                database,
                details: failures,
            })
        };
    }

    let mut required_tables = vec![
        "schema_migrations",
        "installation",
        "nodes",
        "operation_ledger",
    ];
    if expected_schema_version >= 2 {
        required_tables.extend([
            "security_state",
            "users",
            "bootstrap_tokens",
            "sessions",
            "roles",
            "capabilities",
            "user_roles",
            "role_capabilities",
            "audit_events",
        ]);
    }
    if expected_schema_version >= 3 {
        required_tables.extend(["master_key_versions", "secret_records"]);
    }
    if expected_schema_version >= 4 {
        required_tables.push("audit_retention_state");
    }
    for table in required_tables {
        let strict = connection
            .query_row(
                "SELECT strict FROM pragma_table_list WHERE name = ?1 AND schema = 'main'",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        match strict {
            Some(1) => {}
            Some(value) => failures.push(format!("required table {table} has strict={value}")),
            None => failures.push(format!("required table {table} is missing")),
        }
    }
    if !failures.is_empty() {
        return Err(StateError::Integrity {
            database,
            details: failures,
        });
    }

    let migration_rows = connection
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let expected_migrations = match expected_schema_version {
        1 => vec![(1, "foundational-state".to_owned())],
        2 => vec![
            (1, "foundational-state".to_owned()),
            (2, "owner-authentication".to_owned()),
        ],
        3 => vec![
            (1, "foundational-state".to_owned()),
            (2, "owner-authentication".to_owned()),
            (3, "recoverable-secret-storage".to_owned()),
        ],
        4 => vec![
            (1, "foundational-state".to_owned()),
            (2, "owner-authentication".to_owned()),
            (3, "recoverable-secret-storage".to_owned()),
            (4, "bounded-authentication-audit-retention".to_owned()),
        ],
        _ => {
            return Err(StateError::UnsupportedSchema {
                database,
                found: expected_schema_version,
                supported: STATE_SCHEMA_VERSION,
            });
        }
    };
    if migration_rows != expected_migrations {
        failures.push("schema_migrations rows do not match the declared state schema".to_owned());
    }

    if expected_schema_version >= 2 {
        let security_seeded = connection.query_row(
            "SELECT
                EXISTS(SELECT 1 FROM security_state
                       WHERE singleton = 1 AND security_schema_version = 1)
                AND EXISTS(
                    SELECT 1
                    FROM roles r
                    JOIN role_capabilities rc ON rc.role_id = r.id
                    WHERE r.name = 'owner' AND r.is_system = 1
                          AND rc.capability = 'system.view'
                )",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !security_seeded {
            failures.push("required security seed rows are missing".to_owned());
        }
        for trigger in [
            "audit_events_append_only_update",
            "audit_events_append_only_delete",
            "users_auth_version_monotonic",
            "users_credential_change_requires_new_auth_version",
            "user_roles_auth_version_insert",
            "user_roles_auth_version_delete",
            "role_capabilities_auth_version_insert",
            "role_capabilities_auth_version_delete",
        ] {
            let exists = connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_schema
                    WHERE type = 'trigger' AND name = ?1
                 )",
                [trigger],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                failures.push(format!("required trigger {trigger} is missing"));
            }
        }
    }
    if expected_schema_version >= 3 {
        for trigger in [
            "master_key_versions_identity_immutable",
            "master_key_retire_without_references",
            "secret_records_identity_immutable",
            "secret_records_revisions_monotonic",
        ] {
            let exists = connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_schema
                    WHERE type = 'trigger' AND name = ?1
                 )",
                [trigger],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                failures.push(format!("required trigger {trigger} is missing"));
            }
        }
        let active_key_count = connection.query_row(
            "SELECT count(*) FROM master_key_versions
             WHERE status = 'active' AND active_slot = 1",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        if active_key_count > 1 {
            failures.push("more than one active master key exists".to_owned());
        }
    }
    if expected_schema_version >= 4 {
        let retention_index_exists = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema
                WHERE type = 'index' AND name = 'audit_events_retention_priority_idx'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !retention_index_exists {
            failures
                .push("required index audit_events_retention_priority_idx is missing".to_owned());
        }
        for trigger in [
            "audit_retention_policy_immutable",
            "audit_retention_state_append_only",
            "audit_events_retention_count_insert",
            "audit_events_retention_count_delete",
        ] {
            let exists = connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_schema
                    WHERE type = 'trigger' AND name = ?1
                 )",
                [trigger],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                failures.push(format!("required trigger {trigger} is missing"));
            }
        }
        let retention_valid = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM audit_retention_state
                 WHERE singleton = 1
                       AND retention_window_ms = 7776000000
                       AND minimum_rows = 1024
                       AND maximum_rows = 50000
                       AND prune_batch = 256
                       AND retained_event_count = (
                           SELECT count(*) FROM audit_events
                       )
             )",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !retention_valid {
            failures.push("authentication audit retention state is invalid".to_owned());
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(StateError::Integrity {
            database,
            details: failures,
        })
    }
}

fn online_backup(
    source: &Connection,
    destination: &Path,
    database: &'static str,
) -> Result<BackupOutcome, StateError> {
    let temporary = temporary_sibling(destination)?;
    online_backup_with_temporary(source, destination, database, &temporary)
}

fn online_backup_with_temporary(
    source: &Connection,
    destination: &Path,
    database: &'static str,
    temporary: &Path,
) -> Result<BackupOutcome, StateError> {
    let source_schema_version = pragma_i64(source, "user_version")?;
    require_path_absent(destination)?;
    require_path_absent(temporary)?;
    if let Some(parent) = destination.parent()
        && !parent.exists()
    {
        ensure_data_directory(parent)?;
    }

    let result = (|| -> Result<BackupOutcome, StateError> {
        create_private_file(temporary)?;
        #[cfg(test)]
        ONLINE_BACKUP_INVOCATIONS.with(|count| count.set(count.get().saturating_add(1)));
        source.backup(MAIN_DB, temporary, None)?;
        validate_regular_file(temporary)?;
        let backup = Connection::open(temporary)?;
        check_integrity(&backup, database, true)?;
        if database == "helix-state.db" {
            validate_state_semantics(&backup, source_schema_version, "helix-state.db backup")?;
        }
        drop(backup);
        sync_file(temporary)?;
        fs::hard_link(temporary, destination).map_err(|source| StateError::Io {
            operation: "publish verified database backup",
            path: destination.to_path_buf(),
            source,
        })?;
        validate_regular_file(destination)?;
        sync_file(destination)?;
        sync_parent(destination)?;
        if remove_published_backup_temporary(temporary) {
            let _ = sync_parent(destination);
            Ok(BackupOutcome::Published)
        } else {
            Ok(BackupOutcome::PublishedWithResidue {
                temporary_path: temporary.to_path_buf(),
            })
        }
    })();
    if result.is_err() && temporary.exists() {
        let _ = remove_migration_snapshot_artifacts(
            temporary,
            "remove failed database backup temporary file",
        );
    }
    result
}

fn remove_published_backup_temporary(path: &Path) -> bool {
    for _ in 0..PUBLISHED_BACKUP_CLEANUP_ATTEMPTS {
        #[cfg(test)]
        let inject_failure = FAIL_PUBLISHED_BACKUP_CLEANUP_ATTEMPTS.with(|remaining| {
            let current = remaining.get();
            remaining.set(current.saturating_sub(1));
            current > 0
        });
        #[cfg(not(test))]
        let inject_failure = false;
        if inject_failure {
            std::thread::yield_now();
            continue;
        }
        match fs::remove_file(path) {
            Ok(()) => return true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return true,
            Err(_) => std::thread::yield_now(),
        }
    }
    false
}

impl MetricsForensicManifest {
    fn encode(&self) -> String {
        format!(
            "format={METRICS_FORENSIC_FORMAT}\ndestination={}\nwal={}\nshm={}\n",
            self.destination_name,
            if self.wal_present {
                "present"
            } else {
                "absent"
            },
            if self.shm_present {
                "present"
            } else {
                "absent"
            }
        )
    }

    fn parse(value: &str) -> Result<Self, StateError> {
        let mut lines = value.lines();
        let format = lines.next();
        let destination = lines
            .next()
            .and_then(|line| line.strip_prefix("destination="));
        let wal = lines.next().and_then(|line| line.strip_prefix("wal="));
        let shm = lines.next().and_then(|line| line.strip_prefix("shm="));
        if format.and_then(|line| line.strip_prefix("format=")) != Some(METRICS_FORENSIC_FORMAT)
            || lines.next().is_some()
        {
            return Err(metrics_forensic_integrity("manifest structure is invalid"));
        }
        let destination_name = destination
            .filter(|name| valid_metrics_forensic_destination_name(name))
            .ok_or_else(|| metrics_forensic_integrity("manifest destination is invalid"))?
            .to_owned();
        Ok(Self {
            destination_name,
            wal_present: parse_metrics_forensic_presence(wal, "WAL")?,
            shm_present: parse_metrics_forensic_presence(shm, "SHM")?,
        })
    }
}

fn parse_metrics_forensic_presence(
    value: Option<&str>,
    component: &'static str,
) -> Result<bool, StateError> {
    match value {
        Some("present") => Ok(true),
        Some("absent") => Ok(false),
        _ => Err(metrics_forensic_integrity(&format!(
            "manifest {component} presence is invalid"
        ))),
    }
}

fn valid_metrics_forensic_destination_name(name: &str) -> bool {
    let Some(value) = name.strip_prefix("helix-metrics.db.corrupt-") else {
        return false;
    };
    let Some((timestamp, uuid)) = value.split_once('-') else {
        return false;
    };
    !timestamp.is_empty()
        && timestamp.bytes().all(|byte| byte.is_ascii_digit())
        && Uuid::parse_str(uuid).is_ok()
}

fn metrics_forensic_integrity(detail: &str) -> StateError {
    StateError::Integrity {
        database: "helix-metrics.db forensic set",
        details: vec![detail.to_owned()],
    }
}

fn metrics_forensic_staging_path(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(METRICS_FORENSIC_STAGING_DIR)
}

fn metrics_forensic_staged_component(staging: &Path, source: &Path) -> Result<PathBuf, StateError> {
    let name = source
        .file_name()
        .ok_or_else(|| metrics_forensic_integrity("database component has no file name"))?;
    Ok(staging.join(name))
}

fn read_metrics_forensic_manifest(path: &Path) -> Result<MetricsForensicManifest, StateError> {
    validate_regular_file(path)?;
    let metadata = fs::metadata(path).map_err(|source| StateError::Io {
        operation: "inspect metrics forensic manifest",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_METRICS_FORENSIC_MANIFEST_BYTES {
        return Err(metrics_forensic_integrity("manifest is oversized"));
    }
    let mut value = String::new();
    File::open(path)
        .and_then(|mut file| file.read_to_string(&mut value))
        .map_err(|source| StateError::Io {
            operation: "read metrics forensic manifest",
            path: path.to_path_buf(),
            source,
        })?;
    MetricsForensicManifest::parse(&value)
}

fn write_metrics_forensic_manifest(
    staging: &Path,
    manifest: &MetricsForensicManifest,
) -> Result<(), StateError> {
    let partial = staging.join(METRICS_FORENSIC_MANIFEST_PARTIAL);
    let destination = staging.join(METRICS_FORENSIC_MANIFEST);
    create_private_file(&partial)?;
    let encoded = manifest.encode();
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&partial)
        .and_then(|mut file| {
            file.write_all(encoded.as_bytes())?;
            file.sync_all()
        })
        .map_err(|source| StateError::Io {
            operation: "write metrics forensic manifest",
            path: partial.clone(),
            source,
        })?;
    fs::rename(&partial, &destination).map_err(|source| StateError::Io {
        operation: "publish metrics forensic manifest",
        path: destination.clone(),
        source,
    })?;
    sync_parent(&destination)
}

fn discard_unstarted_metrics_forensic_staging(
    staging: &Path,
    source_database: &Path,
) -> Result<bool, StateError> {
    if metadata_if_exists(
        source_database,
        "inspect metrics database during reconciliation",
    )?
    .is_none()
    {
        return Ok(false);
    }
    let mut removable = Vec::new();
    let entries = fs::read_dir(staging).map_err(|source| StateError::Io {
        operation: "enumerate metrics forensic staging directory",
        path: staging.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| StateError::Io {
            operation: "read metrics forensic staging entry",
            path: staging.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        if name != METRICS_FORENSIC_MANIFEST && name != METRICS_FORENSIC_MANIFEST_PARTIAL {
            return Ok(false);
        }
        validate_regular_file(&entry.path())?;
        removable.push(entry.path());
        if removable.len() > 2 {
            return Ok(false);
        }
    }
    for path in removable {
        fs::remove_file(&path).map_err(|source| StateError::Io {
            operation: "remove unstarted metrics forensic manifest",
            path,
            source,
        })?;
    }
    fs::remove_dir(staging).map_err(|source| StateError::Io {
        operation: "remove unstarted metrics forensic staging directory",
        path: staging.to_path_buf(),
        source,
    })?;
    sync_parent(staging)?;
    Ok(true)
}

fn reconcile_metrics_forensic_staging(path: &Path) -> Result<Option<PathBuf>, StateError> {
    let staging = metrics_forensic_staging_path(path);
    if metadata_if_exists(&staging, "inspect metrics forensic staging directory")?.is_none() {
        return Ok(None);
    }
    ensure_data_directory(&staging)?;
    let manifest_path = staging.join(METRICS_FORENSIC_MANIFEST);
    if metadata_if_exists(&manifest_path, "inspect metrics forensic manifest")?.is_none() {
        if discard_unstarted_metrics_forensic_staging(&staging, path)? {
            return Ok(None);
        }
        return Err(metrics_forensic_integrity(
            "staging directory has no durable manifest",
        ));
    }
    let manifest = match read_metrics_forensic_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            if discard_unstarted_metrics_forensic_staging(&staging, path)? {
                return Ok(None);
            }
            return Err(error);
        }
    };
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let destination = parent.join(&manifest.destination_name);
    if metadata_if_exists(&destination, "inspect metrics forensic destination")?.is_some() {
        return Err(metrics_forensic_integrity(
            "forensic destination already exists while staging remains",
        ));
    }

    let source_wal = with_suffix(path, "-wal");
    let source_shm = with_suffix(path, "-shm");
    let components = [
        (path.to_path_buf(), true),
        (source_wal, manifest.wal_present),
        (source_shm, manifest.shm_present),
    ];
    let mut moved = 0_usize;
    for (source, expected) in &components {
        let staged = metrics_forensic_staged_component(&staging, source)?;
        let source_exists =
            metadata_if_exists(source, "inspect metrics forensic source")?.is_some();
        let staged_exists =
            metadata_if_exists(&staged, "inspect staged metrics component")?.is_some();
        if source_exists {
            validate_regular_file(source)?;
        }
        if staged_exists {
            validate_regular_file(&staged)?;
        }
        match (*expected, source_exists, staged_exists) {
            (true, true, false) => {
                fs::rename(source, &staged).map_err(|source_error| StateError::Io {
                    operation: "stage corrupt metrics component",
                    path: source.clone(),
                    source: source_error,
                })?;
                sync_parent(source)?;
                sync_parent(&staged)?;
                moved = moved.saturating_add(1);
                maybe_fail_metrics_forensic_move_for_test(moved)?;
            }
            (true, false, true) | (false, false, false) => {}
            (true, true, true) => {
                return Err(metrics_forensic_integrity(
                    "a metrics component exists in both source and staging",
                ));
            }
            (true, false, false) => {
                return Err(metrics_forensic_integrity(
                    "an expected metrics component is missing",
                ));
            }
            (false, true, _) | (false, _, true) => {
                return Err(metrics_forensic_integrity(
                    "an unexpected metrics sidecar appeared during preservation",
                ));
            }
        }
    }

    let mut expected_names = vec![OsString::from(METRICS_FORENSIC_MANIFEST)];
    for (source, expected) in &components {
        if *expected {
            expected_names.push(
                source
                    .file_name()
                    .ok_or_else(|| metrics_forensic_integrity("component has no file name"))?
                    .to_owned(),
            );
        }
    }
    let entries = fs::read_dir(&staging).map_err(|source| StateError::Io {
        operation: "verify metrics forensic staging directory",
        path: staging.clone(),
        source,
    })?;
    let mut entry_count = 0_usize;
    for entry in entries {
        let entry = entry.map_err(|source| StateError::Io {
            operation: "read metrics forensic staging entry",
            path: staging.clone(),
            source,
        })?;
        entry_count = entry_count.saturating_add(1);
        if entry_count > expected_names.len() || !expected_names.contains(&entry.file_name()) {
            return Err(metrics_forensic_integrity(
                "staging directory contains an unexpected entry",
            ));
        }
        validate_regular_file(&entry.path())?;
        sync_file(&entry.path())?;
    }
    if entry_count != expected_names.len() {
        return Err(metrics_forensic_integrity(
            "staging directory is missing a forensic component",
        ));
    }
    sync_parent(&manifest_path)?;
    fs::rename(&staging, &destination).map_err(|source| StateError::Io {
        operation: "publish metrics forensic set",
        path: destination.clone(),
        source,
    })?;
    sync_parent(&destination)?;
    ensure_data_directory(&destination)?;
    Ok(Some(destination))
}

#[cfg(test)]
fn maybe_fail_metrics_forensic_move_for_test(moved: usize) -> Result<(), StateError> {
    let should_fail = FAIL_METRICS_FORENSIC_AFTER_MOVES.with(|target| {
        if target.get() == Some(moved) {
            target.set(None);
            true
        } else {
            false
        }
    });
    if should_fail {
        Err(StateError::Io {
            operation: "injected metrics forensic move interruption",
            path: PathBuf::from(METRICS_FORENSIC_STAGING_DIR),
            source: io::Error::other("injected metrics forensic move interruption"),
        })
    } else {
        Ok(())
    }
}

#[cfg(not(test))]
fn maybe_fail_metrics_forensic_move_for_test(_moved: usize) -> Result<(), StateError> {
    Ok(())
}

fn preserve_corrupt_database(path: &Path) -> Result<PathBuf, StateError> {
    if let Some(reconciled) = reconcile_metrics_forensic_staging(path)? {
        return Ok(reconciled);
    }
    validate_database_artifacts(path)?;
    let wal_present =
        metadata_if_exists(&with_suffix(path, "-wal"), "inspect metrics WAL")?.is_some();
    let shm_present =
        metadata_if_exists(&with_suffix(path, "-shm"), "inspect metrics SHM")?.is_some();
    let manifest = MetricsForensicManifest {
        destination_name: format!(
            "helix-metrics.db.corrupt-{}-{}",
            unix_timestamp_ms(),
            random_uuid_v4()?
        ),
        wal_present,
        shm_present,
    };
    let staging = metrics_forensic_staging_path(path);
    if metadata_if_exists(&staging, "inspect metrics forensic staging directory")?.is_some() {
        return Err(metrics_forensic_integrity(
            "metrics forensic staging was not reconciled",
        ));
    }
    ensure_data_directory(&staging)?;
    write_metrics_forensic_manifest(&staging, &manifest)?;
    reconcile_metrics_forensic_staging(path)?.ok_or_else(|| {
        metrics_forensic_integrity("metrics forensic staging disappeared before publication")
    })
}

fn temporary_sibling(destination: &Path) -> Result<PathBuf, StateError> {
    let mut name = destination.as_os_str().to_owned();
    name.push(format!(".partial-{}", random_uuid_v4()?));
    Ok(PathBuf::from(name))
}

fn random_uuid_v4() -> Result<Uuid, StateError> {
    #[cfg(test)]
    if FAIL_NEXT_UUID_GENERATION.with(|fail| fail.replace(false)) {
        return Err(StateError::RandomSource);
    }

    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| StateError::RandomSource)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_UUID_GENERATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_MIGRATION_VERSION: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
    static ONLINE_BACKUP_INVOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FORCE_VERIFIED_LOGIN_CAS_MISS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_PUBLISHED_BACKUP_CLEANUP_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FAIL_METRICS_FORENSIC_AFTER_MOVES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn fail_next_uuid_generation_for_test() {
    FAIL_NEXT_UUID_GENERATION.with(|fail| fail.set(true));
}

#[cfg(test)]
fn force_verified_login_cas_miss_for_test() {
    FORCE_VERIFIED_LOGIN_CAS_MISS.with(|force| force.set(true));
}

#[cfg(test)]
fn take_verified_login_cas_miss_for_test() -> bool {
    FORCE_VERIFIED_LOGIN_CAS_MISS.with(|force| force.replace(false))
}

#[cfg(test)]
fn fail_published_backup_cleanup_for_test(attempts: usize) {
    FAIL_PUBLISHED_BACKUP_CLEANUP_ATTEMPTS.with(|remaining| remaining.set(attempts));
}

#[cfg(test)]
fn fail_metrics_forensic_after_moves_for_test(moves: usize) {
    FAIL_METRICS_FORENSIC_AFTER_MOVES.with(|target| target.set(Some(moves)));
}

#[cfg(test)]
fn fail_next_migration_for_test(version: i64) {
    FAIL_NEXT_MIGRATION_VERSION.with(|target| target.set(Some(version)));
}

#[cfg(test)]
fn reset_online_backup_invocations_for_test() {
    ONLINE_BACKUP_INVOCATIONS.with(|count| count.set(0));
}

#[cfg(test)]
fn online_backup_invocations_for_test() -> usize {
    ONLINE_BACKUP_INVOCATIONS.with(std::cell::Cell::get)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn ensure_data_directory(path: &Path) -> Result<(), StateError> {
    let existed = metadata_if_exists(path, "inspect data directory")?.is_some();
    if !existed {
        create_directory(path)?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|source| StateError::Io {
        operation: "inspect data directory",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(StateError::DataDirectorySymlink(path.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(StateError::DataPathNotDirectory(path.to_path_buf()));
    }
    if !existed {
        restrict_directory_permissions(path)?;
    }
    verify_private_unix_path(path, &metadata, 0o700)?;
    Ok(())
}

fn acquire_daemon_lease(data_dir: &Path) -> Result<DaemonLease, StateError> {
    let path = data_dir.join(DAEMON_LEASE_FILE);
    prepare_private_file(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| StateError::Io {
            operation: "open daemon lease",
            path: path.clone(),
            source,
        })?;
    validate_regular_file(&path)?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(DaemonLease { _file: file }),
        Err(error)
            if error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                || error.kind() == io::ErrorKind::WouldBlock =>
        {
            Err(StateError::DaemonLeaseHeld(path))
        }
        Err(source) => Err(StateError::Io {
            operation: "acquire daemon lease",
            path,
            source,
        }),
    }
}

fn prepare_writable_database(path: &Path) -> Result<(), StateError> {
    validate_database_artifacts(path)?;
    if metadata_if_exists(path, "inspect database path")?.is_none() {
        create_private_file(path)?;
    }
    validate_regular_file(path)
}

fn prepare_private_file(path: &Path) -> Result<(), StateError> {
    if metadata_if_exists(path, "inspect private file")?.is_none() {
        create_private_file(path)?;
    }
    validate_regular_file(path)
}

fn create_private_file(path: &Path) -> Result<(), StateError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| StateError::Io {
            operation: "create private file",
            path: path.to_path_buf(),
            source,
        })?
        .sync_all()
        .map_err(|source| StateError::Io {
            operation: "sync private file creation",
            path: path.to_path_buf(),
            source,
        })?;
    restrict_database_permissions(path)?;
    validate_regular_file(path)
}

fn validate_database_artifacts(path: &Path) -> Result<(), StateError> {
    validate_regular_file_if_exists(path)?;
    for suffix in ["-wal", "-shm"] {
        validate_regular_file_if_exists(&with_suffix(path, suffix))?;
    }
    Ok(())
}

fn validate_regular_file_if_exists(path: &Path) -> Result<(), StateError> {
    let Some(metadata) = metadata_if_exists(path, "inspect trusted database path")? else {
        return Ok(());
    };
    validate_regular_file_metadata(path, &metadata)
}

fn validate_regular_file(path: &Path) -> Result<(), StateError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| StateError::Io {
        operation: "inspect trusted database path",
        path: path.to_path_buf(),
        source,
    })?;
    validate_regular_file_metadata(path, &metadata)
}

fn validate_regular_file_metadata(path: &Path, metadata: &fs::Metadata) -> Result<(), StateError> {
    if metadata.file_type().is_symlink() {
        return Err(StateError::DatabasePathSymlink(path.to_path_buf()));
    }
    if !metadata.is_file() {
        return Err(StateError::DatabasePathNotFile(path.to_path_buf()));
    }
    verify_private_unix_path(path, metadata, 0o600)
}

fn require_path_absent(path: &Path) -> Result<(), StateError> {
    if let Some(metadata) = metadata_if_exists(path, "inspect backup destination")? {
        if metadata.file_type().is_symlink() {
            return Err(StateError::DatabasePathSymlink(path.to_path_buf()));
        }
        return Err(StateError::BackupDestinationExists(path.to_path_buf()));
    }
    Ok(())
}

fn metadata_if_exists(
    path: &Path,
    operation: &'static str,
) -> Result<Option<fs::Metadata>, StateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StateError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(unix)]
fn verify_private_unix_path(
    path: &Path,
    metadata: &fs::Metadata,
    expected_mode: u32,
) -> Result<(), StateError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let found_mode = metadata.permissions().mode() & 0o777;
    if found_mode != expected_mode {
        return Err(StateError::UnsafePermissions {
            path: path.to_path_buf(),
            found: found_mode,
            expected: expected_mode,
        });
    }
    let found_uid = metadata.uid();
    let expected_uid = rustix::process::geteuid().as_raw();
    if found_uid != expected_uid {
        return Err(StateError::UnsafeOwner {
            path: path.to_path_buf(),
            found: found_uid,
            expected: expected_uid,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_unix_path(
    _path: &Path,
    _metadata: &fs::Metadata,
    _expected_mode: u32,
) -> Result<(), StateError> {
    Ok(())
}

fn create_directory(path: &Path) -> Result<(), StateError> {
    create_private_directory(path).map_err(|source| StateError::Io {
        operation: "create directory",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

fn sync_file(path: &Path) -> Result<(), StateError> {
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| StateError::Io {
            operation: "sync file",
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), StateError> {
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|source| StateError::Io {
                operation: "sync parent directory",
                path: parent.to_path_buf(),
                source,
            })?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), StateError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) -> Result<(), StateError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| StateError::Io {
        operation: "restrict directory permissions",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_path: &Path) -> Result<(), StateError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_database_permissions(path: &Path) -> Result<(), StateError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| StateError::Io {
        operation: "restrict database permissions",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn restrict_database_permissions(_path: &Path) -> Result<(), StateError> {
    Ok(())
}

fn timestamp_i64() -> i64 {
    i64::try_from(unix_timestamp_ms()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_private_test_file(path: &Path, contents: &[u8]) {
        create_private_file(path).expect("create private test file");
        fs::write(path, contents).expect("write private test file");
    }

    #[test]
    fn state_and_metrics_use_distinct_verified_durability() {
        let temp = crate::private_test_directory("temporary directory");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open databases");
        let state = databases.state().pragma_report().expect("state pragmas");
        let metrics = databases
            .metrics()
            .expect("metrics database")
            .pragma_report()
            .expect("metrics pragmas");

        assert_eq!(state.journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(state.synchronous, 2);
        assert!(state.foreign_keys);
        assert_eq!(state.user_version, STATE_SCHEMA_VERSION);
        assert_eq!(metrics.journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(metrics.synchronous, 1);
        assert!(metrics.foreign_keys);
        assert_eq!(metrics.user_version, METRICS_SCHEMA_VERSION);
    }

    #[test]
    fn installation_identity_survives_reopen() {
        let temp = crate::private_test_directory("temporary directory");
        let first_id = {
            let databases = DatabaseSet::open_for_daemon(temp.path()).expect("first open");
            databases.state().installation_id().to_owned()
        };
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("second open");

        assert_eq!(databases.state().installation_id(), first_id);
    }

    #[test]
    fn unclean_shutdown_is_detected_on_next_runtime_start() {
        let temp = crate::private_test_directory("temporary directory");
        {
            let databases = DatabaseSet::open_for_daemon(temp.path()).expect("first open");
            assert!(!databases.state().begin_runtime().expect("first start"));
        }
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("second open");
        assert!(databases.state().begin_runtime().expect("unclean start"));
        databases.state().mark_clean_shutdown().expect("mark clean");
    }

    #[test]
    fn online_backup_is_verified_and_never_overwritten() {
        let temp = crate::private_test_directory("temporary directory");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open databases");
        let destination = temp.path().join("backups").join("state.db");

        assert_eq!(
            databases
                .state()
                .backup_to(&destination)
                .expect("create backup"),
            BackupOutcome::Published
        );
        assert!(destination.exists());
        assert!(matches!(
            databases.state().backup_to(&destination),
            Err(StateError::BackupDestinationExists(_))
        ));
    }

    #[test]
    fn published_backup_cleanup_is_retried_and_residue_is_a_success_outcome() {
        let temp = crate::private_test_directory("temporary directory");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open databases");
        let transient_destination = temp.path().join("transient-cleanup.db");
        fail_published_backup_cleanup_for_test(1);
        assert_eq!(
            databases
                .state()
                .backup_to(&transient_destination)
                .expect("transient cleanup failure does not fail publication"),
            BackupOutcome::Published
        );

        let residue_destination = temp.path().join("residue-cleanup.db");
        fail_published_backup_cleanup_for_test(PUBLISHED_BACKUP_CLEANUP_ATTEMPTS);
        let outcome = databases
            .state()
            .backup_to(&residue_destination)
            .expect("published backup with residue remains successful");
        let BackupOutcome::PublishedWithResidue { temporary_path } = outcome else {
            panic!("persistent cleanup failure must identify its residue");
        };
        assert!(residue_destination.is_file());
        assert!(temporary_path.is_file());
        assert_eq!(
            fs::read(&residue_destination).expect("read published destination"),
            fs::read(&temporary_path).expect("read published residue")
        );
        let published =
            Connection::open_with_flags(&residue_destination, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("open published destination");
        check_integrity(&published, "helix-state.db", true)
            .expect("published destination remains valid");
        drop(published);
        fs::remove_file(temporary_path).expect("remove reported residue");
    }

    #[test]
    fn failed_migration_reuses_verified_identical_source_snapshot() {
        let temp = crate::private_test_directory("temporary directory");
        create_state_at_version_two(temp.path());
        reset_online_backup_invocations_for_test();

        fail_next_migration_for_test(3);
        match DatabaseSet::open_for_daemon(temp.path()) {
            Err(StateError::Sqlite(rusqlite::Error::ExecuteReturnedResults)) => {}
            Err(error) => panic!("unexpected first migration error: {error:?}"),
            Ok(_) => panic!("first migration unexpectedly succeeded"),
        }
        assert_eq!(online_backup_invocations_for_test(), 1);
        let first_snapshots = migration_snapshot_paths(temp.path());
        assert_eq!(first_snapshots.len(), 1, "{first_snapshots:?}");
        let snapshot_path = first_snapshots[0].clone();
        let snapshot_size = fs::metadata(&snapshot_path)
            .expect("first snapshot metadata")
            .len();

        fail_next_migration_for_test(3);
        match DatabaseSet::open_for_daemon(temp.path()) {
            Err(StateError::Sqlite(rusqlite::Error::ExecuteReturnedResults)) => {}
            Err(error) => panic!("unexpected second migration error: {error:?}"),
            Ok(_) => panic!("second migration unexpectedly succeeded"),
        }
        assert_eq!(online_backup_invocations_for_test(), 1);
        assert_eq!(migration_snapshot_paths(temp.path()), first_snapshots);
        assert_eq!(
            fs::metadata(&snapshot_path)
                .expect("reused snapshot metadata")
                .len(),
            snapshot_size
        );

        verify_v2_migration_snapshot(&snapshot_path);
        let state_path = temp.path().join("state").join("helix-state.db");
        let changed = Connection::open(&state_path).expect("open source to change it");
        changed
            .execute(
                "UPDATE nodes SET display_name = 'Changed before retry' WHERE kind = 'local'",
                [],
            )
            .expect("change pre-migration source");
        drop(changed);

        fail_next_migration_for_test(3);
        match DatabaseSet::open_for_daemon(temp.path()) {
            Err(StateError::Sqlite(rusqlite::Error::ExecuteReturnedResults)) => {}
            Err(error) => panic!("unexpected changed-source migration error: {error:?}"),
            Ok(_) => panic!("changed-source migration unexpectedly succeeded"),
        }
        assert_eq!(online_backup_invocations_for_test(), 2);
        let changed_snapshots = migration_snapshot_paths(temp.path());
        assert_eq!(changed_snapshots.len(), 2, "{changed_snapshots:?}");
        for snapshot in &changed_snapshots {
            verify_v2_migration_snapshot(snapshot);
        }

        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("retry migration");
        assert_eq!(
            databases.state().schema_version().expect("live schema"),
            STATE_SCHEMA_VERSION
        );
        assert_eq!(online_backup_invocations_for_test(), 2);
        assert_eq!(migration_snapshot_paths(temp.path()), changed_snapshots);
    }

    #[test]
    fn altered_aliased_snapshot_is_rejected_without_recopying_or_deletion() {
        let temp = crate::private_test_directory("temporary directory");
        create_state_at_version_two(temp.path());
        reset_online_backup_invocations_for_test();

        fail_next_migration_for_test(3);
        assert!(matches!(
            DatabaseSet::open_for_daemon(temp.path()),
            Err(StateError::Sqlite(rusqlite::Error::ExecuteReturnedResults))
        ));
        assert_eq!(online_backup_invocations_for_test(), 1);
        let snapshots = migration_snapshot_paths(temp.path());
        assert_eq!(snapshots.len(), 1);
        let snapshot = snapshots[0].clone();
        let altered = Connection::open(&snapshot).expect("open aliased snapshot");
        altered
            .execute(
                "UPDATE nodes SET display_name = 'Altered rollback' WHERE kind = 'local'",
                [],
            )
            .expect("alter aliased snapshot");
        drop(altered);

        assert!(matches!(
            DatabaseSet::open_for_daemon(temp.path()),
            Err(StateError::Integrity {
                database: "helix-state.db backup",
                ..
            })
        ));
        assert_eq!(online_backup_invocations_for_test(), 1);
        assert_eq!(migration_snapshot_paths(temp.path()), snapshots);
        assert!(snapshot.is_file());
    }

    #[test]
    fn stranded_legacy_backup_partial_is_published_and_bounded() {
        let temp = crate::private_test_directory("temporary directory");
        create_state_at_version_two(temp.path());
        let backup_dir = temp.path().join("state").join("migration-backups");
        ensure_data_directory(&backup_dir).expect("migration backup directory");
        let pending = backup_dir.join(format!(
            ".helix-state-v2-to-v{STATE_SCHEMA_VERSION}.pending.db"
        ));
        let partial = with_suffix(
            &pending,
            &format!(".partial-{}", Uuid::new_v4().hyphenated()),
        );
        let invalid_partial = with_suffix(
            &pending,
            &format!(".partial-{}", Uuid::new_v4().hyphenated()),
        );
        let source_path = temp.path().join("state").join("helix-state.db");
        let source = Connection::open(source_path).expect("open v2 source");
        create_private_file(&partial).expect("legacy partial file");
        source
            .backup(MAIN_DB, &partial, None)
            .expect("create stranded backup partial");
        drop(source);
        fs::write(&invalid_partial, b"incomplete SQLite backup")
            .expect("invalid legacy partial file");
        assert!(partial.is_file());
        assert!(invalid_partial.is_file());
        reset_online_backup_invocations_for_test();

        fail_next_migration_for_test(3);
        assert!(matches!(
            DatabaseSet::open_for_daemon(temp.path()),
            Err(StateError::Sqlite(rusqlite::Error::ExecuteReturnedResults))
        ));
        assert_eq!(online_backup_invocations_for_test(), 1);
        assert!(!partial.exists());
        assert!(!invalid_partial.exists());
        let snapshots = migration_snapshot_paths(temp.path());
        assert_eq!(snapshots.len(), 1, "{snapshots:?}");
        verify_v2_migration_snapshot(&snapshots[0]);

        fail_next_migration_for_test(3);
        assert!(matches!(
            DatabaseSet::open_for_daemon(temp.path()),
            Err(StateError::Sqlite(rusqlite::Error::ExecuteReturnedResults))
        ));
        assert_eq!(online_backup_invocations_for_test(), 1);
        assert_eq!(migration_snapshot_paths(temp.path()), snapshots);
    }

    #[test]
    fn corrupt_metrics_are_preserved_and_recreated() {
        let temp = crate::private_test_directory("temporary directory");
        {
            let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open databases");
            assert!(matches!(
                databases.metrics_outcome(),
                MetricsOpenOutcome::Healthy
            ));
        }
        let metrics_path = temp.path().join("metrics").join("helix-metrics.db");
        fs::write(&metrics_path, b"this is not SQLite").expect("corrupt metrics file");

        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("recover metrics");
        let MetricsOpenOutcome::Recovered { forensic_path } = databases.metrics_outcome() else {
            panic!("metrics corruption was not reported");
        };
        assert!(forensic_path.is_dir());
        assert_eq!(
            fs::read(forensic_path.join("helix-metrics.db")).expect("preserved metrics database"),
            b"this is not SQLite"
        );
        assert!(forensic_path.join(METRICS_FORENSIC_MANIFEST).is_file());
        databases
            .metrics()
            .expect("replacement metrics database")
            .quick_integrity_check()
            .expect("replacement metrics database is healthy");
        databases
            .state()
            .quick_integrity_check()
            .expect("state database remains healthy");
    }

    #[test]
    fn interrupted_metrics_forensic_move_reconciles_the_complete_component_set() {
        let temp = crate::private_test_directory("temporary directory");
        ensure_data_directory(temp.path()).expect("metrics directory");
        let metrics_path = temp.path().join("helix-metrics.db");
        let wal_path = with_suffix(&metrics_path, "-wal");
        let shm_path = with_suffix(&metrics_path, "-shm");
        write_private_test_file(&metrics_path, b"corrupt-main");
        write_private_test_file(&wal_path, b"corrupt-wal");
        write_private_test_file(&shm_path, b"corrupt-shm");

        fail_metrics_forensic_after_moves_for_test(1);
        assert!(matches!(
            preserve_corrupt_database(&metrics_path),
            Err(StateError::Io {
                operation: "injected metrics forensic move interruption",
                ..
            })
        ));
        let staging = metrics_forensic_staging_path(&metrics_path);
        assert!(staging.is_dir());
        assert!(!metrics_path.exists());
        assert!(wal_path.is_file());
        assert!(shm_path.is_file());

        let forensic_path = reconcile_metrics_forensic_staging(&metrics_path)
            .expect("reconcile interrupted preservation")
            .expect("published forensic set");
        assert!(forensic_path.is_dir());
        for (name, expected) in [
            ("helix-metrics.db", b"corrupt-main".as_slice()),
            ("helix-metrics.db-wal", b"corrupt-wal".as_slice()),
            ("helix-metrics.db-shm", b"corrupt-shm".as_slice()),
        ] {
            assert_eq!(
                fs::read(forensic_path.join(name)).expect("preserved component"),
                expected
            );
        }
        assert_eq!(
            read_metrics_forensic_manifest(&forensic_path.join(METRICS_FORENSIC_MANIFEST))
                .expect("forensic manifest"),
            MetricsForensicManifest {
                destination_name: forensic_path
                    .file_name()
                    .expect("forensic directory name")
                    .to_string_lossy()
                    .into_owned(),
                wal_present: true,
                shm_present: true,
            }
        );
        assert_eq!(
            reconcile_metrics_forensic_staging(&metrics_path).expect("idempotent reconciliation"),
            None
        );
    }

    #[test]
    fn unstarted_metrics_forensic_manifest_write_is_safely_retried() {
        let temp = crate::private_test_directory("temporary directory");
        ensure_data_directory(temp.path()).expect("metrics directory");
        let metrics_path = temp.path().join("helix-metrics.db");
        write_private_test_file(&metrics_path, b"corrupt-main");
        let staging = metrics_forensic_staging_path(&metrics_path);
        ensure_data_directory(&staging).expect("forensic staging directory");
        write_private_test_file(
            &staging.join(METRICS_FORENSIC_MANIFEST_PARTIAL),
            b"partial-manifest",
        );

        assert_eq!(
            reconcile_metrics_forensic_staging(&metrics_path).expect("discard unstarted staging"),
            None
        );
        assert!(!staging.exists());
        let forensic_path =
            preserve_corrupt_database(&metrics_path).expect("retry forensic preservation");
        assert_eq!(
            fs::read(forensic_path.join("helix-metrics.db")).expect("preserved main database"),
            b"corrupt-main"
        );
    }

    #[test]
    fn metrics_forensic_manifest_preserves_partial_sidecar_sets_exactly() {
        for (wal_present, shm_present) in [(true, false), (false, true), (false, false)] {
            let temp = crate::private_test_directory("temporary directory");
            ensure_data_directory(temp.path()).expect("metrics directory");
            let metrics_path = temp.path().join("helix-metrics.db");
            write_private_test_file(&metrics_path, b"corrupt-main");
            if wal_present {
                write_private_test_file(&with_suffix(&metrics_path, "-wal"), b"corrupt-wal");
            }
            if shm_present {
                write_private_test_file(&with_suffix(&metrics_path, "-shm"), b"corrupt-shm");
            }

            let forensic_path =
                preserve_corrupt_database(&metrics_path).expect("preserve forensic set");
            let manifest =
                read_metrics_forensic_manifest(&forensic_path.join(METRICS_FORENSIC_MANIFEST))
                    .expect("forensic manifest");
            assert_eq!(manifest.wal_present, wal_present);
            assert_eq!(manifest.shm_present, shm_present);
            assert_eq!(
                forensic_path.join("helix-metrics.db-wal").exists(),
                wal_present
            );
            assert_eq!(
                forensic_path.join("helix-metrics.db-shm").exists(),
                shm_present
            );
            assert!(!metrics_path.exists());
            assert!(!with_suffix(&metrics_path, "-wal").exists());
            assert!(!with_suffix(&metrics_path, "-shm").exists());
        }
    }

    #[test]
    fn interrupted_metrics_preservation_degrades_only_metrics_then_recovers() {
        let temp = crate::private_test_directory("temporary directory");
        drop(DatabaseSet::open_for_daemon(temp.path()).expect("initialize databases"));
        let metrics_path = temp.path().join("metrics").join("helix-metrics.db");
        fs::write(&metrics_path, b"interrupted corrupt metrics").expect("corrupt metrics file");

        fail_metrics_forensic_after_moves_for_test(1);
        let degraded =
            DatabaseSet::open_for_daemon(temp.path()).expect("critical state remains up");
        assert!(degraded.metrics().is_none());
        assert_eq!(
            degraded.metrics_outcome(),
            &MetricsOpenOutcome::Unavailable {
                reason: MetricsUnavailableReason::Io,
            }
        );
        degraded
            .state()
            .quick_integrity_check()
            .expect("critical state remains healthy");
        drop(degraded);

        let recovered = DatabaseSet::open_for_daemon(temp.path()).expect("reconcile on restart");
        let MetricsOpenOutcome::Recovered { forensic_path } = recovered.metrics_outcome() else {
            panic!("reconciled forensic set was not reported");
        };
        assert!(forensic_path.is_dir());
        recovered
            .metrics()
            .expect("replacement metrics database")
            .quick_integrity_check()
            .expect("replacement metrics database is healthy");
        recovered
            .state()
            .quick_integrity_check()
            .expect("critical state remains healthy");
    }

    #[test]
    fn daemon_lease_rejects_a_second_writer_until_release() {
        let temp = crate::private_test_directory("temporary directory");
        let first = DatabaseSet::open_for_daemon(temp.path()).expect("first daemon");

        assert!(matches!(
            DatabaseSet::open_for_daemon(temp.path()),
            Err(StateError::DaemonLeaseHeld(_))
        ));

        drop(first);
        DatabaseSet::open_for_daemon(temp.path()).expect("daemon after lease release");
    }

    #[test]
    fn unsupported_metrics_schema_does_not_block_critical_state() {
        let temp = crate::private_test_directory("temporary directory");
        let metrics_path = {
            let databases =
                DatabaseSet::open_for_daemon(temp.path()).expect("initialize databases");
            databases
                .metrics()
                .expect("metrics database")
                .path()
                .to_owned()
        };
        let connection = Connection::open(metrics_path).expect("open metrics directly for test");
        connection
            .pragma_update(None, "user_version", METRICS_SCHEMA_VERSION + 1)
            .expect("install future schema marker");
        drop(connection);

        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("state remains available");
        assert!(databases.metrics().is_none());
        assert!(matches!(
            databases.metrics_outcome(),
            MetricsOpenOutcome::Unavailable { .. }
        ));
        databases
            .state()
            .quick_integrity_check()
            .expect("critical state is healthy");
    }

    #[test]
    fn invalid_metrics_directory_does_not_block_critical_state() {
        let temp = crate::private_test_directory("temporary directory");
        fs::write(temp.path().join("metrics"), b"not a directory")
            .expect("create invalid metrics path");

        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("state remains available");

        assert!(databases.metrics().is_none());
        assert_eq!(
            databases.metrics_outcome(),
            &MetricsOpenOutcome::Unavailable {
                reason: MetricsUnavailableReason::StoragePolicy,
            }
        );
        databases
            .state()
            .quick_integrity_check()
            .expect("critical state is healthy");
    }

    #[test]
    fn read_only_state_access_works_while_daemon_holds_lease() {
        let temp = crate::private_test_directory("temporary directory");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("daemon databases");
        let expected_id = databases.state().installation_id().to_owned();
        let reader = StateDatabaseReader::open(temp.path()).expect("read-only state");

        assert_eq!(reader.installation_id(), expected_id);
        assert_eq!(
            reader.schema_version().expect("schema"),
            STATE_SCHEMA_VERSION
        );
        assert!(
            reader
                .connection
                .execute("DELETE FROM installation", [])
                .is_err()
        );
    }

    #[test]
    fn state_reader_backup_does_not_require_metrics() {
        let temp = crate::private_test_directory("temporary directory");
        drop(DatabaseSet::open_for_daemon(temp.path()).expect("initialize databases"));
        let metrics_path = temp.path().join("metrics").join("helix-metrics.db");
        fs::write(&metrics_path, b"broken metrics").expect("break metrics");
        let metrics_before = fs::read(&metrics_path).expect("read broken metrics");
        let destination = temp.path().join("state-snapshot.db");

        StateDatabaseReader::open(temp.path())
            .expect("state reader")
            .backup_to(&destination)
            .expect("state-only backup");

        assert!(destination.is_file());
        assert_eq!(
            fs::read(metrics_path).expect("metrics unchanged"),
            metrics_before
        );
    }

    fn create_state_at_version_two(data_dir: &Path) {
        let state_dir = data_dir.join("state");
        ensure_data_directory(data_dir).expect("data directory");
        ensure_data_directory(&state_dir).expect("state directory");
        let state_path = state_dir.join("helix-state.db");
        create_private_file(&state_path).expect("state file");
        let mut connection = Connection::open(state_path).expect("open state");
        configure_state_connection(&connection).expect("configure state");
        apply_migration(&mut connection, 1, "foundational-state", STATE_MIGRATION_1)
            .expect("apply v1");
        security::migrate_security(&mut connection).expect("apply v2");
        ensure_installation(&mut connection).expect("installation row");
    }

    fn migration_snapshot_paths(data_dir: &Path) -> Vec<PathBuf> {
        let mut paths = fs::read_dir(data_dir.join("state").join("migration-backups"))
            .expect("migration snapshot directory")
            .map(|entry| entry.expect("migration snapshot entry").path())
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    fn verify_v2_migration_snapshot(path: &Path) {
        let snapshot = Connection::open(path).expect("open migration snapshot");
        check_integrity(&snapshot, "helix-state.db backup", true)
            .expect("migration snapshot integrity");
        validate_state_semantics(&snapshot, 2, "helix-state.db backup")
            .expect("migration snapshot semantics");
        assert_eq!(
            pragma_i64(&snapshot, "user_version").expect("snapshot schema"),
            2
        );
    }

    #[cfg(unix)]
    #[test]
    fn direct_database_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let temp = crate::private_test_directory("temporary directory");
        drop(DatabaseSet::open_for_daemon(temp.path()).expect("initialize databases"));
        let database = temp.path().join("state").join("helix-state.db");
        let real_database = temp.path().join("state").join("real-state.db");
        fs::rename(&database, &real_database).expect("move database");
        symlink(&real_database, &database).expect("link database");

        assert!(matches!(
            DatabaseSet::open_for_daemon(temp.path()),
            Err(StateError::DatabasePathSymlink(path)) if path == database
        ));
    }

    #[cfg(unix)]
    #[test]
    fn database_sidecar_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        for suffix in ["-wal", "-shm"] {
            let temp = crate::private_test_directory("temporary directory");
            drop(DatabaseSet::open_for_daemon(temp.path()).expect("initialize databases"));
            let database = temp.path().join("state").join("helix-state.db");
            let sidecar = with_suffix(&database, suffix);
            let target = temp.path().join(format!("target{suffix}"));
            fs::write(&target, b"do not follow").expect("write target");
            symlink(&target, &sidecar).expect("link sidecar");

            assert!(matches!(
                DatabaseSet::open_for_daemon(temp.path()),
                Err(StateError::DatabasePathSymlink(path)) if path == sidecar
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_existing_private_directory_mode_is_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let temp = crate::private_test_directory("temporary directory");
        let state_dir = temp.path().join("state");
        fs::create_dir(&state_dir).expect("create state directory");
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o755))
            .expect("make directory unsafe");

        assert!(matches!(
            DatabaseSet::open_for_daemon(temp.path()),
            Err(StateError::UnsafePermissions {
                path,
                found: 0o755,
                expected: 0o700,
            }) if path == state_dir
        ));
    }
}
