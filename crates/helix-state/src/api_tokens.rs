use super::{StateDatabase, StateError, apply_migration, security::append_audit};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn migrate(connection: &mut Connection) -> Result<(), StateError> {
    apply_migration(
        connection,
        10,
        "server-api-tokens",
        r#"
CREATE TABLE server_api_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    auth_version INTEGER NOT NULL,
    verifier BLOB NOT NULL UNIQUE CHECK(length(verifier) = 32),
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 80),
    servers TEXT NOT NULL,
    permissions TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK(expires_at > created_at),
    revoked_at INTEGER,
    last_used_at INTEGER
) STRICT;
CREATE INDEX server_api_tokens_user ON server_api_tokens(user_id);
CREATE TABLE server_api_jobs (
    token_id TEXT NOT NULL REFERENCES server_api_tokens(id) ON DELETE CASCADE,
    job_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(token_id,job_id)
) STRICT;
"#,
    )
}

pub struct NewApiToken {
    pub user_id: String,
    pub auth_version: i64,
    pub verifier: [u8; 32],
    pub name: String,
    pub servers: Vec<String>,
    pub permissions: Vec<String>,
    pub now: i64,
    pub expires_at: i64,
}

#[derive(Clone)]
pub struct ApiTokenRecord {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub servers: Vec<String>,
    pub permissions: Vec<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

fn record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApiTokenRecord> {
    Ok(ApiTokenRecord {
        id: row.get(0)?,
        user_id: row.get(1)?,
        name: row.get(2)?,
        servers: row
            .get::<_, String>(3)?
            .split('\n')
            .map(str::to_owned)
            .collect(),
        permissions: row
            .get::<_, String>(4)?
            .split('\n')
            .map(str::to_owned)
            .collect(),
        created_at: row.get(5)?,
        expires_at: row.get(6)?,
        revoked_at: row.get(7)?,
        last_used_at: row.get(8)?,
    })
}

const COLUMNS: &str = "t.id,t.user_id,t.name,t.servers,t.permissions,t.created_at,t.expires_at,t.revoked_at,t.last_used_at";

impl StateDatabase {
    pub fn api_token_jobs(&self, id: &str) -> Result<Vec<(String, String)>, StateError> {
        let connection = self.lock()?;
        let mut query=connection.prepare("SELECT job_id,server_id FROM server_api_jobs WHERE token_id=?1 ORDER BY created_at DESC LIMIT 256")?;
        Ok(query
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn track_api_token_job(
        &self,
        id: &str,
        job: &str,
        server: &str,
        now: i64,
    ) -> Result<(), StateError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO server_api_jobs VALUES(?1,?2,?3,?4)",
            params![id, job, server, now],
        )?;
        tx.execute("DELETE FROM server_api_jobs WHERE token_id=?1 AND job_id NOT IN (SELECT job_id FROM server_api_jobs WHERE token_id=?1 ORDER BY created_at DESC LIMIT 256)",[id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn create_api_token(&self, input: NewApiToken) -> Result<String, StateError> {
        let valid_values = |values: &[String]| {
            !values.is_empty()
                && values.len() <= 64
                && values.iter().all(|s| {
                    !s.is_empty()
                        && s.len() <= 128
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
                })
        };
        if input.name.trim().is_empty()
            || input.name.len() > 80
            || input.name.chars().any(char::is_control)
            || !valid_values(&input.servers)
            || !valid_values(&input.permissions)
            || input.now < 0
            || input.expires_at <= input.now
            || input.expires_at.saturating_sub(input.now) > 90 * 86_400_000
        {
            return Err(StateError::InvalidSecurityInput(
                "invalid API token scope or expiration",
            ));
        }
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND auth_version=?2 AND status='active')", params![input.user_id,input.auth_version], |r| r.get(0))?;
        tx.execute(
            "DELETE FROM server_api_tokens WHERE revoked_at IS NOT NULL OR expires_at<=?1",
            [input.now],
        )?;
        let count: i64 =
            tx.query_row("SELECT COUNT(*) FROM server_api_tokens", [], |r| r.get(0))?;
        if !current || count >= 256 {
            return Err(StateError::InvalidSecurityInput(
                "account changed or API token limit reached",
            ));
        }
        let id = super::random_uuid_v4()?.to_string();
        tx.execute("INSERT INTO server_api_tokens(id,user_id,auth_version,verifier,name,servers,permissions,created_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![id,input.user_id,input.auth_version,input.verifier.as_slice(),input.name,input.servers.join("\n"),input.permissions.join("\n"),input.now,input.expires_at])?;
        append_audit(
            &tx,
            input.now,
            Some(&input.user_id),
            "api_token.created",
            Some("api_token"),
            Some(&id),
            "success",
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn list_api_tokens(&self, user_id: &str) -> Result<Vec<ApiTokenRecord>, StateError> {
        let connection = self.lock()?;
        let mut query = connection.prepare(&format!("SELECT {COLUMNS} FROM server_api_tokens t WHERE user_id=?1 ORDER BY created_at DESC LIMIT 256"))?;
        Ok(query
            .query_map([user_id], record)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn revoke_api_token(&self, user_id: &str, id: &str, now: i64) -> Result<bool, StateError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        let changed = tx.execute("UPDATE server_api_tokens SET revoked_at=COALESCE(revoked_at,?3) WHERE user_id=?1 AND id=?2",params![user_id,id,now])? > 0;
        if changed {
            append_audit(
                &tx,
                now,
                Some(user_id),
                "api_token.revoked",
                Some("api_token"),
                Some(id),
                "success",
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn authenticate_api_token(
        &self,
        verifier: &[u8; 32],
        now: i64,
    ) -> Result<Option<ApiTokenRecord>, StateError> {
        let connection = self.lock()?;
        Ok(connection.query_row(&format!("SELECT {COLUMNS} FROM server_api_tokens t JOIN users u ON u.id=t.user_id WHERE verifier=?1 AND revoked_at IS NULL AND expires_at>?2 AND u.status='active' AND u.auth_version=t.auth_version"),params![verifier.as_slice(),now],record).optional()?)
    }

    pub fn audit_api_token(
        &self,
        token: &ApiTokenRecord,
        permission: &str,
        server: &str,
        outcome: &str,
        now: i64,
    ) -> Result<(), StateError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        tx.execute("UPDATE server_api_tokens SET last_used_at=?2 WHERE id=?1 AND (last_used_at IS NULL OR last_used_at < ?2-60000)",params![token.id,now])?;
        let detail: String =
            tx.query_row("SELECT json_object('token_id',?1)", [&token.id], |r| {
                r.get(0)
            })?;
        super::security::append_audit_detail(
            &tx,
            now,
            Some(&token.user_id),
            &format!("api_token.{permission}"),
            (Some("server"), Some(server), outcome, &detail),
        )?;
        tx.commit()?;
        Ok(())
    }
}
