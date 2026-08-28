use super::{StateDatabase, StateError, apply_migration};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

pub const MAX_STRAND_PACKAGES: i64 = 32;
pub const MAX_STRAND_PACKAGE_BYTES: i64 = 8 * 1024 * 1024;
pub const MAX_STRAND_KV_KEYS: i64 = 256;
pub const MAX_STRAND_KV_KEY_CHARS: usize = 128;
pub const MAX_STRAND_KV_VALUE_BYTES: usize = 8 * 1024;
pub const MAX_STRAND_KV_TOTAL_BYTES: i64 = 1_048_576;

const STRAND_MIGRATION_8: &str = r#"
CREATE TABLE strand_packages (
    id TEXT PRIMARY KEY CHECK (
        length(id) = 36
        AND id NOT GLOB '*[^0-9a-f-]*'
    ),
    slug TEXT NOT NULL UNIQUE CHECK (
        length(slug) BETWEEN 2 AND 48
        AND slug GLOB '[a-z]*'
        AND slug NOT GLOB '*[^a-z0-9-]*'
    ),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    version TEXT NOT NULL CHECK (length(version) BETWEEN 1 AND 64),
    description TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 240),
    license TEXT NOT NULL CHECK (length(license) BETWEEN 1 AND 96),
    publisher TEXT NOT NULL CHECK (length(publisher) BETWEEN 1 AND 120),
    kind TEXT NOT NULL CHECK (kind = 'ui-only'),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    origin TEXT NOT NULL CHECK (origin IN ('upload', 'url')),
    origin_detail TEXT NOT NULL CHECK (length(origin_detail) BETWEEN 1 AND 512),
    digest_sha256 TEXT NOT NULL CHECK (
        length(digest_sha256) = 64
        AND digest_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    ui_entry TEXT NOT NULL CHECK (
        length(ui_entry) BETWEEN 8 AND 200
        AND ui_entry GLOB 'ui/*'
    ),
    capabilities_json TEXT NOT NULL CHECK (
        json_valid(capabilities_json)
        AND length(capabilities_json) BETWEEN 2 AND 16384
    ),
    limits_json TEXT NOT NULL CHECK (
        json_valid(limits_json)
        AND length(limits_json) BETWEEN 2 AND 4096
    ),
    package_bytes BLOB NOT NULL CHECK (
        typeof(package_bytes) = 'blob'
        AND length(package_bytes) BETWEEN 32 AND 8388608
    ),
    installed_at_unix_ms INTEGER NOT NULL CHECK (installed_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0)
) STRICT;

CREATE TABLE strand_kv (
    strand_id TEXT NOT NULL REFERENCES strand_packages(id) ON DELETE CASCADE,
    key TEXT NOT NULL CHECK (
        length(key) BETWEEN 1 AND 128
        AND key NOT GLOB '*[^a-z0-9._-]*'
    ),
    value TEXT NOT NULL CHECK (length(value) BETWEEN 0 AND 8192),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0),
    PRIMARY KEY (strand_id, key)
) STRICT;
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StrandOrigin {
    Upload,
    Url,
}

impl StrandOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Url => "url",
        }
    }

    fn parse(value: &str) -> Result<Self, StateError> {
        match value {
            "upload" => Ok(Self::Upload),
            "url" => Ok(Self::Url),
            _ => Err(StateError::InvalidStrandInput("unknown Strand origin")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrandPackageSummary {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: String,
    pub publisher: String,
    pub kind: String,
    pub enabled: bool,
    pub origin: StrandOrigin,
    pub origin_detail: String,
    pub digest_sha256: String,
    pub ui_entry: String,
    pub capabilities_json: String,
    pub limits_json: String,
    pub package_bytes_len: i64,
    pub installed_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrandPackageRecord {
    pub summary: StrandPackageSummary,
    pub package_bytes: Vec<u8>,
}

pub struct StrandInstallInput<'a> {
    pub id: &'a str,
    pub slug: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub description: &'a str,
    pub license: &'a str,
    pub publisher: &'a str,
    pub origin: StrandOrigin,
    pub origin_detail: &'a str,
    pub digest_sha256: &'a str,
    pub ui_entry: &'a str,
    pub capabilities_json: &'a str,
    pub limits_json: &'a str,
    pub package_bytes: &'a [u8],
    pub now_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrandKvEntry {
    pub key: String,
    pub value: String,
    pub updated_at_unix_ms: i64,
}

pub(super) fn migrate_strands(connection: &mut rusqlite::Connection) -> Result<(), StateError> {
    apply_migration(connection, 8, "installable-ui-strands", STRAND_MIGRATION_8)
}

impl StateDatabase {
    pub fn list_strand_packages(&self) -> Result<Vec<StrandPackageSummary>, StateError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, slug, name, version, description, license, publisher, kind, enabled,
                    origin, origin_detail, digest_sha256, ui_entry, capabilities_json, limits_json,
                    length(package_bytes), installed_at_unix_ms, updated_at_unix_ms
             FROM strand_packages ORDER BY slug LIMIT 33",
        )?;
        let records = statement
            .query_map([], row_to_summary)?
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() > usize::try_from(MAX_STRAND_PACKAGES).unwrap_or(usize::MAX) {
            return Err(StateError::Integrity {
                database: "helix-state.db",
                details: vec!["Strand package row limit was exceeded".to_owned()],
            });
        }
        Ok(records)
    }

    pub fn strand_package(&self, id: &str) -> Result<Option<StrandPackageRecord>, StateError> {
        require_strand_id(id)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, slug, name, version, description, license, publisher, kind, enabled,
                        origin, origin_detail, digest_sha256, ui_entry, capabilities_json, limits_json,
                        length(package_bytes), installed_at_unix_ms, updated_at_unix_ms, package_bytes
                 FROM strand_packages WHERE id = ?1",
                [id],
                |row| {
                    let summary = row_to_summary(row)?;
                    let package_bytes: Vec<u8> = row.get(18)?;
                    Ok(StrandPackageRecord {
                        summary,
                        package_bytes,
                    })
                },
            )
            .optional()
            .map_err(StateError::from)
    }

    pub fn install_strand_package(
        &self,
        input: StrandInstallInput<'_>,
    ) -> Result<StrandPackageSummary, StateError> {
        validate_install_input(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_id: Option<String> = transaction
            .query_row(
                "SELECT id FROM strand_packages WHERE slug = ?1",
                [input.slug],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_id) = existing_id.as_deref()
            && existing_id != input.id
        {
            return Err(StateError::StrandConflict);
        }
        let existing_same_id: Option<String> = transaction
            .query_row(
                "SELECT slug FROM strand_packages WHERE id = ?1",
                [input.id],
                |row| row.get(0),
            )
            .optional()?;
        if existing_same_id.is_none() && existing_id.is_none() {
            let count: i64 =
                transaction
                    .query_row("SELECT COUNT(*) FROM strand_packages", [], |row| row.get(0))?;
            if count >= MAX_STRAND_PACKAGES {
                return Err(StateError::StrandQuotaExceeded);
            }
        }
        transaction.execute(
            "INSERT INTO strand_packages (
                id, slug, name, version, description, license, publisher, kind, enabled,
                origin, origin_detail, digest_sha256, ui_entry, capabilities_json, limits_json,
                package_bytes, installed_at_unix_ms, updated_at_unix_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'ui-only', COALESCE((SELECT enabled FROM strand_packages WHERE id = ?1), 0),
                ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15
             )
             ON CONFLICT(id) DO UPDATE SET
                slug = excluded.slug,
                name = excluded.name,
                version = excluded.version,
                description = excluded.description,
                license = excluded.license,
                publisher = excluded.publisher,
                origin = excluded.origin,
                origin_detail = excluded.origin_detail,
                digest_sha256 = excluded.digest_sha256,
                ui_entry = excluded.ui_entry,
                capabilities_json = excluded.capabilities_json,
                limits_json = excluded.limits_json,
                package_bytes = excluded.package_bytes,
                updated_at_unix_ms = excluded.updated_at_unix_ms,
                enabled = 0",
            params![
                input.id,
                input.slug,
                input.name,
                input.version,
                input.description,
                input.license,
                input.publisher,
                input.origin.as_str(),
                input.origin_detail,
                input.digest_sha256,
                input.ui_entry,
                input.capabilities_json,
                input.limits_json,
                input.package_bytes,
                input.now_unix_ms,
            ],
        )?;
        let summary = transaction.query_row(
            "SELECT id, slug, name, version, description, license, publisher, kind, enabled,
                    origin, origin_detail, digest_sha256, ui_entry, capabilities_json, limits_json,
                    length(package_bytes), installed_at_unix_ms, updated_at_unix_ms
             FROM strand_packages WHERE id = ?1",
            [input.id],
            row_to_summary,
        )?;
        transaction.commit()?;
        Ok(summary)
    }

    pub fn set_strand_enabled(
        &self,
        id: &str,
        enabled: bool,
        now_unix_ms: i64,
    ) -> Result<Option<StrandPackageSummary>, StateError> {
        require_strand_id(id)?;
        if now_unix_ms < 0 {
            return Err(StateError::InvalidStrandInput("timestamp is invalid"));
        }
        let connection = self.lock()?;
        let changed = connection.execute(
            "UPDATE strand_packages SET enabled = ?1, updated_at_unix_ms = ?2 WHERE id = ?3",
            params![i64::from(enabled), now_unix_ms, id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        connection
            .query_row(
                "SELECT id, slug, name, version, description, license, publisher, kind, enabled,
                        origin, origin_detail, digest_sha256, ui_entry, capabilities_json, limits_json,
                        length(package_bytes), installed_at_unix_ms, updated_at_unix_ms
                 FROM strand_packages WHERE id = ?1",
                [id],
                row_to_summary,
            )
            .optional()
            .map_err(StateError::from)
    }

    pub fn delete_strand_package(&self, id: &str) -> Result<bool, StateError> {
        require_strand_id(id)?;
        let connection = self.lock()?;
        let deleted = connection.execute("DELETE FROM strand_packages WHERE id = ?1", [id])?;
        Ok(deleted > 0)
    }

    pub fn strand_kv_get(
        &self,
        strand_id: &str,
        key: &str,
    ) -> Result<Option<StrandKvEntry>, StateError> {
        require_strand_id(strand_id)?;
        require_kv_key(key)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT key, value, updated_at_unix_ms FROM strand_kv WHERE strand_id = ?1 AND key = ?2",
                params![strand_id, key],
                row_to_kv,
            )
            .optional()
            .map_err(StateError::from)
    }

    pub fn strand_kv_list(&self, strand_id: &str) -> Result<Vec<StrandKvEntry>, StateError> {
        require_strand_id(strand_id)?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT key, value, updated_at_unix_ms FROM strand_kv WHERE strand_id = ?1 ORDER BY key LIMIT 257",
        )?;
        let records = statement
            .query_map([strand_id], row_to_kv)?
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() > usize::try_from(MAX_STRAND_KV_KEYS).unwrap_or(usize::MAX) {
            return Err(StateError::Integrity {
                database: "helix-state.db",
                details: vec!["Strand storage key limit was exceeded".to_owned()],
            });
        }
        Ok(records)
    }

    pub fn strand_kv_set(
        &self,
        strand_id: &str,
        key: &str,
        value: &str,
        now_unix_ms: i64,
    ) -> Result<StrandKvEntry, StateError> {
        require_strand_id(strand_id)?;
        require_kv_key(key)?;
        require_kv_value(value)?;
        if now_unix_ms < 0 {
            return Err(StateError::InvalidStrandInput("timestamp is invalid"));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM strand_packages WHERE id = ?1",
            [strand_id],
            |row| row.get(0),
        )?;
        if exists == 0 {
            return Err(StateError::StrandNotFound);
        }
        let current_key: Option<String> = transaction
            .query_row(
                "SELECT key FROM strand_kv WHERE strand_id = ?1 AND key = ?2",
                params![strand_id, key],
                |row| row.get(0),
            )
            .optional()?;
        if current_key.is_none() {
            let count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM strand_kv WHERE strand_id = ?1",
                [strand_id],
                |row| row.get(0),
            )?;
            if count >= MAX_STRAND_KV_KEYS {
                return Err(StateError::StrandQuotaExceeded);
            }
        }
        let used: i64 = transaction.query_row(
            "SELECT COALESCE(SUM(length(value)), 0) FROM strand_kv WHERE strand_id = ?1 AND key != ?2",
            params![strand_id, key],
            |row| row.get(0),
        )?;
        let next_total = used.saturating_add(i64::try_from(value.len()).unwrap_or(i64::MAX));
        if next_total > MAX_STRAND_KV_TOTAL_BYTES {
            return Err(StateError::StrandQuotaExceeded);
        }
        transaction.execute(
            "INSERT INTO strand_kv (strand_id, key, value, updated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(strand_id, key) DO UPDATE SET
                value = excluded.value,
                updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![strand_id, key, value, now_unix_ms],
        )?;
        let entry = transaction.query_row(
            "SELECT key, value, updated_at_unix_ms FROM strand_kv WHERE strand_id = ?1 AND key = ?2",
            params![strand_id, key],
            row_to_kv,
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    pub fn strand_kv_delete(&self, strand_id: &str, key: &str) -> Result<bool, StateError> {
        require_strand_id(strand_id)?;
        require_kv_key(key)?;
        let connection = self.lock()?;
        let deleted = connection.execute(
            "DELETE FROM strand_kv WHERE strand_id = ?1 AND key = ?2",
            params![strand_id, key],
        )?;
        Ok(deleted > 0)
    }
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<StrandPackageSummary> {
    Ok(StrandPackageSummary {
        id: row.get(0)?,
        slug: row.get(1)?,
        name: row.get(2)?,
        version: row.get(3)?,
        description: row.get(4)?,
        license: row.get(5)?,
        publisher: row.get(6)?,
        kind: row.get(7)?,
        enabled: row.get::<_, i64>(8)? == 1,
        origin: StrandOrigin::parse(&row.get::<_, String>(9)?).map_err(|_| {
            rusqlite::Error::InvalidColumnType(9, "origin".to_owned(), rusqlite::types::Type::Text)
        })?,
        origin_detail: row.get(10)?,
        digest_sha256: row.get(11)?,
        ui_entry: row.get(12)?,
        capabilities_json: row.get(13)?,
        limits_json: row.get(14)?,
        package_bytes_len: row.get(15)?,
        installed_at_unix_ms: row.get(16)?,
        updated_at_unix_ms: row.get(17)?,
    })
}

fn row_to_kv(row: &rusqlite::Row<'_>) -> rusqlite::Result<StrandKvEntry> {
    Ok(StrandKvEntry {
        key: row.get(0)?,
        value: row.get(1)?,
        updated_at_unix_ms: row.get(2)?,
    })
}

fn require_strand_id(id: &str) -> Result<(), StateError> {
    if id.len() == 36
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        && id.as_bytes()[8] == b'-'
        && id.as_bytes()[13] == b'-'
        && id.as_bytes()[18] == b'-'
        && id.as_bytes()[23] == b'-'
    {
        Ok(())
    } else {
        Err(StateError::InvalidStrandInput("Strand id must be a UUID"))
    }
}

fn require_kv_key(key: &str) -> Result<(), StateError> {
    let valid = (1..=MAX_STRAND_KV_KEY_CHARS).contains(&key.len())
        && key
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && key.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'.'
                || byte == b'_'
                || byte == b'-'
        });
    if valid {
        Ok(())
    } else {
        Err(StateError::InvalidStrandInput(
            "storage keys must be short lowercase identifiers",
        ))
    }
}

fn require_kv_value(value: &str) -> Result<(), StateError> {
    if value.len() > MAX_STRAND_KV_VALUE_BYTES {
        return Err(StateError::InvalidStrandInput(
            "storage values exceed the per-key limit",
        ));
    }
    if value
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(StateError::InvalidStrandInput(
            "storage values must not contain control characters",
        ));
    }
    Ok(())
}

fn validate_install_input(input: &StrandInstallInput<'_>) -> Result<(), StateError> {
    require_strand_id(input.id)?;
    if !(32..=MAX_STRAND_PACKAGE_BYTES).contains(&(input.package_bytes.len() as i64)) {
        return Err(StateError::InvalidStrandInput(
            "Strand package size is outside the allowed range",
        ));
    }
    if input.digest_sha256.len() != 64
        || input
            .digest_sha256
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(StateError::InvalidStrandInput(
            "Strand digest must be lowercase SHA-256 hex",
        ));
    }
    if input.origin_detail.is_empty() || input.origin_detail.len() > 512 {
        return Err(StateError::InvalidStrandInput(
            "Strand origin detail is invalid",
        ));
    }
    if input.now_unix_ms < 0 {
        return Err(StateError::InvalidStrandInput("timestamp is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatabaseSet;

    fn sample_zip() -> Vec<u8> {
        vec![0_u8; 64]
    }

    fn install_input<'a>(bytes: &'a [u8]) -> StrandInstallInput<'a> {
        StrandInstallInput {
            id: "b893d568-327d-4b6e-b0b6-0b7a58e0c852",
            slug: "system-health",
            name: "System Health",
            version: "0.1.0",
            description: "Shows a bounded summary of host health.",
            license: "AGPL-3.0-or-later",
            publisher: "Helix example",
            origin: StrandOrigin::Upload,
            origin_detail: "system-health.strand.zip",
            digest_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ui_entry: "ui/index.html",
            capabilities_json: r#"[{"name":"helix:metrics.read"}]"#,
            limits_json: r#"{"memory_mib":32}"#,
            package_bytes: bytes,
            now_unix_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn install_enable_kv_and_delete_are_bounded() {
        let temp = crate::private_test_directory("strand state");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open");
        let bytes = sample_zip();
        let installed = databases
            .state()
            .install_strand_package(install_input(&bytes))
            .expect("install");
        assert!(!installed.enabled);
        assert_eq!(installed.slug, "system-health");

        let enabled = databases
            .state()
            .set_strand_enabled(&installed.id, true, 1_700_000_000_100)
            .expect("enable")
            .expect("present");
        assert!(enabled.enabled);

        databases
            .state()
            .strand_kv_set(
                &installed.id,
                "last-probe",
                "{\"ok\":true}",
                1_700_000_000_200,
            )
            .expect("kv set");
        let stored = databases
            .state()
            .strand_kv_get(&installed.id, "last-probe")
            .expect("kv get")
            .expect("value");
        assert_eq!(stored.value, "{\"ok\":true}");

        assert!(
            databases
                .state()
                .delete_strand_package(&installed.id)
                .expect("delete")
        );
        assert!(
            databases
                .state()
                .list_strand_packages()
                .expect("list")
                .is_empty()
        );
    }

    #[test]
    fn duplicate_slug_with_a_different_id_conflicts() {
        let temp = crate::private_test_directory("strand conflict");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open");
        let bytes = sample_zip();
        databases
            .state()
            .install_strand_package(install_input(&bytes))
            .expect("install");
        let mut second = install_input(&bytes);
        second.id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        assert!(matches!(
            databases.state().install_strand_package(second),
            Err(StateError::StrandConflict)
        ));
    }

    #[test]
    fn reinstalling_the_same_id_disables_the_package_and_keeps_kv() {
        let temp = crate::private_test_directory("strand update");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open");
        let bytes = sample_zip();
        databases
            .state()
            .install_strand_package(install_input(&bytes))
            .expect("install");
        databases
            .state()
            .set_strand_enabled(
                "b893d568-327d-4b6e-b0b6-0b7a58e0c852",
                true,
                1_700_000_000_100,
            )
            .expect("enable");
        databases
            .state()
            .strand_kv_set(
                "b893d568-327d-4b6e-b0b6-0b7a58e0c852",
                "token",
                "keep-me",
                1_700_000_000_200,
            )
            .expect("kv");

        let mut updated = install_input(&bytes);
        updated.version = "0.1.1";
        updated.now_unix_ms = 1_700_000_000_300;
        let summary = databases
            .state()
            .install_strand_package(updated)
            .expect("reinstall");
        assert_eq!(summary.version, "0.1.1");
        assert!(!summary.enabled);
        let stored = databases
            .state()
            .strand_kv_get("b893d568-327d-4b6e-b0b6-0b7a58e0c852", "token")
            .expect("kv kept")
            .expect("value");
        assert_eq!(stored.value, "keep-me");
    }
}
