use crate::{ApiError, ApiState, auth, broker_value};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use helix_auth::{OpaqueToken, TokenDomain};
use helix_privd::{BrokerRequest, ServerFileRequest};
use helix_state::{ApiTokenRecord, NewApiToken};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const PERMISSIONS: &[&str] = &[
    "view",
    "logs",
    "files.read",
    "files.write",
    "start",
    "stop",
    "restart",
    "kill",
    "console",
    "settings",
    "update",
    "backups.read",
    "backups.write",
];

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/auth/server-tokens", get(list).post(create))
        .route("/auth/server-tokens/{id}", delete(revoke))
        .route("/automation/jobs", get(jobs))
        .route(
            "/automation/server",
            post(execute).layer(tower::limit::ConcurrencyLimitLayer::new(2)),
        )
        .layer(DefaultBodyLimit::max(helix_privd::MAX_REQUEST_BYTES))
}

fn now() -> i64 {
    helix_core::unix_timestamp_ms()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn metadata(t: ApiTokenRecord) -> Value {
    json!({"id":t.id,"name":t.name,"servers":t.servers,"permissions":t.permissions,"created_at":t.created_at,"expires_at":t.expires_at,"revoked_at":t.revoked_at,"last_used_at":t.last_used_at})
}

fn response(value: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}

async fn list(State(state): State<ApiState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let owner = auth::require_capability(&state, &headers, "system.settings.write").await?;
    let db = Arc::clone(&state.databases);
    let tokens = auth::run_blocking_state(&state.blocking_tasks, move || {
        db.state().list_api_tokens(&owner.user_id)
    })
    .await?;
    Ok(response(
        json!({"tokens":tokens.into_iter().map(metadata).collect::<Vec<_>>(),"permissions":PERMISSIONS}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateToken {
    name: String,
    servers: Vec<String>,
    permissions: Vec<String>,
    expires_in_days: u16,
}

async fn create(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<CreateToken>, JsonRejection>,
) -> Result<Response, ApiError> {
    auth::validate_post_headers(&headers)?;
    let owner = auth::require_capability(&state, &headers, "system.settings.write").await?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    auth::require_capability(&state, &headers, "games.view").await?;
    let Json(mut body) = body.map_err(auth::map_json_rejection)?;
    if body.permissions.iter().any(|p| p.starts_with("backups.")) {
        auth::require_capability(&state, &headers, "games.backups.manage").await?;
    }
    if body.name.trim().is_empty()
        || body.name.len() > 80
        || body.name.chars().any(char::is_control)
        || body.servers.is_empty()
        || body.servers.len() > 64
        || body.permissions.is_empty()
        || body.permissions.len() > PERMISSIONS.len()
        || !(1..=90).contains(&body.expires_in_days)
        || body
            .permissions
            .iter()
            .any(|p| !PERMISSIONS.contains(&p.as_str()))
        || body.servers.iter().any(|s| {
            s.is_empty()
                || s.len() > 128
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        })
    {
        return Err(ApiError::AuthorizationDenied);
    }
    body.servers.sort();
    body.servers.dedup();
    body.permissions.sort();
    body.permissions.dedup();
    // Resolve every ID through the existing manager before issuing a credential.
    for instance_id in &body.servers {
        broker_value(
            &state,
            BrokerRequest::ServerCapabilities {
                instance_id: instance_id.clone(),
            },
        )
        .await?;
    }
    let token = OpaqueToken::generate().map_err(|_| ApiError::ServiceUnavailable)?;
    let verifier = *token.verification_hash(TokenDomain::ServerApi).as_bytes();
    let expires_at = now().saturating_add(i64::from(body.expires_in_days) * 86_400_000);
    let db = Arc::clone(&state.databases);
    let id = auth::run_blocking_state(&state.blocking_tasks, move || {
        db.state().create_api_token(NewApiToken {
            user_id: owner.user_id,
            auth_version: owner.auth_version,
            verifier,
            name: body.name,
            servers: body.servers,
            permissions: body.permissions,
            now: now(),
            expires_at,
        })
    })
    .await?;
    Ok(response(
        json!({"id":id,"token":token.encode().expose_secret(),"expires_at":expires_at}),
    ))
}

async fn revoke(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    auth::validate_post_headers(&headers)?;
    let owner = auth::require_capability(&state, &headers, "system.settings.write").await?;
    let db = Arc::clone(&state.databases);
    let changed = auth::run_blocking_state(&state.blocking_tasks, move || {
        db.state().revoke_api_token(&owner.user_id, &id, now())
    })
    .await?;
    if !changed {
        return Err(ApiError::NotFound);
    }
    Ok(response(json!({"revoked":true})))
}

fn bearer(headers: &HeaderMap) -> Result<OpaqueToken, ApiError> {
    // Never fall back to a browser session or accept ambiguous credentials.
    if headers.contains_key(header::COOKIE) || headers.contains_key(header::ORIGIN) {
        return Err(ApiError::AuthorizationDenied);
    }
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::AuthenticationRequired)?;
    if values.next().is_some() {
        return Err(ApiError::AuthenticationRequired);
    }
    let (scheme, encoded) = value
        .split_once(' ')
        .ok_or(ApiError::AuthenticationRequired)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(ApiError::AuthenticationRequired);
    }
    OpaqueToken::from_encoded(encoded).map_err(|_| ApiError::AuthenticationRequired)
}

fn scope(request: &BrokerRequest) -> Option<(&str, &'static str)> {
    use BrokerRequest::*;
    Some(match request {
        ServerCapabilities { instance_id } | ServerDetail { instance_id } => (instance_id, "view"),
        ServerLogs { instance_id, .. } | ServerLogHistory { instance_id, .. } => {
            (instance_id, "logs")
        }
        ServerFiles {
            instance_id,
            request,
        } => (
            instance_id,
            match request {
                ServerFileRequest::List { .. }
                | ServerFileRequest::Stat { .. }
                | ServerFileRequest::Read { .. }
                | ServerFileRequest::Download { .. } => "files.read",
                _ => "files.write",
            },
        ),
        ServerAction {
            instance_id,
            action,
        } => (
            instance_id,
            match action {
                helix_privd::ServerAction::Start => "start",
                helix_privd::ServerAction::Stop => "stop",
                helix_privd::ServerAction::Restart => "restart",
                helix_privd::ServerAction::Kill => "kill",
                helix_privd::ServerAction::Update => "update",
                helix_privd::ServerAction::Backup => "backups.write",
            },
        ),
        ServerConsole { instance_id, .. } => (instance_id, "console"),
        ServerSettings { instance_id } => (instance_id, "settings"),
        UpdateServerSettings { instance_id, .. }
        | SetNativeMemory { instance_id, .. }
        | SetNativeCpu { instance_id, .. }
        | SetNativeStartOnBoot { instance_id, .. }
        | SetNativeBrowserListing { instance_id, .. } => (instance_id, "settings"),
        ChangeNativeRuntime { instance_id, .. } => (instance_id, "update"),
        ListBackups { instance_id } | ServerBackupDownload { instance_id, .. } => {
            (instance_id, "backups.read")
        }
        RestoreBackup { instance_id, .. }
        | TrashBackup { instance_id, .. }
        | RestoreTrashedBackup { instance_id, .. }
        | SetBackupPolicy { instance_id, .. }
        | PruneBackups { instance_id } => (instance_id, "backups.write"),
        _ => return None,
    })
}

async fn authenticate_token(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<ApiTokenRecord, ApiError> {
    let verifier = *bearer(headers)?
        .verification_hash(TokenDomain::ServerApi)
        .as_bytes();
    let db = Arc::clone(&state.databases);
    let token = auth::run_blocking_state(&state.blocking_tasks, move || {
        db.state().authenticate_api_token(&verifier, now())
    })
    .await?
    .ok_or(ApiError::AuthenticationRequired)?;
    if !state.attempt_limiter.allow_api_token(&token.id) {
        return Err(ApiError::AttemptRateLimited);
    }
    Ok(token)
}

async fn jobs(State(state): State<ApiState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let token = authenticate_token(&state, &headers).await?;
    let db = Arc::clone(&state.databases);
    let jobs = auth::run_blocking_state(&state.blocking_tasks, move || {
        db.state().api_token_jobs(&token.id)
    })
    .await?;
    Ok(response(
        json!({"jobs":jobs.into_iter().map(|(id,server)| json!({"job_id":id,"server_id":server})).collect::<Vec<_>>()}),
    ))
}

async fn execute(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<BrokerRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let permit = Arc::clone(&state.server_token_workers)
        .try_acquire_owned()
        .map_err(|_| ApiError::ApplicationCapacityExhausted)?;
    let token = authenticate_token(&state, &headers).await?;
    let Json(request) = body.map_err(auth::map_json_rejection)?;
    let owned_job_server;
    let (server, permission) = if let BrokerRequest::JobStatus { job_id } = &request {
        let db = Arc::clone(&state.databases);
        let id = token.id.clone();
        let jobs = auth::run_blocking_state(&state.blocking_tasks, move || {
            db.state().api_token_jobs(&id)
        })
        .await?;
        let owned = jobs
            .into_iter()
            .find(|(id, _)| id == job_id)
            .map(|(_, s)| s);
        owned_job_server = match owned {
            Some(server) => server,
            None => return denied_operation(&state, token).await,
        };
        (owned_job_server.as_str(), "jobs")
    } else {
        match scope(&request) {
            Some(scope) => scope,
            None => return denied_operation(&state, token).await,
        }
    };
    if server.is_empty()
        || server.len() > 128
        || !server
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
    {
        return denied_operation(&state, token).await;
    }
    let allowed = token.servers.iter().any(|s| s == server)
        && (permission == "jobs" || token.permissions.iter().any(|p| p == permission));
    let db = Arc::clone(&state.databases);
    let audit_token = token.clone();
    let audit_server = server.to_owned();
    auth::run_blocking_state(&state.blocking_tasks, move || {
        db.state().audit_api_token(
            &audit_token,
            permission,
            &audit_server,
            if allowed { "success" } else { "denied" },
            now(),
        )
    })
    .await?;
    if !allowed {
        return Err(ApiError::AuthorizationDenied);
    }
    let server = server.to_owned();
    // Once authorized, finish audit/job registration even if the client disconnects.
    let guard = state.blocking_tasks.start();
    tokio::spawn(async move {
        let _guard = guard;
        let _permit = permit;
        complete_request(state, token, request, server).await
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable)?
}

async fn denied_operation(state: &ApiState, token: ApiTokenRecord) -> Result<Response, ApiError> {
    let db = Arc::clone(&state.databases);
    auth::run_blocking_state(&state.blocking_tasks, move || {
        db.state()
            .audit_api_token(&token, "unsupported", "unscoped", "denied", now())
    })
    .await?;
    Err(ApiError::AuthorizationDenied)
}

async fn complete_request(
    state: ApiState,
    token: ApiTokenRecord,
    request: BrokerRequest,
    server: String,
) -> Result<Response, ApiError> {
    let result = broker_value(&state, request).await;
    let db = Arc::clone(&state.databases);
    let audit_token = token.clone();
    let audit_server = server.clone();
    let outcome = if result.is_ok() { "success" } else { "error" };
    auth::run_blocking_state(&state.blocking_tasks, move || {
        db.state()
            .audit_api_token(&audit_token, "result", &audit_server, outcome, now())
    })
    .await?;
    let value = result?;
    if let Some(job) = value.get("job_id").and_then(Value::as_str) {
        let db = Arc::clone(&state.databases);
        let job = job.to_owned();
        auth::run_blocking_state(&state.blocking_tasks, move || {
            db.state()
                .track_api_token_job(&token.id, &job, &server, now())
        })
        .await?;
    }
    Ok(response(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_privd::ServerAction;
    #[test]
    fn host_and_global_controls_are_not_delegated() {
        for request in [
            BrokerRequest::HostInventory {},
            BrokerRequest::ListServers {},
            BrokerRequest::CheckHelixUpdate {},
            BrokerRequest::JobStatus {
                job_id: "other-job".into(),
            },
        ] {
            assert!(scope(&request).is_none());
        }
    }
    #[test]
    fn lifecycle_permissions_are_independent() {
        for (action, permission) in [
            (ServerAction::Start, "start"),
            (ServerAction::Stop, "stop"),
            (ServerAction::Kill, "kill"),
            (ServerAction::Restart, "restart"),
        ] {
            let request = BrokerRequest::ServerAction {
                instance_id: "helix:one".into(),
                action,
            };
            assert_eq!(scope(&request), Some(("helix:one", permission)));
        }
    }
    #[test]
    fn ambiguous_and_browser_credentials_are_rejected() {
        let token = OpaqueToken::generate().unwrap().encode();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", token.expose_secret()).parse().unwrap(),
        );
        assert!(bearer(&headers).is_ok());
        headers.insert(header::COOKIE, "theme=dark".parse().unwrap());
        assert!(bearer(&headers).is_err());
        headers.remove(header::COOKIE);
        headers.append(header::AUTHORIZATION, "Bearer invalid".parse().unwrap());
        assert!(bearer(&headers).is_err());
    }
}
