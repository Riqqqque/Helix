//! Rack-machine hub: registry CRUD, probes, Wake-on-LAN, power, and SSH
//! terminals that reuse the host-terminal ticket and bridge pipeline.

use crate::{
    ApiError, ApiState, auth, broker_value,
    terminal::{self, MACHINE_TICKET_COOKIE, TERMINAL_SUBPROTOCOL},
};
use axum::{
    Json, Router,
    extract::{
        ConnectInfo, Path as RoutePath, State, rejection::JsonRejection, ws::WebSocketUpgrade,
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use helix_privd::{BrokerRequest, MachineAuthKind, MachinePowerAction, MachineTarget};
use helix_state::{MachineAuth, MachineInput, MachineRecord, StateError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc};

const MACHINE_CONNECT_PATH_PREFIX: &str = "/api/v1/machines";

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/machines", get(list_machines).post(create_machine))
        .route("/machines/identity", get(hub_identity))
        .route("/machines/refresh", post(refresh_machines))
        .route(
            "/machines/{machine_id}",
            put(update_machine).delete(delete_machine),
        )
        .route("/machines/{machine_id}/probe", post(probe_machine))
        .route("/machines/{machine_id}/wake", post(wake_machine))
        .route("/machines/{machine_id}/power", post(machine_power))
        .route(
            "/machines/{machine_id}/terminal/ticket",
            post(issue_machine_ticket),
        )
        .route(
            "/machines/{machine_id}/terminal/connect",
            get(connect_machine_terminal),
        )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MachineView {
    id: String,
    label: String,
    host: String,
    port: i64,
    username: String,
    auth_kind: &'static str,
    notes: String,
    wol_mac: Option<String>,
    probe: Option<Value>,
    probed_at_unix_ms: Option<i64>,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MachineUpsertBody {
    label: String,
    host: String,
    port: u16,
    username: String,
    auth_kind: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    wol_mac: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MachinePowerBody {
    action: String,
    confirmation: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MachineTicketBody {
    current_password: auth::SecretString,
    columns: u16,
    rows: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MachineTicketResponse {
    expires_at_unix_ms: u64,
    connect_path: String,
    subprotocol: &'static str,
}

fn machine_view(record: &MachineRecord) -> Result<MachineView, ApiError> {
    let probe = match record.probe_json.as_deref() {
        Some(raw) => {
            Some(serde_json::from_str::<Value>(raw).map_err(|_| ApiError::ServiceUnavailable)?)
        }
        None => None,
    };
    Ok(MachineView {
        id: record.id.clone(),
        label: record.label.clone(),
        host: record.host.clone(),
        port: record.port,
        username: record.username.clone(),
        auth_kind: record.auth_kind.as_str(),
        notes: record.notes.clone(),
        wol_mac: record.wol_mac.clone(),
        probe,
        probed_at_unix_ms: record.probed_at_unix_ms,
        created_at_unix_ms: record.created_at_unix_ms,
        updated_at_unix_ms: record.updated_at_unix_ms,
    })
}

fn machine_target(record: &MachineRecord) -> MachineTarget {
    MachineTarget {
        label: record.label.clone(),
        host: record.host.clone(),
        port: u16::try_from(record.port).unwrap_or(0),
        username: record.username.clone(),
        auth_kind: match record.auth_kind {
            MachineAuth::HubKey => MachineAuthKind::Key,
            MachineAuth::Password => MachineAuthKind::Password,
            MachineAuth::System => MachineAuthKind::System,
        },
    }
}

struct OwnedMachineInput {
    label: String,
    host: String,
    port: i64,
    username: String,
    auth_kind: MachineAuth,
    notes: String,
    wol_mac: Option<String>,
}

impl OwnedMachineInput {
    fn borrowed(&self) -> MachineInput<'_> {
        MachineInput {
            label: self.label.trim(),
            host: self.host.trim(),
            port: self.port,
            username: self.username.trim(),
            auth_kind: self.auth_kind,
            notes: self.notes.trim(),
            wol_mac: self.wol_mac.as_deref(),
        }
    }
}

fn machine_input(body: MachineUpsertBody) -> Result<OwnedMachineInput, ApiError> {
    let auth_kind = MachineAuth::parse(&body.auth_kind)
        .map_err(|_| ApiError::MachineRejected("unknown machine auth kind".to_owned()))?;
    Ok(OwnedMachineInput {
        label: body.label,
        host: body.host,
        port: i64::from(body.port),
        username: body.username,
        auth_kind,
        notes: body.notes,
        wol_mac: body.wol_mac,
    })
}

async fn machine_state<T, F>(state: &ApiState, operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, StateError> + Send + 'static,
{
    let guard = state.blocking_tasks.start();
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        operation()
    })
    .await
    .map_err(|_| {
        tracing::error!("machine state worker failed");
        ApiError::ServiceUnavailable
    })?
    .map_err(map_machine_state)
}

fn map_machine_state(error: StateError) -> ApiError {
    match error {
        StateError::InvalidMachineInput(message) => ApiError::MachineRejected(message.to_owned()),
        StateError::MachineQuotaExceeded => {
            ApiError::MachineRejected("Helix tracks at most 64 rack machines.".to_owned())
        }
        _ => {
            tracing::error!(error = %error, "machine state operation failed");
            ApiError::ServiceUnavailable
        }
    }
}

async fn load_machine(state: &ApiState, id: &str) -> Result<MachineRecord, ApiError> {
    let databases = Arc::clone(&state.databases);
    let id = id.to_owned();
    machine_state(state, move || databases.state().machine(&id))
        .await?
        .ok_or(ApiError::NotFound)
}

async fn list_machines(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "machines.view").await?;
    let databases = Arc::clone(&state.databases);
    let machines = machine_state(&state, move || databases.state().list_machines()).await?;
    let machines = machines
        .iter()
        .map(machine_view)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "machines": machines })),
    ))
}

async fn create_machine(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<MachineUpsertBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "machines.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    let input = machine_input(body)?;
    let databases = Arc::clone(&state.databases);
    let now = i64::try_from(helix_core::unix_timestamp_ms()).unwrap_or(i64::MAX);
    let id = uuid::Uuid::new_v4().to_string();
    let record = machine_state(&state, move || {
        databases.state().create_machine(&id, input.borrowed(), now)
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "machine": machine_view(&record)? })),
    ))
}

async fn update_machine(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(machine_id): RoutePath<String>,
    body: Result<Json<MachineUpsertBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "machines.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    let input = machine_input(body)?;
    let databases = Arc::clone(&state.databases);
    let now = i64::try_from(helix_core::unix_timestamp_ms()).unwrap_or(i64::MAX);
    let record = machine_state(&state, move || {
        databases
            .state()
            .update_machine(&machine_id, input.borrowed(), now)
    })
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "machine": machine_view(&record)? })),
    ))
}

async fn delete_machine(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(machine_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "machines.manage").await?;
    let databases = Arc::clone(&state.databases);
    let deleted = machine_state(&state, move || {
        databases.state().delete_machine(&machine_id)
    })
    .await?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "deleted": true })),
    ))
}

async fn hub_identity(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "machines.view").await?;
    broker_value(&state, BrokerRequest::HubIdentity {})
        .await
        .map(|value| ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response())
}

async fn probe_machine(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(machine_id): RoutePath<String>,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "machines.view").await?;
    let record = load_machine(&state, &machine_id).await?;
    let value = broker_value(
        &state,
        BrokerRequest::MachineProbe {
            machine: machine_target(&record),
        },
    )
    .await?;
    store_probe(&state, &machine_id, &value).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response())
}

async fn refresh_machines(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "machines.view").await?;
    let databases = Arc::clone(&state.databases);
    let machines = machine_state(&state, move || databases.state().list_machines()).await?;
    let mut results = Vec::with_capacity(machines.len());
    for record in &machines {
        let probed = broker_value(
            &state,
            BrokerRequest::MachineProbe {
                machine: machine_target(record),
            },
        )
        .await
        .unwrap_or_else(|_| {
            json!({
                "status": "error",
                "detail": "the host broker could not probe this machine"
            })
        });
        let _ = store_probe(&state, &record.id, &probed).await;
        results.push(json!({ "id": record.id, "probe": probed }));
    }
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "probes": results })),
    )
        .into_response())
}

async fn store_probe(state: &ApiState, id: &str, probe: &Value) -> Result<(), ApiError> {
    let databases = Arc::clone(&state.databases);
    let id = id.to_owned();
    let probe = probe.to_string();
    let now = i64::try_from(helix_core::unix_timestamp_ms()).unwrap_or(i64::MAX);
    machine_state(state, move || {
        databases.state().set_machine_probe(&id, Some(&probe), now)
    })
    .await?;
    Ok(())
}

async fn wake_machine(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(machine_id): RoutePath<String>,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "machines.manage").await?;
    let record = load_machine(&state, &machine_id).await?;
    let Some(mac) = record.wol_mac.clone() else {
        return Err(ApiError::MachineRejected(
            "this machine has no Wake-on-LAN address saved".to_owned(),
        ));
    };
    broker_value(&state, BrokerRequest::MachineWake { mac })
        .await
        .map(|value| ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response())
}

async fn machine_power(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(machine_id): RoutePath<String>,
    body: Result<Json<MachinePowerBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "machines.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    let record = load_machine(&state, &machine_id).await?;
    let action = match body.action.as_str() {
        "reboot" => MachinePowerAction::Reboot,
        "poweroff" => MachinePowerAction::PowerOff,
        _ => {
            return Err(ApiError::MachineRejected(
                "machine power action must be reboot or poweroff".to_owned(),
            ));
        }
    };
    if body.confirmation != record.label {
        return Err(ApiError::MachineRejected(
            "type the machine label to confirm a remote power action".to_owned(),
        ));
    }
    broker_value(
        &state,
        BrokerRequest::MachinePower {
            machine: machine_target(&record),
            action,
        },
    )
    .await
    .map(|value| ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response())
}

async fn issue_machine_ticket(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    RoutePath(machine_id): RoutePath<String>,
    body: Result<Json<MachineTicketBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    auth::validate_post_headers(&headers)?;
    let Json(request) = body.map_err(auth::map_json_rejection)?;
    let dimensions = helix_terminal::TerminalDimensions {
        columns: request.columns,
        rows: request.rows,
    }
    .validate()
    .map_err(|_| ApiError::InvalidTerminalRequest)?;
    if !state
        .terminal
        .as_ref()
        .is_some_and(terminal::TerminalConnector::available)
    {
        return Err(ApiError::TerminalUnavailable);
    }
    let record = load_machine(&state, &machine_id).await?;
    let authenticated = auth::authorize_terminal_for_capability(
        &state,
        peer.ip(),
        &headers,
        request.current_password,
        "machines.manage",
    )
    .await?;
    let session_hash = auth::session_hash_from_headers(&headers)?;
    let ticket = state.terminal_tickets.issue(
        session_hash,
        authenticated.user_id,
        dimensions,
        Some(record.id.clone()),
    )?;
    let mut response = (
        StatusCode::CREATED,
        Json(MachineTicketResponse {
            expires_at_unix_ms: ticket.expires_at_unix_ms,
            connect_path: format!("{MACHINE_CONNECT_PATH_PREFIX}/{machine_id}/terminal/connect"),
            subprotocol: TERMINAL_SUBPROTOCOL,
        }),
    )
        .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        terminal::ticket_cookie(
            MACHINE_TICKET_COOKIE,
            ticket.token.expose_secret(),
            MACHINE_CONNECT_PATH_PREFIX,
        )?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn connect_machine_terminal(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(machine_id): RoutePath<String>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    auth::validate_same_origin_headers(&headers)?;
    if !terminal::offers_terminal_subprotocol(&headers) {
        return Err(ApiError::TerminalTicketRejected);
    }
    let authenticated =
        auth::require_capability_without_csrf(&state, &headers, "machines.manage").await?;
    let session_hash = auth::session_hash_from_headers(&headers)?;
    let encoded = auth::parse_named_cookie(&headers, MACHINE_TICKET_COOKIE)
        .map_err(|()| ApiError::TerminalTicketRejected)?;
    let grant = state
        .terminal_tickets
        .consume(encoded, &session_hash, &authenticated.user_id)?;
    if grant.machine_id.as_deref() != Some(machine_id.as_str()) {
        return Err(ApiError::TerminalTicketRejected);
    }
    let record = load_machine(&state, &machine_id).await?;
    let spec = broker_value(
        &state,
        BrokerRequest::MachineTerminalSpec {
            machine: machine_target(&record),
        },
    )
    .await?;
    let argv = spec
        .get("argv")
        .and_then(Value::as_array)
        .and_then(|args| {
            args.iter()
                .map(|arg| arg.as_str().map(str::to_owned))
                .collect::<Option<Vec<String>>>()
        })
        .ok_or(ApiError::BrokerUnavailable)?;
    let connector = state
        .terminal
        .clone()
        .filter(terminal::TerminalConnector::available)
        .ok_or(ApiError::TerminalUnavailable)?;
    let user_id = authenticated.user_id;
    let databases = Arc::clone(&state.databases);
    let blocking_tasks = state.blocking_tasks.clone();
    let open = helix_terminal::OpenRequest {
        protocol_version: helix_terminal::PROTOCOL_VERSION,
        dimensions: grant.dimensions,
        command: Some(argv),
    };
    let mut response = websocket
        .max_message_size(terminal::MAX_BROWSER_TERMINAL_MESSAGE_BYTES)
        .max_frame_size(terminal::MAX_BROWSER_TERMINAL_MESSAGE_BYTES)
        .protocols([TERMINAL_SUBPROTOCOL])
        .on_upgrade(move |socket| {
            terminal::bridge_terminal(socket, connector, open, user_id, databases, blocking_tasks)
        })
        .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        terminal::clear_ticket_cookie(MACHINE_TICKET_COOKIE, MACHINE_CONNECT_PATH_PREFIX)?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> MachineRecord {
        MachineRecord {
            id: "machine-1".to_owned(),
            label: "builder".to_owned(),
            host: "192.0.2.10".to_owned(),
            port: 22,
            username: "operator".to_owned(),
            auth_kind: MachineAuth::HubKey,
            notes: "top rack".to_owned(),
            wol_mac: Some("3c:52:82:ab:12:34".to_owned()),
            probe_json: None,
            probed_at_unix_ms: None,
            created_at_unix_ms: 1_758_000_000_000,
            updated_at_unix_ms: 1_758_000_000_000,
        }
    }

    #[test]
    fn machine_target_maps_auth_kinds() {
        let mut machine = record();
        assert_eq!(machine_target(&machine).auth_kind, MachineAuthKind::Key);
        machine.auth_kind = MachineAuth::Password;
        assert_eq!(
            machine_target(&machine).auth_kind,
            MachineAuthKind::Password
        );
        machine.auth_kind = MachineAuth::System;
        assert_eq!(machine_target(&machine).auth_kind, MachineAuthKind::System);
        assert_eq!(machine_target(&machine).port, 22);
    }

    #[test]
    fn upsert_input_rejects_unknown_auth_kinds() {
        let body = MachineUpsertBody {
            label: "builder".to_owned(),
            host: "192.0.2.10".to_owned(),
            port: 22,
            username: "operator".to_owned(),
            auth_kind: "agent".to_owned(),
            notes: String::new(),
            wol_mac: None,
        };
        assert!(matches!(
            machine_input(body),
            Err(ApiError::MachineRejected(_))
        ));
    }

    #[test]
    fn upsert_input_keeps_valid_fields() {
        let body = MachineUpsertBody {
            label: "builder".to_owned(),
            host: "192.0.2.10".to_owned(),
            port: 2222,
            username: "operator".to_owned(),
            auth_kind: "system".to_owned(),
            notes: "near the UPS".to_owned(),
            wol_mac: Some("3c:52:82:ab:12:34".to_owned()),
        };
        let input = machine_input(body).unwrap();
        let borrowed = input.borrowed();
        assert_eq!(borrowed.port, 2_222);
        assert_eq!(borrowed.auth_kind, MachineAuth::System);
        assert_eq!(borrowed.wol_mac, Some("3c:52:82:ab:12:34"));
    }

    #[test]
    fn state_errors_map_to_machine_responses() {
        assert!(matches!(
            map_machine_state(StateError::InvalidMachineInput("bad host")),
            ApiError::MachineRejected(_)
        ));
        assert!(matches!(
            map_machine_state(StateError::MachineQuotaExceeded),
            ApiError::MachineRejected(_)
        ));
    }
}
