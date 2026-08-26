use super::{StateDatabase, StateError, apply_migration, timestamp_i64};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use std::fmt;
use uuid::Uuid;

pub(super) const SECRET_MIGRATION_3: &str = r#"
CREATE TABLE master_key_versions (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    key_version INTEGER NOT NULL UNIQUE CHECK (key_version BETWEEN 1 AND 4294967295),
    algorithm TEXT NOT NULL CHECK (algorithm = 'xchacha20poly1305'),
    format_version INTEGER NOT NULL CHECK (format_version = 1),
    status TEXT NOT NULL CHECK (status IN ('staged', 'active', 'retiring', 'retired')),
    active_slot INTEGER UNIQUE CHECK (active_slot IS NULL OR active_slot = 1),
    check_nonce BLOB NOT NULL CHECK (length(check_nonce) = 24),
    check_ciphertext BLOB NOT NULL CHECK (length(check_ciphertext) = 41),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    activated_at_unix_ms INTEGER,
    retired_at_unix_ms INTEGER,
    CHECK (
        (status = 'staged' AND active_slot IS NULL
            AND activated_at_unix_ms IS NULL AND retired_at_unix_ms IS NULL)
        OR (status = 'active' AND active_slot = 1
            AND activated_at_unix_ms IS NOT NULL AND retired_at_unix_ms IS NULL)
        OR (status = 'retiring' AND active_slot IS NULL
            AND activated_at_unix_ms IS NOT NULL AND retired_at_unix_ms IS NULL)
        OR (status = 'retired' AND active_slot IS NULL
            AND activated_at_unix_ms IS NOT NULL AND retired_at_unix_ms IS NOT NULL)
    ),
    CHECK (activated_at_unix_ms IS NULL OR activated_at_unix_ms >= created_at_unix_ms),
    CHECK (retired_at_unix_ms IS NULL OR retired_at_unix_ms >= activated_at_unix_ms)
) STRICT;

CREATE INDEX master_key_versions_status_idx
    ON master_key_versions (status, key_version);

CREATE TABLE secret_records (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    secret_type TEXT NOT NULL CHECK (
        length(secret_type) BETWEEN 1 AND 64
        AND secret_type NOT GLOB '*[^a-z0-9._-]*'
    ),
    scope_type TEXT NOT NULL CHECK (
        length(scope_type) BETWEEN 1 AND 32
        AND scope_type NOT GLOB '*[^a-z0-9._-]*'
    ),
    scope_id TEXT NOT NULL CHECK (length(scope_id) = 36),
    purpose TEXT NOT NULL CHECK (
        length(purpose) BETWEEN 1 AND 128
        AND purpose NOT GLOB '*[^a-z0-9._-]*'
    ),
    revision INTEGER NOT NULL CHECK (revision > 0),
    wrap_revision INTEGER NOT NULL CHECK (wrap_revision > 0),
    master_key_id TEXT NOT NULL REFERENCES master_key_versions(id) ON DELETE RESTRICT,
    algorithm TEXT NOT NULL CHECK (algorithm = 'xchacha20poly1305'),
    format_version INTEGER NOT NULL CHECK (format_version = 1),
    data_nonce BLOB NOT NULL CHECK (length(data_nonce) = 24),
    ciphertext BLOB NOT NULL CHECK (length(ciphertext) BETWEEN 17 AND 65552),
    dek_wrap_nonce BLOB NOT NULL CHECK (length(dek_wrap_nonce) = 24),
    wrapped_dek BLOB NOT NULL CHECK (length(wrapped_dek) = 48),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= created_at_unix_ms),
    UNIQUE (secret_type, scope_type, scope_id, purpose)
) STRICT;

CREATE INDEX secret_records_scope_idx
    ON secret_records (scope_type, scope_id, secret_type, purpose);
CREATE INDEX secret_records_key_idx
    ON secret_records (master_key_id, id);

CREATE TRIGGER master_key_versions_identity_immutable
BEFORE UPDATE ON master_key_versions
WHEN NEW.id <> OLD.id
    OR NEW.key_version <> OLD.key_version
    OR NEW.algorithm <> OLD.algorithm
    OR NEW.format_version <> OLD.format_version
    OR NEW.check_nonce <> OLD.check_nonce
    OR NEW.check_ciphertext <> OLD.check_ciphertext
    OR NEW.created_at_unix_ms <> OLD.created_at_unix_ms
BEGIN
    SELECT RAISE(ABORT, 'master key identity and check material are immutable');
END;

CREATE TRIGGER master_key_retire_without_references
BEFORE UPDATE OF status ON master_key_versions
WHEN NEW.status = 'retired'
    AND EXISTS (SELECT 1 FROM secret_records WHERE master_key_id = OLD.id)
BEGIN
    SELECT RAISE(ABORT, 'referenced master key cannot be retired');
END;

CREATE TRIGGER secret_records_identity_immutable
BEFORE UPDATE ON secret_records
WHEN NEW.id <> OLD.id
    OR NEW.secret_type <> OLD.secret_type
    OR NEW.scope_type <> OLD.scope_type
    OR NEW.scope_id <> OLD.scope_id
    OR NEW.purpose <> OLD.purpose
    OR NEW.created_at_unix_ms <> OLD.created_at_unix_ms
BEGIN
    SELECT RAISE(ABORT, 'secret record identity is immutable');
END;

CREATE TRIGGER secret_records_revisions_monotonic
BEFORE UPDATE ON secret_records
WHEN NEW.revision < OLD.revision OR NEW.wrap_revision < OLD.wrap_revision
BEGIN
    SELECT RAISE(ABORT, 'secret record revisions cannot decrease');
END;
"#;

const ALGORITHM: &str = "xchacha20poly1305";
const FORMAT_VERSION: i64 = 1;
const MASTER_KEY_CHECK_CIPHERTEXT_LEN: usize = 41;
const NONCE_LEN: usize = 24;
const WRAPPED_DEK_LEN: usize = 48;
const MAX_CIPHERTEXT_LEN: usize = 65_552;

#[derive(Clone, Eq, PartialEq)]
pub struct MasterKeyRecord {
    pub id: String,
    pub key_version: i64,
    pub algorithm: String,
    pub format_version: i64,
    pub check_nonce: Vec<u8>,
    pub check_ciphertext: Vec<u8>,
}

impl fmt::Debug for MasterKeyRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MasterKeyRecord")
            .field("id", &self.id)
            .field("key_version", &self.key_version)
            .field("algorithm", &self.algorithm)
            .field("format_version", &self.format_version)
            .field("check_material", &"[REDACTED]")
            .finish()
    }
}

pub struct InstallMasterKeyInput<'a> {
    pub id: &'a str,
    pub key_version: i64,
    pub algorithm: &'a str,
    pub format_version: i64,
    pub check_nonce: &'a [u8],
    pub check_ciphertext: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallMasterKeyOutcome {
    Installed,
    AlreadyInitialized,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretRecordMetadata {
    pub id: String,
    pub secret_type: String,
    pub scope_type: String,
    pub scope_id: String,
    pub purpose: String,
    pub revision: i64,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Eq, PartialEq)]
pub struct StoredSecretRecord {
    pub id: String,
    pub secret_type: String,
    pub scope_type: String,
    pub scope_id: String,
    pub purpose: String,
    pub revision: i64,
    pub wrap_revision: i64,
    pub master_key_id: String,
    pub algorithm: String,
    pub format_version: i64,
    pub data_nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub dek_wrap_nonce: Vec<u8>,
    pub wrapped_dek: Vec<u8>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

impl StoredSecretRecord {
    #[must_use]
    pub fn metadata(&self) -> SecretRecordMetadata {
        SecretRecordMetadata {
            id: self.id.clone(),
            secret_type: self.secret_type.clone(),
            scope_type: self.scope_type.clone(),
            scope_id: self.scope_id.clone(),
            purpose: self.purpose.clone(),
            revision: self.revision,
            created_at_unix_ms: self.created_at_unix_ms,
            updated_at_unix_ms: self.updated_at_unix_ms,
        }
    }
}

impl fmt::Debug for StoredSecretRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredSecretRecord")
            .field("metadata", &self.metadata())
            .field("wrap_revision", &self.wrap_revision)
            .field("master_key_id", &self.master_key_id)
            .field("algorithm", &self.algorithm)
            .field("format_version", &self.format_version)
            .field("encrypted_material", &"[REDACTED]")
            .finish()
    }
}

pub struct EncryptedSecretWrite<'a> {
    pub id: &'a str,
    pub secret_type: &'a str,
    pub scope_type: &'a str,
    pub scope_id: &'a str,
    pub purpose: &'a str,
    pub revision: i64,
    pub wrap_revision: i64,
    pub master_key_id: &'a str,
    pub algorithm: &'a str,
    pub format_version: i64,
    pub data_nonce: &'a [u8],
    pub ciphertext: &'a [u8],
    pub dek_wrap_nonce: &'a [u8],
    pub wrapped_dek: &'a [u8],
}

pub(super) fn migrate_secrets(connection: &mut Connection) -> Result<(), StateError> {
    apply_migration(
        connection,
        3,
        "recoverable-secret-storage",
        SECRET_MIGRATION_3,
    )
}

impl StateDatabase {
    pub fn active_master_key_record(&self) -> Result<Option<MasterKeyRecord>, StateError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, key_version, algorithm, format_version,
                        check_nonce, check_ciphertext
                 FROM master_key_versions
                 WHERE active_slot = 1 AND status = 'active'",
                [],
                map_master_key_record,
            )
            .optional()
            .map_err(StateError::from)
    }

    pub fn master_key_record_count(&self) -> Result<i64, StateError> {
        let connection = self.lock()?;
        Ok(
            connection.query_row("SELECT count(*) FROM master_key_versions", [], |row| {
                row.get(0)
            })?,
        )
    }

    pub fn install_initial_master_key(
        &self,
        input: InstallMasterKeyInput<'_>,
    ) -> Result<InstallMasterKeyOutcome, StateError> {
        validate_master_key_input(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let initialized = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM master_key_versions)
                    OR EXISTS(SELECT 1 FROM secret_records)",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if initialized {
            transaction.commit()?;
            return Ok(InstallMasterKeyOutcome::AlreadyInitialized);
        }

        let now = timestamp_i64();
        transaction.execute(
            "INSERT INTO master_key_versions (
                id, key_version, algorithm, format_version, status, active_slot,
                check_nonce, check_ciphertext, created_at_unix_ms,
                activated_at_unix_ms, retired_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, 'active', 1, ?5, ?6, ?7, ?7, NULL)",
            params![
                input.id,
                input.key_version,
                input.algorithm,
                input.format_version,
                input.check_nonce,
                input.check_ciphertext,
                now
            ],
        )?;
        transaction.commit()?;
        Ok(InstallMasterKeyOutcome::Installed)
    }

    pub fn insert_secret_record(
        &self,
        input: EncryptedSecretWrite<'_>,
    ) -> Result<SecretRecordMetadata, StateError> {
        validate_secret_write(&input)?;
        if input.revision != 1 || input.wrap_revision != 1 {
            return Err(StateError::InvalidSecretInput(
                "a new secret must start at revision 1",
            ));
        }

        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_active_key(&transaction, input.master_key_id)?;
        let now = timestamp_i64();
        transaction.execute(
            "INSERT INTO secret_records (
                id, secret_type, scope_type, scope_id, purpose,
                revision, wrap_revision, master_key_id, algorithm, format_version,
                data_nonce, ciphertext, dek_wrap_nonce, wrapped_dek,
                created_at_unix_ms, updated_at_unix_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?15
             )",
            params![
                input.id,
                input.secret_type,
                input.scope_type,
                input.scope_id,
                input.purpose,
                input.revision,
                input.wrap_revision,
                input.master_key_id,
                input.algorithm,
                input.format_version,
                input.data_nonce,
                input.ciphertext,
                input.dek_wrap_nonce,
                input.wrapped_dek,
                now
            ],
        )?;
        transaction.commit()?;
        Ok(metadata_from_write(&input, now, now))
    }

    pub fn load_secret_record(&self, id: &str) -> Result<Option<StoredSecretRecord>, StateError> {
        validate_uuid("secret id", id)?;
        let connection = self.lock()?;
        load_secret_record_from(&connection, id)
    }

    pub fn replace_secret_record(
        &self,
        expected_revision: i64,
        input: EncryptedSecretWrite<'_>,
    ) -> Result<SecretRecordMetadata, StateError> {
        validate_secret_write(&input)?;
        if expected_revision < 1 {
            return Err(StateError::InvalidSecretInput(
                "expected revision must be positive",
            ));
        }
        if input.revision
            != expected_revision
                .checked_add(1)
                .ok_or(StateError::InvalidSecretInput("secret revision overflow"))?
        {
            return Err(StateError::InvalidSecretInput(
                "replacement revision must advance by one",
            ));
        }

        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(current) = load_secret_record_from(&transaction, input.id)? else {
            return Err(StateError::SecretNotFound);
        };
        if current.revision != expected_revision {
            return Err(StateError::SecretRevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        if input.wrap_revision
            != current
                .wrap_revision
                .checked_add(1)
                .ok_or(StateError::InvalidSecretInput(
                    "secret wrap revision overflow",
                ))?
        {
            return Err(StateError::InvalidSecretInput(
                "replacement wrap revision must advance by one",
            ));
        }
        require_same_identity(&current, &input)?;
        require_active_key(&transaction, input.master_key_id)?;
        let now = timestamp_i64().max(current.updated_at_unix_ms);
        transaction.execute(
            "UPDATE secret_records
             SET revision = ?2, wrap_revision = ?3, master_key_id = ?4,
                 algorithm = ?5, format_version = ?6, data_nonce = ?7,
                 ciphertext = ?8, dek_wrap_nonce = ?9, wrapped_dek = ?10,
                 updated_at_unix_ms = ?11
             WHERE id = ?1 AND revision = ?12",
            params![
                input.id,
                input.revision,
                input.wrap_revision,
                input.master_key_id,
                input.algorithm,
                input.format_version,
                input.data_nonce,
                input.ciphertext,
                input.dek_wrap_nonce,
                input.wrapped_dek,
                now,
                expected_revision
            ],
        )?;
        transaction.commit()?;
        Ok(metadata_from_write(&input, current.created_at_unix_ms, now))
    }

    pub fn delete_secret_record(
        &self,
        id: &str,
        expected_revision: i64,
    ) -> Result<SecretRecordMetadata, StateError> {
        validate_uuid("secret id", id)?;
        if expected_revision < 1 {
            return Err(StateError::InvalidSecretInput(
                "expected revision must be positive",
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(current) = load_secret_record_from(&transaction, id)? else {
            return Err(StateError::SecretNotFound);
        };
        if current.revision != expected_revision {
            return Err(StateError::SecretRevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        transaction.execute(
            "DELETE FROM secret_records WHERE id = ?1 AND revision = ?2",
            params![id, expected_revision],
        )?;
        transaction.commit()?;
        Ok(current.metadata())
    }
}

fn map_master_key_record(row: &Row<'_>) -> rusqlite::Result<MasterKeyRecord> {
    Ok(MasterKeyRecord {
        id: row.get(0)?,
        key_version: row.get(1)?,
        algorithm: row.get(2)?,
        format_version: row.get(3)?,
        check_nonce: row.get(4)?,
        check_ciphertext: row.get(5)?,
    })
}

fn load_secret_record_from(
    connection: &Connection,
    id: &str,
) -> Result<Option<StoredSecretRecord>, StateError> {
    connection
        .query_row(
            "SELECT id, secret_type, scope_type, scope_id, purpose,
                    revision, wrap_revision, master_key_id, algorithm, format_version,
                    data_nonce, ciphertext, dek_wrap_nonce, wrapped_dek,
                    created_at_unix_ms, updated_at_unix_ms
             FROM secret_records WHERE id = ?1",
            [id],
            |row| {
                Ok(StoredSecretRecord {
                    id: row.get(0)?,
                    secret_type: row.get(1)?,
                    scope_type: row.get(2)?,
                    scope_id: row.get(3)?,
                    purpose: row.get(4)?,
                    revision: row.get(5)?,
                    wrap_revision: row.get(6)?,
                    master_key_id: row.get(7)?,
                    algorithm: row.get(8)?,
                    format_version: row.get(9)?,
                    data_nonce: row.get(10)?,
                    ciphertext: row.get(11)?,
                    dek_wrap_nonce: row.get(12)?,
                    wrapped_dek: row.get(13)?,
                    created_at_unix_ms: row.get(14)?,
                    updated_at_unix_ms: row.get(15)?,
                })
            },
        )
        .optional()
        .map_err(StateError::from)
}

fn validate_master_key_input(input: &InstallMasterKeyInput<'_>) -> Result<(), StateError> {
    validate_uuid("master key id", input.id)?;
    if u32::try_from(input.key_version).is_err() {
        return Err(StateError::InvalidSecretInput(
            "master key version must fit a positive u32",
        ));
    }
    if input.algorithm != ALGORITHM || input.format_version != FORMAT_VERSION {
        return Err(StateError::InvalidSecretInput(
            "unsupported master key envelope format",
        ));
    }
    require_length("master key check nonce", input.check_nonce, NONCE_LEN)?;
    require_length(
        "master key check ciphertext",
        input.check_ciphertext,
        MASTER_KEY_CHECK_CIPHERTEXT_LEN,
    )
}

fn validate_secret_write(input: &EncryptedSecretWrite<'_>) -> Result<(), StateError> {
    validate_uuid("secret id", input.id)?;
    validate_identifier("secret type", input.secret_type, 64)?;
    validate_identifier("scope type", input.scope_type, 32)?;
    validate_uuid("scope id", input.scope_id)?;
    validate_identifier("secret purpose", input.purpose, 128)?;
    validate_uuid("master key id", input.master_key_id)?;
    if input.revision < 1 || input.wrap_revision < 1 {
        return Err(StateError::InvalidSecretInput(
            "secret revisions must be positive",
        ));
    }
    if input.algorithm != ALGORITHM || input.format_version != FORMAT_VERSION {
        return Err(StateError::InvalidSecretInput(
            "unsupported secret envelope format",
        ));
    }
    require_length("secret data nonce", input.data_nonce, NONCE_LEN)?;
    require_length("secret DEK wrap nonce", input.dek_wrap_nonce, NONCE_LEN)?;
    require_length("wrapped secret DEK", input.wrapped_dek, WRAPPED_DEK_LEN)?;
    if !(17..=MAX_CIPHERTEXT_LEN).contains(&input.ciphertext.len()) {
        return Err(StateError::InvalidSecretInput(
            "secret ciphertext length is outside the supported bound",
        ));
    }
    Ok(())
}

fn validate_uuid(field: &'static str, value: &str) -> Result<(), StateError> {
    let canonical = Uuid::parse_str(value)
        .ok()
        .map(|uuid| uuid.hyphenated().to_string());
    if canonical.as_deref() == Some(value) {
        Ok(())
    } else {
        Err(StateError::InvalidSecretInput(field))
    }
}

fn validate_identifier(field: &'static str, value: &str, maximum: usize) -> Result<(), StateError> {
    if !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        Ok(())
    } else {
        Err(StateError::InvalidSecretInput(field))
    }
}

fn require_length(field: &'static str, value: &[u8], expected: usize) -> Result<(), StateError> {
    if value.len() == expected {
        Ok(())
    } else {
        Err(StateError::InvalidSecretInput(field))
    }
}

fn require_active_key(connection: &Connection, key_id: &str) -> Result<(), StateError> {
    let active = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM master_key_versions
            WHERE id = ?1 AND status = 'active' AND active_slot = 1
         )",
        [key_id],
        |row| row.get::<_, bool>(0),
    )?;
    if active {
        Ok(())
    } else {
        Err(StateError::InvalidSecretInput(
            "secret envelope does not reference the active master key",
        ))
    }
}

fn require_same_identity(
    current: &StoredSecretRecord,
    input: &EncryptedSecretWrite<'_>,
) -> Result<(), StateError> {
    if current.secret_type == input.secret_type
        && current.scope_type == input.scope_type
        && current.scope_id == input.scope_id
        && current.purpose == input.purpose
    {
        Ok(())
    } else {
        Err(StateError::InvalidSecretInput(
            "secret identity cannot change during replacement",
        ))
    }
}

fn metadata_from_write(
    input: &EncryptedSecretWrite<'_>,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
) -> SecretRecordMetadata {
    SecretRecordMetadata {
        id: input.id.to_owned(),
        secret_type: input.secret_type.to_owned(),
        scope_type: input.scope_type.to_owned(),
        scope_id: input.scope_id.to_owned(),
        purpose: input.purpose.to_owned(),
        revision: input.revision,
        created_at_unix_ms,
        updated_at_unix_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DatabaseSet, STATE_MIGRATION_1, STATE_SCHEMA_VERSION, apply_migration,
        configure_state_connection, create_private_file, ensure_data_directory,
        ensure_installation, pragma_i64,
    };
    use std::{fs, path::Path};

    #[test]
    fn v2_to_current_migration_creates_a_verified_v2_snapshot_and_v3_secret_schema() {
        let temp = crate::private_test_directory("temporary directory");
        let installation_id = create_state_at_version_two(temp.path());

        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("migrate to current");
        assert_eq!(
            databases.state().schema_version().expect("live schema"),
            STATE_SCHEMA_VERSION
        );
        assert_eq!(databases.state().installation_id(), installation_id);
        let live = Connection::open(databases.state().path()).expect("open live state");
        assert_eq!(
            live.query_row(
                "SELECT count(*) FROM schema_migrations
                 WHERE version = 3 AND name = 'recoverable-secret-storage'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("v3 migration row"),
            1
        );
        assert!(table_exists(&live, "master_key_versions"));
        assert!(table_exists(&live, "secret_records"));

        let backup = only_migration_backup(temp.path());
        let snapshot = Connection::open(backup).expect("open migration snapshot");
        assert_eq!(
            pragma_i64(&snapshot, "user_version").expect("snapshot schema"),
            2
        );
        assert_eq!(
            snapshot
                .query_row(
                    "SELECT group_concat(version || ':' || name, ',')
                     FROM schema_migrations ORDER BY version",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("snapshot migrations"),
            "1:foundational-state,2:owner-authentication"
        );
        assert!(!table_exists(&snapshot, "master_key_versions"));
        assert!(!table_exists(&snapshot, "secret_records"));
        assert_eq!(
            snapshot
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .expect("snapshot integrity"),
            "ok"
        );
    }

    #[test]
    fn invalid_v2_state_is_not_migrated_or_published_as_a_snapshot() {
        let temp = crate::private_test_directory("temporary directory");
        create_state_at_version_two(temp.path());
        let state_path = temp.path().join("state").join("helix-state.db");
        let connection = Connection::open(&state_path).expect("open v2 state");
        connection
            .execute(
                "UPDATE schema_migrations SET name = 'tampered' WHERE version = 2",
                [],
            )
            .expect("tamper migration history");
        drop(connection);

        assert!(matches!(
            DatabaseSet::open_for_daemon(temp.path()),
            Err(StateError::Integrity {
                database: "helix-state.db backup",
                ..
            })
        ));
        let unchanged = Connection::open(&state_path).expect("reopen unchanged state");
        assert_eq!(
            pragma_i64(&unchanged, "user_version").expect("unchanged schema"),
            2
        );
        assert!(!table_exists(&unchanged, "master_key_versions"));
        let backup_dir = temp.path().join("state").join("migration-backups");
        assert_eq!(
            fs::read_dir(backup_dir)
                .expect("migration backup directory")
                .count(),
            0
        );
    }

    #[test]
    fn live_startup_rejects_missing_secret_store_semantics() {
        for tamper_sql in [
            "DROP TABLE secret_records;",
            "DROP TRIGGER master_key_versions_identity_immutable;",
            "DELETE FROM schema_migrations WHERE version = 3;",
        ] {
            let temp = crate::private_test_directory("temporary directory");
            let databases = DatabaseSet::open_for_daemon(temp.path()).expect("initialize state");
            let state_path = databases.state().path().to_path_buf();
            drop(databases);
            let connection = Connection::open(state_path).expect("open state for tamper test");
            connection
                .execute_batch(tamper_sql)
                .expect("tamper secret-store semantics");
            drop(connection);

            assert!(matches!(
                DatabaseSet::open_for_daemon(temp.path()),
                Err(StateError::Integrity {
                    database: "helix-state.db",
                    ..
                })
            ));
        }
    }

    #[test]
    fn state_boundary_rejects_unrepresentable_key_versions_and_secret_identity() {
        let temp = crate::private_test_directory("temporary directory");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("initialize state");
        let key_id = Uuid::new_v4().hyphenated().to_string();
        assert!(matches!(
            databases
                .state()
                .install_initial_master_key(InstallMasterKeyInput {
                    id: &key_id,
                    key_version: i64::MAX,
                    algorithm: ALGORITHM,
                    format_version: FORMAT_VERSION,
                    check_nonce: &[0; NONCE_LEN],
                    check_ciphertext: &[0; MASTER_KEY_CHECK_CIPHERTEXT_LEN],
                }),
            Err(StateError::InvalidSecretInput(_))
        ));

        databases
            .state()
            .install_initial_master_key(InstallMasterKeyInput {
                id: &key_id,
                key_version: 1,
                algorithm: ALGORITHM,
                format_version: FORMAT_VERSION,
                check_nonce: &[1; NONCE_LEN],
                check_ciphertext: &[2; MASTER_KEY_CHECK_CIPHERTEXT_LEN],
            })
            .expect("install valid key metadata");
        let secret_id = Uuid::new_v4().hyphenated().to_string();
        let scope_id = Uuid::new_v4().hyphenated().to_string();
        assert!(matches!(
            databases
                .state()
                .insert_secret_record(EncryptedSecretWrite {
                    id: &secret_id,
                    secret_type: "API-Token",
                    scope_type: "node",
                    scope_id: &scope_id,
                    purpose: "control",
                    revision: 1,
                    wrap_revision: 1,
                    master_key_id: &key_id,
                    algorithm: ALGORITHM,
                    format_version: FORMAT_VERSION,
                    data_nonce: &[3; NONCE_LEN],
                    ciphertext: &[4; 17],
                    dek_wrap_nonce: &[5; NONCE_LEN],
                    wrapped_dek: &[6; WRAPPED_DEK_LEN],
                }),
            Err(StateError::InvalidSecretInput(_))
        ));
    }

    fn create_state_at_version_two(data_dir: &Path) -> String {
        let state_dir = data_dir.join("state");
        ensure_data_directory(data_dir).expect("data directory");
        ensure_data_directory(&state_dir).expect("state directory");
        let state_path = state_dir.join("helix-state.db");
        create_private_file(&state_path).expect("state file");
        let mut connection = Connection::open(state_path).expect("open state");
        configure_state_connection(&connection).expect("configure state");
        apply_migration(&mut connection, 1, "foundational-state", STATE_MIGRATION_1)
            .expect("apply v1");
        crate::security::migrate_security(&mut connection).expect("apply v2");
        ensure_installation(&mut connection).expect("installation row")
    }

    fn only_migration_backup(data_dir: &Path) -> std::path::PathBuf {
        let directory = data_dir.join("state").join("migration-backups");
        let backups = fs::read_dir(directory)
            .expect("migration backup directory")
            .map(|entry| entry.expect("backup entry").path())
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        backups.into_iter().next().expect("one backup")
    }

    fn table_exists(connection: &Connection, table: &str) -> bool {
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
                 )",
                [table],
                |row| row.get(0),
            )
            .expect("check table")
    }
}
