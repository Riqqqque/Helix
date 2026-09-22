use super::{StateDatabase, StateError, apply_migration};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

pub const MAX_MACHINES: i64 = 64;
pub const MAX_MACHINE_LABEL_CHARS: usize = 64;
pub const MAX_MACHINE_HOST_CHARS: usize = 253;
pub const MAX_MACHINE_USERNAME_CHARS: usize = 64;
pub const MAX_MACHINE_NOTES_CHARS: usize = 512;
pub const MAX_MACHINE_PROBE_BYTES: i64 = 8 * 1024;

const MACHINE_MIGRATION_11: &str = r#"
CREATE TABLE machines (
    id TEXT PRIMARY KEY CHECK (
        length(id) = 36
        AND id NOT GLOB '*[^0-9a-f-]*'
    ),
    label TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 64),
    host TEXT NOT NULL CHECK (length(host) BETWEEN 1 AND 253),
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    username TEXT NOT NULL CHECK (length(username) BETWEEN 1 AND 64),
    auth_kind TEXT NOT NULL CHECK (auth_kind IN ('key', 'password', 'system')),
    notes TEXT NOT NULL CHECK (length(notes) <= 512),
    wol_mac TEXT CHECK (
        wol_mac IS NULL
        OR (length(wol_mac) = 17 AND wol_mac NOT GLOB '*[^0-9a-f:]*')
    ),
    probe_json TEXT CHECK (
        probe_json IS NULL
        OR (json_valid(probe_json) AND length(CAST(probe_json AS BLOB)) <= 8192)
    ),
    probed_at_unix_ms INTEGER,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0)
) STRICT;

INSERT INTO capabilities (capability, description, created_at_unix_ms) VALUES
    ('machines.view', 'Inspect registered rack machines', 0),
    ('machines.manage', 'Register, wake, and control rack machines', 0);

INSERT INTO role_capabilities (role_id, capability, granted_at_unix_ms)
SELECT '00000000-0000-0000-0000-000000000001', capability, 0
FROM capabilities
WHERE capability IN ('machines.view', 'machines.manage');
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineAuth {
    /// Helix's own hub keypair, provisioned onto the target's authorized_keys.
    HubKey,
    /// Interactive password prompt inside the SSH terminal; nothing is stored.
    Password,
    /// The host account's own ~/.ssh configuration and keys.
    System,
}

impl MachineAuth {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HubKey => "key",
            Self::Password => "password",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Result<Self, StateError> {
        match value {
            "key" => Ok(Self::HubKey),
            "password" => Ok(Self::Password),
            "system" => Ok(Self::System),
            _ => Err(StateError::InvalidMachineInput("unknown machine auth kind")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineRecord {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_kind: MachineAuth,
    pub notes: String,
    pub wol_mac: Option<String>,
    pub probe_json: Option<String>,
    pub probed_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

pub struct MachineInput<'a> {
    pub label: &'a str,
    pub host: &'a str,
    pub port: i64,
    pub username: &'a str,
    pub auth_kind: MachineAuth,
    pub notes: &'a str,
    pub wol_mac: Option<&'a str>,
}

pub(super) fn migrate_machines(connection: &mut rusqlite::Connection) -> Result<(), StateError> {
    apply_migration(connection, 11, "rack-machines", MACHINE_MIGRATION_11)
}

impl StateDatabase {
    pub fn list_machines(&self) -> Result<Vec<MachineRecord>, StateError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, label, host, port, username, auth_kind, notes, wol_mac,
                    probe_json, probed_at_unix_ms, created_at_unix_ms, updated_at_unix_ms
             FROM machines ORDER BY label COLLATE NOCASE LIMIT 65",
        )?;
        let records = statement
            .query_map([], row_to_machine)?
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() > usize::try_from(MAX_MACHINES).unwrap_or(usize::MAX) {
            return Err(StateError::Integrity {
                database: "helix-state.db",
                details: vec!["machine row limit was exceeded".to_owned()],
            });
        }
        Ok(records)
    }

    pub fn machine(&self, id: &str) -> Result<Option<MachineRecord>, StateError> {
        require_machine_id(id)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, label, host, port, username, auth_kind, notes, wol_mac,
                        probe_json, probed_at_unix_ms, created_at_unix_ms, updated_at_unix_ms
                 FROM machines WHERE id = ?1",
                [id],
                row_to_machine,
            )
            .optional()
            .map_err(StateError::from)
    }

    pub fn create_machine(
        &self,
        id: &str,
        input: MachineInput<'_>,
        now_unix_ms: i64,
    ) -> Result<MachineRecord, StateError> {
        require_machine_id(id)?;
        let wol_mac = validate_machine_input(&input)?;
        if now_unix_ms < 0 {
            return Err(StateError::InvalidMachineInput("timestamp is invalid"));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM machines", [], |row| row.get(0))?;
        if count >= MAX_MACHINES {
            return Err(StateError::MachineQuotaExceeded);
        }
        transaction.execute(
            "INSERT INTO machines (
                id, label, host, port, username, auth_kind, notes, wol_mac,
                probe_json, probed_at_unix_ms, created_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?9)",
            params![
                id,
                input.label,
                input.host,
                input.port,
                input.username,
                input.auth_kind.as_str(),
                input.notes,
                wol_mac.as_deref(),
                now_unix_ms,
            ],
        )?;
        let record = transaction.query_row(
            "SELECT id, label, host, port, username, auth_kind, notes, wol_mac,
                    probe_json, probed_at_unix_ms, created_at_unix_ms, updated_at_unix_ms
             FROM machines WHERE id = ?1",
            [id],
            row_to_machine,
        )?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn update_machine(
        &self,
        id: &str,
        input: MachineInput<'_>,
        now_unix_ms: i64,
    ) -> Result<Option<MachineRecord>, StateError> {
        require_machine_id(id)?;
        let wol_mac = validate_machine_input(&input)?;
        if now_unix_ms < 0 {
            return Err(StateError::InvalidMachineInput("timestamp is invalid"));
        }
        let connection = self.lock()?;
        let changed = connection.execute(
            "UPDATE machines SET label = ?1, host = ?2, port = ?3, username = ?4,
                    auth_kind = ?5, notes = ?6, wol_mac = ?7, updated_at_unix_ms = ?8
             WHERE id = ?9",
            params![
                input.label,
                input.host,
                input.port,
                input.username,
                input.auth_kind.as_str(),
                input.notes,
                wol_mac.as_deref(),
                now_unix_ms,
                id,
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        connection
            .query_row(
                "SELECT id, label, host, port, username, auth_kind, notes, wol_mac,
                        probe_json, probed_at_unix_ms, created_at_unix_ms, updated_at_unix_ms
                 FROM machines WHERE id = ?1",
                [id],
                row_to_machine,
            )
            .optional()
            .map_err(StateError::from)
    }

    pub fn delete_machine(&self, id: &str) -> Result<bool, StateError> {
        require_machine_id(id)?;
        let connection = self.lock()?;
        let deleted = connection.execute("DELETE FROM machines WHERE id = ?1", [id])?;
        Ok(deleted > 0)
    }

    /// Persist the latest probe snapshot so the machine list stays instant
    /// across dashboard reloads. The JSON is produced by the broker probe.
    pub fn set_machine_probe(
        &self,
        id: &str,
        probe_json: Option<&str>,
        probed_at_unix_ms: i64,
    ) -> Result<bool, StateError> {
        require_machine_id(id)?;
        if probed_at_unix_ms < 0 {
            return Err(StateError::InvalidMachineInput("timestamp is invalid"));
        }
        if let Some(probe) = probe_json
            && (probe.len() as i64 > MAX_MACHINE_PROBE_BYTES
                || !serde_json::from_str::<serde_json::Value>(probe)
                    .is_ok_and(|value| value.is_object()))
        {
            return Err(StateError::InvalidMachineInput(
                "probe snapshot must be a small JSON object",
            ));
        }
        let connection = self.lock()?;
        let changed = connection.execute(
            "UPDATE machines SET probe_json = ?1, probed_at_unix_ms = ?2 WHERE id = ?3",
            params![probe_json, probed_at_unix_ms, id],
        )?;
        Ok(changed > 0)
    }
}

fn row_to_machine(row: &rusqlite::Row<'_>) -> rusqlite::Result<MachineRecord> {
    Ok(MachineRecord {
        id: row.get(0)?,
        label: row.get(1)?,
        host: row.get(2)?,
        port: row.get(3)?,
        username: row.get(4)?,
        auth_kind: MachineAuth::parse(&row.get::<_, String>(5)?).map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                5,
                "auth_kind".to_owned(),
                rusqlite::types::Type::Text,
            )
        })?,
        notes: row.get(6)?,
        wol_mac: row.get(7)?,
        probe_json: row.get(8)?,
        probed_at_unix_ms: row.get(9)?,
        created_at_unix_ms: row.get(10)?,
        updated_at_unix_ms: row.get(11)?,
    })
}

fn require_machine_id(id: &str) -> Result<(), StateError> {
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
        Err(StateError::InvalidMachineInput("machine id must be a UUID"))
    }
}

/// Hosts are DNS names or IP literals; leading `-` or shell separators would
/// become option/injection vectors once they reach an ssh argv.
fn require_machine_host(host: &str) -> Result<(), StateError> {
    let valid = (1..=MAX_MACHINE_HOST_CHARS).contains(&host.len())
        && host.is_ascii()
        && host.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        })
        && !host.starts_with('-')
        && !host.contains("..")
        && !host.starts_with('.')
        && !host.ends_with('.');
    if valid {
        Ok(())
    } else {
        Err(StateError::InvalidMachineInput(
            "machine host must be a DNS name or IP address",
        ))
    }
}

fn require_machine_username(username: &str) -> Result<(), StateError> {
    let valid = (1..=MAX_MACHINE_USERNAME_CHARS).contains(&username.len())
        && username.is_ascii()
        && !username.starts_with('-')
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(StateError::InvalidMachineInput(
            "machine username must be a short login name",
        ))
    }
}

/// Returns the normalized `aa:bb:cc:dd:ee:ff` form or a validation error.
fn normalize_wol_mac(value: Option<&str>) -> Result<Option<String>, StateError> {
    let Some(raw) = value else { return Ok(None) };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let compact: Vec<u8> = raw
        .bytes()
        .filter(|byte| *byte != b':' && *byte != b'-')
        .collect();
    if compact.len() != 12
        || !compact.iter().all(|byte| byte.is_ascii_hexdigit())
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b':' | b'-'))
    {
        return Err(StateError::InvalidMachineInput(
            "Wake-on-LAN address must be a MAC address",
        ));
    }
    let lower = String::from_utf8_lossy(&compact).to_lowercase();
    Ok(Some(
        lower
            .as_bytes()
            .chunks(2)
            .map(|pair| std::str::from_utf8(pair).unwrap_or("00").to_owned())
            .collect::<Vec<_>>()
            .join(":"),
    ))
}

fn validate_machine_input(input: &MachineInput<'_>) -> Result<Option<String>, StateError> {
    if input.label.is_empty()
        || input.label.chars().count() > MAX_MACHINE_LABEL_CHARS
        || input.label.chars().any(|character| character.is_control())
    {
        return Err(StateError::InvalidMachineInput(
            "machine label must be 1-64 printable characters",
        ));
    }
    require_machine_host(input.host)?;
    if !(1..=65535).contains(&input.port) {
        return Err(StateError::InvalidMachineInput(
            "machine port must be between 1 and 65535",
        ));
    }
    require_machine_username(input.username)?;
    if input.notes.chars().count() > MAX_MACHINE_NOTES_CHARS
        || input
            .notes
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(StateError::InvalidMachineInput(
            "machine notes exceed the supported length",
        ));
    }
    normalize_wol_mac(input.wol_mac)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatabaseSet;

    fn machine_input<'a>() -> MachineInput<'a> {
        MachineInput {
            label: "nas-1",
            host: "192.0.2.10",
            port: 22,
            username: "operator",
            auth_kind: MachineAuth::HubKey,
            notes: "storage node",
            wol_mac: Some("AA-BB-CC-DD-EE-FF"),
        }
    }

    #[test]
    fn create_update_probe_and_delete_round_trip() {
        let temp = crate::private_test_directory("machine state");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open");
        let created = databases
            .state()
            .create_machine(
                "b893d568-327d-4b6e-b0b6-0b7a58e0c852",
                machine_input(),
                1_700_000_000_000,
            )
            .expect("create");
        assert_eq!(created.wol_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(created.auth_kind, MachineAuth::HubKey);

        let mut renamed = machine_input();
        renamed.label = "nas-primary";
        let updated = databases
            .state()
            .update_machine(
                "b893d568-327d-4b6e-b0b6-0b7a58e0c852",
                renamed,
                1_700_000_000_100,
            )
            .expect("update")
            .expect("present");
        assert_eq!(updated.label, "nas-primary");

        assert!(
            databases
                .state()
                .set_machine_probe(
                    "b893d568-327d-4b6e-b0b6-0b7a58e0c852",
                    Some("{\"reachable\":true}"),
                    1_700_000_000_200,
                )
                .expect("probe")
        );
        let probed = databases
            .state()
            .machine("b893d568-327d-4b6e-b0b6-0b7a58e0c852")
            .expect("read")
            .expect("present");
        assert_eq!(probed.probe_json.as_deref(), Some("{\"reachable\":true}"));
        assert_eq!(probed.probed_at_unix_ms, Some(1_700_000_000_200));

        assert!(
            databases
                .state()
                .delete_machine("b893d568-327d-4b6e-b0b6-0b7a58e0c852")
                .expect("delete")
        );
        assert!(databases.state().list_machines().expect("list").is_empty());
    }

    #[test]
    fn validation_rejects_injection_and_bad_shapes() {
        let temp = crate::private_test_directory("machine validation");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open");
        let id = "b893d568-327d-4b6e-b0b6-0b7a58e0c852";
        for mutate in [
            |input: &mut MachineInput<'_>| input.host = "-oProxyCommand=id",
            |input: &mut MachineInput<'_>| input.host = "bad host",
            |input: &mut MachineInput<'_>| input.host = "a..b",
            |input: &mut MachineInput<'_>| input.port = 0,
            |input: &mut MachineInput<'_>| input.username = "-i",
            |input: &mut MachineInput<'_>| input.username = "root;rm",
            |input: &mut MachineInput<'_>| input.label = "bad\nlabel",
            |input: &mut MachineInput<'_>| input.wol_mac = Some("not-a-mac"),
        ] {
            let mut input = machine_input();
            mutate(&mut input);
            assert!(
                databases.state().create_machine(id, input, 1).is_err(),
                "mutation should be rejected"
            );
        }
        assert!(
            databases
                .state()
                .create_machine(id, machine_input(), 1)
                .is_ok()
        );
    }

    #[test]
    fn probe_snapshots_must_be_small_objects() {
        let temp = crate::private_test_directory("machine probe");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open");
        let id = "b893d568-327d-4b6e-b0b6-0b7a58e0c852";
        databases
            .state()
            .create_machine(id, machine_input(), 1)
            .expect("create");
        let oversized = format!("{{\"a\":\"{}\"}}", "y".repeat(9_000));
        for bad in ["[]", "\"x\"", oversized.as_str()] {
            assert!(
                databases
                    .state()
                    .set_machine_probe(id, Some(bad), 2)
                    .is_err()
            );
        }
        assert!(
            databases
                .state()
                .set_machine_probe(id, Some("{\"ok\":true}"), 2)
                .expect("probe")
        );
    }

    #[test]
    fn owner_role_gains_machine_capabilities() {
        let temp = crate::private_test_directory("machine capabilities");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open");
        let connection = databases.state().lock().expect("lock");
        for capability in ["machines.view", "machines.manage"] {
            let granted = connection
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM role_capabilities rc
                        JOIN roles r ON r.id = rc.role_id
                        WHERE r.name = 'owner' AND r.is_system = 1
                              AND rc.capability = ?1
                    )",
                    [capability],
                    |row| row.get::<_, bool>(0),
                )
                .expect("grant");
            assert!(granted, "owner must hold {capability}");
        }
    }
}
