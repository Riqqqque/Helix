use crate::{ApiError, ApiInitializationError, ApiState, auth};
use axum::{
    Json, Router,
    extract::{
        ConnectInfo, State,
        rejection::JsonRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
#[cfg(unix)]
use futures_util::{SinkExt as _, StreamExt as _};
use helix_auth::{EncodedToken, OpaqueToken, TokenDomain};
use helix_state::{DatabaseSet, SessionTokenHash, TerminalAuditEvent};
#[cfg(unix)]
use helix_terminal::PROTOCOL_VERSION;
use helix_terminal::TerminalDimensions;
#[cfg(unix)]
use helix_terminal::{
    ExitResponse, Frame, OpenRequest, ReadyResponse, decode_frame_length, decode_json,
    encode_frame, encode_json, encode_resize, kind,
};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::io;
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(unix)]
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    time::timeout,
};

const TERMINAL_TICKET_COOKIE: &str = "helix_terminal_ticket";
const TERMINAL_SUBPROTOCOL: &str = "helix-terminal-v1";
const TERMINAL_TICKET_TTL: Duration = Duration::from_secs(30);
const MAX_ACTIVE_TICKETS: usize = 16;
#[cfg(unix)]
const TERMINAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(unix)]
const TERMINAL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const TERMINAL_HEARTBEAT: Duration = Duration::from_secs(20);

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/terminal/status", get(terminal_status))
        .route("/terminal/ticket", post(issue_terminal_ticket))
        .route("/terminal/connect", get(connect_terminal))
}

#[derive(Clone)]
pub(crate) struct TerminalConnector {
    socket_path: PathBuf,
}

impl TerminalConnector {
    pub(crate) fn new(socket_path: PathBuf) -> Result<Self, ApiInitializationError> {
        if !socket_path.is_absolute()
            || socket_path.as_os_str().is_empty()
            || socket_path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
            || socket_path.file_name().is_none()
        {
            return Err(ApiInitializationError::InvalidTerminalSocket);
        }
        Ok(Self { socket_path })
    }

    fn available(&self) -> bool {
        socket_is_unix_socket(&self.socket_path)
    }
}

#[cfg(unix)]
fn socket_is_unix_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

#[cfg(not(unix))]
fn socket_is_unix_socket(_path: &Path) -> bool {
    false
}

struct TerminalTicket {
    session_hash: SessionTokenHash,
    user_id: String,
    dimensions: TerminalDimensions,
    expires_at: Instant,
}

struct TicketIssue {
    token: EncodedToken,
    expires_at_unix_ms: u64,
}

#[derive(Clone)]
pub(crate) struct TerminalTicketStore {
    inner: Arc<Mutex<HashMap<[u8; 32], TerminalTicket>>>,
    capacity: usize,
    ttl: Duration,
}

impl Default for TerminalTicketStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            capacity: MAX_ACTIVE_TICKETS,
            ttl: TERMINAL_TICKET_TTL,
        }
    }
}

impl TerminalTicketStore {
    fn issue(
        &self,
        session_hash: SessionTokenHash,
        user_id: String,
        dimensions: TerminalDimensions,
    ) -> Result<TicketIssue, ApiError> {
        self.issue_at(session_hash, user_id, dimensions, Instant::now())
    }

    fn issue_at(
        &self,
        session_hash: SessionTokenHash,
        user_id: String,
        dimensions: TerminalDimensions,
        now: Instant,
    ) -> Result<TicketIssue, ApiError> {
        let token = OpaqueToken::generate().map_err(|_| ApiError::ServiceUnavailable)?;
        let verifier = token.verification_hash(TokenDomain::TerminalTicket);
        let verifier = *verifier.as_bytes();
        let mut tickets = self
            .inner
            .lock()
            .map_err(|_| ApiError::ServiceUnavailable)?;
        tickets.retain(|_, ticket| ticket.expires_at > now);
        if tickets.len() >= self.capacity {
            return Err(ApiError::TerminalCapacityExhausted);
        }
        let expires_at = now
            .checked_add(self.ttl)
            .ok_or(ApiError::ServiceUnavailable)?;
        tickets.insert(
            verifier,
            TerminalTicket {
                session_hash,
                user_id,
                dimensions,
                expires_at,
            },
        );
        Ok(TicketIssue {
            token: token.encode(),
            expires_at_unix_ms: helix_core::unix_timestamp_ms()
                .saturating_add(u64::try_from(self.ttl.as_millis()).unwrap_or(u64::MAX)),
        })
    }

    fn consume(
        &self,
        encoded: &str,
        session_hash: &SessionTokenHash,
        user_id: &str,
    ) -> Result<TerminalDimensions, ApiError> {
        self.consume_at(encoded, session_hash, user_id, Instant::now())
    }

    fn consume_at(
        &self,
        encoded: &str,
        session_hash: &SessionTokenHash,
        user_id: &str,
        now: Instant,
    ) -> Result<TerminalDimensions, ApiError> {
        let token =
            OpaqueToken::from_encoded(encoded).map_err(|_| ApiError::TerminalTicketRejected)?;
        let verifier = token.verification_hash(TokenDomain::TerminalTicket);
        let verifier = *verifier.as_bytes();
        let mut tickets = self
            .inner
            .lock()
            .map_err(|_| ApiError::ServiceUnavailable)?;
        tickets.retain(|_, ticket| ticket.expires_at > now);
        let ticket = tickets
            .remove(&verifier)
            .ok_or(ApiError::TerminalTicketRejected)?;
        if ticket.expires_at <= now
            || ticket.session_hash != *session_hash
            || ticket.user_id != user_id
        {
            return Err(ApiError::TerminalTicketRejected);
        }
        Ok(ticket.dimensions)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalStatusResponse {
    availability: &'static str,
    reauthentication_required: bool,
    shell_privilege: &'static str,
    persistence: &'static str,
    detail: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalTicketRequest {
    current_password: auth::SecretString,
    columns: u16,
    rows: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalTicketResponse {
    expires_at_unix_ms: u64,
    connect_path: &'static str,
    subprotocol: &'static str,
}

async fn terminal_status(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "terminal.open").await?;
    let available = state
        .terminal
        .as_ref()
        .is_some_and(TerminalConnector::available);
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(TerminalStatusResponse {
            availability: if available {
                "available"
            } else {
                "unavailable"
            },
            reauthentication_required: true,
            shell_privilege: "linux_user",
            persistence: "while_connected",
            detail: if available {
                "A real PTY is ready. Helix rechecks the current dashboard password before every connection."
            } else {
                "The optional unprivileged terminal service is not connected on this host."
            },
        }),
    ))
}

async fn issue_terminal_ticket(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<TerminalTicketRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    auth::validate_post_headers(&headers)?;
    let Json(request) = body.map_err(auth::map_json_rejection)?;
    let dimensions = TerminalDimensions {
        columns: request.columns,
        rows: request.rows,
    }
    .validate()
    .map_err(|_| ApiError::InvalidTerminalRequest)?;
    if !state
        .terminal
        .as_ref()
        .is_some_and(TerminalConnector::available)
    {
        return Err(ApiError::TerminalUnavailable);
    }
    let authenticated = auth::authorize_terminal_with_current_password(
        &state,
        peer.ip(),
        &headers,
        request.current_password,
    )
    .await?;
    let session_hash = auth::session_hash_from_headers(&headers)?;
    let ticket = state
        .terminal_tickets
        .issue(session_hash, authenticated.user_id, dimensions)?;
    let mut response = (
        StatusCode::CREATED,
        Json(TerminalTicketResponse {
            expires_at_unix_ms: ticket.expires_at_unix_ms,
            connect_path: "/api/v1/terminal/connect",
            subprotocol: TERMINAL_SUBPROTOCOL,
        }),
    )
        .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        terminal_ticket_cookie(ticket.token.expose_secret())?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn connect_terminal(
    State(state): State<ApiState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    auth::validate_same_origin_headers(&headers)?;
    if !offers_terminal_subprotocol(&headers) {
        return Err(ApiError::TerminalTicketRejected);
    }
    let authenticated =
        auth::require_capability_without_csrf(&state, &headers, "terminal.open").await?;
    let session_hash = auth::session_hash_from_headers(&headers)?;
    let encoded = auth::parse_named_cookie(&headers, TERMINAL_TICKET_COOKIE)
        .map_err(|()| ApiError::TerminalTicketRejected)?;
    let dimensions =
        state
            .terminal_tickets
            .consume(encoded, &session_hash, &authenticated.user_id)?;
    let connector = state
        .terminal
        .clone()
        .filter(TerminalConnector::available)
        .ok_or(ApiError::TerminalUnavailable)?;
    let user_id = authenticated.user_id;
    let databases = Arc::clone(&state.databases);
    let blocking_tasks = state.blocking_tasks.clone();
    let mut response = websocket
        .protocols([TERMINAL_SUBPROTOCOL])
        .on_upgrade(move |socket| {
            bridge_terminal(
                socket,
                connector,
                dimensions,
                user_id,
                databases,
                blocking_tasks,
            )
        })
        .into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, clear_terminal_ticket_cookie());
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn offers_terminal_subprotocol(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all("sec-websocket-protocol").iter();
    let Some(value) = values.next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    values.next().is_none()
        && value
            .split(',')
            .map(str::trim)
            .filter(|protocol| !protocol.is_empty())
            .eq([TERMINAL_SUBPROTOCOL])
}

fn terminal_ticket_cookie(encoded: &str) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(&format!(
        "{TERMINAL_TICKET_COOKIE}={encoded}; HttpOnly; SameSite=Strict; Path=/api/v1/terminal/connect; Max-Age=30"
    ))
    .map_err(|_| ApiError::ServiceUnavailable)
}

fn clear_terminal_ticket_cookie() -> HeaderValue {
    HeaderValue::from_static(
        "helix_terminal_ticket=; HttpOnly; SameSite=Strict; Path=/api/v1/terminal/connect; Max-Age=0",
    )
}

#[cfg(unix)]
async fn bridge_terminal(
    mut websocket: WebSocket,
    connector: TerminalConnector,
    dimensions: TerminalDimensions,
    user_id: String,
    databases: Arc<DatabaseSet>,
    blocking_tasks: crate::BlockingTaskTracker,
) {
    let stream = match timeout(
        TERMINAL_CONNECT_TIMEOUT,
        tokio::net::UnixStream::connect(&connector.socket_path),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        _ => {
            send_websocket_error(
                &mut websocket,
                "The host terminal service could not be reached.",
            )
            .await;
            record_terminal_audit(
                databases,
                blocking_tasks,
                user_id,
                TerminalAuditEvent::SessionFailed,
            )
            .await;
            return;
        }
    };
    let (mut terminal_reader, mut terminal_writer) = tokio::io::split(stream);
    let open_payload = match encode_json(&OpenRequest {
        protocol_version: PROTOCOL_VERSION,
        dimensions,
    }) {
        Ok(payload) => payload,
        Err(_) => {
            send_websocket_error(
                &mut websocket,
                "The terminal session could not be initialized.",
            )
            .await;
            return;
        }
    };
    let handshake = async {
        write_terminal_frame(&mut terminal_writer, kind::CLIENT_OPEN, &open_payload).await?;
        read_terminal_frame(&mut terminal_reader).await
    };
    let ready = match timeout(TERMINAL_HANDSHAKE_TIMEOUT, handshake).await {
        Ok(Ok(frame)) if frame.kind == kind::SERVER_READY => {
            decode_json::<ReadyResponse>(&frame.payload).ok()
        }
        _ => None,
    };
    let Some(ready) = ready.filter(valid_ready_response) else {
        send_websocket_error(
            &mut websocket,
            "The host terminal service rejected the session.",
        )
        .await;
        record_terminal_audit(
            databases,
            blocking_tasks,
            user_id,
            TerminalAuditEvent::SessionFailed,
        )
        .await;
        return;
    };
    if record_terminal_audit_result(
        Arc::clone(&databases),
        blocking_tasks.clone(),
        user_id.clone(),
        TerminalAuditEvent::SessionOpened,
    )
    .await
    .is_err()
    {
        let _ = write_terminal_frame(&mut terminal_writer, kind::CLIENT_CLOSE, &[]).await;
        send_websocket_error(
            &mut websocket,
            "Helix could not record the protected terminal session.",
        )
        .await;
        return;
    }
    let ready_event = serde_json::json!({
        "type": "ready",
        "user": ready.user,
        "shell": ready.shell,
    })
    .to_string();
    if websocket
        .send(Message::Text(ready_event.into()))
        .await
        .is_err()
    {
        let _ = write_terminal_frame(&mut terminal_writer, kind::CLIENT_CLOSE, &[]).await;
        record_terminal_audit(
            databases,
            blocking_tasks,
            user_id,
            TerminalAuditEvent::SessionClosed,
        )
        .await;
        return;
    }

    let (mut browser_writer, mut browser_reader) = websocket.split();
    let mut heartbeat = tokio::time::interval(TERMINAL_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;
    let mut failed_message: Option<&'static str> = None;
    let mut normal_close = false;
    loop {
        tokio::select! {
            daemon_frame = read_terminal_frame(&mut terminal_reader) => {
                match daemon_frame {
                    Ok(frame) if frame.kind == kind::SERVER_OUTPUT => {
                        if browser_writer.send(Message::Binary(frame.payload.into())).await.is_err() {
                            normal_close = true;
                            break;
                        }
                    }
                    Ok(frame) if frame.kind == kind::SERVER_EXIT => {
                        match decode_json::<ExitResponse>(&frame.payload) {
                            Ok(exit) => {
                                let event = serde_json::json!({
                                    "type": "exit",
                                    "exitCode": exit.exit_code,
                                    "signal": exit.signal,
                                }).to_string();
                                let _ = browser_writer.send(Message::Text(event.into())).await;
                                normal_close = true;
                            }
                            Err(_) => failed_message = Some("The host terminal returned an invalid exit status."),
                        }
                        break;
                    }
                    Ok(frame) if frame.kind == kind::SERVER_ERROR => {
                        failed_message = Some(safe_daemon_error(&frame.payload));
                        break;
                    }
                    Ok(_) => {
                        failed_message = Some("The host terminal returned an invalid message.");
                        break;
                    }
                    Err(_) => {
                        if !normal_close {
                            failed_message = Some("The host terminal connection ended unexpectedly.");
                        }
                        break;
                    }
                }
            }
            browser_message = browser_reader.next() => {
                match browser_message {
                    Some(Ok(Message::Binary(input))) if input.len() <= helix_terminal::MAX_TERMINAL_INPUT_BYTES => {
                        if write_terminal_frame(&mut terminal_writer, kind::CLIENT_INPUT, &input).await.is_err() {
                            failed_message = Some("The terminal could not send input to the host.");
                            break;
                        }
                    }
                    Some(Ok(Message::Text(control))) => {
                        match serde_json::from_str::<BrowserControl>(control.as_str()) {
                            Ok(BrowserControl::Resize { columns, rows }) => {
                                let payload = match encode_resize(TerminalDimensions { columns, rows }) {
                                    Ok(payload) => payload,
                                    Err(_) => {
                                        failed_message = Some("The browser sent an invalid terminal size.");
                                        break;
                                    }
                                };
                                if write_terminal_frame(&mut terminal_writer, kind::CLIENT_RESIZE, &payload).await.is_err() {
                                    failed_message = Some("The terminal could not resize the host PTY.");
                                    break;
                                }
                            }
                            Ok(BrowserControl::Keepalive {}) => {}
                            Ok(BrowserControl::Close {}) => {
                                normal_close = true;
                                break;
                            }
                            Err(_) => {
                                failed_message = Some("The browser sent an invalid terminal control message.");
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(value))) => {
                        if browser_writer.send(Message::Pong(value)).await.is_err() {
                            normal_close = true;
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => {
                        normal_close = true;
                        break;
                    }
                    Some(Ok(_)) => {
                        failed_message = Some("The browser sent an unsupported terminal message.");
                        break;
                    }
                    Some(Err(_)) => {
                        normal_close = true;
                        break;
                    }
                }
            }
            _ = heartbeat.tick() => {
                let event = serde_json::json!({ "type": "heartbeat" }).to_string();
                if browser_writer.send(Message::Text(event.into())).await.is_err() {
                    normal_close = true;
                    break;
                }
            }
        }
    }
    let _ = write_terminal_frame(&mut terminal_writer, kind::CLIENT_CLOSE, &[]).await;
    if let Some(message) = failed_message {
        let event = serde_json::json!({ "type": "error", "message": message }).to_string();
        let _ = browser_writer.send(Message::Text(event.into())).await;
    }
    let _ = browser_writer.send(Message::Close(None)).await;
    record_terminal_audit(
        databases,
        blocking_tasks,
        user_id,
        if normal_close && failed_message.is_none() {
            TerminalAuditEvent::SessionClosed
        } else {
            TerminalAuditEvent::SessionFailed
        },
    )
    .await;
}

#[cfg(not(unix))]
async fn bridge_terminal(
    mut websocket: WebSocket,
    _connector: TerminalConnector,
    _dimensions: TerminalDimensions,
    user_id: String,
    databases: Arc<DatabaseSet>,
    blocking_tasks: crate::BlockingTaskTracker,
) {
    send_websocket_error(
        &mut websocket,
        "Host terminals are available only on Unix systems.",
    )
    .await;
    record_terminal_audit(
        databases,
        blocking_tasks,
        user_id,
        TerminalAuditEvent::SessionFailed,
    )
    .await;
}

#[cfg(unix)]
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserControl {
    Resize { columns: u16, rows: u16 },
    Keepalive {},
    Close {},
}

#[cfg(unix)]
fn valid_ready_response(ready: &ReadyResponse) -> bool {
    ready.protocol_version == PROTOCOL_VERSION
        && !ready.user.is_empty()
        && ready.user.len() <= 64
        && ready
            .user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && ready.shell.starts_with('/')
        && ready.shell.len() <= 512
        && !ready.shell.chars().any(char::is_control)
}

#[cfg(unix)]
fn safe_daemon_error(payload: &[u8]) -> &'static str {
    match std::str::from_utf8(payload) {
        Ok("Helix already has the maximum number of terminal sessions open.") => {
            "Helix already has the maximum number of terminal sessions open."
        }
        Ok("The terminal client and host service use different protocol versions.") => {
            "The terminal client and host service use different protocol versions."
        }
        _ => "The host terminal service rejected the session.",
    }
}

async fn send_websocket_error(websocket: &mut WebSocket, message: &'static str) {
    let event = serde_json::json!({ "type": "error", "message": message }).to_string();
    let _ = websocket.send(Message::Text(event.into())).await;
    let _ = websocket.send(Message::Close(None)).await;
}

async fn record_terminal_audit_result(
    databases: Arc<DatabaseSet>,
    blocking_tasks: crate::BlockingTaskTracker,
    user_id: String,
    event: TerminalAuditEvent,
) -> Result<(), ApiError> {
    auth::run_blocking_state(&blocking_tasks, move || {
        databases.state().record_terminal_audit(
            &user_id,
            event,
            i64::try_from(helix_core::unix_timestamp_ms()).unwrap_or(i64::MAX),
        )
    })
    .await
}

async fn record_terminal_audit(
    databases: Arc<DatabaseSet>,
    blocking_tasks: crate::BlockingTaskTracker,
    user_id: String,
    event: TerminalAuditEvent,
) {
    if record_terminal_audit_result(databases, blocking_tasks, user_id, event)
        .await
        .is_err()
    {
        tracing::error!("terminal lifecycle audit could not be recorded");
    }
}

#[cfg(unix)]
async fn read_terminal_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Frame, io::Error> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header).await?;
    let length = decode_frame_length(header)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).await?;
    let kind = body[0];
    Ok(Frame {
        kind,
        payload: body.split_off(1),
    })
}

#[cfg(unix)]
async fn write_terminal_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame_kind: u8,
    payload: &[u8],
) -> Result<(), io::Error> {
    let frame = encode_frame(frame_kind, payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    writer.write_all(&frame).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(seed: u8) -> SessionTokenHash {
        SessionTokenHash::from_digest([seed; 32])
    }

    #[test]
    fn ticket_is_single_use_session_bound_and_never_returned_in_the_path() {
        let store = TerminalTicketStore {
            inner: Arc::new(Mutex::new(HashMap::new())),
            capacity: 2,
            ttl: Duration::from_secs(30),
        };
        let dimensions = TerminalDimensions {
            columns: 120,
            rows: 32,
        };
        let issued = store
            .issue(session(1), "owner".to_owned(), dimensions)
            .unwrap();
        let encoded = issued.token.expose_secret().to_owned();
        assert_eq!(
            store.consume(&encoded, &session(1), "owner").unwrap(),
            dimensions
        );
        assert!(matches!(
            store.consume(&encoded, &session(1), "owner"),
            Err(ApiError::TerminalTicketRejected)
        ));
        assert_eq!(
            TerminalTicketResponse {
                expires_at_unix_ms: 1,
                connect_path: "/api/v1/terminal/connect",
                subprotocol: TERMINAL_SUBPROTOCOL,
            }
            .connect_path,
            "/api/v1/terminal/connect"
        );
    }

    #[test]
    fn wrong_session_burns_the_ticket_and_expired_entries_free_capacity() {
        let now = Instant::now();
        let store = TerminalTicketStore {
            inner: Arc::new(Mutex::new(HashMap::new())),
            capacity: 1,
            ttl: Duration::from_millis(10),
        };
        let dimensions = TerminalDimensions {
            columns: 80,
            rows: 24,
        };
        let first = store
            .issue_at(session(1), "owner".to_owned(), dimensions, now)
            .unwrap();
        assert!(matches!(
            store.consume_at(first.token.expose_secret(), &session(2), "owner", now),
            Err(ApiError::TerminalTicketRejected)
        ));
        let second = store
            .issue_at(session(1), "owner".to_owned(), dimensions, now)
            .unwrap();
        let later = now + Duration::from_millis(20);
        assert!(matches!(
            store.consume_at(second.token.expose_secret(), &session(1), "owner", later),
            Err(ApiError::TerminalTicketRejected)
        ));
        store
            .issue_at(session(1), "owner".to_owned(), dimensions, later)
            .unwrap();
    }

    #[test]
    fn websocket_subprotocol_is_exact_and_not_a_ticket_transport() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "sec-websocket-protocol",
            HeaderValue::from_static(TERMINAL_SUBPROTOCOL),
        );
        assert!(offers_terminal_subprotocol(&headers));
        headers.insert(
            "sec-websocket-protocol",
            HeaderValue::from_static("helix-terminal-v1, bearer-token"),
        );
        assert!(!offers_terminal_subprotocol(&headers));
    }
}
