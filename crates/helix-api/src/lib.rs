//! Versioned HTTP API and static frontend composition.

mod auth;
mod static_root;

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header, uri::Authority},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use helix_core::{DatabaseStatus, HealthReport, HealthStatus, VERSION, unix_timestamp_ms};
use helix_state::DatabaseSet;
use helix_system::HostSampler;
use serde::Serialize;
use std::{
    net::IpAddr,
    path::PathBuf,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use tower::ServiceBuilder;
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

pub use static_root::StaticRootError;

const API_BODY_LIMIT_BYTES: usize = 16 * 1024;
const MAX_CONCURRENT_APPLICATION_REQUESTS: usize = 64;

#[derive(Clone)]
pub struct ApiState {
    host: HostSampler,
    metrics_database: DatabaseStatus,
    pub(crate) databases: Arc<DatabaseSet>,
    pub(crate) password_workers: Arc<tokio::sync::Semaphore>,
    application_request_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) dummy_password_phc: Arc<str>,
    pub(crate) attempt_limiter: auth::AttemptLimiter,
    pub(crate) blocking_tasks: BlockingTaskTracker,
    #[cfg(test)]
    overview_test_gate: Option<OverviewTestGate>,
}

impl ApiState {
    pub async fn initialize(
        host: HostSampler,
        metrics_database: DatabaseStatus,
        databases: Arc<DatabaseSet>,
    ) -> Result<Self, ApiInitializationError> {
        let (password_workers, dummy_password_phc) = auth::initialize_password_boundary().await?;
        Ok(Self {
            host,
            metrics_database,
            databases,
            password_workers,
            application_request_slots: Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_APPLICATION_REQUESTS,
            )),
            dummy_password_phc,
            attempt_limiter: auth::AttemptLimiter::production(),
            blocking_tasks: BlockingTaskTracker::default(),
            #[cfg(test)]
            overview_test_gate: None,
        })
    }

    #[cfg(test)]
    fn with_overview_test_gate(mut self, gate: OverviewTestGate) -> Self {
        self.overview_test_gate = Some(gate);
        self
    }

    #[cfg(test)]
    fn with_password_workers(mut self, workers: Arc<tokio::sync::Semaphore>) -> Self {
        self.password_workers = workers;
        self
    }

    #[cfg(test)]
    fn with_application_request_slots(mut self, slots: Arc<tokio::sync::Semaphore>) -> Self {
        self.application_request_slots = slots;
        self
    }

    #[must_use]
    pub fn blocking_task_tracker(&self) -> BlockingTaskTracker {
        self.blocking_tasks.clone()
    }
}

#[derive(Clone, Default)]
pub struct BlockingTaskTracker {
    inner: Arc<BlockingTaskTrackerInner>,
}

#[derive(Default)]
struct BlockingTaskTrackerInner {
    active: AtomicUsize,
    idle: tokio::sync::Notify,
}

pub(crate) struct BlockingTaskGuard {
    inner: Arc<BlockingTaskTrackerInner>,
}

impl BlockingTaskTracker {
    pub(crate) fn start(&self) -> BlockingTaskGuard {
        let previous = self.inner.active.fetch_add(1, Ordering::AcqRel);
        assert!(previous != usize::MAX, "blocking task count overflowed");
        BlockingTaskGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    pub async fn wait_idle(&self) {
        loop {
            let notified = self.inner.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.inner.active.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}

impl Drop for BlockingTaskGuard {
    fn drop(&mut self) {
        if self.inner.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.idle.notify_waiters();
        }
    }
}

#[derive(Debug, Error)]
pub enum ApiInitializationError {
    #[error("password worker initialization failed")]
    PasswordWorkerFailed,
    #[error("password primitive initialization failed")]
    PasswordPrimitiveFailed,
}

#[cfg(test)]
#[derive(Clone)]
struct OverviewTestGate {
    entered: Arc<tokio::sync::Semaphore>,
    release: Arc<tokio::sync::Semaphore>,
}

#[cfg(test)]
impl OverviewTestGate {
    fn new() -> Self {
        Self {
            entered: Arc::new(tokio::sync::Semaphore::new(0)),
            release: Arc::new(tokio::sync::Semaphore::new(0)),
        }
    }

    async fn wait_until_entered(&self) {
        self.entered
            .acquire()
            .await
            .expect("overview test gate remains open")
            .forget();
    }

    fn release_one(&self) {
        self.release.add_permits(1);
    }
}

/// Compose the API and frontend. Unknown API routes remain JSON 404 responses;
/// only non-API routes use the single-page application fallback.
pub fn router(state: ApiState, web_root: PathBuf) -> Result<Router, StaticRootError> {
    static_root::validate_static_root(&web_root)?;

    let api = Router::new()
        .route("/health", get(detailed_health))
        .route("/system/overview", get(system_overview))
        .merge(auth::routes())
        .fallback(api_not_found)
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(10),
        ));

    let index = web_root.join("index.html");
    let index_file = ServeFile::new(index)
        .precompressed_br()
        .precompressed_gzip();
    let static_files = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .service(
            ServeDir::new(&web_root)
                .precompressed_br()
                .precompressed_gzip()
                .append_index_html_on_directories(true)
                .fallback(index_file),
        );
    let asset_files = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ))
        .service(
            ServeDir::new(web_root.join("assets"))
                .precompressed_br()
                .precompressed_gzip(),
        );
    let request_id_header = HeaderName::from_static("x-request-id");

    let admission_state = state.clone();
    let application = Router::new()
        .nest("/api/v1", api)
        .nest_service("/assets", asset_files)
        .fallback_service(static_files)
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            admission_state,
            admit_application_request,
        ));

    Ok(Router::new()
        .route("/healthz", get(liveness))
        .merge(application)
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; base-uri 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; script-src 'self'; style-src 'self'",
            ),
        ))
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
        .layer(CatchPanicLayer::new())
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn(require_loopback_host)))
}

async fn admit_application_request(
    State(state): State<ApiState>,
    request: Request,
    next: Next,
) -> Response {
    let Ok(permit) = Arc::clone(&state.application_request_slots).try_acquire_owned() else {
        return ApiError::ApplicationCapacityExhausted.into_response();
    };
    let response = next.run(request).await;
    drop(permit);
    response
}

async fn require_loopback_host(request: Request, next: Next) -> Response {
    match validate_request_host(&request) {
        Ok(()) => next.run(request).await,
        Err(HostHeaderError::MissingOrAmbiguous) => ApiError::InvalidHost.into_response(),
        Err(HostHeaderError::MalformedOrUntrusted) => ApiError::UntrustedHost.into_response(),
    }
}

#[derive(Debug)]
enum HostHeaderError {
    MissingOrAmbiguous,
    MalformedOrUntrusted,
}

fn validate_request_host(request: &Request) -> Result<(), HostHeaderError> {
    let mut host_values = request.headers().get_all(header::HOST).iter();
    let host_value = host_values
        .next()
        .ok_or(HostHeaderError::MissingOrAmbiguous)?;
    if host_values.next().is_some() {
        return Err(HostHeaderError::MissingOrAmbiguous);
    }

    let host_text = host_value
        .to_str()
        .map_err(|_| HostHeaderError::MalformedOrUntrusted)?;
    let host_authority =
        Authority::from_str(host_text).map_err(|_| HostHeaderError::MalformedOrUntrusted)?;
    if !is_allowed_loopback_authority(&host_authority) {
        return Err(HostHeaderError::MalformedOrUntrusted);
    }

    if let Some(scheme) = request.uri().scheme_str()
        && !scheme.eq_ignore_ascii_case("http")
        && !scheme.eq_ignore_ascii_case("https")
    {
        return Err(HostHeaderError::MalformedOrUntrusted);
    }

    if let Some(uri_authority) = request.uri().authority()
        && (!is_allowed_loopback_authority(uri_authority) || uri_authority != &host_authority)
    {
        return Err(HostHeaderError::MalformedOrUntrusted);
    }

    Ok(())
}

fn is_allowed_loopback_authority(authority: &Authority) -> bool {
    if authority.as_str().contains('@') {
        return false;
    }

    let host = authority.host();
    let port_suffix = &authority.as_str()[host.len()..];
    if !port_suffix.is_empty()
        && (!port_suffix.starts_with(':')
            || port_suffix.len() == 1
            || authority.port().is_none_or(|port| port.as_u16() == 0))
    {
        return false;
    }

    let host_without_brackets = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if host_without_brackets.eq_ignore_ascii_case("localhost") {
        return true;
    }

    host_without_brackets
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

async fn liveness() -> impl IntoResponse {
    (
        StatusCode::NO_CONTENT,
        [(header::CACHE_CONTROL, "no-store")],
    )
}

async fn detailed_health(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    let status = if state.metrics_database == DatabaseStatus::Ok {
        HealthStatus::Ok
    } else {
        HealthStatus::Degraded
    };
    let report = HealthReport {
        status,
        version: VERSION.to_owned(),
        state_database: DatabaseStatus::Ok,
        metrics_database: state.metrics_database,
        timestamp_unix_ms: unix_timestamp_ms(),
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(report)))
}

async fn system_overview(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    let permit = state.host.try_acquire().ok_or(ApiError::HostBusy)?;

    #[cfg(test)]
    if let Some(gate) = state.overview_test_gate {
        gate.entered.add_permits(1);
        gate.release
            .acquire()
            .await
            .expect("overview test gate remains open")
            .forget();
    }

    let snapshot = tokio::task::spawn_blocking(move || permit.snapshot())
        .await
        .map_err(|_| {
            tracing::error!("host sampling task failed");
            ApiError::HostUnavailable
        })?
        .map_err(|_| {
            tracing::error!("host sampling failed");
            ApiError::HostUnavailable
        })?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(snapshot)))
}

async fn api_not_found() -> ApiError {
    ApiError::NotFound
}

#[derive(Debug)]
pub(crate) enum ApiError {
    ApplicationCapacityExhausted,
    AttemptRateLimited,
    AuthenticationRequired,
    AuthorizationDenied,
    CrossSiteRequest,
    CsrfRejected,
    HostBusy,
    HostUnavailable,
    InvalidHost,
    InvalidJson,
    InvalidOrigin,
    LoginRejected,
    NotFound,
    PasswordWorkersBusy,
    SessionMaintenance,
    ServiceUnavailable,
    SetupConflict,
    SetupRejected,
    UnsupportedMediaType,
    UntrustedHost,
}

#[derive(Serialize)]
struct ApiProblem {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response<Body> {
        let clear_cookie = matches!(self, Self::AuthenticationRequired);
        let (status, problem, retry_after) = match self {
            Self::ApplicationCapacityExhausted => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiProblem {
                    code: "request_capacity_exhausted",
                    message: "Helix is at its concurrent request limit.",
                },
                Some(HeaderValue::from_static("1")),
            ),
            Self::AttemptRateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                ApiProblem {
                    code: "too_many_attempts",
                    message: "Too many authentication attempts.",
                },
                Some(HeaderValue::from_static("900")),
            ),
            Self::AuthenticationRequired => (
                StatusCode::UNAUTHORIZED,
                ApiProblem {
                    code: "authentication_required",
                    message: "Authentication is required.",
                },
                None,
            ),
            Self::AuthorizationDenied => (
                StatusCode::FORBIDDEN,
                ApiProblem {
                    code: "authorization_denied",
                    message: "The session is not authorized for this operation.",
                },
                None,
            ),
            Self::CrossSiteRequest => (
                StatusCode::FORBIDDEN,
                ApiProblem {
                    code: "cross_site_request_rejected",
                    message: "Cross-site requests are not allowed.",
                },
                None,
            ),
            Self::CsrfRejected => (
                StatusCode::FORBIDDEN,
                ApiProblem {
                    code: "csrf_rejected",
                    message: "The anti-CSRF token was rejected.",
                },
                None,
            ),
            Self::HostBusy => (
                StatusCode::TOO_MANY_REQUESTS,
                ApiProblem {
                    code: "host_snapshot_in_progress",
                    message: "A host snapshot is already in progress.",
                },
                Some(HeaderValue::from_static("1")),
            ),
            Self::HostUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiProblem {
                    code: "host_snapshot_unavailable",
                    message: "Helix could not collect a host snapshot.",
                },
                None,
            ),
            Self::InvalidHost => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "invalid_host_header",
                    message: "A single valid Host header is required.",
                },
                None,
            ),
            Self::InvalidJson => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "invalid_json",
                    message: "The request body must be valid JSON with the expected fields.",
                },
                None,
            ),
            Self::InvalidOrigin => (
                StatusCode::FORBIDDEN,
                ApiProblem {
                    code: "invalid_origin",
                    message: "The request Origin does not match this Helix instance.",
                },
                None,
            ),
            Self::LoginRejected => (
                StatusCode::UNAUTHORIZED,
                ApiProblem {
                    code: "login_rejected",
                    message: "The login could not be accepted.",
                },
                None,
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                ApiProblem {
                    code: "api_route_not_found",
                    message: "The requested API route does not exist.",
                },
                None,
            ),
            Self::PasswordWorkersBusy => (
                StatusCode::TOO_MANY_REQUESTS,
                ApiProblem {
                    code: "password_capacity_exhausted",
                    message: "Password verification capacity is busy.",
                },
                Some(HeaderValue::from_static("1")),
            ),
            Self::SessionMaintenance => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiProblem {
                    code: "session_maintenance_in_progress",
                    message: "Session maintenance is in progress. Retry the login shortly.",
                },
                Some(HeaderValue::from_static("1")),
            ),
            Self::ServiceUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiProblem {
                    code: "service_unavailable",
                    message: "The requested operation is temporarily unavailable.",
                },
                None,
            ),
            Self::SetupConflict => (
                StatusCode::CONFLICT,
                ApiProblem {
                    code: "setup_conflict",
                    message: "Initial owner setup is no longer available.",
                },
                None,
            ),
            Self::SetupRejected => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "setup_rejected",
                    message: "The setup request could not be accepted.",
                },
                None,
            ),
            Self::UnsupportedMediaType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ApiProblem {
                    code: "unsupported_media_type",
                    message: "POST requests require application/json.",
                },
                None,
            ),
            Self::UntrustedHost => (
                StatusCode::MISDIRECTED_REQUEST,
                ApiProblem {
                    code: "untrusted_host",
                    message: "Helix accepts only localhost or loopback IP Host values.",
                },
                None,
            ),
        };
        let mut response =
            (status, [(header::CACHE_CONTROL, "no-store")], Json(problem)).into_response();
        if let Some(retry_after) = retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, retry_after);
        }
        if clear_cookie {
            response
                .headers_mut()
                .insert(header::SET_COOKIE, auth::clear_session_cookie());
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, extract::ConnectInfo, http::Request as HttpRequest};
    use helix_auth::{OpaqueToken, TokenDomain};
    use helix_state::{BootstrapInstallOutcome, BootstrapTokenHash};
    use serde_json::{Value, json};
    use std::net::SocketAddr;
    use tokio::sync::Semaphore;
    use tower::ServiceExt;

    const PASSWORD: &str = "V7!quartz-Meteor#29";

    struct TestApp {
        app: Router,
        databases: Arc<DatabaseSet>,
        data: tempfile::TempDir,
        web: tempfile::TempDir,
    }

    struct AuthClient {
        cookie: String,
        csrf: String,
    }

    async fn test_app(metrics: DatabaseStatus) -> TestApp {
        test_app_with_state(metrics, |state| state).await
    }

    async fn test_app_with_state(
        metrics: DatabaseStatus,
        modify: impl FnOnce(ApiState) -> ApiState,
    ) -> TestApp {
        let data = tempfile::tempdir().expect("temporary data directory");
        let web = tempfile::tempdir().expect("temporary web directory");
        std::fs::create_dir(web.path().join("assets")).expect("create assets");
        std::fs::write(web.path().join("index.html"), "SPA INDEX").expect("write index");
        std::fs::write(web.path().join("assets/app-deadbeef.js"), "void 0").expect("write asset");
        let databases =
            Arc::new(DatabaseSet::open_for_daemon(data.path()).expect("open test database set"));
        let state = ApiState::initialize(HostSampler::new(), metrics, Arc::clone(&databases))
            .await
            .expect("initialize API state");
        let app = router(modify(state), web.path().to_path_buf()).expect("build router");
        TestApp {
            app,
            databases,
            data,
            web,
        }
    }

    fn get(uri: &str) -> HttpRequest<Body> {
        HttpRequest::builder()
            .uri(uri)
            .header(header::HOST, "localhost")
            .body(Body::empty())
            .expect("request")
    }

    fn post_json(uri: &str, value: &Value, peer_octet: u8) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("POST")
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .extension(ConnectInfo(SocketAddr::from((
                [127, 0, 0, peer_octet],
                41000,
            ))))
            .body(Body::from(value.to_string()))
            .expect("request")
    }

    fn with_cookie(mut request: HttpRequest<Body>, cookie: &str) -> HttpRequest<Body> {
        request.headers_mut().insert(
            header::COOKIE,
            HeaderValue::from_str(cookie).expect("cookie header"),
        );
        request
    }

    fn with_csrf(mut request: HttpRequest<Body>, csrf: &str) -> HttpRequest<Body> {
        request.headers_mut().insert(
            HeaderName::from_static("x-helix-csrf"),
            HeaderValue::from_str(csrf).expect("CSRF header"),
        );
        request
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("JSON response")
    }

    fn install_bootstrap(context: &TestApp) -> String {
        let token = OpaqueToken::generate().expect("generate bootstrap");
        let encoded = token.encode();
        let encoded = encoded.expose_secret().to_owned();
        let hash = token.verification_hash(TokenDomain::Bootstrap);
        let hash = BootstrapTokenHash::from_digest(*hash.as_bytes());
        let now = i64::try_from(unix_timestamp_ms()).expect("current time fits i64");
        assert!(matches!(
            context
                .databases
                .state()
                .replace_bootstrap_token(&hash, now, now + 10 * 60 * 1_000)
                .expect("install bootstrap"),
            BootstrapInstallOutcome::Installed { .. }
        ));
        encoded
    }

    async fn claim_owner(context: &TestApp, bootstrap: &str) -> AuthClient {
        let fixation = "helix_session=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let request = with_cookie(
            post_json(
                "/api/v1/setup/owner",
                &json!({
                    "bootstrapToken": bootstrap,
                    "loginName": "owner",
                    "displayName": "Rique\u{301}",
                    "password": PASSWORD
                }),
                1,
            ),
            fixation,
        );
        let response = context
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CREATED);
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("session cookie")
            .to_str()
            .expect("cookie text")
            .to_owned();
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Strict"));
        assert!(set_cookie.contains("Path=/"));
        assert!(set_cookie.contains("Max-Age=28800"));
        assert!(!set_cookie.contains("Domain="));
        assert!(!set_cookie.contains("Secure"));
        assert!(!set_cookie.contains("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
        let cookie = set_cookie
            .split(';')
            .next()
            .expect("cookie pair")
            .to_owned();
        let body = response_json(response).await;
        assert_eq!(body["user"]["loginName"], "owner");
        assert_eq!(body["user"]["displayName"], "Riqué");
        AuthClient {
            cookie,
            csrf: body["csrfToken"].as_str().expect("CSRF token").to_owned(),
        }
    }

    #[tokio::test]
    async fn public_health_discloses_only_liveness_and_details_require_authentication() {
        let context = test_app(DatabaseStatus::Recovered).await;
        let response = context
            .app
            .clone()
            .oneshot(get("/healthz"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers().get(header::X_CONTENT_TYPE_OPTIONS),
            Some(&HeaderValue::from_static("nosniff"))
        );
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("liveness body");
        assert!(body.is_empty());

        let protected = context
            .app
            .oneshot(get("/api/v1/health"))
            .await
            .expect("response");
        assert_eq!(protected.status(), StatusCode::UNAUTHORIZED);
        assert!(
            protected
                .headers()
                .get(header::SET_COOKIE)
                .is_some_and(|value| value.as_bytes().windows(9).any(|part| part == b"Max-Age=0"))
        );
    }

    #[tokio::test]
    async fn host_boundary_rejects_non_loopback_missing_and_duplicate_values() {
        let context = test_app(DatabaseStatus::Ok).await;
        for host in ["example.com", "0.0.0.0", "192.168.1.8:8080", "[::2]"] {
            let request = HttpRequest::builder()
                .uri("/healthz")
                .header(header::HOST, host)
                .body(Body::empty())
                .expect("request");
            let response = context
                .app
                .clone()
                .oneshot(request)
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST);
        }

        let missing = HttpRequest::builder()
            .uri("/healthz")
            .body(Body::empty())
            .expect("request");
        assert_eq!(
            context
                .app
                .clone()
                .oneshot(missing)
                .await
                .expect("response")
                .status(),
            StatusCode::BAD_REQUEST
        );

        let mut duplicate = get("/healthz");
        duplicate
            .headers_mut()
            .append(header::HOST, HeaderValue::from_static("127.0.0.1"));
        assert_eq!(
            context
                .app
                .oneshot(duplicate)
                .await
                .expect("response")
                .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn owner_session_csrf_and_logout_lifecycle_is_enforced() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let status = context
            .app
            .clone()
            .oneshot(get("/api/v1/setup/status"))
            .await
            .expect("status response");
        let status = response_json(status).await;
        assert_eq!(status["ownerExists"], false);
        assert_eq!(status["bootstrapAvailable"], true);
        assert!(status["bootstrapExpiresAtUnixMs"].as_i64().is_some());

        let invalid = context
            .app
            .clone()
            .oneshot(post_json(
                "/api/v1/setup/owner",
                &json!({
                    "bootstrapToken": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                    "loginName": "owner",
                    "displayName": "Rique",
                    "password": PASSWORD
                }),
                2,
            ))
            .await
            .expect("invalid setup response");
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let mut client = claim_owner(&context, &bootstrap).await;
        for route in [
            "/api/v1/auth/me",
            "/api/v1/health",
            "/api/v1/system/overview",
        ] {
            let response = context
                .app
                .clone()
                .oneshot(with_cookie(get(route), &client.cookie))
                .await
                .expect("cookie-only protected response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "route {route}");
        }

        let me = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/auth/me"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("me response");
        assert_eq!(me.status(), StatusCode::OK);
        let me = response_json(me).await;
        assert_eq!(me["user"]["loginName"], "owner");
        assert_eq!(me["user"]["capabilities"], json!(["system.view"]));
        assert!(me["expiresAtUnixMs"].as_i64().is_some());

        for route in ["/api/v1/health", "/api/v1/system/overview"] {
            let response = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(get(route), &client.cookie),
                    &client.csrf,
                ))
                .await
                .expect("protected response");
            assert_eq!(response.status(), StatusCode::OK, "route {route}");
        }

        let missing_csrf = context
            .app
            .clone()
            .oneshot(with_cookie(
                post_json("/api/v1/auth/logout", &json!({}), 1),
                &client.cookie,
            ))
            .await
            .expect("missing CSRF response");
        assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);

        let cookie_only_rotation = context
            .app
            .clone()
            .oneshot(with_cookie(
                post_json("/api/v1/auth/csrf", &json!({}), 1),
                &client.cookie,
            ))
            .await
            .expect("cookie-only rotation response");
        assert_eq!(cookie_only_rotation.status(), StatusCode::FORBIDDEN);

        let rotation = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json("/api/v1/auth/csrf", &json!({}), 1),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("rotation response");
        assert_eq!(rotation.status(), StatusCode::OK);
        let rotation = response_json(rotation).await;
        let new_csrf = rotation["csrfToken"]
            .as_str()
            .expect("rotated CSRF")
            .to_owned();

        let stale_rotation = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json("/api/v1/auth/csrf", &json!({}), 1),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("stale rotation response");
        assert_eq!(stale_rotation.status(), StatusCode::FORBIDDEN);

        for route in [
            "/api/v1/auth/me",
            "/api/v1/health",
            "/api/v1/system/overview",
        ] {
            let response = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(get(route), &client.cookie),
                    &client.csrf,
                ))
                .await
                .expect("old-CSRF protected response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "route {route}");
        }

        let old_csrf = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json("/api/v1/auth/logout", &json!({}), 1),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("old CSRF response");
        assert_eq!(old_csrf.status(), StatusCode::FORBIDDEN);
        client.csrf = new_csrf;

        let logout = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json("/api/v1/auth/logout", &json!({}), 1),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("logout response");
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);
        assert!(
            logout
                .headers()
                .get(header::SET_COOKIE)
                .is_some_and(|value| value.as_bytes().windows(9).any(|part| part == b"Max-Age=0"))
        );

        let revoked = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/auth/me"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("revoked response");
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

        let login = context
            .app
            .oneshot(post_json(
                "/api/v1/auth/login",
                &json!({"loginName": "owner", "password": PASSWORD}),
                3,
            ))
            .await
            .expect("login response");
        assert_eq!(login.status(), StatusCode::OK);
        assert!(login.headers().contains_key(header::SET_COOKIE));
        let login = response_json(login).await;
        assert_eq!(login["user"]["loginName"], "owner");
        assert_eq!(login["user"]["capabilities"], json!(["system.view"]));
        assert!(login["csrfToken"].as_str().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn only_one_concurrent_owner_claim_succeeds() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let body = json!({
            "bootstrapToken": bootstrap,
            "loginName": "owner",
            "displayName": "Rique",
            "password": PASSWORD
        });
        let first = context
            .app
            .clone()
            .oneshot(post_json("/api/v1/setup/owner", &body, 1));
        let second = context
            .app
            .clone()
            .oneshot(post_json("/api/v1/setup/owner", &body, 2));
        let (first, second) = tokio::join!(first, second);
        let mut statuses = [
            first.expect("first response").status(),
            second.expect("second response").status(),
        ];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::CREATED, StatusCode::CONFLICT]);
    }

    #[tokio::test]
    async fn post_boundary_rejects_media_origin_fetch_and_malformed_json() {
        let context = test_app(DatabaseStatus::Ok).await;
        let body = json!({"loginName": "nobody", "password": "wrong"});

        let no_media = HttpRequest::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
            .body(Body::from(body.to_string()))
            .expect("request");
        assert_eq!(
            context
                .app
                .clone()
                .oneshot(no_media)
                .await
                .expect("response")
                .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );

        let mut wrong_origin = post_json("/api/v1/auth/login", &body, 1);
        wrong_origin
            .headers_mut()
            .insert(header::ORIGIN, HeaderValue::from_static("http://127.0.0.1"));
        assert_eq!(
            context
                .app
                .clone()
                .oneshot(wrong_origin)
                .await
                .expect("response")
                .status(),
            StatusCode::FORBIDDEN
        );

        let mut cross_site = post_json("/api/v1/auth/login", &body, 1);
        cross_site.headers_mut().insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("cross-site"),
        );
        assert_eq!(
            context
                .app
                .clone()
                .oneshot(cross_site)
                .await
                .expect("response")
                .status(),
            StatusCode::FORBIDDEN
        );

        let malformed = HttpRequest::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
            .body(Body::from("{"))
            .expect("request");
        assert_eq!(
            context
                .app
                .oneshot(malformed)
                .await
                .expect("response")
                .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn generic_login_rejection_does_not_expose_account_state() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let _client = claim_owner(&context, &bootstrap).await;

        let attempts = [
            ("owner", "wrong-password", 1_u8),
            ("owner", PASSWORD, 2_u8),
            ("unknown", "wrong-password", 3_u8),
            ("NotCanonical", "wrong-password", 4_u8),
        ];
        let mut expected = None;
        for (login, password, peer) in attempts {
            let response = context
                .app
                .clone()
                .oneshot(post_json(
                    "/api/v1/auth/login",
                    &json!({"loginName": login, "password": password}),
                    peer,
                ))
                .await
                .expect("login response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(!response.headers().contains_key(header::SET_COOKIE));
            let body = response_json(response).await;
            if let Some(expected) = &expected {
                assert_eq!(&body, expected);
            } else {
                expected = Some(body);
            }
        }

        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        connection
            .execute(
                "UPDATE users
                 SET status = 'disabled', auth_version = auth_version + 1
                 WHERE login_name = 'owner'",
                [],
            )
            .expect("disable owner");
        let disabled = context
            .app
            .oneshot(post_json(
                "/api/v1/auth/login",
                &json!({"loginName": "owner", "password": PASSWORD}),
                5,
            ))
            .await
            .expect("disabled login response");
        assert_eq!(disabled.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response_json(disabled).await,
            expected.expect("generic body")
        );
    }

    #[tokio::test]
    async fn cookie_parser_rejects_duplicates_whitespace_and_clears_invalid_sessions() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let pair = client.cookie.split_once('=').expect("cookie pair");
        for cookie in [
            format!("{}; {}", client.cookie, client.cookie),
            format!("{} ={}", pair.0, pair.1),
            format!("{}= {}", pair.0, pair.1),
            format!("{}; malformed", client.cookie),
        ] {
            let response = context
                .app
                .clone()
                .oneshot(with_cookie(get("/api/v1/auth/me"), &cookie))
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(
                response
                    .headers()
                    .get(header::SET_COOKIE)
                    .is_some_and(|value| value
                        .as_bytes()
                        .windows(9)
                        .any(|part| part == b"Max-Age=0"))
            );
        }
    }

    #[tokio::test]
    async fn authenticated_user_without_capability_is_forbidden_by_default() {
        let context = test_app(DatabaseStatus::Ok).await;
        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        connection
            .execute("DELETE FROM role_capabilities", [])
            .expect("remove capability grants");
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        let response = context
            .app
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/system/overview"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(!response.headers().contains_key(header::SET_COOKIE));
    }

    #[tokio::test]
    async fn password_capacity_is_non_queueing_and_static_cache_policy_is_scoped() {
        let context = test_app_with_state(DatabaseStatus::Ok, |state| {
            state.with_password_workers(Arc::new(Semaphore::new(0)))
        })
        .await;
        for _ in 0..6 {
            let response = context
                .app
                .clone()
                .oneshot(post_json(
                    "/api/v1/auth/login",
                    &json!({"loginName": "owner", "password": "wrong-password"}),
                    1,
                ))
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(
                response.headers().get(header::RETRY_AFTER),
                Some(&HeaderValue::from_static("1"))
            );
            assert_eq!(
                response_json(response).await["code"],
                "password_capacity_exhausted"
            );
        }

        let asset = context
            .app
            .clone()
            .oneshot(get("/assets/app-deadbeef.js"))
            .await
            .expect("asset response");
        assert_eq!(
            asset.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static(
                "public, max-age=31536000, immutable"
            ))
        );
        let shell = context
            .app
            .oneshot(get("/settings/appearance"))
            .await
            .expect("shell response");
        assert_eq!(
            shell.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-cache"))
        );
        assert!(context.web.path().join("index.html").is_file());
    }

    #[tokio::test]
    async fn session_maintenance_response_is_retryable_without_authentication_details() {
        let response = ApiError::SessionMaintenance.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("1"))
        );
        let body = response_json(response).await;
        assert_eq!(body["code"], "session_maintenance_in_progress");
        assert_eq!(body.as_object().expect("problem object").len(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn authenticated_overview_preserves_host_sampler_single_flight() {
        const FLOOD_REQUESTS: usize = 8;

        let gate = OverviewTestGate::new();
        let configured_gate = gate.clone();
        let context = test_app_with_state(DatabaseStatus::Ok, move |state| {
            state.with_overview_test_gate(configured_gate)
        })
        .await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        let first_app = context.app.clone();
        let first_cookie = client.cookie.clone();
        let first_csrf = client.csrf.clone();
        let first_request = tokio::spawn(async move {
            first_app
                .oneshot(with_csrf(
                    with_cookie(get("/api/v1/system/overview"), &first_cookie),
                    &first_csrf,
                ))
                .await
                .expect("first response")
        });
        gate.wait_until_entered().await;

        let mut flood = Vec::with_capacity(FLOOD_REQUESTS);
        for _ in 0..FLOOD_REQUESTS {
            let app = context.app.clone();
            let cookie = client.cookie.clone();
            let csrf = client.csrf.clone();
            flood.push(tokio::spawn(async move {
                app.oneshot(with_csrf(
                    with_cookie(get("/api/v1/system/overview"), &cookie),
                    &csrf,
                ))
                .await
                .expect("flood response")
            }));
        }
        for task in flood {
            let response = task.await.expect("flood task");
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        }
        gate.release_one();
        assert_eq!(
            first_request.await.expect("first task").status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn unknown_api_routes_never_fall_back_to_the_spa() {
        let context = test_app(DatabaseStatus::Ok).await;
        let response = context
            .app
            .oneshot(get("/api/v1/not-real"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(
            !response_json(response)
                .await
                .to_string()
                .contains("SPA INDEX")
        );
    }

    #[tokio::test]
    async fn application_capacity_is_nonqueueing_and_liveness_bypasses_it() {
        let context = test_app_with_state(DatabaseStatus::Ok, |state| {
            state.with_application_request_slots(Arc::new(Semaphore::new(0)))
        })
        .await;

        for uri in ["/", "/api/v1/setup/status"] {
            let response = context
                .app
                .clone()
                .oneshot(get(uri))
                .await
                .expect("capacity response");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(
                response.headers().get(header::RETRY_AFTER),
                Some(&HeaderValue::from_static("1"))
            );
        }

        let liveness = context
            .app
            .oneshot(get("/healthz"))
            .await
            .expect("liveness response");
        assert_eq!(liveness.status(), StatusCode::NO_CONTENT);
    }
}
