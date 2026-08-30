use crate::{ApiError, ApiInitializationError, ApiState, BlockingTaskTracker};
use axum::{
    Json, Router,
    extract::{ConnectInfo, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use helix_auth::{
    DisplayName, LoginName, OpaqueToken, PASSWORD_POLICY_VERSION, PasswordContext, TokenDomain,
    hash_password, normalize_password_for_verification, password_needs_rehash,
    rehash_verified_password, validate_password, validate_verified_password_for_context,
    verify_password,
};
use helix_state::{
    AuthenticatedSession, BootstrapPreflightOutcome, BootstrapTokenHash, CsrfRequirement,
    CsrfTokenHash, DatabaseSet, OwnerAccountUpdateInput, OwnerAccountUpdateOutcome,
    OwnerClaimInput, OwnerClaimOutcome, OwnerClaimRejection, PasswordPhc as StatePasswordPhc,
    PasswordRehash, SessionAuthenticationInput, SessionAuthorization, SessionCreateInput,
    SessionCreateOutcome, SessionTokenHash, UserStatus,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use zeroize::Zeroize;

const SESSION_COOKIE_NAME: &str = "helix_session";
const SESSION_COOKIE_EXPIRING_MAX_AGE_SECONDS: i64 = 8 * 60 * 60;
const SESSION_COOKIE_PERSISTENT_MAX_AGE_SECONDS: i64 = 400 * 24 * 60 * 60;
const CSRF_HEADER: &str = "x-helix-csrf";
const PASSWORD_WORKERS: usize = 2;
const RATE_LIMIT_MAX_ENTRIES: usize = 2_048;
const RATE_LIMIT_TTL: Duration = Duration::from_secs(15 * 60);
const LOGIN_GLOBAL_ATTEMPTS_PER_WINDOW: u32 = 256;
const LOGIN_PEER_ATTEMPTS_PER_WINDOW: u32 = 5;
const LOGIN_ACCOUNT_ATTEMPTS_PER_WINDOW: u32 = 5;
const SETUP_GLOBAL_ATTEMPTS_PER_WINDOW: u32 = 32;
const SETUP_ATTEMPTS_PER_WINDOW: u32 = 8;
const ACCOUNT_UPDATE_ATTEMPTS_PER_WINDOW: u32 = 5;
const PREFERENCE_USER_WRITES_PER_WINDOW: u32 = 240;
const PREFERENCE_SESSION_WRITES_PER_WINDOW: u32 = 120;
const PREFERENCE_WRITE_WINDOW: Duration = Duration::from_secs(60);
const STORAGE_ANALYSIS_USER_STARTS_PER_WINDOW: u32 = 8;
const STORAGE_ANALYSIS_SESSION_STARTS_PER_WINDOW: u32 = 4;
const STORAGE_ANALYSIS_USER_READS_PER_WINDOW: u32 = 360;
const STORAGE_ANALYSIS_SESSION_READS_PER_WINDOW: u32 = 180;
const STORAGE_ANALYSIS_WINDOW: Duration = Duration::from_secs(60);
const DUMMY_LOGIN: &str = "helix-timing-probe";
const DUMMY_DISPLAY: &str = "Helix Timing Probe";
const DUMMY_PASSWORD: &str = "V7!quartz-Meteor#29";

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/setup/status", get(setup_status))
        .route("/setup/owner", post(setup_owner))
        .route("/auth/login", post(login))
        .route("/auth/csrf", post(rotate_csrf))
        .route("/auth/me", get(me))
        .route("/auth/account", post(update_account))
        .route(
            "/auth/session-expiry",
            get(session_expiry).put(update_session_expiry),
        )
        .route("/auth/logout", post(logout))
}

pub(crate) async fn initialize_password_boundary()
-> Result<(Arc<tokio::sync::Semaphore>, Arc<str>), ApiInitializationError> {
    static DUMMY_PHC: OnceLock<String> = OnceLock::new();

    let dummy_phc = tokio::task::spawn_blocking(|| {
        if let Some(existing) = DUMMY_PHC.get() {
            return Ok(existing.clone());
        }

        let login = LoginName::parse(DUMMY_LOGIN).map_err(|_| ())?;
        let display = DisplayName::parse(DUMMY_DISPLAY).map_err(|_| ())?;
        let password = validate_password(
            DUMMY_PASSWORD.to_owned(),
            &PasswordContext::new(&login, &display),
        )
        .map_err(|_| ())?;
        let phc = hash_password(&password).map_err(|_| ())?;
        let value = phc.as_str().to_owned();
        let _ = DUMMY_PHC.set(value.clone());
        Ok::<String, ()>(value)
    })
    .await
    .map_err(|_| ApiInitializationError::PasswordWorkerFailed)?
    .map_err(|()| ApiInitializationError::PasswordPrimitiveFailed)?;

    Ok((
        Arc::new(tokio::sync::Semaphore::new(PASSWORD_WORKERS)),
        Arc::from(dummy_phc),
    ))
}

#[derive(Clone)]
pub(crate) struct AttemptLimiter {
    inner: Arc<Mutex<HashMap<AttemptKey, AttemptEntry>>>,
    max_entries: usize,
    ttl: Duration,
    login_global_limit: u32,
    login_peer_limit: u32,
    login_account_limit: u32,
    setup_global_limit: u32,
    setup_peer_limit: u32,
    account_update_limit: u32,
    preference_user_limit: u32,
    preference_session_limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum AttemptScope {
    Login,
    Setup,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum AttemptKey {
    GlobalLogin,
    GlobalSetup,
    Peer {
        peer: IpAddr,
        scope: AttemptScope,
    },
    Account(String),
    AccountUpdateUser(String),
    AccountUpdateSessionPeer {
        user_id: String,
        session_hash: SessionTokenHash,
        peer: IpAddr,
    },
    TerminalProofUser(String),
    TerminalProofSessionPeer {
        user_id: String,
        session_hash: SessionTokenHash,
        peer: IpAddr,
    },
    PreferenceUser(String),
    PreferenceSessionPeer {
        user_id: String,
        session_hash: SessionTokenHash,
        peer: IpAddr,
    },
    StorageAnalysisStartUser(String),
    StorageAnalysisStartSessionPeer {
        user_id: String,
        session_hash: SessionTokenHash,
        peer: IpAddr,
    },
    StorageAnalysisReadUser(String),
    StorageAnalysisReadSessionPeer {
        user_id: String,
        session_hash: SessionTokenHash,
        peer: IpAddr,
    },
}

struct AttemptEntry {
    window_started: Instant,
    last_seen: Instant,
    attempts: u32,
}

struct AttemptTicket {
    key: AttemptKey,
    window_started: Instant,
}

struct AttemptReservation {
    limiter: AttemptLimiter,
    tickets: Vec<AttemptTicket>,
    consume: bool,
}

impl AttemptReservation {
    fn consume_failure(mut self) {
        self.consume = true;
    }
}

impl Drop for AttemptReservation {
    fn drop(&mut self) {
        if !self.consume {
            self.limiter.refund(&self.tickets);
        }
    }
}

impl AttemptLimiter {
    pub(crate) fn production() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            max_entries: RATE_LIMIT_MAX_ENTRIES,
            ttl: RATE_LIMIT_TTL,
            login_global_limit: LOGIN_GLOBAL_ATTEMPTS_PER_WINDOW,
            login_peer_limit: LOGIN_PEER_ATTEMPTS_PER_WINDOW,
            login_account_limit: LOGIN_ACCOUNT_ATTEMPTS_PER_WINDOW,
            setup_global_limit: SETUP_GLOBAL_ATTEMPTS_PER_WINDOW,
            setup_peer_limit: SETUP_ATTEMPTS_PER_WINDOW,
            account_update_limit: ACCOUNT_UPDATE_ATTEMPTS_PER_WINDOW,
            preference_user_limit: PREFERENCE_USER_WRITES_PER_WINDOW,
            preference_session_limit: PREFERENCE_SESSION_WRITES_PER_WINDOW,
        }
    }

    fn reserve_setup(&self, peer: IpAddr) -> Option<AttemptReservation> {
        self.reserve_setup_at(peer, Instant::now())
    }

    fn reserve_setup_at(&self, peer: IpAddr, now: Instant) -> Option<AttemptReservation> {
        let tickets = self.reserve_keys(
            &[
                (AttemptKey::GlobalSetup, self.setup_global_limit),
                (
                    AttemptKey::Peer {
                        peer,
                        scope: AttemptScope::Setup,
                    },
                    self.setup_peer_limit,
                ),
            ],
            now,
            self.ttl,
        )?;
        Some(AttemptReservation {
            limiter: self.clone(),
            tickets,
            consume: false,
        })
    }

    fn reserve_login(&self, peer: IpAddr, login_name: &str) -> Option<AttemptReservation> {
        let mut keys = vec![
            (AttemptKey::GlobalLogin, self.login_global_limit),
            (
                AttemptKey::Peer {
                    peer,
                    scope: AttemptScope::Login,
                },
                self.login_peer_limit,
            ),
        ];
        if let Ok(login) = LoginName::parse(login_name) {
            keys.push((
                AttemptKey::Account(login.as_str().to_owned()),
                self.login_account_limit,
            ));
        }
        let tickets = self.reserve_keys(&keys, Instant::now(), self.ttl)?;
        Some(AttemptReservation {
            limiter: self.clone(),
            tickets,
            consume: false,
        })
    }

    fn reserve_account_update(
        &self,
        peer: IpAddr,
        user_id: &str,
        session_hash: &SessionTokenHash,
    ) -> Option<AttemptReservation> {
        self.reserve_account_update_at(peer, user_id, session_hash, Instant::now())
    }

    fn reserve_terminal_proof(
        &self,
        peer: IpAddr,
        user_id: &str,
        session_hash: &SessionTokenHash,
    ) -> Option<AttemptReservation> {
        let tickets = self.reserve_keys(
            &[
                (
                    AttemptKey::TerminalProofUser(user_id.to_owned()),
                    self.account_update_limit,
                ),
                (
                    AttemptKey::TerminalProofSessionPeer {
                        user_id: user_id.to_owned(),
                        session_hash: session_hash.clone(),
                        peer,
                    },
                    self.account_update_limit,
                ),
            ],
            Instant::now(),
            self.ttl,
        )?;
        Some(AttemptReservation {
            limiter: self.clone(),
            tickets,
            consume: false,
        })
    }

    fn reserve_account_update_at(
        &self,
        peer: IpAddr,
        user_id: &str,
        session_hash: &SessionTokenHash,
        now: Instant,
    ) -> Option<AttemptReservation> {
        let tickets = self.reserve_keys(
            &[
                (
                    AttemptKey::AccountUpdateUser(user_id.to_owned()),
                    self.account_update_limit,
                ),
                (
                    AttemptKey::AccountUpdateSessionPeer {
                        user_id: user_id.to_owned(),
                        session_hash: session_hash.clone(),
                        peer,
                    },
                    self.account_update_limit,
                ),
            ],
            now,
            self.ttl,
        )?;
        Some(AttemptReservation {
            limiter: self.clone(),
            tickets,
            consume: false,
        })
    }

    pub(crate) fn allow_preference_write(
        &self,
        peer: IpAddr,
        user_id: &str,
        session_hash: &SessionTokenHash,
    ) -> bool {
        let Some(reservation) = self.reserve_keys(
            &[
                (
                    AttemptKey::PreferenceUser(user_id.to_owned()),
                    self.preference_user_limit,
                ),
                (
                    AttemptKey::PreferenceSessionPeer {
                        user_id: user_id.to_owned(),
                        session_hash: session_hash.clone(),
                        peer,
                    },
                    self.preference_session_limit,
                ),
            ],
            Instant::now(),
            PREFERENCE_WRITE_WINDOW,
        ) else {
            return false;
        };
        let reservation = AttemptReservation {
            limiter: self.clone(),
            tickets: reservation,
            consume: false,
        };
        reservation.consume_failure();
        true
    }

    pub(crate) fn allow_storage_analysis_start(
        &self,
        peer: IpAddr,
        user_id: &str,
        session_hash: &SessionTokenHash,
    ) -> bool {
        self.consume_bounded_request(
            &[
                (
                    AttemptKey::StorageAnalysisStartUser(user_id.to_owned()),
                    STORAGE_ANALYSIS_USER_STARTS_PER_WINDOW,
                ),
                (
                    AttemptKey::StorageAnalysisStartSessionPeer {
                        user_id: user_id.to_owned(),
                        session_hash: session_hash.clone(),
                        peer,
                    },
                    STORAGE_ANALYSIS_SESSION_STARTS_PER_WINDOW,
                ),
            ],
            STORAGE_ANALYSIS_WINDOW,
        )
    }

    pub(crate) fn allow_storage_analysis_read(
        &self,
        peer: IpAddr,
        user_id: &str,
        session_hash: &SessionTokenHash,
    ) -> bool {
        self.consume_bounded_request(
            &[
                (
                    AttemptKey::StorageAnalysisReadUser(user_id.to_owned()),
                    STORAGE_ANALYSIS_USER_READS_PER_WINDOW,
                ),
                (
                    AttemptKey::StorageAnalysisReadSessionPeer {
                        user_id: user_id.to_owned(),
                        session_hash: session_hash.clone(),
                        peer,
                    },
                    STORAGE_ANALYSIS_SESSION_READS_PER_WINDOW,
                ),
            ],
            STORAGE_ANALYSIS_WINDOW,
        )
    }

    fn consume_bounded_request(&self, keys: &[(AttemptKey, u32)], window: Duration) -> bool {
        let Some(tickets) = self.reserve_keys(keys, Instant::now(), window) else {
            return false;
        };
        let reservation = AttemptReservation {
            limiter: self.clone(),
            tickets,
            consume: false,
        };
        reservation.consume_failure();
        true
    }

    fn reserve_keys(
        &self,
        keys: &[(AttemptKey, u32)],
        now: Instant,
        window: Duration,
    ) -> Option<Vec<AttemptTicket>> {
        let Ok(mut entries) = self.inner.lock() else {
            return None;
        };
        entries.retain(|_, entry| now.saturating_duration_since(entry.last_seen) < self.ttl);

        let needed = keys
            .iter()
            .filter(|(key, _)| !entries.contains_key(key))
            .count();
        if needed > self.max_entries.saturating_sub(entries.len()) {
            return None;
        }
        for (key, maximum) in keys {
            if entries.get(key).is_some_and(|entry| {
                now.saturating_duration_since(entry.window_started) < window
                    && entry.attempts >= *maximum
            }) {
                return None;
            }
        }

        let mut tickets = Vec::with_capacity(keys.len());
        for (key, _) in keys {
            let entry = if let Some(entry) = entries.get_mut(key) {
                if now.saturating_duration_since(entry.window_started) >= window {
                    entry.window_started = now;
                    entry.attempts = 0;
                }
                entry.last_seen = now;
                entry.attempts = entry.attempts.saturating_add(1);
                entry
            } else {
                entries.entry(key.clone()).or_insert_with(|| AttemptEntry {
                    window_started: now,
                    last_seen: now,
                    attempts: 1,
                })
            };
            tickets.push(AttemptTicket {
                key: key.clone(),
                window_started: entry.window_started,
            });
        }
        Some(tickets)
    }

    fn refund(&self, tickets: &[AttemptTicket]) {
        let Ok(mut entries) = self.inner.lock() else {
            return;
        };
        let mut empty = Vec::new();
        for ticket in tickets {
            if let Some(entry) = entries.get_mut(&ticket.key)
                && entry.window_started == ticket.window_started
            {
                entry.attempts = entry.attempts.saturating_sub(1);
                if entry.attempts == 0 {
                    empty.push(ticket.key.clone());
                }
            }
        }
        for key in empty {
            entries.remove(&key);
        }
    }

    #[cfg(test)]
    fn with_limits(
        max_entries: usize,
        ttl: Duration,
        login_global_limit: u32,
        login_peer_limit: u32,
        login_account_limit: u32,
        setup_global_limit: u32,
        setup_peer_limit: u32,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            max_entries,
            ttl,
            login_global_limit,
            login_peer_limit,
            login_account_limit,
            setup_global_limit,
            setup_peer_limit,
            account_update_limit: ACCOUNT_UPDATE_ATTEMPTS_PER_WINDOW,
            preference_user_limit: PREFERENCE_USER_WRITES_PER_WINDOW,
            preference_session_limit: PREFERENCE_SESSION_WRITES_PER_WINDOW,
        }
    }

    #[cfg(test)]
    fn allow_setup_at(&self, peer: IpAddr, now: Instant) -> bool {
        let Some(reservation) = self.reserve_setup_at(peer, now) else {
            return false;
        };
        reservation.consume_failure();
        true
    }

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.inner.lock().expect("limiter lock").len()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnerSetupRequest {
    bootstrap_token: SecretString,
    login_name: String,
    display_name: String,
    password: SecretString,
}

#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct SecretString(String);

impl SecretString {
    fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn take(&mut self) -> String {
        std::mem::take(&mut self.0)
    }

    fn zeroize_now(&mut self) {
        self.0.zeroize();
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoginRequest {
    login_name: String,
    password: SecretString,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountUpdateRequest {
    current_password: SecretString,
    login_name: String,
    display_name: String,
    new_password: Option<SecretString>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyRequest {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupStatusResponse {
    owner_exists: bool,
    bootstrap_available: bool,
    bootstrap_expires_at_unix_ms: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthUser {
    id: String,
    login_name: String,
    display_name: String,
    capabilities: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthSuccessResponse<'a> {
    user: AuthUser,
    csrf_token: &'a str,
    expires_at_unix_ms: i64,
    session_expires: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MeResponse {
    user: AuthUser,
    expires_at_unix_ms: i64,
    session_expires: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionExpiryUpdateRequest {
    expires: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionExpiryUpdateResponse {
    expires: bool,
    expires_at_unix_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CsrfResponse<'a> {
    csrf_token: &'a str,
}

struct SessionIssue {
    session_token: OpaqueToken,
    csrf_token: OpaqueToken,
    authenticated: AuthenticatedSession,
}

enum SetupWorkerOutcome {
    Claimed(Box<SessionIssue>),
    Conflict,
    Rejected,
}

enum LoginWorkerOutcome {
    Authenticated(Box<SessionIssue>),
    MaintenanceRequired,
    Rejected,
}

enum AccountWorkerOutcome {
    Updated,
    CurrentPasswordRejected,
    LoginNameUnavailable,
    InvalidReplacement,
    CredentialChangedOrUnavailable,
}

enum TerminalProofOutcome {
    Verified(AuthenticatedSession),
    Rejected,
    CredentialChangedOrUnavailable,
}

fn consumes_login_failure_budget(outcome: &Result<LoginWorkerOutcome, ()>) -> bool {
    matches!(outcome, Ok(LoginWorkerOutcome::Rejected))
}

fn consumes_setup_failure_budget(outcome: &Result<SetupWorkerOutcome, ()>) -> bool {
    matches!(outcome, Ok(SetupWorkerOutcome::Rejected))
}

async fn setup_status(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    let now = now_unix_ms();
    let databases = Arc::clone(&state.databases);
    let status = run_blocking_state(&state.blocking_tasks, move || {
        databases.state().setup_status(now)
    })
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(SetupStatusResponse {
            owner_exists: status.owner_exists,
            bootstrap_available: status.bootstrap_available,
            bootstrap_expires_at_unix_ms: status
                .bootstrap_available
                .then_some(status.bootstrap_expires_at_unix_ms)
                .flatten(),
        }),
    ))
}

async fn setup_owner(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<OwnerSetupRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_post_headers(&headers)?;
    let Json(request) = body.map_err(map_json_rejection)?;
    let reservation = state
        .attempt_limiter
        .reserve_setup(peer.ip())
        .ok_or(ApiError::AttemptRateLimited)?;
    let permit = Arc::clone(&state.password_workers)
        .try_acquire_owned()
        .map_err(|_| ApiError::PasswordWorkersBusy)?;
    let blocking_guard = state.blocking_tasks.start();
    let databases = Arc::clone(&state.databases);

    let outcome = tokio::task::spawn_blocking(move || {
        let _blocking_guard = blocking_guard;
        let _permit = permit;
        let outcome = perform_owner_setup(databases.as_ref(), request);
        if consumes_setup_failure_budget(&outcome) {
            reservation.consume_failure();
        }
        outcome
    })
    .await
    .map_err(|_| {
        tracing::error!("owner setup worker failed");
        ApiError::ServiceUnavailable
    })?
    .map_err(|()| {
        tracing::error!("owner setup could not complete");
        ApiError::ServiceUnavailable
    })?;

    match outcome {
        SetupWorkerOutcome::Claimed(issue) => session_issue_response(StatusCode::CREATED, *issue),
        SetupWorkerOutcome::Conflict => Err(ApiError::SetupConflict),
        SetupWorkerOutcome::Rejected => Err(ApiError::SetupRejected),
    }
}

fn perform_owner_setup(
    databases: &DatabaseSet,
    request: OwnerSetupRequest,
) -> Result<SetupWorkerOutcome, ()> {
    perform_owner_setup_with_hasher(databases, request, hash_password)
}

fn perform_owner_setup_with_hasher<F>(
    databases: &DatabaseSet,
    mut request: OwnerSetupRequest,
    password_hasher: F,
) -> Result<SetupWorkerOutcome, ()>
where
    F: FnOnce(
        &helix_auth::ValidatedPassword,
    ) -> Result<helix_auth::PasswordPhc, helix_auth::PasswordHashError>,
{
    let now = now_unix_ms();
    let status = databases.state().setup_status(now).map_err(|_| ())?;
    if status.owner_exists {
        return Ok(SetupWorkerOutcome::Conflict);
    }
    if !status.bootstrap_available {
        return Ok(SetupWorkerOutcome::Rejected);
    }

    let bootstrap = match OpaqueToken::from_encoded(request.bootstrap_token.as_str()) {
        Ok(token) => token,
        Err(_) => {
            request.bootstrap_token.zeroize_now();
            return Ok(SetupWorkerOutcome::Rejected);
        }
    };
    request.bootstrap_token.zeroize_now();
    let bootstrap_hash = bootstrap.verification_hash(TokenDomain::Bootstrap);
    let bootstrap_hash = BootstrapTokenHash::from_digest(*bootstrap_hash.as_bytes());
    match databases
        .state()
        .preflight_bootstrap_claim(&bootstrap_hash, now)
        .map_err(|_| ())?
    {
        BootstrapPreflightOutcome::Match => {}
        BootstrapPreflightOutcome::OwnerAlreadyExists => {
            return Ok(SetupWorkerOutcome::Conflict);
        }
        BootstrapPreflightOutcome::Rejected => return Ok(SetupWorkerOutcome::Rejected),
    }
    let login = match LoginName::parse(&request.login_name) {
        Ok(login) => login,
        Err(_) => return Ok(SetupWorkerOutcome::Rejected),
    };
    let display = match DisplayName::parse(&request.display_name) {
        Ok(display) => display,
        Err(_) => return Ok(SetupWorkerOutcome::Rejected),
    };
    let password_candidate = request.password.take();
    let password =
        match validate_password(password_candidate, &PasswordContext::new(&login, &display)) {
            Ok(password) => password,
            Err(_) => return Ok(SetupWorkerOutcome::Rejected),
        };
    let password_phc = password_hasher(&password).map_err(|_| ())?;
    let state_password_phc =
        StatePasswordPhc::new(password_phc.as_str().to_owned()).map_err(|_| ())?;
    let session_token = OpaqueToken::generate().map_err(|_| ())?;
    let csrf_token = OpaqueToken::generate().map_err(|_| ())?;
    let session_hash = session_token.verification_hash(TokenDomain::Session);
    let csrf_hash = csrf_token.verification_hash(TokenDomain::Csrf);
    let session_hash = SessionTokenHash::from_digest(*session_hash.as_bytes());
    let csrf_hash = CsrfTokenHash::from_digest(*csrf_hash.as_bytes());
    let outcome = databases
        .state()
        .claim_owner(OwnerClaimInput {
            bootstrap_hash: &bootstrap_hash,
            login_name: login.as_str(),
            display_name: display.as_str(),
            password_phc: &state_password_phc,
            password_policy_version: i64::from(PASSWORD_POLICY_VERSION),
            session_hash: &session_hash,
            csrf_hash: &csrf_hash,
            now_unix_ms: now,
        })
        .map_err(|_| ())?;
    match outcome {
        OwnerClaimOutcome::Claimed { .. } => {
            let authenticated = databases
                .state()
                .authenticate_session(SessionAuthenticationInput {
                    session_hash: &session_hash,
                    authorization: SessionAuthorization::Authenticated,
                    csrf: CsrfRequirement::NotRequired,
                    now_unix_ms: now,
                })
                .map_err(|_| ())?
                .ok_or(())?;
            Ok(SetupWorkerOutcome::Claimed(Box::new(SessionIssue {
                session_token,
                csrf_token,
                authenticated,
            })))
        }
        OwnerClaimOutcome::Rejected(OwnerClaimRejection::OwnerAlreadyExists) => {
            Ok(SetupWorkerOutcome::Conflict)
        }
        OwnerClaimOutcome::Rejected(
            OwnerClaimRejection::NoActiveBootstrap
            | OwnerClaimRejection::BootstrapExpired
            | OwnerClaimRejection::BootstrapMismatch,
        ) => Ok(SetupWorkerOutcome::Rejected),
    }
}

async fn login(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_post_headers(&headers)?;
    let Json(request) = body.map_err(map_json_rejection)?;
    let reservation = state
        .attempt_limiter
        .reserve_login(peer.ip(), &request.login_name)
        .ok_or(ApiError::AttemptRateLimited)?;
    let permit = Arc::clone(&state.password_workers)
        .try_acquire_owned()
        .map_err(|_| ApiError::PasswordWorkersBusy)?;
    let blocking_guard = state.blocking_tasks.start();
    let databases = Arc::clone(&state.databases);
    let dummy_password_phc = Arc::clone(&state.dummy_password_phc);

    let outcome = tokio::task::spawn_blocking(move || {
        let _blocking_guard = blocking_guard;
        let _permit = permit;
        let outcome = perform_login(databases.as_ref(), dummy_password_phc.as_ref(), request);
        if consumes_login_failure_budget(&outcome) {
            reservation.consume_failure();
        }
        outcome
    })
    .await
    .map_err(|_| {
        tracing::error!("login worker failed");
        ApiError::ServiceUnavailable
    })?
    .map_err(|()| {
        tracing::error!("login could not complete");
        ApiError::ServiceUnavailable
    })?;

    match outcome {
        LoginWorkerOutcome::Authenticated(issue) => session_issue_response(StatusCode::OK, *issue),
        LoginWorkerOutcome::MaintenanceRequired => Err(ApiError::SessionMaintenance),
        LoginWorkerOutcome::Rejected => Err(ApiError::LoginRejected),
    }
}

fn perform_login(
    databases: &DatabaseSet,
    dummy_password_phc: &str,
    mut request: LoginRequest,
) -> Result<LoginWorkerOutcome, ()> {
    let login = LoginName::parse(&request.login_name).ok();
    let lookup_name = login.as_ref().map_or(DUMMY_LOGIN, |login| login.as_str());
    let now = now_unix_ms();
    let credential = databases
        .state()
        .credential_by_login(lookup_name, now)
        .map_err(|_| ())?;
    let password_candidate = request.password.take();
    let input = normalize_password_for_verification(password_candidate).ok();
    let active_and_current = credential.as_ref().is_some_and(|credential| {
        credential.status == UserStatus::Active && credential.login_not_before_unix_ms <= now
    });

    let verified = if login.is_some() && active_and_current {
        let Some(active_credential) = credential.as_ref() else {
            return Err(());
        };
        match input.as_ref() {
            Some(input) => match verify_password(
                input,
                active_credential.password_phc.expose_for_verification(),
            ) {
                Ok(verified) => verified,
                Err(_) => {
                    verify_dummy_password(dummy_password_phc)?;
                    false
                }
            },
            None => {
                verify_dummy_password(dummy_password_phc)?;
                false
            }
        }
    } else {
        verify_dummy_password(dummy_password_phc)?;
        false
    };

    if !verified {
        if login.is_some()
            && active_and_current
            && let Some(credential) = credential.as_ref()
        {
            databases
                .state()
                .record_failed_login(&credential.user_id, now)
                .map_err(|_| ())?;
        }
        return Ok(LoginWorkerOutcome::Rejected);
    }

    let (Some(credential), Some(input)) = (credential, input) else {
        return Err(());
    };
    let stored_policy_version =
        u32::try_from(credential.password_policy_version).map_err(|_| ())?;
    let replacement_password = if password_needs_rehash(
        credential.password_phc.expose_for_verification(),
        stored_policy_version,
    )
    .map_err(|_| ())?
    {
        let replacement = rehash_verified_password(&input).map_err(|_| ())?;
        Some(StatePasswordPhc::new(replacement.as_str().to_owned()).map_err(|_| ())?)
    } else {
        None
    };
    let session_token = OpaqueToken::generate().map_err(|_| ())?;
    let csrf_token = OpaqueToken::generate().map_err(|_| ())?;
    let session_hash = session_token.verification_hash(TokenDomain::Session);
    let csrf_hash = csrf_token.verification_hash(TokenDomain::Csrf);
    let session_hash = SessionTokenHash::from_digest(*session_hash.as_bytes());
    let csrf_hash = CsrfTokenHash::from_digest(*csrf_hash.as_bytes());
    let rehash = replacement_password
        .as_ref()
        .map(|replacement| PasswordRehash {
            replacement_password_phc: replacement,
            replacement_password_policy_version: i64::from(PASSWORD_POLICY_VERSION),
        });
    let created = databases
        .state()
        .create_session_after_verified_login(SessionCreateInput {
            user_id: &credential.user_id,
            expected_auth_version: credential.auth_version,
            expected_password_phc: &credential.password_phc,
            expected_password_policy_version: credential.password_policy_version,
            rehash,
            session_hash: &session_hash,
            csrf_hash: &csrf_hash,
            now_unix_ms: now,
        })
        .map_err(|_| ())?;
    match created {
        SessionCreateOutcome::Created { .. } => {}
        SessionCreateOutcome::MaintenanceRequired { .. } => {
            return Ok(LoginWorkerOutcome::MaintenanceRequired);
        }
        SessionCreateOutcome::Delayed { .. }
        | SessionCreateOutcome::CredentialChangedOrUnavailable => {
            return Ok(LoginWorkerOutcome::Rejected);
        }
    }
    let authenticated = databases
        .state()
        .authenticate_session(SessionAuthenticationInput {
            session_hash: &session_hash,
            authorization: SessionAuthorization::Authenticated,
            csrf: CsrfRequirement::NotRequired,
            now_unix_ms: now,
        })
        .map_err(|_| ())?
        .ok_or(())?;
    Ok(LoginWorkerOutcome::Authenticated(Box::new(SessionIssue {
        session_token,
        csrf_token,
        authenticated,
    })))
}

fn verify_dummy_password(dummy_password_phc: &str) -> Result<(), ()> {
    let input = normalize_password_for_verification(DUMMY_PASSWORD.to_owned()).map_err(|_| ())?;
    let _ = verify_password(&input, dummy_password_phc).map_err(|_| ())?;
    Ok(())
}

async fn me(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let authenticated =
        authenticate(&state, &headers, SessionAuthorization::Authenticated, true).await?;
    let expires_at_unix_ms = authenticated.absolute_expires_at_unix_ms;
    let session_expires = authenticated.session_expires;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(MeResponse {
            user: authenticated.into(),
            expires_at_unix_ms,
            session_expires,
        }),
    ))
}

async fn update_account(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<AccountUpdateRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_post_headers(&headers)?;
    let authenticated = require_capability(&state, &headers, "users.manage").await?;
    let Json(request) = body.map_err(map_json_rejection)?;
    let session_hash = session_hash_from_headers(&headers)?;
    let reservation = state
        .attempt_limiter
        .reserve_account_update(peer.ip(), &authenticated.user_id, &session_hash)
        .ok_or(ApiError::AttemptRateLimited)?;
    let permit = Arc::clone(&state.password_workers)
        .try_acquire_owned()
        .map_err(|_| ApiError::PasswordWorkersBusy)?;
    let blocking_guard = state.blocking_tasks.start();
    let databases = Arc::clone(&state.databases);
    let dummy_password_phc = Arc::clone(&state.dummy_password_phc);
    let audit_user_id = authenticated.user_id.clone();

    let outcome = tokio::task::spawn_blocking(move || {
        let _blocking_guard = blocking_guard;
        let _permit = permit;
        let outcome = perform_account_update(
            databases.as_ref(),
            dummy_password_phc.as_ref(),
            authenticated,
            request,
        );
        if matches!(&outcome, Ok(AccountWorkerOutcome::CurrentPasswordRejected)) {
            reservation.consume_failure();
            databases
                .state()
                .record_owner_account_password_rejection(&audit_user_id, now_unix_ms())
                .map_err(|_| ())?;
        }
        outcome
    })
    .await
    .map_err(|_| {
        tracing::error!("account update worker failed");
        ApiError::ServiceUnavailable
    })?
    .map_err(|()| {
        tracing::error!("account update could not complete");
        ApiError::ServiceUnavailable
    })?;

    match outcome {
        AccountWorkerOutcome::Updated => {
            let mut response = StatusCode::NO_CONTENT.into_response();
            response
                .headers_mut()
                .insert(header::SET_COOKIE, clear_session_cookie());
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(response)
        }
        AccountWorkerOutcome::CurrentPasswordRejected => Err(ApiError::CurrentPasswordRejected),
        AccountWorkerOutcome::LoginNameUnavailable => Err(ApiError::AccountConflict),
        AccountWorkerOutcome::InvalidReplacement => Err(ApiError::AccountRejected),
        AccountWorkerOutcome::CredentialChangedOrUnavailable => {
            Err(ApiError::AuthenticationRequired)
        }
    }
}

fn perform_account_update(
    databases: &DatabaseSet,
    dummy_password_phc: &str,
    authenticated: AuthenticatedSession,
    mut request: AccountUpdateRequest,
) -> Result<AccountWorkerOutcome, ()> {
    let now = now_unix_ms();
    let credential = databases
        .state()
        .credential_by_login(&authenticated.login_name, now)
        .map_err(|_| ())?;
    let Some(credential) = credential.filter(|credential| {
        credential.user_id == authenticated.user_id
            && credential.auth_version == authenticated.auth_version
            && credential.status == UserStatus::Active
    }) else {
        verify_dummy_password(dummy_password_phc)?;
        return Ok(AccountWorkerOutcome::CredentialChangedOrUnavailable);
    };

    let current_password = request.current_password.take();
    let normalized = normalize_password_for_verification(current_password).ok();
    let current_password_matches = match normalized.as_ref() {
        Some(password) => {
            verify_password(password, credential.password_phc.expose_for_verification())
                .unwrap_or(false)
        }
        None => {
            verify_dummy_password(dummy_password_phc)?;
            false
        }
    };
    if !current_password_matches {
        return Ok(AccountWorkerOutcome::CurrentPasswordRejected);
    }

    let login = match LoginName::parse(&request.login_name) {
        Ok(login) => login,
        Err(_) => return Ok(AccountWorkerOutcome::InvalidReplacement),
    };
    let display = match DisplayName::parse(&request.display_name) {
        Ok(display) => display,
        Err(_) => return Ok(AccountWorkerOutcome::InvalidReplacement),
    };
    let identity_changed = login.as_str() != authenticated.login_name
        || display.as_str() != authenticated.display_name;
    let replacement_password = match request.new_password.as_mut() {
        Some(candidate) => {
            let candidate = candidate.take();
            let validated =
                match validate_password(candidate, &PasswordContext::new(&login, &display)) {
                    Ok(password) => password,
                    Err(_) => return Ok(AccountWorkerOutcome::InvalidReplacement),
                };
            let hashed = hash_password(&validated).map_err(|_| ())?;
            Some(StatePasswordPhc::new(hashed.as_str().to_owned()).map_err(|_| ())?)
        }
        None => {
            if identity_changed
                && validate_verified_password_for_context(
                    normalized
                        .as_ref()
                        .expect("a matching password must have normalized successfully"),
                    &PasswordContext::new(&login, &display),
                )
                .is_err()
            {
                return Ok(AccountWorkerOutcome::InvalidReplacement);
            }
            None
        }
    };
    let replacement = replacement_password
        .as_ref()
        .map(|replacement| PasswordRehash {
            replacement_password_phc: replacement,
            replacement_password_policy_version: i64::from(PASSWORD_POLICY_VERSION),
        });
    let outcome = databases
        .state()
        .update_owner_account(OwnerAccountUpdateInput {
            user_id: &authenticated.user_id,
            expected_auth_version: credential.auth_version,
            expected_password_phc: &credential.password_phc,
            expected_password_policy_version: credential.password_policy_version,
            login_name: login.as_str(),
            display_name: display.as_str(),
            replacement_password: replacement,
            now_unix_ms: now,
        })
        .map_err(|_| ())?;
    Ok(match outcome {
        OwnerAccountUpdateOutcome::Updated => AccountWorkerOutcome::Updated,
        OwnerAccountUpdateOutcome::LoginNameUnavailable => {
            AccountWorkerOutcome::LoginNameUnavailable
        }
        OwnerAccountUpdateOutcome::CredentialChangedOrUnavailable => {
            AccountWorkerOutcome::CredentialChangedOrUnavailable
        }
    })
}

async fn session_expiry(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let authenticated =
        authenticate(&state, &headers, SessionAuthorization::Authenticated, true).await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(SessionExpiryUpdateResponse {
            expires: authenticated.session_expires,
            expires_at_unix_ms: authenticated.absolute_expires_at_unix_ms,
        }),
    ))
}

async fn update_session_expiry(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<SessionExpiryUpdateRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_post_headers(&headers)?;
    let _authenticated = require_capability(&state, &headers, "users.manage").await?;
    let Json(request) = body.map_err(map_json_rejection)?;
    let encoded = parse_session_cookie(&headers)?;
    let session_hash = session_hash_from_headers(&headers)?;
    let databases = Arc::clone(&state.databases);
    let now = now_unix_ms();
    let outcome = run_blocking_state(&state.blocking_tasks, move || {
        databases
            .state()
            .set_session_expiry(&session_hash, request.expires, now)
    })
    .await?;
    let Some(outcome) = outcome else {
        return Err(ApiError::AuthenticationRequired);
    };
    let mut response = (
        [(header::CACHE_CONTROL, "no-store")],
        Json(SessionExpiryUpdateResponse {
            expires: outcome.expires,
            expires_at_unix_ms: outcome.absolute_expires_at_unix_ms,
        }),
    )
        .into_response();
    let cookie = session_cookie(encoded, outcome.expires)?;
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

async fn rotate_csrf(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<EmptyRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_post_headers(&headers)?;
    let Json(EmptyRequest {}) = body.map_err(map_json_rejection)?;
    authenticate(&state, &headers, SessionAuthorization::Authenticated, true).await?;
    let session_hash = session_hash_from_headers(&headers)?;
    let expected_csrf_hash = csrf_hash_from_headers(&headers)?;
    let csrf_token = OpaqueToken::generate().map_err(|_| ApiError::ServiceUnavailable)?;
    let csrf_hash = csrf_token.verification_hash(TokenDomain::Csrf);
    let csrf_hash = CsrfTokenHash::from_digest(*csrf_hash.as_bytes());
    let databases = Arc::clone(&state.databases);
    let diagnostic_hash = session_hash.clone();
    let now = now_unix_ms();
    let rotated = run_blocking_state(&state.blocking_tasks, move || {
        databases
            .state()
            .rotate_session_csrf(&session_hash, &expected_csrf_hash, &csrf_hash, now)
    })
    .await?;
    if !rotated {
        let databases = Arc::clone(&state.databases);
        let current = run_blocking_state(&state.blocking_tasks, move || {
            databases
                .state()
                .session_is_current_without_touch(&diagnostic_hash, now)
        })
        .await?;
        return Err(if current {
            ApiError::CsrfRejected
        } else {
            ApiError::AuthenticationRequired
        });
    }

    let encoded = csrf_token.encode();
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(CsrfResponse {
            csrf_token: encoded.expose_secret(),
        }),
    )
        .into_response())
}

async fn logout(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<EmptyRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_post_headers(&headers)?;
    let Json(EmptyRequest {}) = body.map_err(map_json_rejection)?;
    let authenticated =
        authenticate(&state, &headers, SessionAuthorization::Authenticated, true).await?;
    let session_hash = session_hash_from_headers(&headers)?;
    let actor_user_id = authenticated.user_id;
    let databases = Arc::clone(&state.databases);
    let now = now_unix_ms();
    let revoked = run_blocking_state(&state.blocking_tasks, move || {
        databases
            .state()
            .revoke_session(&session_hash, Some(&actor_user_id), now)
    })
    .await?;
    if !revoked {
        return Err(ApiError::AuthenticationRequired);
    }

    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, clear_session_cookie());
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

pub(crate) async fn require_capability(
    state: &ApiState,
    headers: &HeaderMap,
    capability: &'static str,
) -> Result<AuthenticatedSession, ApiError> {
    authenticate(
        state,
        headers,
        SessionAuthorization::RequireCapability(capability),
        true,
    )
    .await
}

pub(crate) async fn require_capability_without_csrf(
    state: &ApiState,
    headers: &HeaderMap,
    capability: &'static str,
) -> Result<AuthenticatedSession, ApiError> {
    authenticate(
        state,
        headers,
        SessionAuthorization::RequireCapability(capability),
        false,
    )
    .await
}

pub(crate) async fn authorize_terminal_with_current_password(
    state: &ApiState,
    peer: IpAddr,
    headers: &HeaderMap,
    mut current_password: SecretString,
) -> Result<AuthenticatedSession, ApiError> {
    let authenticated = require_capability(state, headers, "terminal.open").await?;
    let session_hash = session_hash_from_headers(headers)?;
    let reservation = state
        .attempt_limiter
        .reserve_terminal_proof(peer, &authenticated.user_id, &session_hash)
        .ok_or(ApiError::AttemptRateLimited)?;
    let permit = Arc::clone(&state.password_workers)
        .try_acquire_owned()
        .map_err(|_| ApiError::PasswordWorkersBusy)?;
    let blocking_guard = state.blocking_tasks.start();
    let databases = Arc::clone(&state.databases);
    let dummy_password_phc = Arc::clone(&state.dummy_password_phc);
    let audit_user_id = authenticated.user_id.clone();

    let outcome = tokio::task::spawn_blocking(move || {
        let _blocking_guard = blocking_guard;
        let _permit = permit;
        let now = now_unix_ms();
        let credential = databases
            .state()
            .credential_by_login(&authenticated.login_name, now)
            .map_err(|_| ())?;
        let Some(credential) = credential.filter(|credential| {
            credential.user_id == authenticated.user_id
                && credential.auth_version == authenticated.auth_version
                && credential.status == UserStatus::Active
        }) else {
            verify_dummy_password(dummy_password_phc.as_ref())?;
            return Ok(TerminalProofOutcome::CredentialChangedOrUnavailable);
        };
        let candidate = current_password.take();
        let normalized = normalize_password_for_verification(candidate).ok();
        let matches = match normalized.as_ref() {
            Some(password) => {
                verify_password(password, credential.password_phc.expose_for_verification())
                    .unwrap_or(false)
            }
            None => {
                verify_dummy_password(dummy_password_phc.as_ref())?;
                false
            }
        };
        if matches {
            Ok(TerminalProofOutcome::Verified(authenticated))
        } else {
            reservation.consume_failure();
            databases
                .state()
                .record_terminal_audit(
                    &audit_user_id,
                    helix_state::TerminalAuditEvent::PasswordRejected,
                    now,
                )
                .map_err(|_| ())?;
            Ok(TerminalProofOutcome::Rejected)
        }
    })
    .await
    .map_err(|_| {
        tracing::error!("terminal password worker failed");
        ApiError::ServiceUnavailable
    })?
    .map_err(|()| {
        tracing::error!("terminal password proof could not complete");
        ApiError::ServiceUnavailable
    })?;

    match outcome {
        TerminalProofOutcome::Verified(authenticated) => Ok(authenticated),
        TerminalProofOutcome::Rejected => Err(ApiError::CurrentPasswordRejected),
        TerminalProofOutcome::CredentialChangedOrUnavailable => {
            Err(ApiError::AuthenticationRequired)
        }
    }
}

async fn authenticate(
    state: &ApiState,
    headers: &HeaderMap,
    authorization: SessionAuthorization<'static>,
    require_csrf: bool,
) -> Result<AuthenticatedSession, ApiError> {
    let session_hash = session_hash_from_headers(headers)?;
    let now = now_unix_ms();
    // Browser cookies are shared across ports on a loopback host. Protected
    // responses therefore require a second proof held only in frontend memory.
    let csrf = if require_csrf {
        match csrf_hash_from_headers(headers) {
            Ok(csrf) => Some(csrf),
            Err(_) => {
                let diagnostic_hash = session_hash.clone();
                let databases = Arc::clone(&state.databases);
                let current = run_blocking_state(&state.blocking_tasks, move || {
                    databases
                        .state()
                        .session_is_current_without_touch(&diagnostic_hash, now)
                })
                .await?;
                return Err(if current {
                    ApiError::CsrfRejected
                } else {
                    ApiError::AuthenticationRequired
                });
            }
        }
    } else {
        None
    };
    let csrf_was_provided = csrf.is_some();
    let checked_hash = session_hash.clone();
    let databases = Arc::clone(&state.databases);
    let authenticated = run_blocking_state(&state.blocking_tasks, move || {
        let requirement = csrf
            .as_ref()
            .map_or(CsrfRequirement::NotRequired, CsrfRequirement::Match);
        databases
            .state()
            .authenticate_session(SessionAuthenticationInput {
                session_hash: &checked_hash,
                authorization: SessionAuthorization::Authenticated,
                csrf: requirement,
                now_unix_ms: now,
            })
    })
    .await?;

    let authenticated = match authenticated {
        Some(authenticated) if !require_csrf || csrf_was_provided => authenticated,
        Some(_) => return Err(ApiError::CsrfRejected),
        None if require_csrf && csrf_was_provided => {
            // A failed proof may also be an expired or revoked session. Perform
            // the proof-free lookup only on this failure path so successful
            // protected requests use one serialized state operation.
            let databases = Arc::clone(&state.databases);
            let still_authenticated = run_blocking_state(&state.blocking_tasks, move || {
                databases
                    .state()
                    .session_is_current_without_touch(&session_hash, now)
            })
            .await?;
            return Err(if still_authenticated {
                ApiError::CsrfRejected
            } else {
                ApiError::AuthenticationRequired
            });
        }
        None => return Err(ApiError::AuthenticationRequired),
    };

    match authorization {
        SessionAuthorization::Authenticated => Ok(authenticated),
        SessionAuthorization::RequireCapability(capability)
            if authenticated
                .capabilities
                .iter()
                .any(|granted| granted == capability) =>
        {
            Ok(authenticated)
        }
        SessionAuthorization::RequireCapability(_) => Err(ApiError::AuthorizationDenied),
    }
}

pub(crate) fn session_hash_from_headers(headers: &HeaderMap) -> Result<SessionTokenHash, ApiError> {
    let encoded = parse_session_cookie(headers)?;
    let token = OpaqueToken::from_encoded(encoded).map_err(|_| ApiError::AuthenticationRequired)?;
    let hash = token.verification_hash(TokenDomain::Session);
    Ok(SessionTokenHash::from_digest(*hash.as_bytes()))
}

fn csrf_hash_from_headers(headers: &HeaderMap) -> Result<CsrfTokenHash, ApiError> {
    let mut values = headers.get_all(CSRF_HEADER).iter();
    let encoded = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::CsrfRejected)?;
    if values.next().is_some() {
        return Err(ApiError::CsrfRejected);
    }
    let token = OpaqueToken::from_encoded(encoded).map_err(|_| ApiError::CsrfRejected)?;
    let hash = token.verification_hash(TokenDomain::Csrf);
    Ok(CsrfTokenHash::from_digest(*hash.as_bytes()))
}

fn parse_session_cookie(headers: &HeaderMap) -> Result<&str, ApiError> {
    parse_named_cookie(headers, SESSION_COOKIE_NAME).map_err(|()| ApiError::AuthenticationRequired)
}

pub(crate) fn parse_named_cookie<'a>(
    headers: &'a HeaderMap,
    target_name: &str,
) -> Result<&'a str, ()> {
    let mut header_values = headers.get_all(header::COOKIE).iter();
    let header_value = header_values.next().ok_or(())?;
    if header_values.next().is_some() {
        return Err(());
    }
    let cookie = header_value.to_str().map_err(|_| ())?;
    let mut selected = None;
    for (index, raw_pair) in cookie.split(';').enumerate() {
        let pair = if index == 0 {
            raw_pair
        } else {
            raw_pair.trim_start_matches([' ', '\t'])
        };
        if pair.is_empty() {
            return Err(());
        }
        let (name, value) = pair.split_once('=').ok_or(())?;
        if !valid_cookie_name(name) || !valid_cookie_value(value) {
            return Err(());
        }
        if name == target_name && (value.is_empty() || selected.replace(value).is_some()) {
            return Err(());
        }
    }
    selected.ok_or(())
}

fn valid_cookie_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn valid_cookie_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| matches!(byte, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e))
}

pub(crate) fn validate_post_headers(headers: &HeaderMap) -> Result<(), ApiError> {
    let content_type =
        single_header(headers, header::CONTENT_TYPE).ok_or(ApiError::UnsupportedMediaType)?;
    let media_type =
        mime::Mime::from_str(content_type).map_err(|_| ApiError::UnsupportedMediaType)?;
    if media_type.type_() != mime::APPLICATION || media_type.subtype() != mime::JSON {
        return Err(ApiError::UnsupportedMediaType);
    }

    validate_same_origin_headers(headers)
}

pub(crate) fn validate_same_origin_headers(headers: &HeaderMap) -> Result<(), ApiError> {
    let host = single_header(headers, header::HOST).ok_or(ApiError::InvalidHost)?;
    let origin = single_header(headers, header::ORIGIN).ok_or(ApiError::InvalidOrigin)?;
    if origin != format!("http://{host}") {
        return Err(ApiError::InvalidOrigin);
    }

    let mut fetch_site = headers.get_all("sec-fetch-site").iter();
    if let Some(value) = fetch_site.next() {
        let value = value.to_str().map_err(|_| ApiError::CrossSiteRequest)?;
        if fetch_site.next().is_some() || value.eq_ignore_ascii_case("cross-site") {
            return Err(ApiError::CrossSiteRequest);
        }
    }
    Ok(())
}

pub(crate) fn map_json_rejection(rejection: JsonRejection) -> ApiError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge
    } else {
        ApiError::InvalidJson
    }
}

fn single_header(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

pub(crate) async fn run_blocking_state<T, F>(
    tracker: &BlockingTaskTracker,
    operation: F,
) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, helix_state::StateError> + Send + 'static,
{
    let guard = tracker.start();
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        operation()
    })
    .await
    .map_err(|_| {
        tracing::error!("state worker failed");
        ApiError::ServiceUnavailable
    })?
    .map_err(|_| {
        tracing::error!("state operation failed");
        ApiError::ServiceUnavailable
    })
}

fn session_issue_response(status: StatusCode, issue: SessionIssue) -> Result<Response, ApiError> {
    let session = issue.session_token.encode();
    let csrf = issue.csrf_token.encode();
    let expires_at_unix_ms = issue.authenticated.absolute_expires_at_unix_ms;
    let session_expires = issue.authenticated.session_expires;
    let payload = AuthSuccessResponse {
        user: issue.authenticated.into(),
        csrf_token: csrf.expose_secret(),
        expires_at_unix_ms,
        session_expires,
    };
    let mut response = (status, Json(payload)).into_response();
    let cookie = session_cookie(session.expose_secret(), session_expires)?;
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn session_cookie_max_age_seconds(expires: bool) -> i64 {
    if expires {
        SESSION_COOKIE_EXPIRING_MAX_AGE_SECONDS
    } else {
        SESSION_COOKIE_PERSISTENT_MAX_AGE_SECONDS
    }
}

fn session_cookie(encoded: &str, expires: bool) -> Result<HeaderValue, ApiError> {
    // Helix currently serves loopback HTTP only. `Secure` must be added when a
    // reviewed HTTPS boundary exists; no Domain attribute keeps this host-only.
    let max_age = session_cookie_max_age_seconds(expires);
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE_NAME}={encoded}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}"
    ))
    .map_err(|_| ApiError::ServiceUnavailable)
}

pub(crate) fn clear_session_cookie() -> HeaderValue {
    HeaderValue::from_static("helix_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
}

fn now_unix_ms() -> i64 {
    i64::try_from(helix_core::unix_timestamp_ms()).unwrap_or(i64::MAX)
}

impl From<AuthenticatedSession> for AuthUser {
    fn from(session: AuthenticatedSession) -> Self {
        Self {
            id: session.user_id,
            login_name: session.login_name,
            display_name: session.display_name,
            capabilities: session.capabilities,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_origin_header_gate_rejects_other_ports_and_cross_site_fetches() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("192.0.2.10:3100"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://192.0.2.10:3100"),
        );
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(validate_same_origin_headers(&headers).is_ok());

        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://192.0.2.10:8080"),
        );
        assert!(matches!(
            validate_same_origin_headers(&headers),
            Err(ApiError::InvalidOrigin)
        ));

        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://192.0.2.10:3100"),
        );
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(matches!(
            validate_same_origin_headers(&headers),
            Err(ApiError::CrossSiteRequest)
        ));
    }

    #[test]
    fn rate_map_is_hard_bounded_and_expired_entries_are_evicted() {
        let limiter = AttemptLimiter::with_limits(3, Duration::from_secs(60), 2, 2, 2, 2, 2);
        limiter
            .reserve_setup(IpAddr::from([127, 0, 0, 1]))
            .expect("first setup attempt")
            .consume_failure();
        limiter
            .reserve_setup(IpAddr::from([127, 0, 0, 2]))
            .expect("second setup attempt")
            .consume_failure();
        assert!(
            limiter
                .reserve_setup(IpAddr::from([127, 0, 0, 3]))
                .is_none()
        );
        assert_eq!(limiter.entry_count(), 3);

        let expiring = AttemptLimiter::with_limits(2, Duration::from_secs(60), 1, 1, 1, 1, 1);
        let start = Instant::now();
        assert!(expiring.allow_setup_at(IpAddr::from([127, 0, 0, 1]), start));
        assert!(expiring.allow_setup_at(
            IpAddr::from([127, 0, 0, 2]),
            start + Duration::from_secs(60)
        ));
        assert_eq!(expiring.entry_count(), 2);
    }

    #[test]
    fn login_budget_covers_peer_and_canonical_account_keys() {
        let limiter = AttemptLimiter::with_limits(8, Duration::from_secs(60), 2, 2, 2, 2, 2);
        let first_peer = IpAddr::from([127, 0, 0, 1]);
        let second_peer = IpAddr::from([127, 0, 0, 2]);
        limiter
            .reserve_login(first_peer, "owner")
            .expect("first attempt")
            .consume_failure();
        limiter
            .reserve_login(first_peer, "owner")
            .expect("second attempt")
            .consume_failure();
        assert!(limiter.reserve_login(first_peer, "another").is_none());
        assert!(limiter.reserve_login(second_peer, "owner").is_none());
    }

    #[test]
    fn successful_login_reservations_refund_every_budget() {
        let limiter = AttemptLimiter::with_limits(8, Duration::from_secs(60), 2, 2, 2, 2, 2);
        let peer = IpAddr::from([127, 0, 0, 1]);
        for _ in 0..10 {
            drop(
                limiter
                    .reserve_login(peer, "owner")
                    .expect("refunded attempt remains available"),
            );
        }
        assert_eq!(limiter.entry_count(), 0);
    }

    #[test]
    fn account_update_budget_is_user_scoped_and_refunds_non_proof_outcomes() {
        let limiter = AttemptLimiter::production();
        let peer = IpAddr::from([127, 0, 0, 1]);
        let session = SessionTokenHash::from_digest([1; 32]);
        for _ in 0..ACCOUNT_UPDATE_ATTEMPTS_PER_WINDOW {
            limiter
                .reserve_account_update(peer, "owner-user", &session)
                .expect("attempt within the account-update budget")
                .consume_failure();
        }
        assert!(
            limiter
                .reserve_account_update(
                    IpAddr::from([127, 0, 0, 2]),
                    "owner-user",
                    &SessionTokenHash::from_digest([2; 32]),
                )
                .is_none(),
            "peer or session cycling must not bypass the authenticated-user budget"
        );

        let refundable = AttemptLimiter::production();
        for _ in 0..(ACCOUNT_UPDATE_ATTEMPTS_PER_WINDOW * 2) {
            drop(
                refundable
                    .reserve_account_update(peer, "owner-user", &session)
                    .expect("non-proof outcomes refund the reservation"),
            );
        }
        assert_eq!(refundable.entry_count(), 0);
    }

    #[test]
    fn preference_write_budget_allows_normal_debounce_and_bounds_user_fanout() {
        const { assert!(PREFERENCE_SESSION_WRITES_PER_WINDOW >= 86) };
        let limiter = AttemptLimiter::production();
        let first_peer = IpAddr::from([127, 0, 0, 1]);
        let first_session = SessionTokenHash::from_digest([3; 32]);
        for _ in 0..PREFERENCE_SESSION_WRITES_PER_WINDOW {
            assert!(limiter.allow_preference_write(first_peer, "owner-user", &first_session,));
        }
        assert!(!limiter.allow_preference_write(first_peer, "owner-user", &first_session,));

        let second_peer = IpAddr::from([127, 0, 0, 2]);
        let second_session = SessionTokenHash::from_digest([4; 32]);
        for _ in 0..PREFERENCE_SESSION_WRITES_PER_WINDOW {
            assert!(limiter.allow_preference_write(second_peer, "owner-user", &second_session,));
        }
        assert!(!limiter.allow_preference_write(
            IpAddr::from([127, 0, 0, 3]),
            "owner-user",
            &SessionTokenHash::from_digest([5; 32]),
        ));
    }

    #[test]
    fn storage_analysis_pressure_is_bounded_per_session_and_authenticated_user() {
        let limiter = AttemptLimiter::production();
        let first_peer = IpAddr::from([127, 0, 0, 1]);
        let first_session = SessionTokenHash::from_digest([6; 32]);
        for _ in 0..STORAGE_ANALYSIS_SESSION_STARTS_PER_WINDOW {
            assert!(
                limiter.allow_storage_analysis_start(first_peer, "owner-user", &first_session,)
            );
        }
        assert!(!limiter.allow_storage_analysis_start(first_peer, "owner-user", &first_session,));

        let second_peer = IpAddr::from([127, 0, 0, 2]);
        let second_session = SessionTokenHash::from_digest([7; 32]);
        for _ in STORAGE_ANALYSIS_SESSION_STARTS_PER_WINDOW..STORAGE_ANALYSIS_USER_STARTS_PER_WINDOW
        {
            assert!(limiter.allow_storage_analysis_start(
                second_peer,
                "owner-user",
                &second_session,
            ));
        }
        assert!(!limiter.allow_storage_analysis_start(
            IpAddr::from([127, 0, 0, 3]),
            "owner-user",
            &SessionTokenHash::from_digest([8; 32]),
        ));

        let reads = AttemptLimiter::production();
        for _ in 0..STORAGE_ANALYSIS_SESSION_READS_PER_WINDOW {
            assert!(reads.allow_storage_analysis_read(first_peer, "owner-user", &first_session,));
        }
        assert!(!reads.allow_storage_analysis_read(first_peer, "owner-user", &first_session,));
    }

    #[test]
    fn session_maintenance_does_not_consume_a_login_failure_budget() {
        assert!(!consumes_login_failure_budget(&Ok(
            LoginWorkerOutcome::MaintenanceRequired
        )));
        assert!(consumes_login_failure_budget(&Ok(
            LoginWorkerOutcome::Rejected
        )));
    }

    #[test]
    fn setup_global_budget_spans_peers_and_refunds_non_failures() {
        let limiter = AttemptLimiter::with_limits(16, Duration::from_secs(60), 10, 10, 10, 3, 8);
        for peer_octet in 1..=3 {
            limiter
                .reserve_setup(IpAddr::from([127, 0, 0, peer_octet]))
                .expect("attempt within global setup budget")
                .consume_failure();
        }
        for peer_octet in 4..=64 {
            assert!(
                limiter
                    .reserve_setup(IpAddr::from([127, 0, 0, peer_octet]))
                    .is_none(),
                "peer cycling must not bypass the global setup budget"
            );
        }

        let refundable = AttemptLimiter::with_limits(16, Duration::from_secs(60), 10, 10, 10, 1, 8);
        for peer_octet in 1..=64 {
            drop(
                refundable
                    .reserve_setup(IpAddr::from([127, 0, 0, peer_octet]))
                    .expect("refunded setup reservation remains available"),
            );
        }
        assert_eq!(refundable.entry_count(), 0);
    }

    #[test]
    fn only_rejected_setup_consumes_a_failure_budget() {
        assert!(!consumes_setup_failure_budget(&Ok(
            SetupWorkerOutcome::Conflict
        )));
        assert!(consumes_setup_failure_budget(&Ok(
            SetupWorkerOutcome::Rejected
        )));
        assert!(!consumes_setup_failure_budget(&Err(())));
    }

    #[test]
    fn denied_login_does_not_insert_or_touch_secondary_keys() {
        let limiter = AttemptLimiter::with_limits(4, Duration::from_secs(60), 10, 1, 10, 2, 2);
        let peer = IpAddr::from([127, 0, 0, 1]);
        limiter
            .reserve_login(peer, "owner")
            .expect("first attempt")
            .consume_failure();
        assert_eq!(limiter.entry_count(), 3);

        for suffix in 0..32 {
            assert!(
                limiter
                    .reserve_login(peer, &format!("account-{suffix}"))
                    .is_none()
            );
        }
        assert_eq!(limiter.entry_count(), 3);
    }

    #[test]
    fn installation_global_login_budget_spans_peers_and_accounts() {
        let limiter = AttemptLimiter::with_limits(16, Duration::from_secs(60), 2, 10, 10, 2, 2);
        for (peer, login) in [([127, 0, 0, 1], "owner"), ([127, 0, 0, 2], "other")] {
            limiter
                .reserve_login(IpAddr::from(peer), login)
                .expect("attempt within global budget")
                .consume_failure();
        }
        assert!(
            limiter
                .reserve_login(IpAddr::from([127, 0, 0, 3]), "third")
                .is_none()
        );
    }

    #[test]
    fn mismatched_bootstrap_token_is_rejected_before_password_hashing() {
        let temporary = crate::private_test_directory("temporary state directory");
        let databases = DatabaseSet::open_for_daemon(temporary.path()).expect("open databases");
        let active = OpaqueToken::generate().expect("active bootstrap token");
        let active_hash = active.verification_hash(TokenDomain::Bootstrap);
        let active_hash = BootstrapTokenHash::from_digest(*active_hash.as_bytes());
        let now = now_unix_ms();
        databases
            .state()
            .replace_bootstrap_token(&active_hash, now, now + 60_000)
            .expect("install active bootstrap");
        let mismatched = OpaqueToken::generate().expect("mismatched bootstrap token");
        let mismatched = mismatched.encode();

        let outcome = perform_owner_setup_with_hasher(
            &databases,
            OwnerSetupRequest {
                bootstrap_token: SecretString(mismatched.expose_secret().to_owned()),
                login_name: "owner".to_owned(),
                display_name: "Rique".to_owned(),
                password: SecretString(DUMMY_PASSWORD.to_owned()),
            },
            |_| panic!("a mismatched bootstrap token must not reach Argon2"),
        )
        .expect("setup preflight");
        assert!(matches!(outcome, SetupWorkerOutcome::Rejected));
    }

    #[test]
    fn cookie_parser_rejects_duplicates_and_malformed_entries() {
        let token = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        for cookie in [
            format!("helix_session={token}; helix_session={token}"),
            format!("helix_session={token}; malformed"),
            format!("helix_session={token}; bad name=value"),
            format!("helix_session ={token}"),
            format!("helix_session= {token}"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::COOKIE,
                HeaderValue::from_str(&cookie).expect("cookie"),
            );
            assert!(parse_session_cookie(&headers).is_err(), "cookie {cookie}");
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("theme=; helix_session={token}")).expect("cookie"),
        );
        assert_eq!(parse_session_cookie(&headers).expect("session"), token);

        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=value; helix_session="),
        );
        assert!(parse_session_cookie(&headers).is_err());
    }

    #[tokio::test]
    async fn cancelled_state_request_remains_tracked_until_blocking_work_finishes() {
        let tracker = BlockingTaskTracker::default();
        let request_tracker = tracker.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
        let request = tokio::spawn(async move {
            run_blocking_state(&request_tracker, move || {
                let _ = entered_tx.send(());
                release_rx.recv().expect("release blocking state work");
                Ok(())
            })
            .await
        });

        entered_rx.await.expect("blocking state work started");
        request.abort();
        assert!(
            request
                .await
                .expect_err("request future must be cancelled")
                .is_cancelled()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), tracker.wait_idle())
                .await
                .is_err(),
            "drain completed while detached state work was still active"
        );

        release_tx.send(()).expect("release state work");
        tokio::time::timeout(Duration::from_secs(1), tracker.wait_idle())
            .await
            .expect("blocking task drain completed");
    }
}
