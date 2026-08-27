use super::{StateDatabase, StateError, apply_migration};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

pub const MAX_SERVER_ICON_BYTES: usize = 512 * 1024;
const MAX_SERVER_APPEARANCES: i64 = 2_048;

const SERVER_APPEARANCE_MIGRATION_6: &str = r#"
CREATE TABLE server_appearances (
    server_id TEXT PRIMARY KEY CHECK (
        length(server_id) BETWEEN 7 AND 165
        AND server_id NOT GLOB '*[^A-Za-z0-9:._-]*'
    ),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    preset TEXT CHECK (preset IN (
        'grass', 'portal', 'crystal', 'fortress', 'ember', 'ocean'
    )),
    content_type TEXT CHECK (content_type IN ('image/png', 'image/jpeg')),
    image_bytes BLOB,
    width INTEGER CHECK (width BETWEEN 32 AND 2048),
    height INTEGER CHECK (height BETWEEN 32 AND 2048),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0),
    CHECK (
        (preset IS NOT NULL AND content_type IS NULL AND image_bytes IS NULL
            AND width IS NULL AND height IS NULL)
        OR
        (preset IS NULL AND content_type IS NOT NULL AND image_bytes IS NOT NULL
            AND typeof(image_bytes) = 'blob'
            AND length(image_bytes) BETWEEN 32 AND 524288
            AND width IS NOT NULL AND height IS NOT NULL)
    )
) STRICT;
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerIconPreset {
    Grass,
    Portal,
    Crystal,
    Fortress,
    Ember,
    Ocean,
}

impl ServerIconPreset {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Grass => "grass",
            Self::Portal => "portal",
            Self::Crystal => "crystal",
            Self::Fortress => "fortress",
            Self::Ember => "ember",
            Self::Ocean => "ocean",
        }
    }

    fn parse(value: &str) -> Result<Self, StateError> {
        match value {
            "grass" => Ok(Self::Grass),
            "portal" => Ok(Self::Portal),
            "crystal" => Ok(Self::Crystal),
            "fortress" => Ok(Self::Fortress),
            "ember" => Ok(Self::Ember),
            "ocean" => Ok(Self::Ocean),
            _ => Err(StateError::InvalidServerAppearanceInput(
                "unknown server icon preset",
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerAppearanceSummary {
    pub server_id: String,
    pub revision: i64,
    pub preset: Option<ServerIconPreset>,
    pub content_type: Option<String>,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerAppearanceRecord {
    pub summary: ServerAppearanceSummary,
    pub image_bytes: Option<Vec<u8>>,
}

pub enum ServerAppearanceUpdate<'a> {
    Preset(ServerIconPreset),
    Custom {
        content_type: &'a str,
        image_bytes: &'a [u8],
        width: u16,
        height: u16,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerAppearanceUpdateOutcome {
    Updated(Option<ServerAppearanceSummary>),
    Conflict(Option<ServerAppearanceSummary>),
}

pub(super) fn migrate_server_appearances(
    connection: &mut rusqlite::Connection,
) -> Result<(), StateError> {
    apply_migration(
        connection,
        6,
        "server-appearance-customization",
        SERVER_APPEARANCE_MIGRATION_6,
    )
}

impl StateDatabase {
    pub fn server_appearance(
        &self,
        server_id: &str,
    ) -> Result<Option<ServerAppearanceRecord>, StateError> {
        require_server_id(server_id)?;
        let connection = self.lock()?;
        load_server_appearance(&connection, server_id, true)
    }

    pub fn server_appearance_summaries(&self) -> Result<Vec<ServerAppearanceSummary>, StateError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT server_id, revision, preset, content_type, width, height,
                    updated_at_unix_ms
             FROM server_appearances ORDER BY server_id LIMIT 2049",
        )?;
        let records = statement
            .query_map([], row_to_summary)?
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() > usize::try_from(MAX_SERVER_APPEARANCES).unwrap_or(usize::MAX) {
            return Err(StateError::Integrity {
                database: "helix-state.db",
                details: vec!["server appearance row limit was exceeded".to_owned()],
            });
        }
        Ok(records)
    }

    pub fn update_server_appearance(
        &self,
        server_id: &str,
        expected_revision: i64,
        update: ServerAppearanceUpdate<'_>,
        now_unix_ms: i64,
    ) -> Result<ServerAppearanceUpdateOutcome, StateError> {
        require_server_id(server_id)?;
        require_revision_and_time(expected_revision, now_unix_ms)?;
        validate_update(&update)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_server_appearance(&transaction, server_id, false)?;
        if current.as_ref().map_or(0, |record| record.summary.revision) != expected_revision {
            transaction.commit()?;
            return Ok(ServerAppearanceUpdateOutcome::Conflict(
                current.map(|record| record.summary),
            ));
        }
        if current.is_none() {
            enforce_row_limit(&transaction)?;
        }
        let revision =
            expected_revision
                .checked_add(1)
                .ok_or(StateError::InvalidServerAppearanceInput(
                    "server icon revision overflowed",
                ))?;
        let (preset, content_type, image_bytes, width, height) = match update {
            ServerAppearanceUpdate::Preset(preset) => {
                (Some(preset.as_str()), None, None, None, None)
            }
            ServerAppearanceUpdate::Custom {
                content_type,
                image_bytes,
                width,
                height,
            } => (
                None,
                Some(content_type),
                Some(image_bytes),
                Some(i64::from(width)),
                Some(i64::from(height)),
            ),
        };
        transaction.execute(
            "INSERT INTO server_appearances (
                server_id, revision, preset, content_type, image_bytes, width,
                height, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(server_id) DO UPDATE SET
                revision = excluded.revision,
                preset = excluded.preset,
                content_type = excluded.content_type,
                image_bytes = excluded.image_bytes,
                width = excluded.width,
                height = excluded.height,
                updated_at_unix_ms = max(
                    server_appearances.updated_at_unix_ms,
                    excluded.updated_at_unix_ms
                )",
            params![
                server_id,
                revision,
                preset,
                content_type,
                image_bytes,
                width,
                height,
                now_unix_ms,
            ],
        )?;
        let updated = load_server_appearance(&transaction, server_id, false)?.ok_or(
            StateError::InvalidServerAppearanceInput("saved server icon could not be read back"),
        )?;
        transaction.commit()?;
        Ok(ServerAppearanceUpdateOutcome::Updated(Some(
            updated.summary,
        )))
    }

    pub fn clear_server_appearance(
        &self,
        server_id: &str,
        expected_revision: i64,
    ) -> Result<ServerAppearanceUpdateOutcome, StateError> {
        require_server_id(server_id)?;
        if expected_revision < 0 {
            return Err(StateError::InvalidServerAppearanceInput(
                "server icon revision cannot be negative",
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_server_appearance(&transaction, server_id, false)?;
        if current.as_ref().map_or(0, |record| record.summary.revision) != expected_revision {
            transaction.commit()?;
            return Ok(ServerAppearanceUpdateOutcome::Conflict(
                current.map(|record| record.summary),
            ));
        }
        if current.is_some() {
            transaction.execute(
                "DELETE FROM server_appearances WHERE server_id = ?1",
                [server_id],
            )?;
        }
        transaction.commit()?;
        Ok(ServerAppearanceUpdateOutcome::Updated(None))
    }
}

fn require_server_id(server_id: &str) -> Result<(), StateError> {
    let valid_prefix = server_id.starts_with("helix:") || server_id.starts_with("amp:");
    if !valid_prefix
        || !(7..=165).contains(&server_id.len())
        || !server_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-'))
    {
        return Err(StateError::InvalidServerAppearanceInput(
            "server identity is invalid",
        ));
    }
    Ok(())
}

fn require_revision_and_time(revision: i64, now_unix_ms: i64) -> Result<(), StateError> {
    if revision < 0 {
        return Err(StateError::InvalidServerAppearanceInput(
            "server icon revision cannot be negative",
        ));
    }
    if now_unix_ms < 0 {
        return Err(StateError::InvalidServerAppearanceInput(
            "server icon timestamp cannot be negative",
        ));
    }
    Ok(())
}

fn validate_update(update: &ServerAppearanceUpdate<'_>) -> Result<(), StateError> {
    if let ServerAppearanceUpdate::Custom {
        content_type,
        image_bytes,
        width,
        height,
    } = update
    {
        if !matches!(*content_type, "image/png" | "image/jpeg") {
            return Err(StateError::InvalidServerAppearanceInput(
                "server icon media type is invalid",
            ));
        }
        if !(32..=MAX_SERVER_ICON_BYTES).contains(&image_bytes.len()) {
            return Err(StateError::InvalidServerAppearanceInput(
                "server icon size is invalid",
            ));
        }
        if !(32..=2_048).contains(width) || !(32..=2_048).contains(height) {
            return Err(StateError::InvalidServerAppearanceInput(
                "server icon dimensions are invalid",
            ));
        }
    }
    Ok(())
}

fn enforce_row_limit(transaction: &Transaction<'_>) -> Result<(), StateError> {
    let count = transaction.query_row("SELECT count(*) FROM server_appearances", [], |row| {
        row.get::<_, i64>(0)
    })?;
    if count >= MAX_SERVER_APPEARANCES {
        return Err(StateError::InvalidServerAppearanceInput(
            "server icon limit was reached",
        ));
    }
    Ok(())
}

fn load_server_appearance(
    connection: &rusqlite::Connection,
    server_id: &str,
    include_image: bool,
) -> Result<Option<ServerAppearanceRecord>, StateError> {
    connection
        .query_row(
            "SELECT server_id, revision, preset, content_type,
                    width, height, updated_at_unix_ms,
                    CASE WHEN ?2 THEN image_bytes ELSE NULL END
             FROM server_appearances WHERE server_id = ?1",
            params![server_id, include_image],
            |row| {
                let summary = row_to_summary(row)?;
                Ok(ServerAppearanceRecord {
                    summary,
                    image_bytes: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(StateError::from)
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ServerAppearanceSummary> {
    let preset = row
        .get::<_, Option<String>>(2)?
        .map(|value| {
            ServerIconPreset::parse(&value).map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    2,
                    "preset".to_owned(),
                    rusqlite::types::Type::Text,
                )
            })
        })
        .transpose()?;
    let width = row
        .get::<_, Option<i64>>(4)?
        .map(|value| {
            u16::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, value))
        })
        .transpose()?;
    let height = row
        .get::<_, Option<i64>>(5)?
        .map(|value| {
            u16::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, value))
        })
        .transpose()?;
    Ok(ServerAppearanceSummary {
        server_id: row.get(0)?,
        revision: row.get(1)?,
        preset,
        content_type: row.get(3)?,
        width,
        height,
        updated_at_unix_ms: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DatabaseSet, private_test_directory};

    #[test]
    fn custom_and_preset_icons_are_revisioned_backed_up_and_clearable() {
        let temporary = private_test_directory("server appearance state");
        let databases = DatabaseSet::open_for_daemon(temporary.path()).expect("open state");
        let state = databases.state();
        let server_id = "helix:2876a033-11d1-4035-9734-236bc7723792";

        let preset = state
            .update_server_appearance(
                server_id,
                0,
                ServerAppearanceUpdate::Preset(ServerIconPreset::Grass),
                1_800_000_000_000,
            )
            .expect("save preset");
        assert!(matches!(
            preset,
            ServerAppearanceUpdateOutcome::Updated(Some(ServerAppearanceSummary {
                revision: 1,
                preset: Some(ServerIconPreset::Grass),
                ..
            }))
        ));

        let png = [
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0,
            0, 0, 64, 0, 0, 0, 64, 8, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        let custom = state
            .update_server_appearance(
                server_id,
                1,
                ServerAppearanceUpdate::Custom {
                    content_type: "image/png",
                    image_bytes: &png,
                    width: 64,
                    height: 64,
                },
                1_800_000_000_001,
            )
            .expect("save custom icon");
        assert!(matches!(
            custom,
            ServerAppearanceUpdateOutcome::Updated(Some(ServerAppearanceSummary {
                revision: 2,
                preset: None,
                ..
            }))
        ));
        assert_eq!(
            state
                .server_appearance(server_id)
                .expect("read icon")
                .expect("icon exists")
                .image_bytes,
            Some(png.to_vec())
        );

        assert!(matches!(
            state
                .clear_server_appearance(server_id, 1)
                .expect("conflict"),
            ServerAppearanceUpdateOutcome::Conflict(Some(ServerAppearanceSummary {
                revision: 2,
                ..
            }))
        ));
        assert!(matches!(
            state
                .clear_server_appearance(server_id, 2)
                .expect("clear icon"),
            ServerAppearanceUpdateOutcome::Updated(None)
        ));
    }

    #[test]
    fn invalid_server_appearance_inputs_fail_before_storage() {
        let temporary = private_test_directory("invalid server appearance");
        let databases = DatabaseSet::open_for_daemon(temporary.path()).expect("open state");
        let state = databases.state();
        assert!(matches!(
            state.update_server_appearance(
                "helix:../../escape",
                0,
                ServerAppearanceUpdate::Preset(ServerIconPreset::Portal),
                1,
            ),
            Err(StateError::InvalidServerAppearanceInput(_))
        ));
        assert!(matches!(
            state.update_server_appearance(
                "helix:server",
                0,
                ServerAppearanceUpdate::Custom {
                    content_type: "image/svg+xml",
                    image_bytes: &[0; 64],
                    width: 64,
                    height: 64,
                },
                1,
            ),
            Err(StateError::InvalidServerAppearanceInput(_))
        ));
    }
}
