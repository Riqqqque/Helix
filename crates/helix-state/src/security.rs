use super::{StateDatabase, StateError, apply_migration, random_uuid_v4};
use helix_auth::{DisplayName as CanonicalDisplayName, LoginName as CanonicalLoginName};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::fmt;
use subtle::ConstantTimeEq;
use uuid::Uuid;
use zeroize::Zeroizing;

const OWNER_ROLE_ID: &str = "00000000-0000-0000-0000-000000000001";
const BOOTSTRAP_MAX_LIFETIME_MS: i64 = 15 * 60 * 1_000;
const SESSION_IDLE_LIFETIME_MS: i64 = 30 * 60 * 1_000;
const SESSION_ABSOLUTE_LIFETIME_MS: i64 = 8 * 60 * 60 * 1_000;
const SESSION_PERSISTENT_LIFETIME_MS: i64 = 400 * 24 * 60 * 60 * 1_000;
const SESSION_TOUCH_INTERVAL_MS: i64 = 60 * 1_000;
const SESSION_ROW_RETENTION_MS: i64 = 24 * 60 * 60 * 1_000;
const SESSION_PRUNE_BATCH: i64 = 256;
const SESSION_ROW_LIMIT_DELETE_BATCH: i64 = 256;
const MAX_SESSION_ROWS_PER_USER: i64 = 64;
const MAX_FAILED_LOGIN_COUNT: i64 = 32;
const MAX_FAILED_LOGIN_DELAY_MS: i64 = 60 * 1_000;
const AUDIT_RETENTION_WINDOW_MS: i64 = 90 * 24 * 60 * 60 * 1_000;
const AUDIT_MIN_RETAINED_ROWS: i64 = 1_024;
const AUDIT_MAX_RETAINED_ROWS: i64 = 50_000;
const AUDIT_PRUNE_BATCH: i64 = 256;

#[derive(Clone, Copy)]
struct AuditRetentionPolicy {
    retention_window_ms: i64,
    minimum_rows: i64,
    maximum_rows: i64,
    prune_batch: i64,
}

const AUDIT_RETENTION_POLICY: AuditRetentionPolicy = AuditRetentionPolicy {
    retention_window_ms: AUDIT_RETENTION_WINDOW_MS,
    minimum_rows: AUDIT_MIN_RETAINED_ROWS,
    maximum_rows: AUDIT_MAX_RETAINED_ROWS,
    prune_batch: AUDIT_PRUNE_BATCH,
};

const AUDIT_APPEND_ONLY_DELETE_TRIGGER: &str = r#"
CREATE TRIGGER audit_events_append_only_delete
BEFORE DELETE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events are append-only');
END;
"#;

#[cfg(test)]
std::thread_local! {
    static FAIL_AUDIT_DELETE_GUARD_RECREATE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
fn fail_audit_delete_guard_recreate_for_test() {
    FAIL_AUDIT_DELETE_GUARD_RECREATE.with(|fail| fail.set(true));
}

const SECURITY_MIGRATION_2: &str = r#"
CREATE TABLE security_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    security_schema_version INTEGER NOT NULL CHECK (security_schema_version = 1),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
) STRICT;

INSERT INTO security_state (singleton, security_schema_version, created_at_unix_ms)
VALUES (1, 1, 0);

CREATE TABLE roles (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    name TEXT NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 128),
    is_system INTEGER NOT NULL CHECK (is_system IN (0, 1)),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
) STRICT;

CREATE TABLE capabilities (
    capability TEXT PRIMARY KEY CHECK (length(capability) BETWEEN 1 AND 128),
    description TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 512),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
) STRICT;

CREATE TABLE role_capabilities (
    role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    capability TEXT NOT NULL REFERENCES capabilities(capability) ON DELETE RESTRICT,
    granted_at_unix_ms INTEGER NOT NULL CHECK (granted_at_unix_ms >= 0),
    PRIMARY KEY (role_id, capability)
) STRICT;

CREATE INDEX role_capabilities_capability_idx
    ON role_capabilities (capability, role_id);

CREATE TABLE users (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    login_name TEXT NOT NULL UNIQUE CHECK (
        length(login_name) BETWEEN 3 AND 64
        AND login_name NOT GLOB '*[^a-z0-9._-]*'
        AND substr(login_name, 1, 1) GLOB '[a-z0-9]'
        AND substr(login_name, -1, 1) GLOB '[a-z0-9]'
    ),
    display_name TEXT NOT NULL CHECK (
        length(display_name) BETWEEN 1 AND 128
        AND length(CAST(display_name AS BLOB)) <= 512
        AND display_name = trim(display_name)
        AND instr(display_name, char(0)) = 0
        AND instr(display_name, char(9)) = 0
        AND instr(display_name, char(10)) = 0
        AND instr(display_name, char(13)) = 0
    ),
    password_phc TEXT NOT NULL CHECK (
        length(password_phc) BETWEEN 20 AND 1024
        AND password_phc GLOB '$argon2id$*'
        AND password_phc NOT GLOB '*[^!-~]*'
    ),
    password_policy_version INTEGER NOT NULL CHECK (password_policy_version >= 1),
    status TEXT NOT NULL CHECK (status IN ('active', 'disabled')),
    is_owner INTEGER NOT NULL DEFAULT 0 CHECK (is_owner IN (0, 1)),
    auth_version INTEGER NOT NULL DEFAULT 1 CHECK (auth_version >= 1),
    failed_login_count INTEGER NOT NULL DEFAULT 0
        CHECK (failed_login_count BETWEEN 0 AND 32),
    login_not_before_unix_ms INTEGER NOT NULL DEFAULT 0
        CHECK (login_not_before_unix_ms >= 0),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= created_at_unix_ms)
) STRICT;

CREATE UNIQUE INDEX users_single_owner_idx ON users (is_owner) WHERE is_owner = 1;
CREATE INDEX users_status_idx ON users (status, id);

CREATE TABLE user_roles (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE RESTRICT,
    assigned_at_unix_ms INTEGER NOT NULL CHECK (assigned_at_unix_ms >= 0),
    PRIMARY KEY (user_id, role_id)
) STRICT;

CREATE INDEX user_roles_role_idx ON user_roles (role_id, user_id);

CREATE TABLE bootstrap_tokens (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    expires_at_unix_ms INTEGER NOT NULL,
    consumed_at_unix_ms INTEGER,
    active_slot INTEGER UNIQUE CHECK (active_slot IS NULL OR active_slot = 1),
    CHECK (
        expires_at_unix_ms > created_at_unix_ms
        AND expires_at_unix_ms - created_at_unix_ms <= 900000
    ),
    CHECK (consumed_at_unix_ms IS NULL OR consumed_at_unix_ms >= created_at_unix_ms),
    CHECK (
        (consumed_at_unix_ms IS NULL AND active_slot = 1)
        OR (consumed_at_unix_ms IS NOT NULL AND active_slot IS NULL)
    )
) STRICT;

CREATE INDEX bootstrap_tokens_expiry_idx
    ON bootstrap_tokens (active_slot, expires_at_unix_ms);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    csrf_hash BLOB NOT NULL CHECK (length(csrf_hash) = 32),
    auth_version INTEGER NOT NULL CHECK (auth_version >= 1),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    last_seen_at_unix_ms INTEGER NOT NULL,
    absolute_expires_at_unix_ms INTEGER NOT NULL,
    revoked_at_unix_ms INTEGER,
    CHECK (last_seen_at_unix_ms >= created_at_unix_ms),
    CHECK (
        absolute_expires_at_unix_ms > created_at_unix_ms
        AND absolute_expires_at_unix_ms - created_at_unix_ms <= 28800000
    ),
    CHECK (last_seen_at_unix_ms <= absolute_expires_at_unix_ms),
    CHECK (revoked_at_unix_ms IS NULL OR revoked_at_unix_ms >= created_at_unix_ms)
) STRICT;

CREATE INDEX sessions_user_active_idx
    ON sessions (user_id, revoked_at_unix_ms, absolute_expires_at_unix_ms);
CREATE INDEX sessions_expiry_idx
    ON sessions (absolute_expires_at_unix_ms, revoked_at_unix_ms);

CREATE TABLE audit_events (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    occurred_at_unix_ms INTEGER NOT NULL CHECK (occurred_at_unix_ms >= 0),
    actor_user_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
    action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 128),
    target_type TEXT CHECK (target_type IS NULL OR length(target_type) BETWEEN 1 AND 64),
    target_id TEXT CHECK (target_id IS NULL OR length(target_id) BETWEEN 1 AND 255),
    outcome TEXT NOT NULL CHECK (outcome IN ('success', 'denied', 'error')),
    correlation_id TEXT NOT NULL CHECK (length(correlation_id) = 36),
    detail_json TEXT NOT NULL DEFAULT '{}' CHECK (
        json_valid(detail_json)
        AND json_type(detail_json) = 'object'
        AND length(detail_json) <= 4096
    )
) STRICT;

CREATE INDEX audit_events_time_idx ON audit_events (occurred_at_unix_ms DESC, id);
CREATE INDEX audit_events_actor_idx ON audit_events (actor_user_id, occurred_at_unix_ms DESC);
CREATE INDEX audit_events_action_idx ON audit_events (action, occurred_at_unix_ms DESC);

CREATE TRIGGER audit_events_append_only_update
BEFORE UPDATE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events are append-only');
END;

CREATE TRIGGER audit_events_append_only_delete
BEFORE DELETE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events are append-only');
END;

CREATE TRIGGER users_auth_version_monotonic
BEFORE UPDATE ON users
WHEN NEW.auth_version < OLD.auth_version
BEGIN
    SELECT RAISE(ABORT, 'auth_version cannot decrease');
END;

CREATE TRIGGER users_credential_change_requires_new_auth_version
BEFORE UPDATE OF password_phc, password_policy_version, status ON users
WHEN (
    NEW.password_phc <> OLD.password_phc
    OR NEW.password_policy_version <> OLD.password_policy_version
    OR NEW.status <> OLD.status
) AND NEW.auth_version <= OLD.auth_version
BEGIN
    SELECT RAISE(ABORT, 'credential changes must increase auth_version');
END;

CREATE TRIGGER user_roles_auth_version_insert
AFTER INSERT ON user_roles
BEGIN
    UPDATE users
    SET auth_version = auth_version + 1,
        updated_at_unix_ms = max(updated_at_unix_ms, NEW.assigned_at_unix_ms)
    WHERE id = NEW.user_id;
END;

CREATE TRIGGER user_roles_auth_version_delete
AFTER DELETE ON user_roles
BEGIN
    UPDATE users SET auth_version = auth_version + 1 WHERE id = OLD.user_id;
END;

CREATE TRIGGER role_capabilities_auth_version_insert
AFTER INSERT ON role_capabilities
BEGIN
    UPDATE users
    SET auth_version = auth_version + 1,
        updated_at_unix_ms = max(updated_at_unix_ms, NEW.granted_at_unix_ms)
    WHERE id IN (SELECT user_id FROM user_roles WHERE role_id = NEW.role_id);
END;

CREATE TRIGGER role_capabilities_auth_version_delete
AFTER DELETE ON role_capabilities
BEGIN
    UPDATE users
    SET auth_version = auth_version + 1
    WHERE id IN (SELECT user_id FROM user_roles WHERE role_id = OLD.role_id);
END;

INSERT INTO roles (id, name, is_system, created_at_unix_ms)
VALUES ('00000000-0000-0000-0000-000000000001', 'owner', 1, 0);

INSERT INTO capabilities (capability, description, created_at_unix_ms)
VALUES ('system.view', 'View Helix system status', 0);

INSERT INTO role_capabilities (role_id, capability, granted_at_unix_ms)
VALUES ('00000000-0000-0000-0000-000000000001', 'system.view', 0);
"#;

const SECURITY_MIGRATION_4: &str = r#"
CREATE TABLE audit_retention_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    retention_window_ms INTEGER NOT NULL CHECK (retention_window_ms = 7776000000),
    minimum_rows INTEGER NOT NULL CHECK (minimum_rows = 1024),
    maximum_rows INTEGER NOT NULL CHECK (maximum_rows = 50000),
    prune_batch INTEGER NOT NULL CHECK (prune_batch = 256),
    retained_event_count INTEGER NOT NULL CHECK (retained_event_count >= 0),
    CHECK (minimum_rows <= maximum_rows)
) STRICT;

INSERT INTO audit_retention_state (
    singleton, retention_window_ms, minimum_rows, maximum_rows,
    prune_batch, retained_event_count
)
SELECT 1, 7776000000, 1024, 50000, 256, count(*) FROM audit_events;

CREATE INDEX audit_events_retention_priority_idx
ON audit_events (
    CASE WHEN outcome = 'denied' THEN 0 ELSE 1 END,
    occurred_at_unix_ms,
    id
);

CREATE TRIGGER audit_retention_policy_immutable
BEFORE UPDATE OF retention_window_ms, minimum_rows, maximum_rows, prune_batch
ON audit_retention_state
BEGIN
    SELECT RAISE(ABORT, 'audit retention policy is immutable');
END;

CREATE TRIGGER audit_retention_state_append_only
BEFORE DELETE ON audit_retention_state
BEGIN
    SELECT RAISE(ABORT, 'audit retention state cannot be deleted');
END;

CREATE TRIGGER audit_events_retention_count_insert
AFTER INSERT ON audit_events
BEGIN
    SELECT CASE
        WHEN NOT EXISTS (
            SELECT 1 FROM audit_retention_state WHERE singleton = 1
        ) THEN RAISE(ABORT, 'audit retention state is missing')
    END;
    UPDATE audit_retention_state
    SET retained_event_count = retained_event_count + 1
    WHERE singleton = 1;
END;

CREATE TRIGGER audit_events_retention_count_delete
AFTER DELETE ON audit_events
BEGIN
    SELECT CASE
        WHEN NOT EXISTS (
            SELECT 1 FROM audit_retention_state
            WHERE singleton = 1 AND retained_event_count > 0
        ) THEN RAISE(ABORT, 'audit retention count is invalid')
    END;
    UPDATE audit_retention_state
    SET retained_event_count = retained_event_count - 1
    WHERE singleton = 1;
END;
"#;

const SECURITY_MIGRATION_5: &str = r#"
CREATE TABLE user_preferences (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    preferences_json TEXT NOT NULL CHECK (
        length(CAST(preferences_json AS BLOB)) BETWEEN 2 AND 65536
        AND json_valid(preferences_json)
        AND json_type(preferences_json) = 'object'
    ),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0)
) STRICT;

INSERT INTO capabilities (capability, description, created_at_unix_ms) VALUES
    ('dashboard.customize', 'Customize personal Helix dashboards', 0),
    ('games.view', 'Inspect managed game servers', 0),
    ('games.manage', 'Create and control managed game servers', 0),
    ('games.backups.manage', 'Delete and restore managed game backups', 0),
    ('network.firewall.read', 'Inspect firewall and port exposure state', 0),
    ('network.firewall.write', 'Change Helix-owned firewall rules', 0),
    ('storage.analyze', 'Analyze storage usage and large paths', 0),
    ('storage.files.read', 'Browse and read managed files', 0),
    ('storage.files.manage', 'Create, edit, rename, and trash managed files', 0),
    ('system.packages.read', 'Inspect operating system package updates', 0),
    ('system.packages.write', 'Apply operating system package updates', 0),
    ('system.power', 'Schedule and cancel host power operations', 0),
    ('system.settings.write', 'Change Helix host integration settings', 0),
    ('users.manage', 'Change Helix account credentials', 0);

INSERT INTO role_capabilities (role_id, capability, granted_at_unix_ms)
SELECT '00000000-0000-0000-0000-000000000001', capability, 0
FROM capabilities
WHERE capability IN (
    'dashboard.customize',
    'games.view',
    'games.manage',
    'games.backups.manage',
    'network.firewall.read',
    'network.firewall.write',
    'storage.analyze',
    'storage.files.read',
    'storage.files.manage',
    'system.packages.read',
    'system.packages.write',
    'system.power',
    'system.settings.write',
    'users.manage'
);
"#;

const SECURITY_MIGRATION_7: &str = r#"
INSERT INTO capabilities (capability, description, created_at_unix_ms)
VALUES ('terminal.open', 'Open an authenticated unprivileged host terminal', 0);

INSERT INTO role_capabilities (role_id, capability, granted_at_unix_ms)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'terminal.open',
    0
);
"#;

const SECURITY_MIGRATION_9: &str = r#"
ALTER TABLE security_state
ADD COLUMN session_expiry_enabled INTEGER NOT NULL DEFAULT 1
CHECK (session_expiry_enabled IN (0, 1));

CREATE TABLE sessions_new (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    csrf_hash BLOB NOT NULL CHECK (length(csrf_hash) = 32),
    auth_version INTEGER NOT NULL CHECK (auth_version >= 1),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    last_seen_at_unix_ms INTEGER NOT NULL,
    absolute_expires_at_unix_ms INTEGER NOT NULL,
    revoked_at_unix_ms INTEGER,
    CHECK (last_seen_at_unix_ms >= created_at_unix_ms),
    CHECK (
        absolute_expires_at_unix_ms > created_at_unix_ms
        AND absolute_expires_at_unix_ms - created_at_unix_ms <= 34560000000
    ),
    CHECK (last_seen_at_unix_ms <= absolute_expires_at_unix_ms),
    CHECK (revoked_at_unix_ms IS NULL OR revoked_at_unix_ms >= created_at_unix_ms)
) STRICT;

INSERT INTO sessions_new
SELECT id, user_id, token_hash, csrf_hash, auth_version,
       created_at_unix_ms, last_seen_at_unix_ms,
       absolute_expires_at_unix_ms, revoked_at_unix_ms
FROM sessions;

DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;

CREATE INDEX sessions_user_active_idx
    ON sessions (user_id, revoked_at_unix_ms, absolute_expires_at_unix_ms);
CREATE INDEX sessions_expiry_idx
    ON sessions (absolute_expires_at_unix_ms, revoked_at_unix_ms);
"#;

macro_rules! digest_type {
    ($name:ident) => {
        #[derive(Clone, Eq, Hash, PartialEq)]
        pub struct $name([u8; 32]);

        impl $name {
            #[must_use]
            pub const fn from_digest(digest: [u8; 32]) -> Self {
                Self(digest)
            }

            fn as_bytes(&self) -> &[u8] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }
    };
}

digest_type!(BootstrapTokenHash);
digest_type!(SessionTokenHash);
digest_type!(CsrfTokenHash);

#[derive(Clone, Eq, PartialEq)]
pub struct PasswordPhc(Zeroizing<String>);

impl PasswordPhc {
    pub fn new(value: String) -> Result<Self, StateError> {
        let value = Zeroizing::new(value);
        if !(20..=1024).contains(&value.len())
            || !value.starts_with("$argon2id$")
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(StateError::InvalidSecurityInput(
                "password PHC must be a bounded printable Argon2id encoding",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_for_verification(&self) -> &str {
        &self.0
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PasswordPhc {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PasswordPhc([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserStatus {
    Active,
    Disabled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupStatus {
    pub owner_exists: bool,
    pub bootstrap_available: bool,
    pub bootstrap_expires_at_unix_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootstrapInstallOutcome {
    Installed { expires_at_unix_ms: i64 },
    OwnerAlreadyExists,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootstrapPreflightOutcome {
    Match,
    OwnerAlreadyExists,
    Rejected,
}

pub struct OwnerClaimInput<'a> {
    pub bootstrap_hash: &'a BootstrapTokenHash,
    pub login_name: &'a str,
    pub display_name: &'a str,
    pub password_phc: &'a PasswordPhc,
    pub password_policy_version: i64,
    pub session_hash: &'a SessionTokenHash,
    pub csrf_hash: &'a CsrfTokenHash,
    pub now_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerClaimOutcome {
    Claimed {
        user_id: String,
        session_id: String,
        absolute_expires_at_unix_ms: i64,
    },
    Rejected(OwnerClaimRejection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerClaimRejection {
    OwnerAlreadyExists,
    NoActiveBootstrap,
    BootstrapExpired,
    BootstrapMismatch,
}

pub struct OwnerAccountUpdateInput<'a> {
    pub user_id: &'a str,
    pub expected_auth_version: i64,
    pub expected_password_phc: &'a PasswordPhc,
    pub expected_password_policy_version: i64,
    pub login_name: &'a str,
    pub display_name: &'a str,
    pub replacement_password: Option<PasswordRehash<'a>>,
    pub now_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerAccountUpdateOutcome {
    Updated,
    LoginNameUnavailable,
    CredentialChangedOrUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserPreferencesRecord {
    pub revision: i64,
    pub preferences_json: String,
    pub updated_at_unix_ms: i64,
}

pub struct UserPreferencesUpdateInput<'a> {
    pub user_id: &'a str,
    pub expected_revision: i64,
    pub preferences_json: &'a str,
    pub now_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserPreferencesUpdateOutcome {
    Updated(UserPreferencesRecord),
    Conflict(Option<UserPreferencesRecord>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialRecord {
    pub user_id: String,
    pub password_phc: PasswordPhc,
    pub password_policy_version: i64,
    pub status: UserStatus,
    pub auth_version: i64,
    pub failed_login_count: u32,
    pub login_not_before_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginDelayState {
    pub failed_login_count: u32,
    pub login_not_before_unix_ms: i64,
}

pub struct SessionCreateInput<'a> {
    pub user_id: &'a str,
    pub expected_auth_version: i64,
    pub expected_password_phc: &'a PasswordPhc,
    pub expected_password_policy_version: i64,
    pub rehash: Option<PasswordRehash<'a>>,
    pub session_hash: &'a SessionTokenHash,
    pub csrf_hash: &'a CsrfTokenHash,
    pub now_unix_ms: i64,
}

#[derive(Clone, Copy)]
pub struct PasswordRehash<'a> {
    pub replacement_password_phc: &'a PasswordPhc,
    pub replacement_password_policy_version: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionCreateOutcome {
    Created {
        session_id: String,
        auth_version: i64,
        absolute_expires_at_unix_ms: i64,
    },
    Delayed {
        retry_at_unix_ms: i64,
    },
    MaintenanceRequired {
        remaining_excess_rows: u64,
    },
    CredentialChangedOrUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedSession {
    pub session_id: String,
    pub user_id: String,
    pub login_name: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub auth_version: i64,
    pub absolute_expires_at_unix_ms: i64,
    pub last_seen_touched: bool,
    pub session_expires: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionExpiryUpdateOutcome {
    pub expires: bool,
    pub absolute_expires_at_unix_ms: i64,
}

#[derive(Clone, Copy)]
pub enum CsrfRequirement<'a> {
    NotRequired,
    Match(&'a CsrfTokenHash),
}

#[derive(Clone, Copy)]
pub enum SessionAuthorization<'a> {
    Authenticated,
    RequireCapability(&'a str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalAuditEvent {
    PasswordRejected,
    SessionOpened,
    SessionClosed,
    SessionFailed,
}

pub struct SessionAuthenticationInput<'a> {
    pub session_hash: &'a SessionTokenHash,
    pub authorization: SessionAuthorization<'a>,
    pub csrf: CsrfRequirement<'a>,
    pub now_unix_ms: i64,
}

pub(super) fn migrate_security(connection: &mut rusqlite::Connection) -> Result<(), StateError> {
    apply_migration(connection, 2, "owner-authentication", SECURITY_MIGRATION_2)
}

pub(super) fn migrate_audit_retention(
    connection: &mut rusqlite::Connection,
) -> Result<(), StateError> {
    apply_migration(
        connection,
        4,
        "bounded-authentication-audit-retention",
        SECURITY_MIGRATION_4,
    )
}

pub(super) fn migrate_preferences_and_capabilities(
    connection: &mut rusqlite::Connection,
) -> Result<(), StateError> {
    apply_migration(
        connection,
        5,
        "dashboard-preferences-and-owner-capabilities",
        SECURITY_MIGRATION_5,
    )
}

pub(super) fn migrate_terminal_capability(
    connection: &mut rusqlite::Connection,
) -> Result<(), StateError> {
    apply_migration(connection, 7, "terminal-capability", SECURITY_MIGRATION_7)
}

pub(super) fn migrate_session_expiry(
    connection: &mut rusqlite::Connection,
) -> Result<(), StateError> {
    apply_migration(
        connection,
        9,
        "optional-session-expiry",
        SECURITY_MIGRATION_9,
    )
}

impl StateDatabase {
    pub fn setup_status(&self, now_unix_ms: i64) -> Result<SetupStatus, StateError> {
        require_nonnegative_time(now_unix_ms)?;
        let connection = self.lock()?;
        let owner_exists = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE is_owner = 1)",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        let bootstrap_window = connection
            .query_row(
                "SELECT created_at_unix_ms, expires_at_unix_ms
                 FROM bootstrap_tokens
                 WHERE active_slot = 1 AND consumed_at_unix_ms IS NULL",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let bootstrap_expiry = bootstrap_window.map(|(_, expiry)| expiry);
        Ok(SetupStatus {
            owner_exists,
            bootstrap_available: !owner_exists
                && bootstrap_window.is_some_and(|(created, expiry)| {
                    created <= now_unix_ms && now_unix_ms < expiry
                }),
            bootstrap_expires_at_unix_ms: (!owner_exists).then_some(bootstrap_expiry).flatten(),
        })
    }

    pub fn preflight_bootstrap_claim(
        &self,
        token_hash: &BootstrapTokenHash,
        now_unix_ms: i64,
    ) -> Result<BootstrapPreflightOutcome, StateError> {
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if owner_exists(&transaction)? {
            return commit_bootstrap_preflight_rejection(
                transaction,
                now_unix_ms,
                BootstrapPreflightOutcome::OwnerAlreadyExists,
            );
        }
        let active_bootstrap = transaction
            .query_row(
                "SELECT token_hash, created_at_unix_ms, expires_at_unix_ms
                 FROM bootstrap_tokens
                 WHERE active_slot = 1 AND consumed_at_unix_ms IS NULL",
                [],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let matches = active_bootstrap.is_some_and(|(stored_hash, created, expires)| {
            created <= now_unix_ms
                && now_unix_ms < expires
                && digest_matches(stored_hash.as_slice(), token_hash.as_bytes())
        });
        if !matches {
            return commit_bootstrap_preflight_rejection(
                transaction,
                now_unix_ms,
                BootstrapPreflightOutcome::Rejected,
            );
        }
        transaction.commit()?;
        Ok(BootstrapPreflightOutcome::Match)
    }

    pub fn replace_bootstrap_token(
        &self,
        token_hash: &BootstrapTokenHash,
        now_unix_ms: i64,
        expires_at_unix_ms: i64,
    ) -> Result<BootstrapInstallOutcome, StateError> {
        validate_bootstrap_lifetime(now_unix_ms, expires_at_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if owner_exists(&transaction)? {
            transaction.commit()?;
            return Ok(BootstrapInstallOutcome::OwnerAlreadyExists);
        }

        transaction.execute(
            "UPDATE bootstrap_tokens
             SET consumed_at_unix_ms = max(?1, created_at_unix_ms), active_slot = NULL
             WHERE active_slot = 1 AND consumed_at_unix_ms IS NULL",
            [now_unix_ms],
        )?;
        let bootstrap_id = random_uuid_v4()?.to_string();
        transaction.execute(
            "INSERT INTO bootstrap_tokens (
                id, token_hash, created_at_unix_ms, expires_at_unix_ms,
                consumed_at_unix_ms, active_slot
             ) VALUES (?1, ?2, ?3, ?4, NULL, 1)",
            params![
                bootstrap_id,
                token_hash.as_bytes(),
                now_unix_ms,
                expires_at_unix_ms
            ],
        )?;
        append_audit(
            &transaction,
            now_unix_ms,
            None,
            "bootstrap.installed",
            Some("security_setup"),
            Some("owner"),
            "success",
        )?;
        transaction.commit()?;
        Ok(BootstrapInstallOutcome::Installed { expires_at_unix_ms })
    }

    pub fn claim_owner(&self, input: OwnerClaimInput<'_>) -> Result<OwnerClaimOutcome, StateError> {
        validate_owner_claim_input(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if owner_exists(&transaction)? {
            return commit_owner_rejection(
                transaction,
                input.now_unix_ms,
                OwnerClaimRejection::OwnerAlreadyExists,
            );
        }

        let active_bootstrap = transaction
            .query_row(
                "SELECT id, token_hash, created_at_unix_ms, expires_at_unix_ms
                 FROM bootstrap_tokens
                 WHERE active_slot = 1 AND consumed_at_unix_ms IS NULL",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((bootstrap_id, stored_hash, bootstrap_created, bootstrap_expiry)) =
            active_bootstrap
        else {
            return commit_owner_rejection(
                transaction,
                input.now_unix_ms,
                OwnerClaimRejection::NoActiveBootstrap,
            );
        };
        if input.now_unix_ms < bootstrap_created || bootstrap_expiry <= input.now_unix_ms {
            return commit_owner_rejection(
                transaction,
                input.now_unix_ms,
                OwnerClaimRejection::BootstrapExpired,
            );
        }
        if !digest_matches(stored_hash.as_slice(), input.bootstrap_hash.as_bytes()) {
            return commit_owner_rejection(
                transaction,
                input.now_unix_ms,
                OwnerClaimRejection::BootstrapMismatch,
            );
        }

        let user_id = random_uuid_v4()?.to_string();
        transaction.execute(
            "INSERT INTO users (
                id, login_name, display_name, password_phc, password_policy_version,
                status, is_owner,
                auth_version, failed_login_count, login_not_before_unix_ms,
                created_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', 1, 1, 0, 0, ?6, ?6)",
            params![
                user_id,
                input.login_name,
                input.display_name,
                input.password_phc.as_str(),
                input.password_policy_version,
                input.now_unix_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO user_roles (user_id, role_id, assigned_at_unix_ms)
             VALUES (?1, ?2, ?3)",
            params![user_id, OWNER_ROLE_ID, input.now_unix_ms],
        )?;
        let auth_version = transaction.query_row(
            "SELECT auth_version FROM users WHERE id = ?1",
            [&user_id],
            |row| row.get::<_, i64>(0),
        )?;
        let session_id = random_uuid_v4()?.to_string();
        let session_expires = session_expiry_enabled_in(&transaction)?;
        let absolute_expiry = checked_deadline(
            input.now_unix_ms,
            session_lifetime_ms(session_expires),
            "session absolute expiry overflowed",
        )?;
        transaction.execute(
            "INSERT INTO sessions (
                id, user_id, token_hash, csrf_hash, auth_version,
                created_at_unix_ms, last_seen_at_unix_ms,
                absolute_expires_at_unix_ms, revoked_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, NULL)",
            params![
                session_id,
                user_id,
                input.session_hash.as_bytes(),
                input.csrf_hash.as_bytes(),
                auth_version,
                input.now_unix_ms,
                absolute_expiry
            ],
        )?;
        let consumed = transaction.execute(
            "UPDATE bootstrap_tokens
             SET consumed_at_unix_ms = ?1, active_slot = NULL
             WHERE id = ?2 AND active_slot = 1 AND consumed_at_unix_ms IS NULL
                   AND expires_at_unix_ms > ?1 AND token_hash = ?3",
            params![
                input.now_unix_ms,
                bootstrap_id,
                input.bootstrap_hash.as_bytes()
            ],
        )?;
        if consumed != 1 {
            return Err(StateError::InvalidSecurityInput(
                "bootstrap changed during owner claim",
            ));
        }
        append_audit(
            &transaction,
            input.now_unix_ms,
            Some(&user_id),
            "owner.claimed",
            Some("user"),
            Some(&user_id),
            "success",
        )?;
        transaction.commit()?;
        Ok(OwnerClaimOutcome::Claimed {
            user_id,
            session_id,
            absolute_expires_at_unix_ms: absolute_expiry,
        })
    }
}

fn owner_exists(transaction: &Transaction<'_>) -> Result<bool, StateError> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE is_owner = 1)",
        [],
        |row| row.get::<_, bool>(0),
    )?)
}

fn commit_owner_rejection(
    transaction: Transaction<'_>,
    now_unix_ms: i64,
    rejection: OwnerClaimRejection,
) -> Result<OwnerClaimOutcome, StateError> {
    append_audit(
        &transaction,
        now_unix_ms,
        None,
        "bootstrap.claim",
        Some("security_setup"),
        Some("owner"),
        "denied",
    )?;
    transaction.commit()?;
    Ok(OwnerClaimOutcome::Rejected(rejection))
}

fn commit_bootstrap_preflight_rejection(
    transaction: Transaction<'_>,
    now_unix_ms: i64,
    outcome: BootstrapPreflightOutcome,
) -> Result<BootstrapPreflightOutcome, StateError> {
    append_audit(
        &transaction,
        now_unix_ms,
        None,
        "bootstrap.claim",
        Some("security_setup"),
        Some("owner"),
        "denied",
    )?;
    transaction.commit()?;
    Ok(outcome)
}

fn validate_bootstrap_lifetime(
    now_unix_ms: i64,
    expires_at_unix_ms: i64,
) -> Result<(), StateError> {
    require_nonnegative_time(now_unix_ms)?;
    let lifetime =
        expires_at_unix_ms
            .checked_sub(now_unix_ms)
            .ok_or(StateError::InvalidSecurityInput(
                "bootstrap expiry is outside the supported range",
            ))?;
    if !(1..=BOOTSTRAP_MAX_LIFETIME_MS).contains(&lifetime) {
        return Err(StateError::InvalidSecurityInput(
            "bootstrap lifetime must be positive and no longer than 15 minutes",
        ));
    }
    Ok(())
}

fn validate_owner_claim_input(input: &OwnerClaimInput<'_>) -> Result<(), StateError> {
    require_nonnegative_time(input.now_unix_ms)?;
    validate_login_name(input.login_name)?;
    validate_display_name(input.display_name)?;
    if input.password_policy_version < 1 {
        return Err(StateError::InvalidSecurityInput(
            "password policy version must be positive",
        ));
    }
    Ok(())
}

fn validate_login_name(value: &str) -> Result<(), StateError> {
    if CanonicalLoginName::parse(value).is_ok() {
        Ok(())
    } else {
        Err(StateError::InvalidSecurityInput(
            "login name must be canonical lowercase ASCII, 3-64 bytes, with alphanumeric endpoints",
        ))
    }
}

fn validate_display_name(value: &str) -> Result<(), StateError> {
    let canonical = CanonicalDisplayName::parse(value)
        .map_err(|_| StateError::InvalidSecurityInput("invalid display name"))?;
    if canonical.as_str() != value {
        return Err(StateError::InvalidSecurityInput("invalid display name"));
    }
    Ok(())
}

fn require_nonnegative_time(now_unix_ms: i64) -> Result<(), StateError> {
    if now_unix_ms < 0 {
        Err(StateError::InvalidSecurityInput(
            "security timestamps must be nonnegative",
        ))
    } else {
        Ok(())
    }
}

fn checked_deadline(
    now_unix_ms: i64,
    duration_ms: i64,
    message: &'static str,
) -> Result<i64, StateError> {
    now_unix_ms
        .checked_add(duration_ms)
        .ok_or(StateError::InvalidSecurityInput(message))
}

fn append_audit(
    transaction: &Transaction<'_>,
    now_unix_ms: i64,
    actor_user_id: Option<&str>,
    action: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    outcome: &str,
) -> Result<(), StateError> {
    transaction.execute(
        "INSERT INTO audit_events (
            id, occurred_at_unix_ms, actor_user_id, action, target_type,
            target_id, outcome, correlation_id, detail_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}')",
        params![
            random_uuid_v4()?.to_string(),
            now_unix_ms,
            actor_user_id,
            action,
            target_type,
            target_id,
            outcome,
            random_uuid_v4()?.to_string()
        ],
    )?;
    prune_authentication_audit_in(transaction, now_unix_ms, AUDIT_RETENTION_POLICY)?;
    Ok(())
}

fn prune_authentication_audit_in(
    transaction: &Transaction<'_>,
    now_unix_ms: i64,
    policy: AuditRetentionPolicy,
) -> Result<usize, StateError> {
    if policy.retention_window_ms <= 0
        || policy.minimum_rows < 0
        || policy.maximum_rows < policy.minimum_rows
        || policy.prune_batch <= 0
    {
        return Err(StateError::InvalidSecurityInput(
            "invalid authentication audit retention policy",
        ));
    }

    let retained_count = transaction.query_row(
        "SELECT retained_event_count
         FROM audit_retention_state WHERE singleton = 1",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let overflow_delete = retained_count
        .saturating_sub(policy.maximum_rows)
        .max(0)
        .min(policy.prune_batch);
    let time_delete_limit = retained_count
        .saturating_sub(overflow_delete)
        .saturating_sub(policy.minimum_rows)
        .max(0)
        .min(policy.prune_batch.saturating_sub(overflow_delete));
    let retention_cutoff = now_unix_ms.saturating_sub(policy.retention_window_ms);
    let has_expired_rows = if time_delete_limit > 0 {
        transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM audit_events
                 WHERE occurred_at_unix_ms < ?1
                 LIMIT 1
             )",
            [retention_cutoff],
            |row| row.get::<_, bool>(0),
        )?
    } else {
        false
    };
    if overflow_delete == 0 && !has_expired_rows {
        return Ok(0);
    }

    transaction.execute_batch("DROP TRIGGER audit_events_append_only_delete;")?;
    let delete_result = transaction.execute(
        "WITH
             protected_newest(id) AS MATERIALIZED (
                 SELECT id FROM audit_events
                 ORDER BY occurred_at_unix_ms DESC, id DESC
                 LIMIT ?1
             ),
             overflow_candidates(id) AS MATERIALIZED (
                 SELECT id FROM audit_events
                 WHERE id NOT IN (SELECT id FROM protected_newest)
                 ORDER BY
                     CASE WHEN outcome = 'denied' THEN 0 ELSE 1 END,
                     occurred_at_unix_ms,
                     id
                 LIMIT ?2
             ),
             time_candidates(id) AS MATERIALIZED (
                 SELECT id FROM audit_events
                 WHERE occurred_at_unix_ms < ?3
                       AND id NOT IN (SELECT id FROM protected_newest)
                       AND id NOT IN (SELECT id FROM overflow_candidates)
                 ORDER BY occurred_at_unix_ms, id
                 LIMIT ?4
             )
         DELETE FROM audit_events
         WHERE id IN (
             SELECT id FROM overflow_candidates
             UNION ALL
             SELECT id FROM time_candidates
         )",
        params![
            policy.minimum_rows,
            overflow_delete,
            retention_cutoff,
            time_delete_limit
        ],
    );
    #[cfg(test)]
    let inject_recreate_failure = FAIL_AUDIT_DELETE_GUARD_RECREATE.with(|fail| fail.replace(false));
    #[cfg(not(test))]
    let inject_recreate_failure = false;
    let recreate_result = if inject_recreate_failure {
        Err(rusqlite::Error::ExecuteReturnedResults)
    } else {
        transaction.execute_batch(AUDIT_APPEND_ONLY_DELETE_TRIGGER)
    };
    let deleted = delete_result?;
    recreate_result?;

    if deleted < usize::try_from(overflow_delete).unwrap_or(usize::MAX) {
        return Err(StateError::Integrity {
            database: "helix-state.db",
            details: vec!["audit retention could not remove the bounded record overage".to_owned()],
        });
    }
    let retained_after = transaction.query_row(
        "SELECT retained_event_count
         FROM audit_retention_state WHERE singleton = 1",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let deleted_i64 = i64::try_from(deleted).unwrap_or(i64::MAX);
    if retained_after != retained_count.saturating_sub(deleted_i64) {
        return Err(StateError::Integrity {
            database: "helix-state.db",
            details: vec!["audit retention counter did not match the pruned rows".to_owned()],
        });
    }
    Ok(deleted)
}

impl StateDatabase {
    /// Look up the password verifier and its compare-and-swap version. This is
    /// the only state API that returns a password PHC.
    pub fn credential_by_login(
        &self,
        login_name: &str,
        now_unix_ms: i64,
    ) -> Result<Option<CredentialRecord>, StateError> {
        validate_login_name(login_name)?;
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = transaction
            .query_row(
                "SELECT id, password_phc, password_policy_version, status, auth_version,
                        failed_login_count, login_not_before_unix_ms
                 FROM users WHERE login_name = ?1",
                [login_name],
                |row| {
                    let status: String = row.get(3)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        status,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            user_id,
            password_phc,
            password_policy_version,
            status,
            auth_version,
            failed_count,
            not_before,
        )) = stored
        else {
            transaction.commit()?;
            return Ok(None);
        };
        let mut credential = CredentialRecord {
            user_id,
            password_phc: PasswordPhc::new(password_phc)?,
            password_policy_version,
            status: parse_user_status(&status)?,
            auth_version,
            failed_login_count: u32::try_from(failed_count).map_err(|_| {
                StateError::InvalidSecurityInput(
                    "stored failed-login count is outside the supported range",
                )
            })?,
            login_not_before_unix_ms: not_before,
        };
        let maximum_deadline = checked_deadline(
            now_unix_ms,
            MAX_FAILED_LOGIN_DELAY_MS,
            "failed-login delay overflowed",
        )?;
        if credential.login_not_before_unix_ms > maximum_deadline {
            let clamped = transaction.execute(
                "UPDATE users
                 SET login_not_before_unix_ms = ?1,
                     updated_at_unix_ms = max(updated_at_unix_ms, ?2)
                 WHERE id = ?3 AND auth_version = ?4
                       AND login_not_before_unix_ms = ?5",
                params![
                    maximum_deadline,
                    now_unix_ms,
                    credential.user_id,
                    credential.auth_version,
                    credential.login_not_before_unix_ms
                ],
            )?;
            if clamped != 1 {
                return Err(StateError::InvalidSecurityInput(
                    "login delay changed during clock-anomaly repair",
                ));
            }
            append_audit(
                &transaction,
                now_unix_ms,
                None,
                "authentication.delay_clock_clamped",
                Some("user"),
                Some(&credential.user_id),
                "success",
            )?;
            credential.login_not_before_unix_ms = maximum_deadline;
        }
        transaction.commit()?;
        Ok(Some(credential))
    }

    /// Atomically replace the owner's public identity and, optionally, password.
    /// Every successful change advances the authentication version and revokes
    /// all sessions so a stolen session cannot survive a credential change.
    pub fn update_owner_account(
        &self,
        input: OwnerAccountUpdateInput<'_>,
    ) -> Result<OwnerAccountUpdateOutcome, StateError> {
        require_identifier(input.user_id, "invalid user identifier")?;
        require_nonnegative_time(input.now_unix_ms)?;
        validate_login_name(input.login_name)?;
        validate_display_name(input.display_name)?;
        if input.expected_auth_version < 1 {
            return Err(StateError::InvalidSecurityInput(
                "auth version must be positive",
            ));
        }
        if input.expected_password_policy_version < 1 {
            return Err(StateError::InvalidSecurityInput(
                "password policy version must be positive",
            ));
        }
        if let Some(replacement) = input.replacement_password
            && replacement.replacement_password_policy_version
                < input.expected_password_policy_version
        {
            return Err(StateError::InvalidSecurityInput(
                "replacement password policy version cannot decrease",
            ));
        }

        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT auth_version, password_phc, password_policy_version, status, is_owner
                 FROM users WHERE id = ?1",
                [input.user_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((auth_version, password_phc, password_policy_version, status, is_owner)) = current
        else {
            transaction.commit()?;
            return Ok(OwnerAccountUpdateOutcome::CredentialChangedOrUnavailable);
        };
        let password_phc = PasswordPhc::new(password_phc)?;
        if status != "active"
            || is_owner != 1
            || auth_version != input.expected_auth_version
            || password_phc != *input.expected_password_phc
            || password_policy_version != input.expected_password_policy_version
        {
            transaction.commit()?;
            return Ok(OwnerAccountUpdateOutcome::CredentialChangedOrUnavailable);
        }

        let login_in_use = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE login_name = ?1 AND id <> ?2)",
            params![input.login_name, input.user_id],
            |row| row.get::<_, bool>(0),
        )?;
        if login_in_use {
            transaction.commit()?;
            return Ok(OwnerAccountUpdateOutcome::LoginNameUnavailable);
        }

        let (replacement_phc, replacement_policy_version) = input.replacement_password.map_or(
            (
                input.expected_password_phc.as_str(),
                input.expected_password_policy_version,
            ),
            |replacement| {
                (
                    replacement.replacement_password_phc.as_str(),
                    replacement.replacement_password_policy_version,
                )
            },
        );
        let next_auth_version =
            input
                .expected_auth_version
                .checked_add(1)
                .ok_or(StateError::InvalidSecurityInput(
                    "auth version overflowed during account update",
                ))?;
        let updated = transaction.execute(
            "UPDATE users
             SET login_name = ?1, display_name = ?2, password_phc = ?3,
                 password_policy_version = ?4, auth_version = ?5,
                 failed_login_count = 0, login_not_before_unix_ms = 0,
                 updated_at_unix_ms = max(updated_at_unix_ms, ?6)
             WHERE id = ?7 AND is_owner = 1 AND status = 'active'
                   AND auth_version = ?8 AND password_phc = ?9
                   AND password_policy_version = ?10",
            params![
                input.login_name,
                input.display_name,
                replacement_phc,
                replacement_policy_version,
                next_auth_version,
                input.now_unix_ms,
                input.user_id,
                input.expected_auth_version,
                input.expected_password_phc.as_str(),
                input.expected_password_policy_version,
            ],
        )?;
        if updated != 1 {
            transaction.commit()?;
            return Ok(OwnerAccountUpdateOutcome::CredentialChangedOrUnavailable);
        }
        transaction.execute("DELETE FROM sessions WHERE user_id = ?1", [input.user_id])?;
        append_audit(
            &transaction,
            input.now_unix_ms,
            Some(input.user_id),
            "account.owner_updated",
            Some("user"),
            Some(input.user_id),
            "success",
        )?;
        transaction.commit()?;
        Ok(OwnerAccountUpdateOutcome::Updated)
    }

    /// Record a rejected current-password proof for an authenticated owner
    /// account update without coupling it to login-delay state.
    pub fn record_owner_account_password_rejection(
        &self,
        user_id: &str,
        now_unix_ms: i64,
    ) -> Result<(), StateError> {
        require_identifier(user_id, "invalid user identifier")?;
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        append_audit(
            &transaction,
            now_unix_ms,
            Some(user_id),
            "account.owner_update_password_rejected",
            Some("user"),
            Some(user_id),
            "denied",
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Record terminal authorization and lifecycle outcomes without recording
    /// terminal commands, input, output, environment values, or paths.
    pub fn record_terminal_audit(
        &self,
        user_id: &str,
        event: TerminalAuditEvent,
        now_unix_ms: i64,
    ) -> Result<(), StateError> {
        require_identifier(user_id, "invalid user identifier")?;
        require_nonnegative_time(now_unix_ms)?;
        let (action, outcome) = match event {
            TerminalAuditEvent::PasswordRejected => ("terminal.password_rejected", "denied"),
            TerminalAuditEvent::SessionOpened => ("terminal.session_opened", "success"),
            TerminalAuditEvent::SessionClosed => ("terminal.session_closed", "success"),
            TerminalAuditEvent::SessionFailed => ("terminal.session_failed", "error"),
        };
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        append_audit(
            &transaction,
            now_unix_ms,
            Some(user_id),
            action,
            Some("terminal"),
            None,
            outcome,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn user_preferences(
        &self,
        user_id: &str,
    ) -> Result<Option<UserPreferencesRecord>, StateError> {
        require_identifier(user_id, "invalid user identifier")?;
        let connection = self.lock()?;
        load_user_preferences(&connection, user_id)
    }

    pub fn update_user_preferences(
        &self,
        input: UserPreferencesUpdateInput<'_>,
    ) -> Result<UserPreferencesUpdateOutcome, StateError> {
        require_identifier(input.user_id, "invalid user identifier")?;
        require_nonnegative_time(input.now_unix_ms)?;
        if input.expected_revision < 0 {
            return Err(StateError::InvalidSecurityInput(
                "preference revision cannot be negative",
            ));
        }
        let next_revision =
            input
                .expected_revision
                .checked_add(1)
                .ok_or(StateError::InvalidSecurityInput(
                    "preference revision overflowed",
                ))?;
        if !(2..=65_536).contains(&input.preferences_json.len()) {
            return Err(StateError::InvalidSecurityInput(
                "preferences must be a bounded JSON object",
            ));
        }

        let mut connection = self.lock()?;
        let valid_json =
            connection.query_row("SELECT json_valid(?1)", [input.preferences_json], |row| {
                row.get::<_, bool>(0)
            })?;
        if !valid_json
            || !connection.query_row(
                "SELECT json_type(?1) = 'object'",
                [input.preferences_json],
                |row| row.get::<_, bool>(0),
            )?
        {
            return Err(StateError::InvalidSecurityInput(
                "preferences must be a bounded JSON object",
            ));
        }

        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_user_preferences(&transaction, input.user_id)?;
        if current.as_ref().map_or(0, |record| record.revision) != input.expected_revision {
            transaction.commit()?;
            return Ok(UserPreferencesUpdateOutcome::Conflict(current));
        }
        if let Some(current) = current
            && current.preferences_json == input.preferences_json
        {
            transaction.commit()?;
            return Ok(UserPreferencesUpdateOutcome::Updated(current));
        }
        if input.expected_revision == 0 {
            let inserted = transaction.execute(
                "INSERT INTO user_preferences (
                    user_id, revision, preferences_json, updated_at_unix_ms
                 )
                 SELECT id, ?1, ?2, ?3 FROM users
                 WHERE id = ?4 AND status = 'active'",
                params![
                    next_revision,
                    input.preferences_json,
                    input.now_unix_ms,
                    input.user_id,
                ],
            )?;
            if inserted != 1 {
                transaction.commit()?;
                return Ok(UserPreferencesUpdateOutcome::Conflict(None));
            }
        } else {
            let updated = transaction.execute(
                "UPDATE user_preferences
                 SET revision = ?1, preferences_json = ?2,
                     updated_at_unix_ms = max(updated_at_unix_ms, ?3)
                 WHERE user_id = ?4 AND revision = ?5",
                params![
                    next_revision,
                    input.preferences_json,
                    input.now_unix_ms,
                    input.user_id,
                    input.expected_revision,
                ],
            )?;
            if updated != 1 {
                let current = load_user_preferences(&transaction, input.user_id)?;
                transaction.commit()?;
                return Ok(UserPreferencesUpdateOutcome::Conflict(current));
            }
        }
        // Layout autosaves are ordinary user data, not security events. Keeping
        // them out of the append-only authentication audit prevents rapid
        // widget edits from displacing higher-value access and denial records.
        let updated = load_user_preferences(&transaction, input.user_id)?.ok_or(
            StateError::InvalidSecurityInput("saved preferences could not be read back"),
        )?;
        transaction.commit()?;
        Ok(UserPreferencesUpdateOutcome::Updated(updated))
    }

    pub fn record_failed_login(
        &self,
        user_id: &str,
        now_unix_ms: i64,
    ) -> Result<Option<LoginDelayState>, StateError> {
        require_identifier(user_id, "invalid user identifier")?;
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT failed_login_count, login_not_before_unix_ms
                 FROM users WHERE id = ?1",
                [user_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((current_count, current_not_before)) = current else {
            transaction.commit()?;
            return Ok(None);
        };
        let next_count = (current_count + 1).min(MAX_FAILED_LOGIN_COUNT);
        let delay_ms = failed_login_delay_ms(next_count);
        let maximum_deadline = checked_deadline(
            now_unix_ms,
            MAX_FAILED_LOGIN_DELAY_MS,
            "failed-login delay overflowed",
        )?;
        let base = current_not_before.max(now_unix_ms).min(maximum_deadline);
        let next_not_before = base.saturating_add(delay_ms).min(maximum_deadline);
        transaction.execute(
            "UPDATE users
             SET failed_login_count = ?1, login_not_before_unix_ms = ?2
             WHERE id = ?3",
            params![next_count, next_not_before, user_id],
        )?;
        append_audit(
            &transaction,
            now_unix_ms,
            None,
            "authentication.failed",
            Some("user"),
            Some(user_id),
            "denied",
        )?;
        transaction.commit()?;
        Ok(Some(LoginDelayState {
            failed_login_count: u32::try_from(next_count).unwrap_or(u32::MAX),
            login_not_before_unix_ms: next_not_before,
        }))
    }

    pub fn reset_failed_login(
        &self,
        user_id: &str,
        expected_auth_version: i64,
        now_unix_ms: i64,
    ) -> Result<bool, StateError> {
        require_identifier(user_id, "invalid user identifier")?;
        require_nonnegative_time(now_unix_ms)?;
        if expected_auth_version < 1 {
            return Err(StateError::InvalidSecurityInput(
                "auth version must be positive",
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reset = transaction.execute(
            "UPDATE users
             SET failed_login_count = 0, login_not_before_unix_ms = 0
             WHERE id = ?1 AND auth_version = ?2",
            params![user_id, expected_auth_version],
        )? == 1;
        if reset {
            append_audit(
                &transaction,
                now_unix_ms,
                Some(user_id),
                "authentication.delay_reset",
                Some("user"),
                Some(user_id),
                "success",
            )?;
        }
        transaction.commit()?;
        Ok(reset)
    }

    pub fn create_session_after_verified_login(
        &self,
        input: SessionCreateInput<'_>,
    ) -> Result<SessionCreateOutcome, StateError> {
        require_identifier(input.user_id, "invalid user identifier")?;
        require_nonnegative_time(input.now_unix_ms)?;
        if input.expected_auth_version < 1 {
            return Err(StateError::InvalidSecurityInput(
                "auth version must be positive",
            ));
        }
        if input.expected_password_policy_version < 1 {
            return Err(StateError::InvalidSecurityInput(
                "password policy version must be positive",
            ));
        }
        if let Some(rehash) = input.rehash
            && rehash.replacement_password_policy_version < input.expected_password_policy_version
        {
            return Err(StateError::InvalidSecurityInput(
                "replacement password policy version cannot decrease",
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT status, auth_version, password_phc,
                            password_policy_version, login_not_before_unix_ms
                     FROM users WHERE id = ?1",
                [input.user_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((status, auth_version, password_phc, password_policy_version, not_before)) =
            current
        else {
            transaction.commit()?;
            return Ok(SessionCreateOutcome::CredentialChangedOrUnavailable);
        };
        let password_phc = PasswordPhc::new(password_phc)?;
        if status != "active"
            || auth_version != input.expected_auth_version
            || password_phc != *input.expected_password_phc
            || password_policy_version != input.expected_password_policy_version
        {
            append_audit(
                &transaction,
                input.now_unix_ms,
                None,
                "authentication.session_rejected",
                Some("user"),
                Some(input.user_id),
                "denied",
            )?;
            transaction.commit()?;
            return Ok(SessionCreateOutcome::CredentialChangedOrUnavailable);
        }
        if not_before > input.now_unix_ms {
            transaction.commit()?;
            return Ok(SessionCreateOutcome::Delayed {
                retry_at_unix_ms: not_before,
            });
        }

        let mut existing_session_rows = transaction.query_row(
            "SELECT count(*) FROM sessions WHERE user_id = ?1",
            [input.user_id],
            |row| row.get::<_, i64>(0),
        )?;
        if existing_session_rows > MAX_SESSION_ROWS_PER_USER {
            let deleted = delete_excess_session_rows_in_batch(
                &transaction,
                input.user_id,
                MAX_SESSION_ROWS_PER_USER,
            )?;
            existing_session_rows = existing_session_rows
                .checked_sub(i64::try_from(deleted).map_err(|_| {
                    StateError::InvalidSecurityInput("session cleanup count overflowed")
                })?)
                .ok_or(StateError::InvalidSecurityInput(
                    "session cleanup removed an invalid row count",
                ))?;
        }
        if existing_session_rows > MAX_SESSION_ROWS_PER_USER {
            let remaining_excess_rows =
                u64::try_from(existing_session_rows.saturating_sub(MAX_SESSION_ROWS_PER_USER))
                    .map_err(|_| {
                        StateError::InvalidSecurityInput("session cleanup count was invalid")
                    })?;
            transaction.commit()?;
            return Ok(SessionCreateOutcome::MaintenanceRequired {
                remaining_excess_rows,
            });
        }
        if existing_session_rows == MAX_SESSION_ROWS_PER_USER {
            let evicted = transaction.execute(
                "DELETE FROM sessions
                     WHERE id = (
                         SELECT id FROM sessions
                         WHERE user_id = ?1
                         ORDER BY created_at_unix_ms ASC, id ASC
                         LIMIT 1
                     )",
                [input.user_id],
            )?;
            if evicted != 1 {
                return Err(StateError::InvalidSecurityInput(
                    "session row cap changed during creation",
                ));
            }
        }

        let resulting_auth_version =
            if input.rehash.is_some() {
                input.expected_auth_version.checked_add(1).ok_or(
                    StateError::InvalidSecurityInput("auth version overflowed during rehash"),
                )?
            } else {
                input.expected_auth_version
            };
        #[cfg(test)]
        let force_cas_miss = crate::take_verified_login_cas_miss_for_test();
        #[cfg(not(test))]
        let force_cas_miss = false;
        let reset = if force_cas_miss {
            0
        } else if let Some(rehash) = input.rehash {
            transaction.execute(
                "UPDATE users
                     SET password_phc = ?1, password_policy_version = ?2,
                         auth_version = ?3, failed_login_count = 0,
                         login_not_before_unix_ms = 0,
                         updated_at_unix_ms = max(updated_at_unix_ms, ?4)
                     WHERE id = ?5 AND status = 'active' AND auth_version = ?6
                           AND password_phc = ?7 AND password_policy_version = ?8
                           AND login_not_before_unix_ms <= ?4",
                params![
                    rehash.replacement_password_phc.as_str(),
                    rehash.replacement_password_policy_version,
                    resulting_auth_version,
                    input.now_unix_ms,
                    input.user_id,
                    input.expected_auth_version,
                    input.expected_password_phc.as_str(),
                    input.expected_password_policy_version
                ],
            )?
        } else {
            transaction.execute(
                "UPDATE users
                     SET failed_login_count = 0, login_not_before_unix_ms = 0
                     WHERE id = ?1 AND status = 'active' AND auth_version = ?2
                           AND password_phc = ?3 AND password_policy_version = ?4
                           AND login_not_before_unix_ms <= ?5",
                params![
                    input.user_id,
                    input.expected_auth_version,
                    input.expected_password_phc.as_str(),
                    input.expected_password_policy_version,
                    input.now_unix_ms
                ],
            )?
        };
        if reset != 1 {
            return Ok(SessionCreateOutcome::CredentialChangedOrUnavailable);
        }

        let session_id = random_uuid_v4()?.to_string();
        let session_expires = session_expiry_enabled_in(&transaction)?;
        let absolute_expiry = checked_deadline(
            input.now_unix_ms,
            session_lifetime_ms(session_expires),
            "session absolute expiry overflowed",
        )?;
        transaction.execute(
            "INSERT INTO sessions (
                    id, user_id, token_hash, csrf_hash, auth_version,
                    created_at_unix_ms, last_seen_at_unix_ms,
                    absolute_expires_at_unix_ms, revoked_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, NULL)",
            params![
                session_id,
                input.user_id,
                input.session_hash.as_bytes(),
                input.csrf_hash.as_bytes(),
                resulting_auth_version,
                input.now_unix_ms,
                absolute_expiry
            ],
        )?;
        prune_stale_sessions_in(&transaction, input.now_unix_ms, SESSION_PRUNE_BATCH)?;
        append_audit(
            &transaction,
            input.now_unix_ms,
            Some(input.user_id),
            "authentication.succeeded",
            Some("session"),
            Some(&session_id),
            "success",
        )?;
        transaction.commit()?;
        Ok(SessionCreateOutcome::Created {
            session_id,
            auth_version: resulting_auth_version,
            absolute_expires_at_unix_ms: absolute_expiry,
        })
    }
}

fn parse_user_status(value: &str) -> Result<UserStatus, StateError> {
    match value {
        "active" => Ok(UserStatus::Active),
        "disabled" => Ok(UserStatus::Disabled),
        _ => Err(StateError::InvalidSecurityInput(
            "stored user status is unsupported",
        )),
    }
}

fn load_user_preferences(
    connection: &rusqlite::Connection,
    user_id: &str,
) -> Result<Option<UserPreferencesRecord>, StateError> {
    Ok(connection
        .query_row(
            "SELECT revision, preferences_json, updated_at_unix_ms
             FROM user_preferences WHERE user_id = ?1",
            [user_id],
            |row| {
                Ok(UserPreferencesRecord {
                    revision: row.get(0)?,
                    preferences_json: row.get(1)?,
                    updated_at_unix_ms: row.get(2)?,
                })
            },
        )
        .optional()?)
}

fn failed_login_delay_ms(failed_count: i64) -> i64 {
    let exponent = u32::try_from((failed_count - 1).clamp(0, 6)).unwrap_or(6);
    (1_000_i64.saturating_mul(1_i64 << exponent)).min(MAX_FAILED_LOGIN_DELAY_MS)
}

fn require_identifier(value: &str, message: &'static str) -> Result<(), StateError> {
    if Uuid::parse_str(value).is_err() {
        Err(StateError::InvalidSecurityInput(message))
    } else {
        Ok(())
    }
}

impl StateDatabase {
    /// Delete a bounded batch of long-expired or long-revoked session rows.
    pub fn prune_stale_sessions(&self, now_unix_ms: i64) -> Result<usize, StateError> {
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pruned = prune_stale_sessions_in(&transaction, now_unix_ms, SESSION_PRUNE_BATCH)?;
        transaction.commit()?;
        Ok(pruned)
    }

    /// Repair at most one bounded batch of excess per-user session rows.
    /// The newest 64 rows for every user are always retained.
    pub fn maintain_session_row_limit(&self) -> Result<usize, StateError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let repaired = transaction.execute(
            "DELETE FROM sessions
             WHERE id IN (
                 SELECT id FROM sessions
                 WHERE user_id = (
                     SELECT user_id FROM sessions
                     GROUP BY user_id
                     HAVING count(*) > ?1
                     ORDER BY user_id
                     LIMIT 1
                 )
                 ORDER BY created_at_unix_ms DESC, id DESC
                 LIMIT ?2 OFFSET ?1
             )",
            params![MAX_SESSION_ROWS_PER_USER, SESSION_ROW_LIMIT_DELETE_BATCH],
        )?;
        transaction.commit()?;
        Ok(repaired)
    }

    /// Apply one bounded authentication-audit retention pass. The newest 1,024
    /// events are protected, at most 256 older rows are removed, and ordinary
    /// writes remain append-only outside this internal transaction.
    pub fn maintain_authentication_audit_retention(
        &self,
        now_unix_ms: i64,
    ) -> Result<usize, StateError> {
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pruned =
            prune_authentication_audit_in(&transaction, now_unix_ms, AUDIT_RETENTION_POLICY)?;
        transaction.commit()?;
        Ok(pruned)
    }

    pub fn authenticate_session(
        &self,
        input: SessionAuthenticationInput<'_>,
    ) -> Result<Option<AuthenticatedSession>, StateError> {
        require_nonnegative_time(input.now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let session = transaction
            .query_row(
                "SELECT s.id, s.user_id, u.login_name, u.display_name,
                        s.csrf_hash, s.auth_version,
                        s.created_at_unix_ms, s.last_seen_at_unix_ms,
                        s.absolute_expires_at_unix_ms, s.revoked_at_unix_ms,
                        u.status, u.auth_version
                 FROM sessions s
                 JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = ?1",
                [input.session_hash.as_bytes()],
                |row| {
                    Ok(SessionRow {
                        session_id: row.get(0)?,
                        user_id: row.get(1)?,
                        login_name: row.get(2)?,
                        display_name: row.get(3)?,
                        csrf_hash: row.get(4)?,
                        session_auth_version: row.get(5)?,
                        created_at_unix_ms: row.get(6)?,
                        last_seen_at_unix_ms: row.get(7)?,
                        absolute_expires_at_unix_ms: row.get(8)?,
                        revoked_at_unix_ms: row.get(9)?,
                        user_status: row.get(10)?,
                        user_auth_version: row.get(11)?,
                    })
                },
            )
            .optional()?;
        let Some(session) = session else {
            transaction.commit()?;
            return Ok(None);
        };
        let session_expires = session_expiry_enabled_in(&transaction)?;
        if !session_is_current(&session, input.now_unix_ms, session_expires)
            || !csrf_matches(&session.csrf_hash, input.csrf)
        {
            transaction.commit()?;
            return Ok(None);
        }

        let capabilities = capabilities_for_user(&transaction, &session.user_id)?;
        if let SessionAuthorization::RequireCapability(required) = input.authorization
            && (!capability_is_valid(required)
                || capabilities
                    .binary_search_by(|candidate| candidate.as_str().cmp(required))
                    .is_err())
        {
            transaction.commit()?;
            return Ok(None);
        }

        let should_touch =
            input.now_unix_ms - session.last_seen_at_unix_ms >= SESSION_TOUCH_INTERVAL_MS;
        let touched = if should_touch {
            transaction.execute(
                "UPDATE sessions
                 SET last_seen_at_unix_ms = ?1
                 WHERE id = ?2 AND last_seen_at_unix_ms = ?3
                       AND revoked_at_unix_ms IS NULL",
                params![
                    input.now_unix_ms,
                    session.session_id,
                    session.last_seen_at_unix_ms
                ],
            )? == 1
        } else {
            false
        };
        transaction.commit()?;
        Ok(Some(AuthenticatedSession {
            session_id: session.session_id,
            user_id: session.user_id,
            login_name: session.login_name,
            display_name: session.display_name,
            capabilities,
            auth_version: session.user_auth_version,
            absolute_expires_at_unix_ms: session.absolute_expires_at_unix_ms,
            last_seen_touched: touched,
            session_expires,
        }))
    }

    /// Classify a session without extending its idle lifetime.
    ///
    /// This is intended for failure handling where a caller must distinguish
    /// an invalid session from a rejected second proof. It never authorizes an
    /// operation and must not replace `authenticate_session` on a success path.
    pub fn session_is_current_without_touch(
        &self,
        session_hash: &SessionTokenHash,
        now_unix_ms: i64,
    ) -> Result<bool, StateError> {
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let session_expires = session_expiry_enabled_in(&transaction)?;
        let current = lookup_session_for_lifecycle(&transaction, session_hash)?
            .is_some_and(|session| session_is_current(&session, now_unix_ms, session_expires));
        transaction.commit()?;
        Ok(current)
    }

    pub fn rotate_session_csrf(
        &self,
        session_hash: &SessionTokenHash,
        expected_csrf_hash: &CsrfTokenHash,
        replacement_csrf_hash: &CsrfTokenHash,
        now_unix_ms: i64,
    ) -> Result<bool, StateError> {
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = lookup_session_for_lifecycle(&transaction, session_hash)?;
        let Some(session) = current else {
            transaction.commit()?;
            return Ok(false);
        };
        let session_expires = session_expiry_enabled_in(&transaction)?;
        if !session_is_current(&session, now_unix_ms, session_expires) {
            transaction.commit()?;
            return Ok(false);
        }
        let rotated = transaction.execute(
            "UPDATE sessions SET csrf_hash = ?1
              WHERE id = ?2 AND revoked_at_unix_ms IS NULL
                    AND auth_version = ?3 AND csrf_hash = ?4",
            params![
                replacement_csrf_hash.as_bytes(),
                session.session_id,
                session.session_auth_version,
                expected_csrf_hash.as_bytes()
            ],
        )? == 1;
        if rotated {
            append_audit(
                &transaction,
                now_unix_ms,
                Some(&session.user_id),
                "session.csrf_rotated",
                Some("session"),
                Some(&session.session_id),
                "success",
            )?;
        }
        transaction.commit()?;
        Ok(rotated)
    }

    pub fn set_session_expiry(
        &self,
        session_hash: &SessionTokenHash,
        expires: bool,
        now_unix_ms: i64,
    ) -> Result<Option<SessionExpiryUpdateOutcome>, StateError> {
        require_nonnegative_time(now_unix_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = lookup_session_for_lifecycle(&transaction, session_hash)?;
        let Some(session) = current else {
            transaction.commit()?;
            return Ok(None);
        };
        let current_expires = session_expiry_enabled_in(&transaction)?;
        if !session_is_current(&session, now_unix_ms, current_expires) {
            transaction.commit()?;
            return Ok(None);
        }
        transaction.execute(
            "UPDATE security_state SET session_expiry_enabled = ?1 WHERE singleton = 1",
            [i64::from(expires)],
        )?;
        if expires {
            transaction.execute(
                "UPDATE sessions
                 SET last_seen_at_unix_ms = CASE
                         WHEN ?1 < created_at_unix_ms THEN created_at_unix_ms
                         ELSE ?1
                     END,
                     absolute_expires_at_unix_ms = min(
                         CASE
                             WHEN ?1 < created_at_unix_ms THEN created_at_unix_ms
                             ELSE ?1
                         END + ?2,
                         created_at_unix_ms + ?3
                     )
                 WHERE revoked_at_unix_ms IS NULL",
                params![
                    now_unix_ms,
                    SESSION_ABSOLUTE_LIFETIME_MS,
                    SESSION_PERSISTENT_LIFETIME_MS
                ],
            )?;
        } else {
            transaction.execute(
                "UPDATE sessions
                 SET absolute_expires_at_unix_ms = created_at_unix_ms + ?1
                 WHERE revoked_at_unix_ms IS NULL",
                [SESSION_PERSISTENT_LIFETIME_MS],
            )?;
        }
        let updated_absolute = transaction.query_row(
            "SELECT absolute_expires_at_unix_ms FROM sessions WHERE id = ?1",
            [&session.session_id],
            |row| row.get::<_, i64>(0),
        )?;
        append_audit(
            &transaction,
            now_unix_ms,
            Some(&session.user_id),
            if expires {
                "session.expiry.enabled"
            } else {
                "session.expiry.disabled"
            },
            Some("session"),
            Some(&session.session_id),
            "success",
        )?;
        transaction.commit()?;
        Ok(Some(SessionExpiryUpdateOutcome {
            expires,
            absolute_expires_at_unix_ms: updated_absolute,
        }))
    }

    pub fn revoke_session(
        &self,
        session_hash: &SessionTokenHash,
        actor_user_id: Option<&str>,
        now_unix_ms: i64,
    ) -> Result<bool, StateError> {
        require_nonnegative_time(now_unix_ms)?;
        if let Some(actor) = actor_user_id {
            require_identifier(actor, "invalid actor user identifier")?;
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let session = transaction
            .query_row(
                "SELECT id, user_id FROM sessions
                 WHERE token_hash = ?1 AND revoked_at_unix_ms IS NULL",
                [session_hash.as_bytes()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((session_id, session_user_id)) = session else {
            transaction.commit()?;
            return Ok(false);
        };
        let revoked = transaction.execute(
            "UPDATE sessions SET revoked_at_unix_ms = ?1
             WHERE id = ?2 AND revoked_at_unix_ms IS NULL",
            params![now_unix_ms, session_id],
        )? == 1;
        if revoked {
            append_audit(
                &transaction,
                now_unix_ms,
                actor_user_id.or(Some(&session_user_id)),
                "session.revoked",
                Some("session"),
                Some(&session_id),
                "success",
            )?;
        }
        transaction.commit()?;
        Ok(revoked)
    }
}

struct SessionRow {
    session_id: String,
    user_id: String,
    login_name: String,
    display_name: String,
    csrf_hash: Vec<u8>,
    session_auth_version: i64,
    created_at_unix_ms: i64,
    last_seen_at_unix_ms: i64,
    absolute_expires_at_unix_ms: i64,
    revoked_at_unix_ms: Option<i64>,
    user_status: String,
    user_auth_version: i64,
}

fn lookup_session_for_lifecycle(
    transaction: &Transaction<'_>,
    session_hash: &SessionTokenHash,
) -> Result<Option<SessionRow>, StateError> {
    Ok(transaction
        .query_row(
            "SELECT s.id, s.user_id, u.login_name, u.display_name,
                    s.csrf_hash, s.auth_version,
                    s.created_at_unix_ms, s.last_seen_at_unix_ms,
                    s.absolute_expires_at_unix_ms, s.revoked_at_unix_ms,
                    u.status, u.auth_version
             FROM sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?1",
            [session_hash.as_bytes()],
            |row| {
                Ok(SessionRow {
                    session_id: row.get(0)?,
                    user_id: row.get(1)?,
                    login_name: row.get(2)?,
                    display_name: row.get(3)?,
                    csrf_hash: row.get(4)?,
                    session_auth_version: row.get(5)?,
                    created_at_unix_ms: row.get(6)?,
                    last_seen_at_unix_ms: row.get(7)?,
                    absolute_expires_at_unix_ms: row.get(8)?,
                    revoked_at_unix_ms: row.get(9)?,
                    user_status: row.get(10)?,
                    user_auth_version: row.get(11)?,
                })
            },
        )
        .optional()?)
}

fn session_is_current(session: &SessionRow, now_unix_ms: i64, session_expires: bool) -> bool {
    let idle_ok = if session_expires {
        session
            .last_seen_at_unix_ms
            .checked_add(SESSION_IDLE_LIFETIME_MS)
            .is_some_and(|idle_expiry| now_unix_ms < idle_expiry)
    } else {
        now_unix_ms >= session.last_seen_at_unix_ms
    };
    session.revoked_at_unix_ms.is_none()
        && session.user_status == "active"
        && session.session_auth_version == session.user_auth_version
        && now_unix_ms >= session.created_at_unix_ms
        && now_unix_ms >= session.last_seen_at_unix_ms
        && idle_ok
        && now_unix_ms < session.absolute_expires_at_unix_ms
}

fn session_expiry_enabled_in(connection: &rusqlite::Connection) -> Result<bool, StateError> {
    Ok(connection.query_row(
        "SELECT session_expiry_enabled FROM security_state WHERE singleton = 1",
        [],
        |row| row.get::<_, bool>(0),
    )?)
}

fn session_lifetime_ms(expires: bool) -> i64 {
    if expires {
        SESSION_ABSOLUTE_LIFETIME_MS
    } else {
        SESSION_PERSISTENT_LIFETIME_MS
    }
}

fn prune_stale_sessions_in(
    connection: &rusqlite::Connection,
    now_unix_ms: i64,
    maximum: i64,
) -> Result<usize, StateError> {
    let retention_cutoff = now_unix_ms.saturating_sub(SESSION_ROW_RETENTION_MS);
    Ok(connection.execute(
        "DELETE FROM sessions
         WHERE id IN (
             SELECT id FROM sessions
             WHERE absolute_expires_at_unix_ms <= ?1
                OR (revoked_at_unix_ms IS NOT NULL AND revoked_at_unix_ms <= ?1)
             ORDER BY coalesce(revoked_at_unix_ms, absolute_expires_at_unix_ms), id
             LIMIT ?2
         )",
        params![retention_cutoff, maximum],
    )?)
}

fn delete_excess_session_rows_in_batch(
    connection: &rusqlite::Connection,
    user_id: &str,
    retained_rows: i64,
) -> Result<usize, StateError> {
    Ok(connection.execute(
        "DELETE FROM sessions
         WHERE id IN (
             SELECT id FROM sessions
             WHERE user_id = ?1
             ORDER BY created_at_unix_ms DESC, id DESC
             LIMIT ?2 OFFSET ?3
         )",
        params![user_id, SESSION_ROW_LIMIT_DELETE_BATCH, retained_rows],
    )?)
}

fn csrf_matches(stored_hash: &[u8], requirement: CsrfRequirement<'_>) -> bool {
    match requirement {
        CsrfRequirement::NotRequired => true,
        CsrfRequirement::Match(provided) => digest_matches(stored_hash, provided.as_bytes()),
    }
}

fn digest_matches(stored: &[u8], provided: &[u8]) -> bool {
    stored.len() == provided.len() && bool::from(stored.ct_eq(provided))
}

fn capabilities_for_user(
    transaction: &Transaction<'_>,
    user_id: &str,
) -> Result<Vec<String>, StateError> {
    Ok(transaction
        .prepare(
            "SELECT DISTINCT rc.capability
             FROM user_roles ur
             JOIN role_capabilities rc ON rc.role_id = ur.role_id
             WHERE ur.user_id = ?1
             ORDER BY rc.capability",
        )?
        .query_map([user_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn capability_is_valid(capability: &str) -> bool {
    !capability.is_empty()
        && capability.len() <= 128
        && capability
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DatabaseSet, STATE_MIGRATION_1, STATE_SCHEMA_VERSION, StateDatabaseReader, apply_migration,
        configure_state_connection, create_private_file, ensure_data_directory,
        ensure_installation, pragma_i64,
    };
    use rusqlite::Connection;
    use std::{
        fs,
        path::Path,
        sync::{Arc, Barrier},
        thread,
    };

    const NOW: i64 = 1_800_000_000_000;

    fn bootstrap_hash(byte: u8) -> BootstrapTokenHash {
        BootstrapTokenHash::from_digest([byte; 32])
    }

    fn session_hash(byte: u8) -> SessionTokenHash {
        SessionTokenHash::from_digest([byte; 32])
    }

    fn csrf_hash(byte: u8) -> CsrfTokenHash {
        CsrfTokenHash::from_digest([byte; 32])
    }

    fn password_phc(label: &str) -> PasswordPhc {
        PasswordPhc::new(format!(
            "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA${label:0<24}"
        ))
        .expect("test PHC")
    }

    struct OwnerFixture {
        user_id: String,
        session_hash: SessionTokenHash,
        csrf_hash: CsrfTokenHash,
    }

    fn claim_owner(state: &StateDatabase, now_unix_ms: i64) -> OwnerFixture {
        let bootstrap = bootstrap_hash(1);
        let session = session_hash(2);
        let csrf = csrf_hash(3);
        let phc = password_phc("owner");
        assert_eq!(
            state
                .replace_bootstrap_token(&bootstrap, now_unix_ms, now_unix_ms + 60_000)
                .expect("install bootstrap"),
            BootstrapInstallOutcome::Installed {
                expires_at_unix_ms: now_unix_ms + 60_000
            }
        );
        let outcome = state
            .claim_owner(OwnerClaimInput {
                bootstrap_hash: &bootstrap,
                login_name: "owner",
                display_name: "Owner",
                password_phc: &phc,
                password_policy_version: 1,
                session_hash: &session,
                csrf_hash: &csrf,
                now_unix_ms,
            })
            .expect("claim owner");
        let OwnerClaimOutcome::Claimed { user_id, .. } = outcome else {
            panic!("owner claim was rejected: {outcome:?}");
        };
        OwnerFixture {
            user_id,
            session_hash: session,
            csrf_hash: csrf,
        }
    }

    fn open_databases() -> (tempfile::TempDir, DatabaseSet) {
        let temp = crate::private_test_directory("temporary directory");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open databases");
        (temp, databases)
    }

    #[test]
    fn owner_account_update_is_guarded_and_revokes_every_session() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let credential = state
            .credential_by_login("owner", NOW)
            .expect("credential lookup")
            .expect("owner credential");
        let replacement = password_phc("replacement");

        assert_eq!(
            state
                .update_owner_account(OwnerAccountUpdateInput {
                    user_id: &fixture.user_id,
                    expected_auth_version: credential.auth_version,
                    expected_password_phc: &credential.password_phc,
                    expected_password_policy_version: credential.password_policy_version,
                    login_name: "rique",
                    display_name: "Rique",
                    replacement_password: Some(PasswordRehash {
                        replacement_password_phc: &replacement,
                        replacement_password_policy_version: credential.password_policy_version + 1,
                    }),
                    now_unix_ms: NOW + 1,
                })
                .expect("update owner account"),
            OwnerAccountUpdateOutcome::Updated
        );
        assert!(
            state
                .credential_by_login("owner", NOW + 1)
                .expect("old login lookup")
                .is_none()
        );
        let updated = state
            .credential_by_login("rique", NOW + 1)
            .expect("new login lookup")
            .expect("updated owner credential");
        assert_eq!(updated.password_phc, replacement);
        assert_eq!(updated.auth_version, credential.auth_version + 1);
        assert_eq!(
            updated.password_policy_version,
            credential.password_policy_version + 1
        );
        assert!(
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &fixture.session_hash,
                    authorization: SessionAuthorization::Authenticated,
                    csrf: CsrfRequirement::Match(&fixture.csrf_hash),
                    now_unix_ms: NOW + 1,
                })
                .expect("old session lookup")
                .is_none()
        );
        assert_eq!(
            state
                .update_owner_account(OwnerAccountUpdateInput {
                    user_id: &fixture.user_id,
                    expected_auth_version: credential.auth_version,
                    expected_password_phc: &credential.password_phc,
                    expected_password_policy_version: credential.password_policy_version,
                    login_name: "owner-again",
                    display_name: "Owner Again",
                    replacement_password: None,
                    now_unix_ms: NOW + 2,
                })
                .expect("stale account update"),
            OwnerAccountUpdateOutcome::CredentialChangedOrUnavailable
        );
        let audit_rows = state
            .lock()
            .expect("state lock")
            .query_row(
                "SELECT count(*) FROM audit_events WHERE action = 'account.owner_updated'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("account audit count");
        assert_eq!(audit_rows, 1);
    }

    #[test]
    fn user_preferences_are_bounded_and_revision_guarded() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        assert_eq!(
            state
                .user_preferences(&fixture.user_id)
                .expect("initial preferences"),
            None
        );

        let first = state
            .update_user_preferences(UserPreferencesUpdateInput {
                user_id: &fixture.user_id,
                expected_revision: 0,
                preferences_json: r#"{"metricsRefreshMs":1000}"#,
                now_unix_ms: NOW + 1,
            })
            .expect("create preferences");
        let UserPreferencesUpdateOutcome::Updated(first) = first else {
            panic!("initial preference write conflicted");
        };
        assert_eq!(first.revision, 1);
        assert_eq!(first.preferences_json, r#"{"metricsRefreshMs":1000}"#);

        assert_eq!(
            state
                .update_user_preferences(UserPreferencesUpdateInput {
                    user_id: &fixture.user_id,
                    expected_revision: 1,
                    preferences_json: r#"{"metricsRefreshMs":1000}"#,
                    now_unix_ms: NOW + 2,
                })
                .expect("coalesce unchanged preferences"),
            UserPreferencesUpdateOutcome::Updated(first.clone())
        );

        assert_eq!(
            state
                .update_user_preferences(UserPreferencesUpdateInput {
                    user_id: &fixture.user_id,
                    expected_revision: 0,
                    preferences_json: r#"{"metricsRefreshMs":5000}"#,
                    now_unix_ms: NOW + 2,
                })
                .expect("stale preference write"),
            UserPreferencesUpdateOutcome::Conflict(Some(first.clone()))
        );
        let second = state
            .update_user_preferences(UserPreferencesUpdateInput {
                user_id: &fixture.user_id,
                expected_revision: 1,
                preferences_json: r#"{"metricsRefreshMs":5000}"#,
                now_unix_ms: NOW + 3,
            })
            .expect("update preferences");
        let UserPreferencesUpdateOutcome::Updated(second) = second else {
            panic!("current preference write conflicted");
        };
        assert_eq!(second.revision, 2);
        assert!(matches!(
            state.update_user_preferences(UserPreferencesUpdateInput {
                user_id: &fixture.user_id,
                expected_revision: 2,
                preferences_json: "[]",
                now_unix_ms: NOW + 4,
            }),
            Err(StateError::InvalidSecurityInput(_))
        ));
        for expected_revision in [-1, i64::MAX] {
            assert!(matches!(
                state.update_user_preferences(UserPreferencesUpdateInput {
                    user_id: &fixture.user_id,
                    expected_revision,
                    preferences_json: r#"{}"#,
                    now_unix_ms: NOW + 5,
                }),
                Err(StateError::InvalidSecurityInput(_))
            ));
        }
    }

    #[test]
    fn concurrent_preference_compare_and_swap_has_one_winner() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let databases = Arc::new(databases);
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();

        for preferences_json in [
            r#"{"metricsRefreshMs":2000}"#,
            r#"{"metricsRefreshMs":5000}"#,
        ] {
            let databases = Arc::clone(&databases);
            let barrier = Arc::clone(&barrier);
            let user_id = fixture.user_id.clone();
            workers.push(thread::spawn(move || {
                barrier.wait();
                databases
                    .state()
                    .update_user_preferences(UserPreferencesUpdateInput {
                        user_id: &user_id,
                        expected_revision: 0,
                        preferences_json,
                        now_unix_ms: NOW + 1,
                    })
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .expect("preference worker")
                    .expect("preference write")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, UserPreferencesUpdateOutcome::Updated(_)))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, UserPreferencesUpdateOutcome::Conflict(Some(current)) if current.revision == 1))
                .count(),
            1
        );
    }

    fn imported_session_token_hash(sequence: i64) -> [u8; 32] {
        let mut digest = [0xa5; 32];
        digest[..8].copy_from_slice(&sequence.to_be_bytes());
        digest
    }

    fn seed_imported_sessions(state: &StateDatabase, user_id: &str, auth_version: i64, rows: i64) {
        let seeded_csrf_hash = [0x5a_u8; 32];
        let mut connection = state.lock().expect("state lock");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("seed transaction");
        for sequence in 0..rows {
            let created_at = NOW + sequence + 1;
            let session_id = Uuid::from_u128(
                0x10000000000040008000000000000000_u128
                    + u128::try_from(sequence).expect("positive sequence"),
            )
            .to_string();
            transaction
                .execute(
                    "INSERT INTO sessions (
                        id, user_id, token_hash, csrf_hash, auth_version,
                        created_at_unix_ms, last_seen_at_unix_ms,
                        absolute_expires_at_unix_ms, revoked_at_unix_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, NULL)",
                    params![
                        session_id,
                        user_id,
                        imported_session_token_hash(sequence),
                        seeded_csrf_hash,
                        auth_version,
                        created_at,
                        created_at + SESSION_ABSOLUTE_LIFETIME_MS
                    ],
                )
                .expect("seed imported session");
        }
        transaction.commit().expect("commit imported sessions");
    }

    fn session_row_count(state: &StateDatabase, user_id: &str) -> i64 {
        state
            .lock()
            .expect("state lock")
            .query_row(
                "SELECT count(*) FROM sessions WHERE user_id = ?1",
                [user_id],
                |row| row.get(0),
            )
            .expect("session row count")
    }

    fn session_snapshot(state: &StateDatabase, user_id: &str) -> Vec<(String, Vec<u8>)> {
        let connection = state.lock().expect("state lock");
        let mut statement = connection
            .prepare(
                "SELECT id, token_hash FROM sessions
                 WHERE user_id = ?1 ORDER BY id",
            )
            .expect("prepare session snapshot");
        statement
            .query_map([user_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query session snapshot")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect session snapshot")
    }

    fn seed_audit_events(
        transaction: &Transaction<'_>,
        events: &[(i64, &'static str)],
    ) -> Vec<String> {
        events
            .iter()
            .enumerate()
            .map(|(sequence, (occurred_at_unix_ms, outcome))| {
                let sequence = u128::try_from(sequence).expect("audit sequence");
                let id =
                    Uuid::from_u128(0x20000000000040008000000000000000_u128 + sequence).to_string();
                let correlation_id =
                    Uuid::from_u128(0x30000000000040008000000000000000_u128 + sequence).to_string();
                transaction
                    .execute(
                        "INSERT INTO audit_events (
                            id, occurred_at_unix_ms, actor_user_id, action,
                            target_type, target_id, outcome, correlation_id, detail_json
                         ) VALUES (
                            ?1, ?2, NULL, 'authentication.test',
                            'user', 'retention-fixture', ?3, ?4, '{}'
                         )",
                        params![id, occurred_at_unix_ms, outcome, correlation_id],
                    )
                    .expect("seed audit event");
                id
            })
            .collect()
    }

    fn audit_ids(state: &StateDatabase) -> Vec<String> {
        let connection = state.lock().expect("state lock");
        let mut statement = connection
            .prepare("SELECT id FROM audit_events ORDER BY occurred_at_unix_ms, id")
            .expect("prepare audit identifiers");
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query audit identifiers")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect audit identifiers")
    }

    #[test]
    fn setup_status_and_bootstrap_replacement_are_minimal_and_single_slot() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        assert_eq!(
            state.setup_status(NOW).expect("initial status"),
            SetupStatus {
                owner_exists: false,
                bootstrap_available: false,
                bootstrap_expires_at_unix_ms: None,
            }
        );

        state
            .replace_bootstrap_token(&bootstrap_hash(10), NOW, NOW + 30_000)
            .expect("first bootstrap");
        state
            .replace_bootstrap_token(&bootstrap_hash(11), NOW + 1, NOW + 40_000)
            .expect("replace bootstrap");
        assert_eq!(
            state.setup_status(NOW + 2).expect("active status"),
            SetupStatus {
                owner_exists: false,
                bootstrap_available: true,
                bootstrap_expires_at_unix_ms: Some(NOW + 40_000),
            }
        );
        assert_eq!(
            state
                .preflight_bootstrap_claim(&bootstrap_hash(11), NOW + 2)
                .expect("matching active bootstrap"),
            BootstrapPreflightOutcome::Match
        );
        assert_eq!(
            state
                .preflight_bootstrap_claim(&bootstrap_hash(10), NOW + 2)
                .expect("replaced bootstrap does not match"),
            BootstrapPreflightOutcome::Rejected
        );
        assert_eq!(
            state
                .preflight_bootstrap_claim(&bootstrap_hash(11), NOW + 40_000)
                .expect("expired bootstrap does not match"),
            BootstrapPreflightOutcome::Rejected
        );
        let connection = state.lock().expect("state lock");
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM bootstrap_tokens WHERE active_slot = 1",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .expect("active bootstrap count"),
            1
        );
    }

    #[test]
    fn backward_clock_never_extends_or_strands_bootstrap_state() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let first = bootstrap_hash(10);
        let replacement = bootstrap_hash(11);
        state
            .replace_bootstrap_token(&first, NOW, NOW + 60_000)
            .expect("install first bootstrap");
        let rolled_back_now = NOW - 10_000;

        assert_eq!(
            state
                .setup_status(rolled_back_now)
                .expect("rolled-back status"),
            SetupStatus {
                owner_exists: false,
                bootstrap_available: false,
                bootstrap_expires_at_unix_ms: Some(NOW + 60_000),
            }
        );
        assert_eq!(
            state
                .claim_owner(OwnerClaimInput {
                    bootstrap_hash: &first,
                    login_name: "owner",
                    display_name: "Owner",
                    password_phc: &password_phc("clock"),
                    password_policy_version: 1,
                    session_hash: &session_hash(12),
                    csrf_hash: &csrf_hash(13),
                    now_unix_ms: rolled_back_now,
                })
                .expect("reject future-created bootstrap"),
            OwnerClaimOutcome::Rejected(OwnerClaimRejection::BootstrapExpired)
        );

        state
            .replace_bootstrap_token(&replacement, rolled_back_now, rolled_back_now + 60_000)
            .expect("replace after clock rollback");
        assert!(
            state
                .setup_status(rolled_back_now)
                .expect("replacement status")
                .bootstrap_available
        );
        let first_consumed: i64 = state
            .lock()
            .expect("state lock")
            .query_row(
                "SELECT consumed_at_unix_ms FROM bootstrap_tokens WHERE token_hash = ?1",
                [first.as_bytes()],
                |row| row.get(0),
            )
            .expect("first consumption timestamp");
        assert_eq!(first_consumed, NOW);
    }

    #[test]
    fn random_source_failure_rolls_back_without_poisoning_state() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let bootstrap = bootstrap_hash(9);
        crate::fail_next_uuid_generation_for_test();

        assert!(matches!(
            state.replace_bootstrap_token(&bootstrap, NOW, NOW + 60_000),
            Err(StateError::RandomSource)
        ));
        let connection = state.lock().expect("state lock after RNG failure");
        let bootstrap_rows: i64 = connection
            .query_row("SELECT count(*) FROM bootstrap_tokens", [], |row| {
                row.get(0)
            })
            .expect("bootstrap row count");
        let audit_rows: i64 = connection
            .query_row("SELECT count(*) FROM audit_events", [], |row| row.get(0))
            .expect("audit row count");
        drop(connection);
        assert_eq!((bootstrap_rows, audit_rows), (0, 0));

        assert_eq!(
            state
                .replace_bootstrap_token(&bootstrap, NOW, NOW + 60_000)
                .expect("retry after RNG failure"),
            BootstrapInstallOutcome::Installed {
                expires_at_unix_ms: NOW + 60_000
            }
        );
    }

    #[test]
    fn competing_owner_claims_commit_exactly_one_owner_session_and_success_audit() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let bootstrap = bootstrap_hash(20);
        let phc = password_phc("race");
        state
            .replace_bootstrap_token(&bootstrap, NOW, NOW + 60_000)
            .expect("bootstrap");

        let outcomes = thread::scope(|scope| {
            let first = scope.spawn(|| {
                state.claim_owner(OwnerClaimInput {
                    bootstrap_hash: &bootstrap,
                    login_name: "owner-one",
                    display_name: "Owner One",
                    password_phc: &phc,
                    password_policy_version: 1,
                    session_hash: &session_hash(21),
                    csrf_hash: &csrf_hash(22),
                    now_unix_ms: NOW + 1,
                })
            });
            let second = scope.spawn(|| {
                state.claim_owner(OwnerClaimInput {
                    bootstrap_hash: &bootstrap,
                    login_name: "owner-two",
                    display_name: "Owner Two",
                    password_phc: &phc,
                    password_policy_version: 1,
                    session_hash: &session_hash(23),
                    csrf_hash: &csrf_hash(24),
                    now_unix_ms: NOW + 1,
                })
            });
            [
                first.join().expect("first thread"),
                second.join().expect("second thread"),
            ]
        });
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(OwnerClaimOutcome::Claimed { .. })))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| {
                    matches!(
                        outcome,
                        Ok(OwnerClaimOutcome::Rejected(
                            OwnerClaimRejection::OwnerAlreadyExists
                        ))
                    )
                })
                .count(),
            1
        );
        let connection = state.lock().expect("state lock");
        for (sql, expected) in [
            ("SELECT count(*) FROM users WHERE is_owner = 1", 1),
            ("SELECT count(*) FROM sessions", 1),
            (
                "SELECT count(*) FROM audit_events WHERE action = 'owner.claimed' AND outcome = 'success'",
                1,
            ),
        ] {
            assert_eq!(
                connection
                    .query_row(sql, [], |row| row.get::<_, i64>(0))
                    .expect("count invariant"),
                expected
            );
        }
    }

    #[test]
    fn expired_bootstrap_is_rejected_and_consumed_bootstrap_cannot_be_reused() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let expired = bootstrap_hash(30);
        let phc = password_phc("expiry");
        state
            .replace_bootstrap_token(&expired, NOW, NOW + 1)
            .expect("bootstrap");
        assert_eq!(
            state
                .claim_owner(OwnerClaimInput {
                    bootstrap_hash: &expired,
                    login_name: "owner",
                    display_name: "Owner",
                    password_phc: &phc,
                    password_policy_version: 1,
                    session_hash: &session_hash(31),
                    csrf_hash: &csrf_hash(32),
                    now_unix_ms: NOW + 1,
                })
                .expect("expired claim"),
            OwnerClaimOutcome::Rejected(OwnerClaimRejection::BootstrapExpired)
        );

        let active = bootstrap_hash(33);
        state
            .replace_bootstrap_token(&active, NOW + 2, NOW + 60_000)
            .expect("replacement bootstrap");
        let claimed_session = session_hash(34);
        let claimed_csrf = csrf_hash(35);
        let claim = || OwnerClaimInput {
            bootstrap_hash: &active,
            login_name: "owner",
            display_name: "Owner",
            password_phc: &phc,
            password_policy_version: 1,
            session_hash: &claimed_session,
            csrf_hash: &claimed_csrf,
            now_unix_ms: NOW + 3,
        };
        assert!(matches!(
            state.claim_owner(claim()).expect("first claim"),
            OwnerClaimOutcome::Claimed { .. }
        ));
        assert_eq!(
            state.claim_owner(claim()).expect("reused claim"),
            OwnerClaimOutcome::Rejected(OwnerClaimRejection::OwnerAlreadyExists)
        );
    }

    #[test]
    fn storage_contains_only_hashes_and_phc_debug_output_is_redacted() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let connection = state.lock().expect("state lock");
        let bootstrap: Vec<u8> = connection
            .query_row("SELECT token_hash FROM bootstrap_tokens", [], |row| {
                row.get(0)
            })
            .expect("bootstrap hash");
        let (session, csrf): (Vec<u8>, Vec<u8>) = connection
            .query_row("SELECT token_hash, csrf_hash FROM sessions", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("session hashes");
        assert_eq!(bootstrap, vec![1; 32]);
        assert_eq!(session, vec![2; 32]);
        assert_eq!(csrf, vec![3; 32]);
        drop(connection);

        let credential = state
            .credential_by_login("owner", NOW)
            .expect("credential lookup")
            .expect("owner credential");
        assert_eq!(credential.user_id, fixture.user_id);
        assert_eq!(credential.password_policy_version, 1);
        assert_eq!(
            format!("{:?}", credential.password_phc),
            "PasswordPhc([REDACTED])"
        );
        assert_eq!(
            format!("{:?}", fixture.session_hash),
            "SessionTokenHash([REDACTED])"
        );
    }

    #[test]
    fn capabilities_and_csrf_are_default_deny_then_rotation_and_revoke_take_effect() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let wrong_csrf = csrf_hash(99);
        let authenticate = |capability, csrf, now| {
            state.authenticate_session(SessionAuthenticationInput {
                session_hash: &fixture.session_hash,
                authorization: SessionAuthorization::RequireCapability(capability),
                csrf,
                now_unix_ms: now,
            })
        };
        assert!(
            authenticate("system.view", CsrfRequirement::NotRequired, NOW + 1)
                .expect("authorized")
                .is_some()
        );
        assert!(
            authenticate("system.control", CsrfRequirement::NotRequired, NOW + 1)
                .expect("default deny")
                .is_none()
        );
        assert!(
            authenticate("system.view", CsrfRequirement::Match(&wrong_csrf), NOW + 1)
                .expect("wrong CSRF")
                .is_none()
        );
        assert!(
            authenticate(
                "system.view",
                CsrfRequirement::Match(&fixture.csrf_hash),
                NOW + 1
            )
            .expect("correct CSRF")
            .is_some()
        );

        let replacement_csrf = csrf_hash(40);
        assert!(
            state
                .rotate_session_csrf(
                    &fixture.session_hash,
                    &fixture.csrf_hash,
                    &replacement_csrf,
                    NOW + 2
                )
                .expect("rotate CSRF")
        );
        assert!(
            authenticate(
                "system.view",
                CsrfRequirement::Match(&fixture.csrf_hash),
                NOW + 3
            )
            .expect("old CSRF")
            .is_none()
        );
        assert!(
            authenticate(
                "system.view",
                CsrfRequirement::Match(&replacement_csrf),
                NOW + 3
            )
            .expect("new CSRF")
            .is_some()
        );
        assert!(
            state
                .revoke_session(&fixture.session_hash, Some(&fixture.user_id), NOW + 4)
                .expect("revoke")
        );
        assert!(
            authenticate("system.view", CsrfRequirement::NotRequired, NOW + 5)
                .expect("revoked auth")
                .is_none()
        );
        assert!(
            !state
                .revoke_session(&fixture.session_hash, Some(&fixture.user_id), NOW + 6)
                .expect("idempotent revoke")
        );
    }

    #[test]
    fn current_session_classification_never_extends_idle_lifetime() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let classification_time = NOW + SESSION_TOUCH_INTERVAL_MS + 1;

        assert!(
            state
                .session_is_current_without_touch(&fixture.session_hash, classification_time)
                .expect("classify current session")
        );
        let last_seen_after_classification: i64 = state
            .lock()
            .expect("state lock")
            .query_row(
                "SELECT last_seen_at_unix_ms FROM sessions WHERE token_hash = ?1",
                [fixture.session_hash.as_bytes()],
                |row| row.get(0),
            )
            .expect("session timestamp");
        assert_eq!(last_seen_after_classification, NOW);

        let authenticated = state
            .authenticate_session(SessionAuthenticationInput {
                session_hash: &fixture.session_hash,
                authorization: SessionAuthorization::Authenticated,
                csrf: CsrfRequirement::Match(&fixture.csrf_hash),
                now_unix_ms: classification_time,
            })
            .expect("authenticate session")
            .expect("current session");
        assert!(authenticated.last_seen_touched);
    }

    #[test]
    fn session_rows_are_capped_per_user_and_pruned_in_bounded_batches() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let credential = state
            .credential_by_login("owner", NOW)
            .expect("credential lookup")
            .expect("owner credential");
        let mut latest_session = session_hash(0);
        let mut latest_csrf = csrf_hash(0);

        for sequence in 10_u8..90 {
            latest_session = session_hash(sequence);
            latest_csrf = csrf_hash(sequence + 100);
            assert!(matches!(
                state
                    .create_session_after_verified_login(SessionCreateInput {
                        user_id: &credential.user_id,
                        expected_auth_version: credential.auth_version,
                        expected_password_phc: &credential.password_phc,
                        expected_password_policy_version: credential.password_policy_version,
                        rehash: None,
                        session_hash: &latest_session,
                        csrf_hash: &latest_csrf,
                        now_unix_ms: NOW + i64::from(sequence),
                    })
                    .expect("create session"),
                SessionCreateOutcome::Created { .. }
            ));
        }

        let session_rows: i64 = state
            .lock()
            .expect("state lock")
            .query_row(
                "SELECT count(*) FROM sessions WHERE user_id = ?1",
                [&credential.user_id],
                |row| row.get(0),
            )
            .expect("session row count");
        assert_eq!(session_rows, MAX_SESSION_ROWS_PER_USER);
        assert!(
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &fixture.session_hash,
                    authorization: SessionAuthorization::Authenticated,
                    csrf: CsrfRequirement::Match(&fixture.csrf_hash),
                    now_unix_ms: NOW + 100,
                })
                .expect("authenticate oldest session")
                .is_none()
        );
        assert!(
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &latest_session,
                    authorization: SessionAuthorization::Authenticated,
                    csrf: CsrfRequirement::Match(&latest_csrf),
                    now_unix_ms: NOW + 100,
                })
                .expect("authenticate newest session")
                .is_some()
        );

        let prune_time = NOW + 100 + SESSION_ABSOLUTE_LIFETIME_MS + SESSION_ROW_RETENTION_MS;
        assert_eq!(
            state
                .prune_stale_sessions(prune_time)
                .expect("prune stale sessions"),
            usize::try_from(MAX_SESSION_ROWS_PER_USER).expect("session cap")
        );
        let remaining: i64 = state
            .lock()
            .expect("state lock after prune")
            .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))
            .expect("remaining session count");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn oversized_imported_session_set_requires_fixed_batch_retries_then_converges() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let credential = state
            .credential_by_login("owner", NOW)
            .expect("credential lookup")
            .expect("owner credential");
        let oversized_rows = SESSION_ROW_LIMIT_DELETE_BATCH * 8 + 97;
        seed_imported_sessions(
            state,
            &fixture.user_id,
            credential.auth_version,
            oversized_rows,
        );

        let new_session = session_hash(250);
        let new_csrf = csrf_hash(251);
        let login_time = NOW + oversized_rows + 10;
        let create = || {
            state.create_session_after_verified_login(SessionCreateInput {
                user_id: &credential.user_id,
                expected_auth_version: credential.auth_version,
                expected_password_phc: &credential.password_phc,
                expected_password_policy_version: credential.password_policy_version,
                rehash: None,
                session_hash: &new_session,
                csrf_hash: &new_csrf,
                now_unix_ms: login_time,
            })
        };

        let first_count = session_row_count(state, &credential.user_id);
        let first = create().expect("first bounded repair");
        let first_remaining =
            first_count - SESSION_ROW_LIMIT_DELETE_BATCH - MAX_SESSION_ROWS_PER_USER;
        assert_eq!(
            first,
            SessionCreateOutcome::MaintenanceRequired {
                remaining_excess_rows: u64::try_from(first_remaining)
                    .expect("remaining excess rows")
            }
        );
        assert_eq!(
            session_row_count(state, &credential.user_id),
            first_count - SESSION_ROW_LIMIT_DELETE_BATCH
        );
        assert!(
            state
                .lock()
                .expect("state lock")
                .query_row(
                    "SELECT count(*) FROM sessions WHERE token_hash = ?1",
                    [new_session.as_bytes()],
                    |row| row.get::<_, i64>(0),
                )
                .expect("unissued session count")
                == 0
        );

        let mut maintenance_outcomes = 1;
        loop {
            let before = session_row_count(state, &credential.user_id);
            match create().expect("bounded repair retry") {
                SessionCreateOutcome::MaintenanceRequired {
                    remaining_excess_rows,
                } => {
                    maintenance_outcomes += 1;
                    let after = session_row_count(state, &credential.user_id);
                    assert_eq!(before - after, SESSION_ROW_LIMIT_DELETE_BATCH);
                    assert_eq!(
                        remaining_excess_rows,
                        u64::try_from(after - MAX_SESSION_ROWS_PER_USER)
                            .expect("remaining excess rows")
                    );
                }
                SessionCreateOutcome::Created { .. } => break,
                outcome => panic!("unexpected repair outcome: {outcome:?}"),
            }
        }
        assert!(maintenance_outcomes > 1);

        assert_eq!(
            session_row_count(state, &credential.user_id),
            MAX_SESSION_ROWS_PER_USER
        );
        assert!(
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &new_session,
                    authorization: SessionAuthorization::Authenticated,
                    csrf: CsrfRequirement::Match(&new_csrf),
                    now_unix_ms: login_time + 1,
                })
                .expect("authenticate newly issued session")
                .is_some()
        );

        let newest_seeded_session =
            SessionTokenHash::from_digest(imported_session_token_hash(oversized_rows - 1));
        let newest_seeded_csrf = CsrfTokenHash::from_digest([0x5a; 32]);
        assert!(
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &newest_seeded_session,
                    authorization: SessionAuthorization::Authenticated,
                    csrf: CsrfRequirement::Match(&newest_seeded_csrf),
                    now_unix_ms: login_time + 1,
                })
                .expect("authenticate newest retained imported session")
                .is_some()
        );
    }

    #[test]
    fn startup_session_row_maintenance_repairs_only_one_global_batch() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let credential = state
            .credential_by_login("owner", NOW)
            .expect("credential lookup")
            .expect("owner credential");
        let oversized_rows = SESSION_ROW_LIMIT_DELETE_BATCH * 5;
        seed_imported_sessions(
            state,
            &fixture.user_id,
            credential.auth_version,
            oversized_rows,
        );
        let before = session_row_count(state, &fixture.user_id);

        assert_eq!(
            state
                .maintain_session_row_limit()
                .expect("startup row maintenance"),
            usize::try_from(SESSION_ROW_LIMIT_DELETE_BATCH).expect("maintenance batch")
        );
        assert_eq!(
            session_row_count(state, &fixture.user_id),
            before - SESSION_ROW_LIMIT_DELETE_BATCH
        );
    }

    #[test]
    fn failures_at_the_session_cap_preserve_every_prior_session() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let credential = state
            .credential_by_login("owner", NOW)
            .expect("credential lookup")
            .expect("owner credential");
        seed_imported_sessions(
            state,
            &fixture.user_id,
            credential.auth_version,
            MAX_SESSION_ROWS_PER_USER - 1,
        );
        let failed_at = NOW + MAX_SESSION_ROWS_PER_USER + 1;
        state
            .record_failed_login(&fixture.user_id, failed_at)
            .expect("record failed login")
            .expect("active user delay");
        let login_time = failed_at + MAX_FAILED_LOGIN_DELAY_MS;
        let credential = state
            .credential_by_login("owner", login_time)
            .expect("credential lookup after delay")
            .expect("owner credential after delay");
        let replacement_password = password_phc("replacement");
        let replacement_policy_version = credential.password_policy_version + 1;
        let before = session_snapshot(state, &fixture.user_id);
        assert_eq!(
            before.len(),
            usize::try_from(MAX_SESSION_ROWS_PER_USER).expect("session cap")
        );
        let audit_rows_before: i64 = state
            .lock()
            .expect("state lock")
            .query_row("SELECT count(*) FROM audit_events", [], |row| row.get(0))
            .expect("audit row count");

        crate::force_verified_login_cas_miss_for_test();
        assert_eq!(
            state
                .create_session_after_verified_login(SessionCreateInput {
                    user_id: &credential.user_id,
                    expected_auth_version: credential.auth_version,
                    expected_password_phc: &credential.password_phc,
                    expected_password_policy_version: credential.password_policy_version,
                    rehash: Some(PasswordRehash {
                        replacement_password_phc: &replacement_password,
                        replacement_password_policy_version: replacement_policy_version,
                    }),
                    session_hash: &session_hash(238),
                    csrf_hash: &csrf_hash(239),
                    now_unix_ms: login_time,
                })
                .expect("injected CAS miss"),
            SessionCreateOutcome::CredentialChangedOrUnavailable
        );
        assert_eq!(session_snapshot(state, &fixture.user_id), before);
        assert_eq!(
            state
                .credential_by_login("owner", login_time)
                .expect("credential after CAS miss")
                .expect("owner credential after CAS miss"),
            credential
        );
        assert_eq!(
            state
                .lock()
                .expect("state lock after CAS miss")
                .query_row("SELECT count(*) FROM audit_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("audit row count after CAS miss"),
            audit_rows_before
        );

        crate::fail_next_uuid_generation_for_test();
        assert!(matches!(
            state.create_session_after_verified_login(SessionCreateInput {
                user_id: &credential.user_id,
                expected_auth_version: credential.auth_version,
                expected_password_phc: &credential.password_phc,
                expected_password_policy_version: credential.password_policy_version,
                rehash: Some(PasswordRehash {
                    replacement_password_phc: &replacement_password,
                    replacement_password_policy_version: replacement_policy_version,
                }),
                session_hash: &session_hash(240),
                csrf_hash: &csrf_hash(241),
                now_unix_ms: login_time,
            }),
            Err(StateError::RandomSource)
        ));
        assert_eq!(session_snapshot(state, &fixture.user_id), before);
        assert_eq!(
            state
                .credential_by_login("owner", login_time)
                .expect("credential after random failure")
                .expect("owner credential after random failure"),
            credential
        );

        state
            .lock()
            .expect("state lock")
            .execute_batch(
                "CREATE TEMP TRIGGER fail_authentication_success_audit
                 BEFORE INSERT ON audit_events
                 WHEN NEW.action = 'authentication.succeeded'
                 BEGIN
                     SELECT RAISE(ABORT, 'injected authentication audit failure');
                 END;",
            )
            .expect("install audit failure trigger");
        assert!(matches!(
            state.create_session_after_verified_login(SessionCreateInput {
                user_id: &credential.user_id,
                expected_auth_version: credential.auth_version,
                expected_password_phc: &credential.password_phc,
                expected_password_policy_version: credential.password_policy_version,
                rehash: Some(PasswordRehash {
                    replacement_password_phc: &replacement_password,
                    replacement_password_policy_version: replacement_policy_version,
                }),
                session_hash: &session_hash(242),
                csrf_hash: &csrf_hash(243),
                now_unix_ms: login_time + 1,
            }),
            Err(StateError::Sqlite(_))
        ));
        assert_eq!(session_snapshot(state, &fixture.user_id), before);
        assert_eq!(
            state
                .credential_by_login("owner", login_time + 1)
                .expect("credential after audit failure")
                .expect("owner credential after audit failure"),
            credential
        );
        let connection = state.lock().expect("state lock");
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM audit_events", [], |row| row
                    .get::<_, i64>(0))
                .expect("audit row count after failures"),
            audit_rows_before
        );
        connection
            .execute_batch("DROP TRIGGER fail_authentication_success_audit;")
            .expect("remove audit failure trigger");
    }

    #[test]
    fn concurrent_csrf_rotation_is_compare_and_swap() {
        use std::sync::{Arc, Barrier};

        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let first_replacement = csrf_hash(40);
        let second_replacement = csrf_hash(41);
        let barrier = Arc::new(Barrier::new(2));
        let state = databases.state();

        let outcomes = std::thread::scope(|scope| {
            let first_barrier = Arc::clone(&barrier);
            let first_session = fixture.session_hash.clone();
            let first_expected = fixture.csrf_hash.clone();
            let first_replacement = first_replacement.clone();
            let first = scope.spawn(move || {
                first_barrier.wait();
                state.rotate_session_csrf(
                    &first_session,
                    &first_expected,
                    &first_replacement,
                    NOW + 1,
                )
            });
            let second_barrier = Arc::clone(&barrier);
            let second_session = fixture.session_hash.clone();
            let second_expected = fixture.csrf_hash.clone();
            let second_replacement = second_replacement.clone();
            let second = scope.spawn(move || {
                second_barrier.wait();
                state.rotate_session_csrf(
                    &second_session,
                    &second_expected,
                    &second_replacement,
                    NOW + 1,
                )
            });
            [
                first.join().expect("first rotation thread"),
                second.join().expect("second rotation thread"),
            ]
        });
        let outcomes = outcomes.map(|outcome| outcome.expect("rotation result"));
        assert_eq!(outcomes.into_iter().filter(|rotated| *rotated).count(), 1);

        let accepts = [&first_replacement, &second_replacement].map(|csrf| {
            databases
                .state()
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &fixture.session_hash,
                    authorization: SessionAuthorization::Authenticated,
                    csrf: CsrfRequirement::Match(csrf),
                    now_unix_ms: NOW + 2,
                })
                .expect("authenticate replacement")
                .is_some()
        });
        assert_eq!(accepts.into_iter().filter(|accepted| *accepted).count(), 1);
    }

    #[test]
    fn auth_only_returns_canonical_identity_and_sorted_capabilities() {
        let (_temp, databases) = open_databases();
        {
            let connection = databases.state().lock().expect("state lock");
            for capability in ["zeta.view", "alpha.view"] {
                connection
                    .execute(
                        "INSERT INTO capabilities (capability, description, created_at_unix_ms)
                         VALUES (?1, ?2, ?3)",
                        params![capability, capability, NOW],
                    )
                    .expect("add capability");
                connection
                    .execute(
                        "INSERT INTO role_capabilities (
                            role_id, capability, granted_at_unix_ms
                         ) VALUES (?1, ?2, ?3)",
                        params![OWNER_ROLE_ID, capability, NOW],
                    )
                    .expect("grant capability");
            }
        }
        let fixture = claim_owner(databases.state(), NOW);
        let authenticated = databases
            .state()
            .authenticate_session(SessionAuthenticationInput {
                session_hash: &fixture.session_hash,
                authorization: SessionAuthorization::Authenticated,
                csrf: CsrfRequirement::NotRequired,
                now_unix_ms: NOW + 1,
            })
            .expect("authenticate")
            .expect("current session");

        assert_eq!(authenticated.user_id, fixture.user_id);
        assert_eq!(authenticated.login_name, "owner");
        assert_eq!(authenticated.display_name, "Owner");
        assert_eq!(
            authenticated.capabilities,
            [
                "alpha.view",
                "dashboard.customize",
                "games.backups.manage",
                "games.manage",
                "games.view",
                "network.firewall.read",
                "network.firewall.write",
                "storage.analyze",
                "storage.files.manage",
                "storage.files.read",
                "system.packages.read",
                "system.packages.write",
                "system.power",
                "system.settings.write",
                "system.view",
                "terminal.open",
                "users.manage",
                "zeta.view",
            ]
        );
        assert_eq!(
            authenticated.absolute_expires_at_unix_ms,
            NOW + SESSION_ABSOLUTE_LIFETIME_MS
        );
        assert!(authenticated.session_expires);
        for capability in ["missing.view", "INVALID"] {
            assert!(
                databases
                    .state()
                    .authenticate_session(SessionAuthenticationInput {
                        session_hash: &fixture.session_hash,
                        authorization: SessionAuthorization::RequireCapability(capability),
                        csrf: CsrfRequirement::NotRequired,
                        now_unix_ms: NOW + 1,
                    })
                    .expect("default deny")
                    .is_none()
            );
        }
    }

    #[test]
    fn auth_only_rejects_every_stale_session_boundary() {
        for case in ["disabled", "revoked", "expired", "auth-version"] {
            let (_temp, databases) = open_databases();
            let fixture = claim_owner(databases.state(), NOW);
            let mut authenticate_at = NOW + 1;
            match case {
                "disabled" => {
                    databases
                        .state()
                        .lock()
                        .expect("state lock")
                        .execute(
                            "UPDATE users
                             SET status = 'disabled', auth_version = auth_version + 1
                             WHERE id = ?1",
                            [&fixture.user_id],
                        )
                        .expect("disable user");
                }
                "revoked" => {
                    databases
                        .state()
                        .revoke_session(&fixture.session_hash, Some(&fixture.user_id), NOW + 1)
                        .expect("revoke session");
                }
                "expired" => authenticate_at = NOW + SESSION_ABSOLUTE_LIFETIME_MS,
                "auth-version" => {
                    databases
                        .state()
                        .lock()
                        .expect("state lock")
                        .execute(
                            "UPDATE users SET auth_version = auth_version + 1 WHERE id = ?1",
                            [&fixture.user_id],
                        )
                        .expect("change auth version");
                }
                _ => unreachable!(),
            }
            assert!(
                databases
                    .state()
                    .authenticate_session(SessionAuthenticationInput {
                        session_hash: &fixture.session_hash,
                        authorization: SessionAuthorization::Authenticated,
                        csrf: CsrfRequirement::NotRequired,
                        now_unix_ms: authenticate_at,
                    })
                    .expect("authenticate stale session")
                    .is_none(),
                "case {case}"
            );
        }
    }

    #[test]
    fn idle_absolute_and_touch_boundaries_are_exact() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let authenticate_at = |now| {
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &fixture.session_hash,
                    authorization: SessionAuthorization::RequireCapability("system.view"),
                    csrf: CsrfRequirement::NotRequired,
                    now_unix_ms: now,
                })
                .expect("authenticate")
        };
        assert!(
            !authenticate_at(NOW + 59_999)
                .expect("active before touch")
                .last_seen_touched
        );
        assert!(
            authenticate_at(NOW + 60_000)
                .expect("active at touch")
                .last_seen_touched
        );
        assert!(authenticate_at(NOW + 60_000 + SESSION_IDLE_LIFETIME_MS - 1).is_some());

        {
            let connection = state.lock().expect("state lock");
            connection
                .execute(
                    "UPDATE sessions SET last_seen_at_unix_ms = ?1 WHERE token_hash = ?2",
                    params![NOW, fixture.session_hash.as_bytes()],
                )
                .expect("reset last seen for exact idle test");
        }
        assert!(authenticate_at(NOW + SESSION_IDLE_LIFETIME_MS).is_none());

        {
            let connection = state.lock().expect("state lock");
            connection
                .execute(
                    "UPDATE sessions SET last_seen_at_unix_ms = absolute_expires_at_unix_ms - 1
                     WHERE token_hash = ?1",
                    [fixture.session_hash.as_bytes()],
                )
                .expect("move last seen near absolute expiry");
        }
        assert!(authenticate_at(NOW + SESSION_ABSOLUTE_LIFETIME_MS).is_none());
    }

    #[test]
    fn disabled_session_expiry_skips_idle_and_eight_hour_caps() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let updated = state
            .set_session_expiry(&fixture.session_hash, false, NOW + 1)
            .expect("disable expiry")
            .expect("current session");
        assert!(!updated.expires);
        assert_eq!(
            updated.absolute_expires_at_unix_ms,
            NOW + SESSION_PERSISTENT_LIFETIME_MS
        );

        let authenticate_at = |now| {
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &fixture.session_hash,
                    authorization: SessionAuthorization::RequireCapability("system.view"),
                    csrf: CsrfRequirement::NotRequired,
                    now_unix_ms: now,
                })
                .expect("authenticate")
        };
        let idle =
            authenticate_at(NOW + SESSION_IDLE_LIFETIME_MS + 60_000).expect("idle must not expire");
        assert!(!idle.session_expires);
        let later = authenticate_at(NOW + SESSION_ABSOLUTE_LIFETIME_MS + 60_000)
            .expect("eight hours must not expire");
        assert!(!later.session_expires);
        assert!(authenticate_at(NOW + SESSION_PERSISTENT_LIFETIME_MS).is_none());

        let restored = state
            .set_session_expiry(
                &fixture.session_hash,
                true,
                NOW + SESSION_ABSOLUTE_LIFETIME_MS + 120_000,
            )
            .expect("enable expiry")
            .expect("current session");
        assert!(restored.expires);
        assert!(
            authenticate_at(
                NOW + SESSION_ABSOLUTE_LIFETIME_MS + 120_000 + SESSION_IDLE_LIFETIME_MS
            )
            .is_none()
        );
    }

    #[test]
    fn session_rows_accept_the_persistent_lifetime_bound() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        databases
            .state()
            .lock()
            .expect("state lock")
            .execute(
                "UPDATE sessions SET absolute_expires_at_unix_ms = created_at_unix_ms + ?1
                 WHERE token_hash = ?2",
                params![
                    SESSION_PERSISTENT_LIFETIME_MS,
                    fixture.session_hash.as_bytes()
                ],
            )
            .expect("apply persistent bound");
        let enabled: bool = databases
            .state()
            .lock()
            .expect("state lock")
            .query_row(
                "SELECT session_expiry_enabled FROM security_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .expect("flag");
        assert!(enabled);
    }

    #[test]
    fn failed_login_delay_is_persistent_bounded_and_resettable() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let mut last = None;
        for _ in 0..40 {
            last = state
                .record_failed_login(&fixture.user_id, NOW + 10)
                .expect("record failed login");
        }
        let delay = last.expect("known user delay");
        assert_eq!(delay.failed_login_count, 32);
        assert!(delay.login_not_before_unix_ms <= NOW + 10 + MAX_FAILED_LOGIN_DELAY_MS);
        let credential = state
            .credential_by_login("owner", NOW + 10)
            .expect("lookup")
            .expect("credential");
        assert_eq!(credential.failed_login_count, 32);
        assert!(
            state
                .reset_failed_login(&fixture.user_id, credential.auth_version, NOW + 11)
                .expect("reset")
        );
        let reset = state
            .credential_by_login("owner", NOW + 11)
            .expect("lookup after reset")
            .expect("credential");
        assert_eq!(reset.failed_login_count, 0);
        assert_eq!(reset.login_not_before_unix_ms, 0);
    }

    #[test]
    fn far_future_login_delay_is_clamped_and_audited_after_clock_repair() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        state
            .lock()
            .expect("state lock")
            .execute(
                "UPDATE users SET login_not_before_unix_ms = ?1 WHERE id = ?2",
                params![NOW + 24 * 60 * 60 * 1_000, fixture.user_id],
            )
            .expect("install anomalous deadline");

        let repaired_now = NOW + 1;
        let credential = state
            .credential_by_login("owner", repaired_now)
            .expect("repair lookup")
            .expect("owner credential");
        let expected_deadline = repaired_now + MAX_FAILED_LOGIN_DELAY_MS;
        assert_eq!(credential.login_not_before_unix_ms, expected_deadline);
        let (stored_deadline, audit_count): (i64, i64) = state
            .lock()
            .expect("state lock after repair")
            .query_row(
                "SELECT u.login_not_before_unix_ms,
                        (SELECT count(*) FROM audit_events
                         WHERE action = 'authentication.delay_clock_clamped')
                 FROM users u WHERE u.id = ?1",
                [&fixture.user_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("stored repaired deadline");
        assert_eq!((stored_deadline, audit_count), (expected_deadline, 1));

        assert!(matches!(
            state
                .create_session_after_verified_login(SessionCreateInput {
                    user_id: &credential.user_id,
                    expected_auth_version: credential.auth_version,
                    expected_password_phc: &credential.password_phc,
                    expected_password_policy_version: credential.password_policy_version,
                    rehash: None,
                    session_hash: &session_hash(70),
                    csrf_hash: &csrf_hash(71),
                    now_unix_ms: expected_deadline,
                })
                .expect("create session at repaired deadline"),
            SessionCreateOutcome::Created { .. }
        ));
    }

    #[test]
    fn verified_login_rehash_and_session_creation_are_one_cas_transaction() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        let state = databases.state();
        let credential = state
            .credential_by_login("owner", NOW)
            .expect("lookup")
            .expect("credential");
        let replacement_phc = password_phc("rehash");
        let new_session = session_hash(50);
        let new_csrf = csrf_hash(51);
        let outcome = state
            .create_session_after_verified_login(SessionCreateInput {
                user_id: &credential.user_id,
                expected_auth_version: credential.auth_version,
                expected_password_phc: &credential.password_phc,
                expected_password_policy_version: credential.password_policy_version,
                rehash: Some(PasswordRehash {
                    replacement_password_phc: &replacement_phc,
                    replacement_password_policy_version: 2,
                }),
                session_hash: &new_session,
                csrf_hash: &new_csrf,
                now_unix_ms: NOW + 100,
            })
            .expect("rehash and session");
        let SessionCreateOutcome::Created { auth_version, .. } = outcome else {
            panic!("session was not created: {outcome:?}");
        };
        assert_eq!(auth_version, credential.auth_version + 1);
        let updated = state
            .credential_by_login("owner", NOW + 1)
            .expect("updated lookup")
            .expect("updated credential");
        assert_eq!(updated.password_phc, replacement_phc);
        assert_eq!(updated.password_policy_version, 2);
        assert_eq!(updated.auth_version, auth_version);
        assert!(
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &fixture.session_hash,
                    authorization: SessionAuthorization::RequireCapability("system.view"),
                    csrf: CsrfRequirement::NotRequired,
                    now_unix_ms: NOW + 101,
                })
                .expect("old session")
                .is_none()
        );
        assert!(
            state
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &new_session,
                    authorization: SessionAuthorization::RequireCapability("system.view"),
                    csrf: CsrfRequirement::Match(&new_csrf),
                    now_unix_ms: NOW + 101,
                })
                .expect("new session")
                .is_some()
        );

        assert_eq!(
            state
                .create_session_after_verified_login(SessionCreateInput {
                    user_id: &credential.user_id,
                    expected_auth_version: credential.auth_version,
                    expected_password_phc: &credential.password_phc,
                    expected_password_policy_version: 1,
                    rehash: None,
                    session_hash: &session_hash(52),
                    csrf_hash: &csrf_hash(53),
                    now_unix_ms: NOW + 102,
                })
                .expect("stale CAS"),
            SessionCreateOutcome::CredentialChangedOrUnavailable
        );
    }

    #[test]
    fn audit_record_pressure_is_bounded_and_prefers_older_denials() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let policy = AuditRetentionPolicy {
            retention_window_ms: 100,
            minimum_rows: 3,
            maximum_rows: 6,
            prune_batch: 2,
        };
        let mut connection = state.lock().expect("state lock");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("retention transaction");
        let ids = seed_audit_events(
            &transaction,
            &[
                (NOW - 500, "success"),
                (NOW - 490, "denied"),
                (NOW - 480, "success"),
                (NOW - 20, "denied"),
                (NOW - 10, "success"),
                (NOW - 9, "denied"),
                (NOW - 8, "success"),
                (NOW - 7, "denied"),
            ],
        );

        assert_eq!(
            prune_authentication_audit_in(&transaction, NOW, policy)
                .expect("bounded record-pressure cleanup"),
            2
        );
        transaction.commit().expect("commit retention cleanup");
        drop(connection);

        let retained = audit_ids(state);
        assert_eq!(retained.len(), 6);
        assert!(!retained.contains(&ids[1]));
        assert!(!retained.contains(&ids[3]));
        assert!(retained.contains(&ids[0]));
        assert!(retained.contains(&ids[2]));
        assert!(retained.contains(&ids[5]));
        assert!(retained.contains(&ids[6]));
        assert!(retained.contains(&ids[7]));
        assert!(
            state
                .lock()
                .expect("state lock after cleanup")
                .execute("DELETE FROM audit_events WHERE id = ?1", [&ids[0]])
                .is_err()
        );
    }

    #[test]
    fn production_audit_write_keeps_the_exact_steady_state_ceiling() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let mut connection = state.lock().expect("state lock");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("audit flood transaction");
        transaction
            .execute(
                "WITH RECURSIVE sequence(value) AS (
                     VALUES (0)
                     UNION ALL
                     SELECT value + 1 FROM sequence WHERE value + 1 < ?1
                 )
                 INSERT INTO audit_events (
                     id, occurred_at_unix_ms, actor_user_id, action,
                     target_type, target_id, outcome, correlation_id, detail_json
                 )
                 SELECT
                     printf('60000000-0000-4000-8000-%012d', value),
                     ?2 + value,
                     NULL,
                     'authentication.failed',
                     'user',
                     'flood-fixture',
                     'denied',
                     printf('70000000-0000-4000-8000-%012d', value),
                     '{}'
                 FROM sequence",
                params![AUDIT_MAX_RETAINED_ROWS, NOW],
            )
            .expect("seed the production audit ceiling");
        append_audit(
            &transaction,
            NOW + AUDIT_MAX_RETAINED_ROWS,
            None,
            "authentication.succeeded",
            Some("session"),
            Some("ceiling-fixture"),
            "success",
        )
        .expect("append at the production ceiling");
        transaction.commit().expect("commit bounded audit write");
        drop(connection);

        let (tracked, actual, success_rows, oldest): (i64, i64, i64, i64) = state
            .lock()
            .expect("state lock after audit flood")
            .query_row(
                "SELECT
                     retained_event_count,
                     (SELECT count(*) FROM audit_events),
                     (SELECT count(*) FROM audit_events
                      WHERE action = 'authentication.succeeded'),
                     (SELECT min(occurred_at_unix_ms) FROM audit_events)
                 FROM audit_retention_state WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("bounded audit counts");
        assert_eq!(
            (tracked, actual, success_rows, oldest),
            (AUDIT_MAX_RETAINED_ROWS, AUDIT_MAX_RETAINED_ROWS, 1, NOW + 1)
        );
    }

    #[test]
    fn audit_age_cleanup_converges_in_fixed_batches_and_keeps_the_newest_floor() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let policy = AuditRetentionPolicy {
            retention_window_ms: 100,
            minimum_rows: 3,
            maximum_rows: 10,
            prune_batch: 2,
        };
        let mut connection = state.lock().expect("state lock");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("seed transaction");
        let ids = seed_audit_events(
            &transaction,
            &[
                (NOW - 500, "success"),
                (NOW - 400, "denied"),
                (NOW - 300, "success"),
                (NOW - 200, "denied"),
                (NOW - 10, "success"),
                (NOW - 9, "denied"),
                (NOW - 8, "success"),
            ],
        );
        transaction.commit().expect("commit audit fixtures");
        drop(connection);

        for expected in [2, 2, 0] {
            let mut connection = state.lock().expect("state lock for retention pass");
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("retention transaction");
            assert_eq!(
                prune_authentication_audit_in(&transaction, NOW, policy).expect("age cleanup pass"),
                expected
            );
            transaction.commit().expect("commit retention pass");
        }

        assert_eq!(audit_ids(state), ids[4..].to_vec());
        let (tracked, actual): (i64, i64) = state
            .lock()
            .expect("state lock after convergence")
            .query_row(
                "SELECT retained_event_count, (SELECT count(*) FROM audit_events)
                 FROM audit_retention_state WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("retention counts");
        assert_eq!((tracked, actual), (3, 3));
    }

    #[test]
    fn failed_audit_guard_restore_rolls_back_pruning_and_restores_append_only_state() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let policy = AuditRetentionPolicy {
            retention_window_ms: 100,
            minimum_rows: 3,
            maximum_rows: 10,
            prune_batch: 2,
        };
        let mut connection = state.lock().expect("state lock");
        let seed_transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("seed transaction");
        seed_audit_events(
            &seed_transaction,
            &[
                (NOW - 500, "success"),
                (NOW - 400, "denied"),
                (NOW - 300, "success"),
                (NOW - 10, "denied"),
                (NOW - 9, "success"),
            ],
        );
        seed_transaction.commit().expect("commit audit fixtures");
        drop(connection);
        let before = audit_ids(state);

        let mut connection = state.lock().expect("state lock for failed pruning");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("retention transaction");
        fail_audit_delete_guard_recreate_for_test();
        assert!(matches!(
            prune_authentication_audit_in(&transaction, NOW, policy),
            Err(StateError::Sqlite(rusqlite::Error::ExecuteReturnedResults))
        ));
        drop(transaction);
        drop(connection);

        assert_eq!(audit_ids(state), before);
        assert!(
            state
                .lock()
                .expect("state lock after rollback")
                .execute("DELETE FROM audit_events WHERE id = ?1", [&before[0]])
                .is_err()
        );
    }

    #[test]
    fn audit_events_are_append_only_and_secret_free() {
        let (_temp, databases) = open_databases();
        let fixture = claim_owner(databases.state(), NOW);
        databases
            .state()
            .revoke_session(&fixture.session_hash, Some(&fixture.user_id), NOW + 1)
            .expect("revoke");
        let connection = databases.state().lock().expect("state lock");
        assert!(
            connection
                .execute("UPDATE audit_events SET outcome = 'error'", [])
                .is_err()
        );
        assert!(connection.execute("DELETE FROM audit_events", []).is_err());
        assert!(
            connection
                .execute("DELETE FROM audit_retention_state", [])
                .is_err()
        );
        assert!(
            connection
                .execute(
                    "UPDATE audit_retention_state SET maximum_rows = maximum_rows + 1",
                    [],
                )
                .is_err()
        );
        let (tracked, actual): (i64, i64) = connection
            .query_row(
                "SELECT retained_event_count, (SELECT count(*) FROM audit_events)
                 FROM audit_retention_state WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("retention counts");
        assert_eq!(tracked, actual);
        let details = connection
            .prepare("SELECT detail_json FROM audit_events")
            .expect("audit query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("audit rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("audit details");
        assert!(details.iter().all(|detail| detail == "{}"));
    }

    #[test]
    fn state_boundary_rejects_noncanonical_identity_fields() {
        let (_temp, databases) = open_databases();
        let state = databases.state();
        let bootstrap = bootstrap_hash(60);
        state
            .replace_bootstrap_token(&bootstrap, NOW, NOW + 60_000)
            .expect("bootstrap");
        let phc = password_phc("identity");
        for login in ["Owner", "_owner", "ow", "owner!", "owner_"] {
            assert!(matches!(
                state.claim_owner(OwnerClaimInput {
                    bootstrap_hash: &bootstrap,
                    login_name: login,
                    display_name: "Owner",
                    password_phc: &phc,
                    password_policy_version: 1,
                    session_hash: &session_hash(61),
                    csrf_hash: &csrf_hash(62),
                    now_unix_ms: NOW + 1,
                }),
                Err(StateError::InvalidSecurityInput(_))
            ));
        }
        for display_name in [" Owner", "Owner ", "Owner\nName", "Rique\u{301}"] {
            assert!(matches!(
                state.claim_owner(OwnerClaimInput {
                    bootstrap_hash: &bootstrap,
                    login_name: "owner",
                    display_name,
                    password_phc: &phc,
                    password_policy_version: 1,
                    session_hash: &session_hash(63),
                    csrf_hash: &csrf_hash(64),
                    now_unix_ms: NOW + 1,
                }),
                Err(StateError::InvalidSecurityInput(_))
            ));
        }
        assert!(matches!(
            state
                .claim_owner(OwnerClaimInput {
                    bootstrap_hash: &bootstrap,
                    login_name: "owner",
                    display_name: "Riqu\u{e9}",
                    password_phc: &phc,
                    password_policy_version: 1,
                    session_hash: &session_hash(65),
                    csrf_hash: &csrf_hash(66),
                    now_unix_ms: NOW + 1,
                })
                .expect("canonical NFC display name"),
            OwnerClaimOutcome::Claimed { .. }
        ));
    }

    #[test]
    fn digest_comparison_accepts_only_exact_matches() {
        let expected = [7_u8; 32];
        let mut mismatch = expected;
        mismatch[31] ^= 1;

        assert!(digest_matches(&expected, &expected));
        assert!(!digest_matches(&mismatch, &expected));
        assert!(!digest_matches(&expected[..31], &expected));
    }

    #[test]
    fn v1_to_current_migration_creates_verified_v1_snapshot() {
        let temp = crate::private_test_directory("temporary directory");
        create_state_at_version_one(temp.path());

        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("migrate state");
        assert_eq!(
            databases.state().schema_version().expect("schema"),
            STATE_SCHEMA_VERSION
        );
        let backup = only_migration_backup(temp.path());
        let connection = Connection::open(backup).expect("open migration backup");
        assert_eq!(
            pragma_i64(&connection, "user_version").expect("backup schema"),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM schema_migrations WHERE version = 1 AND name = 'foundational-state'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .expect("migration row"),
            1
        );
    }

    #[test]
    fn v3_audit_rows_are_counted_and_preserved_across_the_v4_migration() {
        let temp = crate::private_test_directory("temporary directory");
        create_state_at_version_three_with_audit(temp.path());

        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("migrate v3 state");
        assert_eq!(
            databases.state().schema_version().expect("schema"),
            STATE_SCHEMA_VERSION
        );
        let (tracked, actual): (i64, i64) = databases
            .state()
            .lock()
            .expect("state lock")
            .query_row(
                "SELECT retained_event_count, (SELECT count(*) FROM audit_events)
                 FROM audit_retention_state WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("migrated audit retention state");
        assert_eq!((tracked, actual), (1, 1));
        drop(databases);

        let backup = only_migration_backup(temp.path());
        let connection = Connection::open(backup).expect("open v3 migration backup");
        assert_eq!(
            pragma_i64(&connection, "user_version").expect("backup schema"),
            3
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM audit_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("preserved audit rows"),
            1
        );
    }

    #[test]
    fn nonempty_v0_database_is_snapshotted_before_foundational_migration() {
        let temp = crate::private_test_directory("temporary directory");
        let state_dir = temp.path().join("state");
        ensure_data_directory(temp.path()).expect("data directory");
        ensure_data_directory(&state_dir).expect("state directory");
        let state_path = state_dir.join("helix-state.db");
        create_private_file(&state_path).expect("state file");
        let connection = Connection::open(&state_path).expect("open v0 database");
        configure_state_connection(&connection).expect("configure v0");
        connection
            .execute_batch(
                "CREATE TABLE legacy_marker (value TEXT NOT NULL) STRICT;
                 INSERT INTO legacy_marker VALUES ('preserve-me');",
            )
            .expect("create legacy content");
        drop(connection);

        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("migrate v0");
        assert_eq!(
            databases.state().schema_version().expect("schema"),
            STATE_SCHEMA_VERSION
        );
        let backup = only_migration_backup(temp.path());
        let connection = Connection::open(backup).expect("open v0 backup");
        assert_eq!(
            pragma_i64(&connection, "user_version").expect("backup schema"),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT value FROM legacy_marker", [], |row| {
                    row.get::<_, String>(0)
                })
                .expect("preserved legacy row"),
            "preserve-me"
        );
    }

    #[test]
    fn verified_backup_rejects_semantically_invalid_migration_history() {
        let (temp, databases) = open_databases();
        {
            let connection = databases.state().lock().expect("state lock");
            connection
                .execute(
                    "UPDATE schema_migrations SET name = 'tampered' WHERE version = 2",
                    [],
                )
                .expect("tamper migration metadata");
        }
        let destination = temp.path().join("invalid-backup.db");
        assert!(matches!(
            databases.state().backup_to(&destination),
            Err(StateError::Integrity { .. })
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn live_startup_rejects_missing_required_security_semantics() {
        for tamper_sql in [
            "DROP TABLE security_state;",
            "DELETE FROM role_capabilities WHERE capability = 'system.view';",
            "DELETE FROM role_capabilities WHERE capability = 'games.view';",
            "DELETE FROM role_capabilities WHERE capability = 'games.manage';",
            "DELETE FROM role_capabilities WHERE capability = 'storage.files.read';",
            "DELETE FROM role_capabilities WHERE capability = 'storage.files.manage';",
            "DROP TRIGGER audit_events_append_only_delete;",
            "DROP TRIGGER audit_events_retention_count_insert;",
            "DROP INDEX audit_events_retention_priority_idx;",
            "UPDATE audit_retention_state SET retained_event_count = retained_event_count + 1;",
        ] {
            let (temp, databases) = open_databases();
            drop(databases);
            let state_path = temp.path().join("state").join("helix-state.db");
            let connection = Connection::open(state_path).expect("open state for tamper test");
            connection
                .execute_batch(tamper_sql)
                .expect("tamper with required semantics");
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

    fn create_state_at_version_one(data_dir: &Path) {
        let state_dir = data_dir.join("state");
        ensure_data_directory(data_dir).expect("data directory");
        ensure_data_directory(&state_dir).expect("state directory");
        let state_path = state_dir.join("helix-state.db");
        create_private_file(&state_path).expect("state file");
        let mut connection = Connection::open(state_path).expect("open state");
        configure_state_connection(&connection).expect("configure state");
        apply_migration(&mut connection, 1, "foundational-state", STATE_MIGRATION_1)
            .expect("apply v1");
        ensure_installation(&mut connection).expect("installation row");
    }

    fn create_state_at_version_three_with_audit(data_dir: &Path) {
        let state_dir = data_dir.join("state");
        ensure_data_directory(data_dir).expect("data directory");
        ensure_data_directory(&state_dir).expect("state directory");
        let state_path = state_dir.join("helix-state.db");
        create_private_file(&state_path).expect("state file");
        let mut connection = Connection::open(state_path).expect("open state");
        configure_state_connection(&connection).expect("configure state");
        apply_migration(&mut connection, 1, "foundational-state", STATE_MIGRATION_1)
            .expect("apply v1");
        migrate_security(&mut connection).expect("apply v2");
        crate::secrets::migrate_secrets(&mut connection).expect("apply v3");
        ensure_installation(&mut connection).expect("installation row");
        connection
            .execute(
                "INSERT INTO audit_events (
                    id, occurred_at_unix_ms, actor_user_id, action,
                    target_type, target_id, outcome, correlation_id, detail_json
                 ) VALUES (?1, ?2, NULL, 'authentication.test',
                           'user', 'migration-fixture', 'success', ?3, '{}')",
                params![
                    "40000000-0000-4000-8000-000000000000",
                    NOW,
                    "50000000-0000-4000-8000-000000000000"
                ],
            )
            .expect("seed v3 audit event");
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

    #[test]
    fn fresh_database_does_not_create_a_pointless_v0_snapshot() {
        let (temp, _databases) = open_databases();
        assert!(!temp.path().join("state").join("migration-backups").exists());
    }

    #[test]
    fn read_only_reader_reports_current_schema_without_mutating_security_state() {
        let (temp, databases) = open_databases();
        claim_owner(databases.state(), NOW);
        drop(databases);
        let reader = StateDatabaseReader::open(temp.path()).expect("state reader");
        assert_eq!(
            reader.schema_version().expect("schema"),
            STATE_SCHEMA_VERSION
        );
    }
}
