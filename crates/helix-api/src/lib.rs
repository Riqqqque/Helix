//! Versioned HTTP API and static frontend composition.

mod auth;
mod marketplace_media;
mod server_media;
mod static_root;
mod strand_net;
mod strands;
mod terminal;
mod weather;

use axum::{
    Json, Router,
    body::Body,
    extract::{
        ConnectInfo, DefaultBodyLimit, Path as RoutePath, Query, Request, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header, uri::Authority},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use helix_core::{DatabaseStatus, HealthReport, HealthStatus, VERSION, unix_timestamp_ms};
use helix_privd::{
    BrokerClient, BrokerClientError, BrokerRequest, DockerContainerActionKind, FileUploadPurpose,
    FileUploadTarget, FirewallRuleSpec, GameKind, GamePortPolicySpec, HookServiceAction,
    MarketplaceCatalog, MinecraftCreateSpec, MinecraftModpackCreateSpec, MinecraftSettingsPatch,
    MinecraftSoftware, ModpackProvider, PackageUpdateCandidate, RecurringRebootSpec, ServerAction,
    ServerNetworkExposure, StorageAnalysisMode, TerrariaCreateSpec, VRisingCreateSpec,
    ValheimCreateSpec,
};
use helix_state::{
    DatabaseSet, ServerAppearanceUpdateOutcome, UserPreferencesRecord, UserPreferencesUpdateInput,
    UserPreferencesUpdateOutcome,
};
use helix_system::HostSampler;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
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
const FILE_API_BODY_LIMIT_BYTES: usize = 5 * 1024 * 1024;
const STORAGE_ANALYSIS_BODY_LIMIT_BYTES: usize = 8 * 1024;
const FIREWALL_RULE_BODY_LIMIT_BYTES: usize = 8 * 1024;
const PACKAGE_UPDATE_BODY_LIMIT_BYTES: usize = 384 * 1024;
const MAX_CONCURRENT_APPLICATION_REQUESTS: usize = 64;

#[derive(Clone)]
pub struct ApiState {
    host: HostSampler,
    metrics_database: DatabaseStatus,
    pub(crate) databases: Arc<DatabaseSet>,
    pub(crate) password_workers: Arc<tokio::sync::Semaphore>,
    weather_workers: Arc<tokio::sync::Semaphore>,
    marketplace_media_workers: Arc<tokio::sync::Semaphore>,
    application_request_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) dummy_password_phc: Arc<str>,
    pub(crate) attempt_limiter: auth::AttemptLimiter,
    pub(crate) blocking_tasks: BlockingTaskTracker,
    broker: Option<BrokerClient>,
    terminal: Option<terminal::TerminalConnector>,
    terminal_tickets: terminal::TerminalTicketStore,
    #[cfg(test)]
    overview_test_gate: Option<OverviewTestGate>,
}

impl ApiState {
    pub async fn initialize(
        host: HostSampler,
        metrics_database: DatabaseStatus,
        databases: Arc<DatabaseSet>,
    ) -> Result<Self, ApiInitializationError> {
        Self::initialize_with_broker(host, metrics_database, databases, None).await
    }

    pub async fn initialize_with_broker(
        host: HostSampler,
        metrics_database: DatabaseStatus,
        databases: Arc<DatabaseSet>,
        broker: Option<BrokerClient>,
    ) -> Result<Self, ApiInitializationError> {
        let (password_workers, dummy_password_phc) = auth::initialize_password_boundary().await?;
        Ok(Self {
            host,
            metrics_database,
            databases,
            password_workers,
            weather_workers: Arc::new(tokio::sync::Semaphore::new(2)),
            marketplace_media_workers: Arc::new(tokio::sync::Semaphore::new(8)),
            application_request_slots: Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_APPLICATION_REQUESTS,
            )),
            dummy_password_phc,
            attempt_limiter: auth::AttemptLimiter::production(),
            blocking_tasks: BlockingTaskTracker::default(),
            broker,
            terminal: None,
            terminal_tickets: terminal::TerminalTicketStore::default(),
            #[cfg(test)]
            overview_test_gate: None,
        })
    }

    pub fn with_terminal_socket(
        mut self,
        socket_path: PathBuf,
    ) -> Result<Self, ApiInitializationError> {
        self.terminal = Some(terminal::TerminalConnector::new(socket_path)?);
        Ok(self)
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
    #[error("terminal socket configuration is invalid")]
    InvalidTerminalSocket,
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

    let broker_api = Router::new()
        .route("/host/inventory", get(host_inventory))
        .route("/host/integration", get(host_integration_status))
        .route(
            "/host/integration/start-on-boot",
            put(set_helix_start_on_boot),
        )
        .route("/host/reboot/preflight", get(host_reboot_preflight))
        .route(
            "/host/reboot/recurring",
            put(set_recurring_host_reboot).delete(delete_recurring_host_reboot),
        )
        .route("/host/reboot", post(schedule_host_reboot))
        .route("/host/reboot/{operation_id}", delete(cancel_host_reboot))
        .route("/network/inventory", get(network_inventory))
        .route("/network/globe", get(network_globe))
        .route(
            "/network/amp-router-forwards/release",
            post(release_amp_router_forward),
        )
        .route(
            "/network/firewall/rules",
            post(create_firewall_rule).layer(DefaultBodyLimit::max(FIREWALL_RULE_BODY_LIMIT_BYTES)),
        )
        .route(
            "/network/firewall/rules/{rule_id}",
            delete(delete_firewall_rule),
        )
        .route(
            "/network/firewall/rules/{rule_id}/restore",
            post(restore_firewall_rule),
        )
        .route("/network/firewall/enable", post(enable_firewall))
        .route("/system/packages", get(system_package_inventory))
        .route(
            "/system/packages/refresh",
            post(refresh_system_package_lists),
        )
        .route(
            "/system/packages/apply",
            post(apply_system_package_updates)
                .layer(DefaultBodyLimit::max(PACKAGE_UPDATE_BODY_LIMIT_BYTES)),
        )
        .route("/system/packages/jobs/{job_id}", get(system_package_job))
        .route("/system/helix/check", post(check_helix_update))
        .route("/system/helix/apply", post(apply_helix_update))
        .route("/hooks", get(hook_inventory))
        .route(
            "/hooks/{hook_id}/install/preflight",
            get(hook_install_preflight),
        )
        .route("/hooks/{hook_id}/install", post(install_hook))
        .route("/hooks/jobs/{job_id}", get(hook_install_job))
        .route("/hooks/{hook_id}/actions", post(manage_hook_service))
        .route("/docker/inventory", get(docker_inventory))
        .route("/docker/actions", post(docker_container_action))
        .route("/docker/homarr", get(homarr_widget_catalog))
        .route("/security", get(security_inventory))
        .route("/security/controls", post(set_security_control))
        .route(
            "/marketplace/modrinth/image",
            get(marketplace_modrinth_image),
        )
        .route(
            "/marketplace/curseforge/image",
            get(marketplace_curseforge_image),
        )
        .route("/files", get(list_directory))
        .route("/files/directory", post(create_directory))
        .route("/files/file", post(create_file))
        .route("/files/read", post(read_text_file))
        .route("/files/write", post(write_text_file))
        .route("/files/rename", post(rename_file))
        .route("/files/trash", post(trash_file))
        .route("/files/upload/begin", post(begin_file_upload))
        .route("/files/upload/chunk", post(write_file_upload_chunk))
        .route("/files/upload/finish", post(finish_file_upload))
        .route("/files/upload/abort", post(abort_file_upload))
        .route(
            "/storage/analysis",
            post(start_storage_analysis)
                .layer(DefaultBodyLimit::max(STORAGE_ANALYSIS_BODY_LIMIT_BYTES)),
        )
        .route(
            "/storage/analysis/{job_id}",
            get(storage_analysis_status).delete(cancel_storage_analysis),
        )
        .route("/servers", get(list_servers))
        .route("/servers/removed", get(list_trashed_servers))
        .route(
            "/servers/removed/{trash_id}/restore",
            post(restore_trashed_server),
        )
        .route("/servers/inventory-health", get(server_inventory_health))
        .route("/servers/manager/readiness", get(game_hosting_readiness))
        .route(
            "/servers/port-policies/minecraft",
            get(minecraft_port_policy).put(set_minecraft_port_policy),
        )
        .route(
            "/servers/port-policies/vrising",
            get(vrising_port_policy).put(set_vrising_port_policy),
        )
        .route(
            "/servers/port-policies/valheim",
            get(valheim_port_policy).put(set_valheim_port_policy),
        )
        .route(
            "/servers/port-policies/terraria",
            get(terraria_port_policy).put(set_terraria_port_policy),
        )
        .route("/servers/minecraft", post(create_minecraft))
        .route("/servers/minecraft/versions", get(list_minecraft_versions))
        .route(
            "/servers/minecraft/modpacks/search",
            get(minecraft_modpack_search),
        )
        .route(
            "/servers/minecraft/modpacks/projects/{project_id}",
            get(minecraft_modpack_project),
        )
        .route(
            "/servers/minecraft/modpacks",
            post(create_minecraft_modpack).layer(DefaultBodyLimit::max(API_BODY_LIMIT_BYTES)),
        )
        .route("/servers/vrising", post(create_vrising))
        .route("/servers/valheim", post(create_valheim))
        .route("/servers/terraria", post(create_terraria))
        .route("/servers/{instance_id}", get(server_detail))
        .route(
            "/servers/{instance_id}/appearance",
            get(server_appearance)
                .put(set_server_appearance)
                .delete(clear_server_appearance)
                .layer(DefaultBodyLimit::max(768 * 1024)),
        )
        .route(
            "/servers/{instance_id}/appearance/image",
            get(server_appearance_image),
        )
        .route("/servers/{instance_id}/logs", get(server_logs))
        .route(
            "/servers/{instance_id}/logs/history",
            get(server_log_history),
        )
        .route("/servers/{instance_id}/console", post(server_console))
        .route(
            "/servers/{instance_id}/settings",
            get(server_settings).post(update_server_settings),
        )
        .route(
            "/servers/{instance_id}/marketplace/search",
            get(server_marketplace_search),
        )
        .route(
            "/servers/{instance_id}/marketplace/projects/{project_id}",
            get(server_marketplace_project),
        )
        .route(
            "/servers/{instance_id}/marketplace/install",
            post(install_server_marketplace_content),
        )
        .route("/servers/{instance_id}/backups", get(list_server_backups))
        .route(
            "/servers/{instance_id}/backup-policy",
            put(set_server_backup_policy).post(prune_server_backups),
        )
        .route(
            "/servers/{instance_id}/backups/{backup_id}/restore",
            post(restore_server_backup),
        )
        .route(
            "/servers/{instance_id}/backups/{backup_id}",
            delete(trash_server_backup),
        )
        .route(
            "/servers/{instance_id}/backups/trash/{trash_id}/restore",
            post(restore_trashed_server_backup),
        )
        .route(
            "/servers/{instance_id}/backups/trash/{trash_id}",
            delete(purge_trashed_server_backup),
        )
        .route("/servers/{instance_id}/actions", post(server_action))
        .route(
            "/servers/{instance_id}/network",
            put(set_server_network_exposure),
        )
        .route(
            "/servers/{instance_id}/start-on-boot",
            put(set_native_start_on_boot),
        )
        .route("/servers/{instance_id}/memory", put(set_native_memory))
        .route("/servers/{instance_id}/cpu", put(set_native_cpu))
        .route(
            "/servers/{instance_id}/browser-listing",
            put(set_native_browser_listing),
        )
        .route("/servers/{instance_id}/remove", post(trash_native_server))
        .route("/jobs/{job_id}", get(job_status))
        .layer(DefaultBodyLimit::max(FILE_API_BODY_LIMIT_BYTES));
    let auth_api = auth::routes().layer(DefaultBodyLimit::max(API_BODY_LIMIT_BYTES));
    let terminal_api = terminal::routes().layer(DefaultBodyLimit::max(API_BODY_LIMIT_BYTES));
    let settings_api = Router::new()
        .route(
            "/settings/preferences",
            get(user_preferences).put(update_user_preferences),
        )
        .layer(DefaultBodyLimit::max(80 * 1024));
    let strand_api = strands::routes();
    let api = Router::new()
        .route("/health", get(detailed_health))
        .route("/system/overview", get(system_overview))
        .route("/weather", get(weather_forecast))
        .route("/games/readiness", get(game_hosting_readiness))
        .merge(broker_api)
        .merge(auth_api)
        .merge(terminal_api)
        .merge(settings_api)
        .merge(strand_api)
        .fallback(api_not_found)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(50),
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
                "default-src 'self'; base-uri 'self'; connect-src 'self'; img-src 'self' data: https: http:; object-src 'none'; frame-ancestors 'none'; script-src 'self'; style-src 'self'",
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

async fn weather_forecast(
    State(state): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<weather::WeatherQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "dashboard.customize").await?;
    let Query(query) = query.map_err(|_| ApiError::InvalidWeatherLocation)?;
    let permit = Arc::clone(&state.weather_workers)
        .try_acquire_owned()
        .map_err(|_| ApiError::WeatherBusy)?;
    let guard = state.blocking_tasks.start();
    let result = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        let _permit = permit;
        weather::forecast(query)
    })
    .await
    .map_err(|_| {
        tracing::error!("weather worker failed");
        ApiError::WeatherUnavailable
    })?;
    let forecast = match result {
        Ok(forecast) => forecast,
        Err(weather::WeatherError::InvalidLocation) => {
            return Err(ApiError::InvalidWeatherLocation);
        }
        Err(weather::WeatherError::LocationNotFound) => {
            return Err(ApiError::WeatherLocationNotFound);
        }
        Err(weather::WeatherError::ProviderUnavailable) => {
            return Err(ApiError::WeatherUnavailable);
        }
        Err(weather::WeatherError::InvalidProviderResponse) => {
            tracing::warn!("weather provider returned an invalid response");
            return Err(ApiError::WeatherUnavailable);
        }
    };
    Ok((
        [(header::CACHE_CONTROL, "private, max-age=300")],
        Json(forecast),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarketplaceImageQuery {
    path: String,
}

async fn marketplace_modrinth_image(
    State(state): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<MarketplaceImageQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    marketplace_image(
        state,
        headers,
        query,
        marketplace_media::MarketplaceImageOrigin::Modrinth,
    )
    .await
}

async fn marketplace_curseforge_image(
    State(state): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<MarketplaceImageQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    marketplace_image(
        state,
        headers,
        query,
        marketplace_media::MarketplaceImageOrigin::Curseforge,
    )
    .await
}

async fn marketplace_image(
    state: ApiState,
    headers: HeaderMap,
    query: Result<Query<MarketplaceImageQuery>, QueryRejection>,
    origin: marketplace_media::MarketplaceImageOrigin,
) -> Result<Response, ApiError> {
    // Image elements cannot attach Helix's in-memory CSRF proof. This is a
    // read-only, same-origin, allowlisted media proxy, so the authenticated
    // session and exact capability are sufficient here.
    auth::require_capability_without_csrf(&state, &headers, "games.view").await?;
    let Query(query) = query.map_err(|_| ApiError::NotFound)?;
    marketplace_media::validate_path(origin, &query.path).map_err(|_| ApiError::NotFound)?;
    let permit = Arc::clone(&state.marketplace_media_workers)
        .acquire_owned()
        .await
        .map_err(|_| ApiError::ServiceUnavailable)?;
    let guard = state.blocking_tasks.start();
    let image = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        let _permit = permit;
        marketplace_media::image(origin, &query.path)
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable)?
    .map_err(|error| match error {
        marketplace_media::MarketplaceMediaError::InvalidPath => ApiError::NotFound,
        marketplace_media::MarketplaceMediaError::ProviderUnavailable
        | marketplace_media::MarketplaceMediaError::InvalidResponse => ApiError::ServiceUnavailable,
    })?;
    let mut response = Response::new(Body::from(image.body.to_vec()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(image.content_type),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=1800"),
    );
    Ok(response)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PrimaryDashboardSection {
    Overview,
    Home,
    Storage,
    Network,
    Host,
    Security,
    Terminal,
    Servers,
    Hooks,
    Strands,
    Globe,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum HomeWidgetKind {
    Clock,
    Host,
    Servers,
    Storage,
    Weather,
    Note,
    Shortcut,
    Graphs,
    Docker,
    Strand,
    Globe,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum HomeWidgetSize {
    Compact,
    Wide,
    Full,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum HomeWidgetHeight {
    Short,
    Medium,
    Tall,
}

const fn default_home_widget_height() -> HomeWidgetHeight {
    HomeWidgetHeight::Medium
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HomeWidgetPreference {
    id: String,
    kind: HomeWidgetKind,
    size: HomeWidgetSize,
    #[serde(default = "default_home_widget_height")]
    height: HomeWidgetHeight,
    title: String,
    content: String,
    url: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    icon: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HomeTemplatePreference {
    id: String,
    name: String,
    accent: String,
    widgets: Vec<HomeWidgetPreference>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DashboardColorPreferences {
    accent: String,
    text: String,
    surface: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DashboardPreferences {
    navigation_order: Vec<PrimaryDashboardSection>,
    metrics_refresh_ms: u64,
    home_widgets: Vec<HomeWidgetPreference>,
    #[serde(default)]
    home_templates: Vec<HomeTemplatePreference>,
    #[serde(default)]
    active_home_id: String,
    #[serde(default)]
    colors: DashboardColorPreferences,
    #[serde(default = "default_true")]
    servers_enabled: bool,
    #[serde(default = "default_hidden_pages")]
    hidden_pages: Vec<PrimaryDashboardSection>,
}

fn default_true() -> bool {
    true
}

fn default_hidden_pages() -> Vec<PrimaryDashboardSection> {
    vec![PrimaryDashboardSection::Globe]
}

impl Default for DashboardPreferences {
    fn default() -> Self {
        let home_widgets = vec![
            home_widget(
                "clock",
                HomeWidgetKind::Clock,
                HomeWidgetSize::Compact,
                "Right now",
            ),
            home_widget(
                "weather",
                HomeWidgetKind::Weather,
                HomeWidgetSize::Wide,
                "Weather",
            ),
            home_widget(
                "host",
                HomeWidgetKind::Host,
                HomeWidgetSize::Wide,
                "Host pulse",
            ),
            home_widget(
                "servers",
                HomeWidgetKind::Servers,
                HomeWidgetSize::Wide,
                "Servers",
            ),
            home_widget(
                "storage",
                HomeWidgetKind::Storage,
                HomeWidgetSize::Wide,
                "Storage",
            ),
            home_widget(
                "notes",
                HomeWidgetKind::Note,
                HomeWidgetSize::Compact,
                "Notes",
            ),
        ];
        Self {
            navigation_order: vec![
                PrimaryDashboardSection::Overview,
                PrimaryDashboardSection::Home,
                PrimaryDashboardSection::Storage,
                PrimaryDashboardSection::Network,
                PrimaryDashboardSection::Host,
                PrimaryDashboardSection::Security,
                PrimaryDashboardSection::Terminal,
                PrimaryDashboardSection::Servers,
                PrimaryDashboardSection::Hooks,
                PrimaryDashboardSection::Strands,
                PrimaryDashboardSection::Globe,
            ],
            metrics_refresh_ms: 5_000,
            home_widgets: home_widgets.clone(),
            home_templates: vec![HomeTemplatePreference {
                id: "home-main".to_owned(),
                name: "Main".to_owned(),
                accent: "#d7f64d".to_owned(),
                widgets: home_widgets,
            }],
            active_home_id: "home-main".to_owned(),
            colors: DashboardColorPreferences::default(),
            servers_enabled: true,
            hidden_pages: default_hidden_pages(),
        }
    }
}

fn home_widget(
    id: &str,
    kind: HomeWidgetKind,
    size: HomeWidgetSize,
    title: &str,
) -> HomeWidgetPreference {
    HomeWidgetPreference {
        id: id.to_owned(),
        kind,
        size,
        height: HomeWidgetHeight::Medium,
        title: title.to_owned(),
        content: String::new(),
        url: String::new(),
        color: String::new(),
        icon: String::new(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DashboardPreferencesUpdate {
    expected_revision: i64,
    preferences: DashboardPreferences,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardPreferencesResponse {
    revision: i64,
    preferences: DashboardPreferences,
    updated_at_unix_ms: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardPreferencesConflict {
    code: &'static str,
    message: &'static str,
    current: DashboardPreferencesResponse,
}

async fn user_preferences(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let authenticated = auth::require_capability(&state, &headers, "dashboard.customize").await?;
    let databases = Arc::clone(&state.databases);
    let user_id = authenticated.user_id;
    let record = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases.state().user_preferences(&user_id)
    })
    .await?;
    let response = preferences_response(record)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(response)).into_response())
}

async fn update_user_preferences(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<DashboardPreferencesUpdate>, JsonRejection>,
) -> Result<Response, ApiError> {
    auth::validate_post_headers(&headers)?;
    let authenticated = auth::require_capability(&state, &headers, "dashboard.customize").await?;
    let Json(mut body) = body.map_err(auth::map_json_rejection)?;
    if body.expected_revision < 0 || body.expected_revision.checked_add(1).is_none() {
        return Err(ApiError::InvalidPreferences);
    }
    reconcile_legacy_home_preferences(&mut body.preferences);
    validate_dashboard_preferences(&body.preferences).map_err(|()| ApiError::InvalidPreferences)?;
    let preferences_json =
        serde_json::to_string(&body.preferences).map_err(|_| ApiError::ServiceUnavailable)?;
    if !(2..=65_536).contains(&preferences_json.len()) {
        return Err(ApiError::InvalidPreferences);
    }
    let session_hash = auth::session_hash_from_headers(&headers)?;
    if !state.attempt_limiter.allow_preference_write(
        peer.ip(),
        &authenticated.user_id,
        &session_hash,
    ) {
        return Err(ApiError::PreferenceWriteRateLimited);
    }
    let databases = Arc::clone(&state.databases);
    let user_id = authenticated.user_id;
    let expected_revision = body.expected_revision;
    let now = i64::try_from(unix_timestamp_ms()).unwrap_or(i64::MAX);
    let outcome = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases
            .state()
            .update_user_preferences(UserPreferencesUpdateInput {
                user_id: &user_id,
                expected_revision,
                preferences_json: &preferences_json,
                now_unix_ms: now,
            })
    })
    .await?;
    match outcome {
        UserPreferencesUpdateOutcome::Updated(record) => Ok((
            [(header::CACHE_CONTROL, "no-store")],
            Json(preferences_response(Some(record))?),
        )
            .into_response()),
        UserPreferencesUpdateOutcome::Conflict(record) => Ok((
            StatusCode::CONFLICT,
            [(header::CACHE_CONTROL, "no-store")],
            Json(DashboardPreferencesConflict {
                code: "preferences_revision_conflict",
                message: "Dashboard preferences changed in another session.",
                current: preferences_response(record)?,
            }),
        )
            .into_response()),
    }
}

fn preferences_response(
    record: Option<UserPreferencesRecord>,
) -> Result<DashboardPreferencesResponse, ApiError> {
    match record {
        Some(record) => {
            let mut preferences: DashboardPreferences =
                serde_json::from_str(&record.preferences_json)
                    .map_err(|_| ApiError::ServiceUnavailable)?;
            reconcile_legacy_home_preferences(&mut preferences);
            validate_dashboard_preferences(&preferences)
                .map_err(|()| ApiError::ServiceUnavailable)?;
            Ok(DashboardPreferencesResponse {
                revision: record.revision,
                preferences,
                updated_at_unix_ms: Some(record.updated_at_unix_ms),
            })
        }
        None => Ok(DashboardPreferencesResponse {
            revision: 0,
            preferences: DashboardPreferences::default(),
            updated_at_unix_ms: None,
        }),
    }
}

fn reconcile_legacy_home_preferences(preferences: &mut DashboardPreferences) {
    if !preferences
        .navigation_order
        .contains(&PrimaryDashboardSection::Terminal)
    {
        let insertion = preferences
            .navigation_order
            .iter()
            .position(|section| *section == PrimaryDashboardSection::Servers)
            .unwrap_or(preferences.navigation_order.len());
        preferences
            .navigation_order
            .insert(insertion, PrimaryDashboardSection::Terminal);
    }
    if !preferences
        .navigation_order
        .contains(&PrimaryDashboardSection::Security)
    {
        let insertion = preferences
            .navigation_order
            .iter()
            .position(|section| *section == PrimaryDashboardSection::Terminal)
            .unwrap_or(preferences.navigation_order.len());
        preferences
            .navigation_order
            .insert(insertion, PrimaryDashboardSection::Security);
    }
    if !preferences
        .navigation_order
        .contains(&PrimaryDashboardSection::Hooks)
    {
        preferences
            .navigation_order
            .push(PrimaryDashboardSection::Hooks);
    }
    if !preferences
        .navigation_order
        .contains(&PrimaryDashboardSection::Strands)
    {
        preferences
            .navigation_order
            .push(PrimaryDashboardSection::Strands);
    }
    if !preferences
        .navigation_order
        .contains(&PrimaryDashboardSection::Globe)
    {
        preferences
            .navigation_order
            .push(PrimaryDashboardSection::Globe);
    }
    if preferences.home_templates.is_empty() {
        preferences.home_templates.push(HomeTemplatePreference {
            id: "home-main".to_owned(),
            name: "Main".to_owned(),
            accent: "#d7f64d".to_owned(),
            widgets: preferences.home_widgets.clone(),
        });
    }
    if preferences.active_home_id.is_empty()
        || !preferences
            .home_templates
            .iter()
            .any(|template| template.id == preferences.active_home_id)
    {
        preferences.active_home_id = preferences
            .home_templates
            .first()
            .map(|template| template.id.clone())
            .unwrap_or_default();
    }
    if let Some(active) = preferences
        .home_templates
        .iter()
        .find(|template| template.id == preferences.active_home_id)
    {
        preferences.home_widgets = active.widgets.clone();
    }
}

fn validate_dashboard_preferences(preferences: &DashboardPreferences) -> Result<(), ()> {
    if !matches!(
        preferences.metrics_refresh_ms,
        1_000 | 2_000 | 5_000 | 10_000 | 30_000
    ) || preferences.navigation_order.len() != 11
        || preferences.home_widgets.len() > 32
        || preferences.home_templates.is_empty()
        || preferences.home_templates.len() > 8
    {
        return Err(());
    }
    let mut section_mask = 0_u16;
    for section in &preferences.navigation_order {
        let bit = match section {
            PrimaryDashboardSection::Overview => 1 << 0,
            PrimaryDashboardSection::Home => 1 << 1,
            PrimaryDashboardSection::Storage => 1 << 2,
            PrimaryDashboardSection::Network => 1 << 3,
            PrimaryDashboardSection::Host => 1 << 4,
            PrimaryDashboardSection::Security => 1 << 5,
            PrimaryDashboardSection::Terminal => 1 << 6,
            PrimaryDashboardSection::Servers => 1 << 7,
            PrimaryDashboardSection::Hooks => 1 << 8,
            PrimaryDashboardSection::Strands => 1 << 9,
            PrimaryDashboardSection::Globe => 1 << 10,
        };
        if section_mask & bit != 0 {
            return Err(());
        }
        section_mask |= bit;
    }
    if section_mask != 0b111_1111_1111 {
        return Err(());
    }
    let mut hidden_mask = 0_u16;
    if preferences.hidden_pages.len() > 11 {
        return Err(());
    }
    for section in &preferences.hidden_pages {
        let bit = match section {
            PrimaryDashboardSection::Overview => 1 << 0,
            PrimaryDashboardSection::Home => 1 << 1,
            PrimaryDashboardSection::Storage => 1 << 2,
            PrimaryDashboardSection::Network => 1 << 3,
            PrimaryDashboardSection::Host => 1 << 4,
            PrimaryDashboardSection::Security => 1 << 5,
            PrimaryDashboardSection::Terminal => 1 << 6,
            PrimaryDashboardSection::Servers => 1 << 7,
            PrimaryDashboardSection::Hooks => 1 << 8,
            PrimaryDashboardSection::Strands => 1 << 9,
            PrimaryDashboardSection::Globe => 1 << 10,
        };
        if hidden_mask & bit != 0 {
            return Err(());
        }
        hidden_mask |= bit;
    }

    validate_home_widgets(&preferences.home_widgets)?;
    let mut template_ids =
        std::collections::HashSet::with_capacity(preferences.home_templates.len());
    let mut total_widgets = 0_usize;
    let mut active_widgets = None;
    for template in &preferences.home_templates {
        total_widgets = total_widgets
            .checked_add(template.widgets.len())
            .ok_or(())?;
        if template.id.is_empty()
            || template.id.len() > 96
            || !template
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            || !template_ids.insert(template.id.as_str())
            || template.name.trim() != template.name
            || template.name.is_empty()
            || template.name.chars().count() > 48
            || !valid_hex_color(&template.accent)
            || template.widgets.len() > 32
        {
            return Err(());
        }
        validate_home_widgets(&template.widgets)?;
        if template.id == preferences.active_home_id {
            active_widgets = Some(template.widgets.as_slice());
        }
    }
    if total_widgets > 64 || active_widgets != Some(preferences.home_widgets.as_slice()) {
        return Err(());
    }
    for color in [
        &preferences.colors.accent,
        &preferences.colors.text,
        &preferences.colors.surface,
    ] {
        if !color.is_empty() && !valid_hex_color(color) {
            return Err(());
        }
    }
    Ok(())
}

fn validate_home_widgets(widgets: &[HomeWidgetPreference]) -> Result<(), ()> {
    let mut widget_ids = std::collections::HashSet::with_capacity(widgets.len());
    for widget in widgets {
        if widget.id.is_empty()
            || widget.id.len() > 96
            || !widget
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            || !widget_ids.insert(widget.id.as_str())
            || widget.title.trim() != widget.title
            || widget.title.is_empty()
            || widget.title.chars().count() > 80
            || widget.content.chars().count() > 8_000
            || widget.url.chars().count() > 2_048
            || (!widget.color.is_empty() && !valid_hex_color(&widget.color))
        {
            return Err(());
        }
        match widget.kind {
            HomeWidgetKind::Shortcut => {
                let uri = widget.url.parse::<axum::http::Uri>().map_err(|_| ())?;
                if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.authority().is_none()
                {
                    return Err(());
                }
            }
            HomeWidgetKind::Strand => {
                if widget.url.len() != 36
                    || widget
                        .url
                        .bytes()
                        .any(|byte| !byte.is_ascii_hexdigit() && byte != b'-')
                {
                    return Err(());
                }
            }
            HomeWidgetKind::Globe => {
                if !widget.url.is_empty() {
                    return Err(());
                }
            }
            _ if !widget.url.is_empty() => return Err(()),
            _ => {}
        }
    }
    Ok(())
}

fn valid_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryQuery {
    path: String,
    cursor: Option<String>,
    #[serde(default = "default_directory_page_limit")]
    limit: u16,
}

const fn default_directory_page_limit() -> u16 {
    50
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateEntryBody {
    parent: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathBody {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageAnalysisStartBody {
    path: String,
    mode: StorageAnalysisMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteTextBody {
    path: String,
    content: String,
    expected_modified_unix_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BeginFileUploadBody {
    target: FileUploadTarget,
    name: String,
    expected_size: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileUploadChunkBody {
    upload_id: String,
    purpose: FileUploadPurpose,
    offset: u64,
    data_base64: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileUploadFinishBody {
    upload_id: String,
    purpose: FileUploadPurpose,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MinecraftVersionsQuery {
    software: MinecraftSoftware,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameBody {
    path: String,
    new_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AmpRouterForwardReleaseBody {
    port: u16,
    confirmation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnableFirewallBody {
    ssh_port: u16,
    confirmation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyPackageActionBody {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplySystemPackageUpdatesBody {
    packages: Vec<PackageUpdateCandidate>,
    confirmation: String,
    disruption_acknowledged: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyHelixUpdateBody {
    target_tag: String,
    confirmation: String,
    disruption_acknowledged: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerActionBody {
    action: ServerAction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerNetworkExposureBody {
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveNativeServerBody {
    confirmation_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerLogsQuery {
    lines: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerLogHistoryQuery {
    cursor: Option<String>,
    lines: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerMarketplaceSearchQuery {
    #[serde(default)]
    query: String,
    #[serde(default)]
    offset: u32,
    #[serde(default = "default_marketplace_search_limit")]
    limit: u8,
    #[serde(default)]
    provider: ModpackProvider,
    #[serde(default)]
    catalog: MarketplaceCatalog,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MinecraftModpackSearchQuery {
    #[serde(default)]
    query: String,
    #[serde(default)]
    offset: u32,
    #[serde(default = "default_marketplace_search_limit")]
    limit: u8,
    #[serde(default)]
    provider: ModpackProvider,
}

const fn default_marketplace_search_limit() -> u8 {
    20
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerMarketplaceProjectQuery {
    #[serde(default)]
    provider: ModpackProvider,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallServerMarketplaceContentBody {
    project_id: String,
    version_id: Option<String>,
    #[serde(default)]
    provider: ModpackProvider,
    #[serde(default)]
    restart_server: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupPolicyBody {
    keep_count: u16,
    keep_days: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartOnBootBody {
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeMemoryBody {
    memory_mb: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeCpuBody {
    cpu_millis: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeBrowserListingBody {
    list_on_browser: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookServiceActionBody {
    action: HookServiceAction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookInstallBody {
    confirmation: String,
    repository_change_acknowledged: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DockerContainerActionBody {
    name: String,
    action: DockerContainerActionKind,
    confirmation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecurityControlBody {
    id: String,
    enabled: bool,
    confirmation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleHostRebootBody {
    confirmation_hostname: String,
    delay_seconds: u16,
    disruption_acknowledged: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteRecurringHostRebootBody {
    confirmation_hostname: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerConsoleBody {
    command: String,
}

async fn host_inventory(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::HostInventory {}).await
}

async fn host_integration_status(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::HostIntegrationStatus {}).await
}

async fn set_helix_start_on_boot(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<StartOnBootBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::SetHelixStartOnBoot {
            enabled: body.enabled,
        },
    )
    .await
}

async fn host_reboot_preflight(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::HostRebootPreflight {}).await
}

async fn schedule_host_reboot(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<ScheduleHostRebootBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.power").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::ScheduleHostReboot {
            confirmation_hostname: body.confirmation_hostname,
            delay_seconds: body.delay_seconds,
            disruption_acknowledged: body.disruption_acknowledged,
        },
    )
    .await
}

async fn set_recurring_host_reboot(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<RecurringRebootSpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.power").await?;
    let Json(schedule) = body.map_err(auth::map_json_rejection)?;
    broker_json(&state, BrokerRequest::SetRecurringHostReboot { schedule }).await
}

async fn delete_recurring_host_reboot(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<DeleteRecurringHostRebootBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.power").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::DeleteRecurringHostReboot {
            confirmation_hostname: body.confirmation_hostname,
        },
    )
    .await
}

async fn cancel_host_reboot(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(operation_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.power").await?;
    broker_json(&state, BrokerRequest::CancelHostReboot { operation_id }).await
}

async fn network_inventory(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "network.firewall.read").await?;
    broker_json(&state, BrokerRequest::NetworkInventory {}).await
}

async fn network_globe(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::GlobeSnapshot {}).await
}

async fn release_amp_router_forward(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<AmpRouterForwardReleaseBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    auth::require_capability(&state, &headers, "network.firewall.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::ReleaseAmpRouterForward {
            port: body.port,
            confirmation: body.confirmation,
        },
    )
    .await
}

async fn minecraft_port_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::GamePortPolicy {
            game: GameKind::Minecraft,
        },
    )
    .await
}

async fn set_minecraft_port_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<GamePortPolicySpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(policy) = body.map_err(auth::map_json_rejection)?;
    if policy.game != GameKind::Minecraft {
        return Err(ApiError::BrokerRejected(
            "the policy game must match Minecraft".to_owned(),
        ));
    }
    broker_json(&state, BrokerRequest::SetGamePortPolicy { policy }).await
}

async fn vrising_port_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::GamePortPolicy {
            game: GameKind::VRising,
        },
    )
    .await
}

async fn set_vrising_port_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<GamePortPolicySpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(policy) = body.map_err(auth::map_json_rejection)?;
    if policy.game != GameKind::VRising {
        return Err(ApiError::BrokerRejected(
            "the policy game must match V Rising".to_owned(),
        ));
    }
    broker_json(&state, BrokerRequest::SetGamePortPolicy { policy }).await
}

async fn valheim_port_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::GamePortPolicy {
            game: GameKind::Valheim,
        },
    )
    .await
}

async fn set_valheim_port_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<GamePortPolicySpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(policy) = body.map_err(auth::map_json_rejection)?;
    if policy.game != GameKind::Valheim {
        return Err(ApiError::BrokerRejected(
            "the policy game must match Valheim".to_owned(),
        ));
    }
    broker_json(&state, BrokerRequest::SetGamePortPolicy { policy }).await
}

async fn terraria_port_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::GamePortPolicy {
            game: GameKind::Terraria,
        },
    )
    .await
}

async fn set_terraria_port_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<GamePortPolicySpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(policy) = body.map_err(auth::map_json_rejection)?;
    if policy.game != GameKind::Terraria {
        return Err(ApiError::BrokerRejected(
            "the policy game must match Terraria".to_owned(),
        ));
    }
    broker_json(&state, BrokerRequest::SetGamePortPolicy { policy }).await
}

async fn create_firewall_rule(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<FirewallRuleSpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "network.firewall.write").await?;
    let Json(rule) = body.map_err(auth::map_json_rejection)?;
    broker_json(&state, BrokerRequest::CreateFirewallRule { rule }).await
}

async fn delete_firewall_rule(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(rule_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "network.firewall.write").await?;
    broker_json(&state, BrokerRequest::DeleteFirewallRule { rule_id }).await
}

async fn restore_firewall_rule(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(rule_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "network.firewall.write").await?;
    broker_json(&state, BrokerRequest::RestoreFirewallRule { rule_id }).await
}

async fn enable_firewall(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<EnableFirewallBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "network.firewall.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::EnableFirewall {
            ssh_port: body.ssh_port,
            confirmation: body.confirmation,
        },
    )
    .await
}

async fn system_package_inventory(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.packages.read").await?;
    broker_json(&state, BrokerRequest::SystemPackageInventory {}).await
}

async fn refresh_system_package_lists(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<EmptyPackageActionBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.packages.write").await?;
    let Json(EmptyPackageActionBody {}) = body.map_err(auth::map_json_rejection)?;
    broker_json(&state, BrokerRequest::RefreshSystemPackageLists {}).await
}

async fn apply_system_package_updates(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<ApplySystemPackageUpdatesBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.packages.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::ApplySystemPackageUpdates {
            packages: body.packages,
            confirmation: body.confirmation,
            disruption_acknowledged: body.disruption_acknowledged,
        },
    )
    .await
}

async fn check_helix_update(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<EmptyPackageActionBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.packages.read").await?;
    let Json(EmptyPackageActionBody {}) = body.map_err(auth::map_json_rejection)?;
    broker_json(&state, BrokerRequest::CheckHelixUpdate {}).await
}

async fn apply_helix_update(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<ApplyHelixUpdateBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.packages.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::ApplyHelixUpdate {
            target_tag: body.target_tag,
            confirmation: body.confirmation,
            disruption_acknowledged: body.disruption_acknowledged,
        },
    )
    .await
}

async fn system_package_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(job_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.packages.read").await?;
    broker_json(&state, BrokerRequest::JobStatus { job_id }).await
}

async fn hook_inventory(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::HookInventory {}).await
}

async fn hook_install_preflight(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(hook_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::HookInstallPreflight { hook_id }).await
}

async fn install_hook(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(hook_id): RoutePath<String>,
    body: Result<Json<HookInstallBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::InstallHook {
            hook_id,
            confirmation: body.confirmation,
            repository_change_acknowledged: body.repository_change_acknowledged,
        },
    )
    .await
}

async fn hook_install_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(job_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::JobStatus { job_id }).await
}

async fn manage_hook_service(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(hook_id): RoutePath<String>,
    body: Result<Json<HookServiceActionBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::ManageHookService {
            hook_id,
            action: body.action,
        },
    )
    .await
}

async fn docker_inventory(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::DockerInventory {}).await
}

async fn docker_container_action(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<DockerContainerActionBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::DockerContainerAction {
            name: body.name,
            action: body.action,
            confirmation: body.confirmation,
        },
    )
    .await
}

async fn homarr_widget_catalog(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "dashboard.customize").await?;
    broker_json(&state, BrokerRequest::HomarrWidgetCatalog {}).await
}

async fn security_inventory(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    broker_json(&state, BrokerRequest::SecurityInventory {}).await
}

async fn set_security_control(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<SecurityControlBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::SetSecurityControl {
            id: body.id,
            enabled: body.enabled,
            confirmation: body.confirmation,
        },
    )
    .await
}

async fn list_directory(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<DirectoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "storage.files.read").await?;
    broker_json(
        &state,
        BrokerRequest::ListDirectory {
            path: query.path,
            cursor: query.cursor,
            limit: query.limit,
        },
    )
    .await
}

async fn create_directory(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<CreateEntryBody>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "storage.files.manage").await?;
    broker_json(
        &state,
        BrokerRequest::CreateDirectory {
            parent: body.parent,
            name: body.name,
        },
    )
    .await
}

async fn create_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<CreateEntryBody>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "storage.files.manage").await?;
    broker_json(
        &state,
        BrokerRequest::CreateFile {
            parent: body.parent,
            name: body.name,
        },
    )
    .await
}

async fn read_text_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "storage.files.read").await?;
    broker_json(&state, BrokerRequest::ReadText { path: body.path }).await
}

async fn write_text_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<WriteTextBody>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "storage.files.manage").await?;
    broker_json(
        &state,
        BrokerRequest::WriteText {
            path: body.path,
            content: body.content,
            expected_modified_unix_ms: body.expected_modified_unix_ms,
        },
    )
    .await
}

async fn rename_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<RenameBody>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "storage.files.manage").await?;
    broker_json(
        &state,
        BrokerRequest::Rename {
            path: body.path,
            new_name: body.new_name,
        },
    )
    .await
}

async fn trash_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "storage.files.manage").await?;
    broker_json(&state, BrokerRequest::Trash { path: body.path }).await
}

async fn begin_file_upload(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<BeginFileUploadBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    match &body.target {
        FileUploadTarget::Directory { .. } => {
            auth::require_capability(&state, &headers, "storage.files.manage").await?;
        }
        FileUploadTarget::CustomJar => {
            auth::require_capability(&state, &headers, "games.manage").await?;
        }
    }
    broker_json(
        &state,
        BrokerRequest::BeginFileUpload {
            target: body.target,
            name: body.name,
            expected_size: body.expected_size,
        },
    )
    .await
}

async fn write_file_upload_chunk(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<FileUploadChunkBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    require_upload_capability(&state, &headers, body.purpose).await?;
    broker_json(
        &state,
        BrokerRequest::WriteFileUploadChunk {
            upload_id: body.upload_id,
            purpose: body.purpose,
            offset: body.offset,
            data_base64: body.data_base64,
        },
    )
    .await
}

async fn finish_file_upload(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<FileUploadFinishBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    require_upload_capability(&state, &headers, body.purpose).await?;
    broker_json(
        &state,
        BrokerRequest::FinishFileUpload {
            upload_id: body.upload_id,
            purpose: body.purpose,
        },
    )
    .await
}

async fn abort_file_upload(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<FileUploadFinishBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    require_upload_capability(&state, &headers, body.purpose).await?;
    broker_json(
        &state,
        BrokerRequest::AbortFileUpload {
            upload_id: body.upload_id,
            purpose: body.purpose,
        },
    )
    .await
}

async fn require_upload_capability(
    state: &ApiState,
    headers: &HeaderMap,
    purpose: FileUploadPurpose,
) -> Result<(), ApiError> {
    match purpose {
        FileUploadPurpose::Storage => {
            auth::require_capability(state, headers, "storage.files.manage").await?;
        }
        FileUploadPurpose::CustomJar => {
            auth::require_capability(state, headers, "games.manage").await?;
        }
    }
    Ok(())
}

async fn list_minecraft_versions(
    State(state): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<MinecraftVersionsQuery>, QueryRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    let Query(query) = query.map_err(|_| ApiError::InvalidJson)?;
    broker_json(
        &state,
        BrokerRequest::ListMinecraftVersions {
            software: query.software,
        },
    )
    .await
}

async fn start_storage_analysis(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<StorageAnalysisStartBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    let authenticated = auth::require_capability(&state, &headers, "storage.analyze").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    if body.path.is_empty()
        || body.path.len() > 4096
        || !body.path.starts_with('/')
        || body.path.chars().any(char::is_control)
    {
        return Err(ApiError::InvalidStorageAnalysisRequest);
    }
    let session_hash = auth::session_hash_from_headers(&headers)?;
    if !state.attempt_limiter.allow_storage_analysis_start(
        peer.ip(),
        &authenticated.user_id,
        &session_hash,
    ) {
        return Err(ApiError::StorageAnalysisRateLimited);
    }
    let mut response = broker_json(
        &state,
        BrokerRequest::StartStorageAnalysis {
            path: body.path,
            mode: body.mode,
        },
    )
    .await?;
    *response.status_mut() = StatusCode::ACCEPTED;
    Ok(response)
}

async fn storage_analysis_status(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    RoutePath(job_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let authenticated = auth::require_capability(&state, &headers, "storage.analyze").await?;
    if !valid_storage_analysis_job_id(&job_id) {
        return Err(ApiError::InvalidStorageAnalysisRequest);
    }
    let session_hash = auth::session_hash_from_headers(&headers)?;
    if !state.attempt_limiter.allow_storage_analysis_read(
        peer.ip(),
        &authenticated.user_id,
        &session_hash,
    ) {
        return Err(ApiError::StorageAnalysisRateLimited);
    }
    broker_json(&state, BrokerRequest::StorageAnalysisStatus { job_id }).await
}

async fn cancel_storage_analysis(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    RoutePath(job_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    let authenticated = auth::require_capability(&state, &headers, "storage.analyze").await?;
    if !valid_storage_analysis_job_id(&job_id) {
        return Err(ApiError::InvalidStorageAnalysisRequest);
    }
    let session_hash = auth::session_hash_from_headers(&headers)?;
    if !state.attempt_limiter.allow_storage_analysis_read(
        peer.ip(),
        &authenticated.user_id,
        &session_hash,
    ) {
        return Err(ApiError::StorageAnalysisRateLimited);
    }
    broker_json(&state, BrokerRequest::CancelStorageAnalysis { job_id }).await
}

fn valid_storage_analysis_job_id(job_id: &str) -> bool {
    job_id.len() == 36
        && job_id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

async fn list_servers(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    let mut value = broker_value(&state, BrokerRequest::ListServers {}).await?;
    let databases = Arc::clone(&state.databases);
    let tracker = state.blocking_tasks.clone();
    let appearances = tokio::task::spawn_blocking(move || {
        let _guard = tracker.start();
        databases.state().server_appearance_summaries()
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable)?
    .map_err(|_| ApiError::ServiceUnavailable)?;
    server_media::attach_appearances(&mut value, &appearances)
        .map_err(|()| ApiError::BrokerUnavailable)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response())
}

async fn list_trashed_servers(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(&state, BrokerRequest::ListTrashedServers {}).await
}

async fn trash_native_server(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<RemoveNativeServerBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::TrashNativeServer {
            instance_id,
            confirmation_name: body.confirmation_name,
        },
    )
    .await
}

async fn restore_trashed_server(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(trash_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    broker_json(&state, BrokerRequest::RestoreTrashedServer { trash_id }).await
}

async fn server_appearance(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    server_media::validate_server_id(&instance_id)?;
    let databases = Arc::clone(&state.databases);
    let tracker = state.blocking_tasks.clone();
    let appearance = tokio::task::spawn_blocking(move || {
        let _guard = tracker.start();
        databases.state().server_appearance(&instance_id)
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable)?
    .map_err(|_| ApiError::ServiceUnavailable)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(server_media::appearance_json(
            appearance.as_ref().map(|record| &record.summary),
        )),
    )
        .into_response())
}

async fn set_server_appearance(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<server_media::ServerAppearanceBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    server_media::validate_server_id(&instance_id)?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    ensure_server_exists(&state, &instance_id).await?;
    let (expected_revision, owned_update) = server_media::validate_appearance_body(body)?;
    let now_unix_ms =
        i64::try_from(unix_timestamp_ms()).map_err(|_| ApiError::ServiceUnavailable)?;
    let databases = Arc::clone(&state.databases);
    let tracker = state.blocking_tasks.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let _guard = tracker.start();
        let update = owned_update.as_borrowed();
        databases.state().update_server_appearance(
            &instance_id,
            expected_revision,
            update,
            now_unix_ms,
        )
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable)?
    .map_err(|_| ApiError::InvalidServerAppearance)?;
    server_appearance_update_response(outcome)
}

async fn clear_server_appearance(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<server_media::ClearServerAppearanceBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    server_media::validate_server_id(&instance_id)?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    if body.expected_revision < 0 {
        return Err(ApiError::InvalidServerAppearance);
    }
    let databases = Arc::clone(&state.databases);
    let tracker = state.blocking_tasks.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let _guard = tracker.start();
        databases
            .state()
            .clear_server_appearance(&instance_id, body.expected_revision)
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable)?
    .map_err(|_| ApiError::InvalidServerAppearance)?;
    server_appearance_update_response(outcome)
}

async fn server_appearance_image(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    query: Result<Query<server_media::ServerAppearanceImageQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    server_media::validate_server_id(&instance_id)?;
    let Query(query) = query.map_err(|_| ApiError::InvalidServerAppearance)?;
    if query.revision < 1 {
        return Err(ApiError::InvalidServerAppearance);
    }
    let databases = Arc::clone(&state.databases);
    let tracker = state.blocking_tasks.clone();
    let appearance = tokio::task::spawn_blocking(move || {
        let _guard = tracker.start();
        databases.state().server_appearance(&instance_id)
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable)?
    .map_err(|_| ApiError::ServiceUnavailable)?
    .filter(|record| record.summary.revision == query.revision)
    .ok_or(ApiError::NotFound)?;
    let content_type = appearance
        .summary
        .content_type
        .as_deref()
        .ok_or(ApiError::NotFound)?;
    let image = appearance.image_bytes.ok_or(ApiError::NotFound)?;
    let content_type =
        HeaderValue::from_str(content_type).map_err(|_| ApiError::ServiceUnavailable)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        ],
        image,
    )
        .into_response())
}

fn server_appearance_update_response(
    outcome: ServerAppearanceUpdateOutcome,
) -> Result<Response, ApiError> {
    match outcome {
        ServerAppearanceUpdateOutcome::Updated(summary) => Ok((
            [(header::CACHE_CONTROL, "no-store")],
            Json(server_media::appearance_json(summary.as_ref())),
        )
            .into_response()),
        ServerAppearanceUpdateOutcome::Conflict(_) => Err(ApiError::ServerAppearanceConflict),
    }
}

async fn ensure_server_exists(state: &ApiState, instance_id: &str) -> Result<(), ApiError> {
    let value = broker_value(state, BrokerRequest::ListServers {}).await?;
    if server_media::server_list_contains(&value, instance_id) {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

async fn server_inventory_health(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(&state, BrokerRequest::ServerInventoryHealth {}).await
}

async fn server_detail(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(&state, BrokerRequest::ServerDetail { instance_id }).await
}

async fn server_logs(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    Query(query): Query<ServerLogsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::ServerLogs {
            instance_id,
            lines: query.lines.unwrap_or(300),
        },
    )
    .await
}

async fn server_log_history(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    Query(query): Query<ServerLogHistoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::ServerLogHistory {
            instance_id,
            cursor: query.cursor,
            lines: query.lines.unwrap_or(200),
        },
    )
    .await
}

async fn server_console(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<ServerConsoleBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::ServerConsole {
            instance_id,
            command: body.command,
        },
    )
    .await
}

async fn server_settings(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(&state, BrokerRequest::ServerSettings { instance_id }).await
}

async fn update_server_settings(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<MinecraftSettingsPatch>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(settings) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::UpdateServerSettings {
            instance_id,
            settings,
        },
    )
    .await
}

async fn list_server_backups(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(&state, BrokerRequest::ListBackups { instance_id }).await
}

async fn restore_server_backup(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath((instance_id, backup_id)): RoutePath<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.backups.manage").await?;
    broker_json(
        &state,
        BrokerRequest::RestoreBackup {
            instance_id,
            backup_id,
        },
    )
    .await
}

async fn trash_server_backup(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath((instance_id, backup_id)): RoutePath<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.backups.manage").await?;
    broker_json(
        &state,
        BrokerRequest::TrashBackup {
            instance_id,
            backup_id,
        },
    )
    .await
}

async fn restore_trashed_server_backup(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath((instance_id, trash_id)): RoutePath<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.backups.manage").await?;
    broker_json(
        &state,
        BrokerRequest::RestoreTrashedBackup {
            instance_id,
            trash_id,
        },
    )
    .await
}

async fn purge_trashed_server_backup(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath((instance_id, trash_id)): RoutePath<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.backups.manage").await?;
    broker_json(
        &state,
        BrokerRequest::PurgeBackupTrash {
            instance_id,
            trash_id,
        },
    )
    .await
}

async fn set_server_backup_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<BackupPolicyBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.backups.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::SetBackupPolicy {
            instance_id,
            keep_count: body.keep_count,
            keep_days: body.keep_days,
        },
    )
    .await
}

async fn prune_server_backups(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.backups.manage").await?;
    broker_json(&state, BrokerRequest::PruneBackups { instance_id }).await
}

async fn server_action(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<ServerActionBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::ServerAction {
            instance_id,
            action: body.action,
        },
    )
    .await
}

async fn set_server_network_exposure(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<ServerNetworkExposureBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    auth::require_capability(&state, &headers, "network.firewall.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::SetServerNetworkExposure {
            instance_id,
            enabled: body.enabled,
        },
    )
    .await
}

async fn server_marketplace_search(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    Query(query): Query<ServerMarketplaceSearchQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::ServerMarketplaceSearch {
            instance_id,
            query: query.query,
            offset: query.offset,
            limit: query.limit,
            provider: query.provider,
            catalog: query.catalog,
        },
    )
    .await
}

async fn server_marketplace_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath((instance_id, project_id)): RoutePath<(String, String)>,
    Query(query): Query<ServerMarketplaceProjectQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::ServerMarketplaceProject {
            instance_id,
            project_id,
            provider: query.provider,
        },
    )
    .await
}

async fn install_server_marketplace_content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<InstallServerMarketplaceContentBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::InstallServerMarketplaceContent {
            instance_id,
            project_id: body.project_id,
            version_id: body.version_id,
            provider: body.provider,
            restart_server: body.restart_server,
        },
    )
    .await
}

async fn create_minecraft(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<MinecraftCreateSpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(spec) = body.map_err(auth::map_json_rejection)?;
    if spec.network_exposure == ServerNetworkExposure::Public {
        auth::require_capability(&state, &headers, "network.firewall.write").await?;
    }
    broker_json(&state, BrokerRequest::CreateMinecraft { spec }).await
}

async fn create_vrising(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<VRisingCreateSpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(mut spec) = body.map_err(auth::map_json_rejection)?;
    spec.wine_runtime_acknowledged = true;
    spec.validate().map_err(ApiError::BrokerRejected)?;
    if spec.network_exposure == ServerNetworkExposure::Public {
        auth::require_capability(&state, &headers, "network.firewall.write").await?;
    }
    broker_json(&state, BrokerRequest::CreateVRising { spec }).await
}

async fn create_valheim(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<ValheimCreateSpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(spec) = body.map_err(auth::map_json_rejection)?;
    spec.validate().map_err(ApiError::BrokerRejected)?;
    if spec.network_exposure == ServerNetworkExposure::Public {
        auth::require_capability(&state, &headers, "network.firewall.write").await?;
    }
    broker_json(&state, BrokerRequest::CreateValheim { spec }).await
}

async fn create_terraria(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<TerrariaCreateSpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(spec) = body.map_err(auth::map_json_rejection)?;
    spec.validate().map_err(ApiError::BrokerRejected)?;
    if spec.network_exposure == ServerNetworkExposure::Public {
        auth::require_capability(&state, &headers, "network.firewall.write").await?;
    }
    broker_json(&state, BrokerRequest::CreateTerraria { spec }).await
}

async fn set_native_start_on_boot(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<StartOnBootBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::SetNativeStartOnBoot {
            instance_id,
            enabled: body.enabled,
        },
    )
    .await
}

async fn set_native_memory(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<NativeMemoryBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::SetNativeMemory {
            instance_id,
            memory_mb: body.memory_mb,
        },
    )
    .await
}

async fn set_native_cpu(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<NativeCpuBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    helix_privd::validate_cpu_millis(body.cpu_millis).map_err(ApiError::BrokerRejected)?;
    broker_json(
        &state,
        BrokerRequest::SetNativeCpu {
            instance_id,
            cpu_millis: body.cpu_millis,
        },
    )
    .await
}

async fn set_native_browser_listing(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(instance_id): RoutePath<String>,
    body: Result<Json<NativeBrowserListingBody>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    broker_json(
        &state,
        BrokerRequest::SetNativeBrowserListing {
            instance_id,
            list_on_browser: body.list_on_browser,
        },
    )
    .await
}

async fn minecraft_modpack_search(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<MinecraftModpackSearchQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::MinecraftModpackSearch {
            query: query.query,
            offset: query.offset,
            limit: query.limit,
            provider: query.provider,
        },
    )
    .await
}

async fn minecraft_modpack_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(project_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(
        &state,
        BrokerRequest::MinecraftModpackProject { project_id },
    )
    .await
}

async fn create_minecraft_modpack(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<MinecraftModpackCreateSpec>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "games.manage").await?;
    let Json(spec) = body.map_err(auth::map_json_rejection)?;
    if spec.network_exposure == ServerNetworkExposure::Public {
        auth::require_capability(&state, &headers, "network.firewall.write").await?;
    }
    broker_json(&state, BrokerRequest::CreateMinecraftModpack { spec }).await
}

async fn job_status(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(job_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    broker_json(&state, BrokerRequest::JobStatus { job_id }).await
}

async fn broker_json(state: &ApiState, request: BrokerRequest) -> Result<Response, ApiError> {
    let value = broker_value(state, request).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response())
}

async fn broker_value(
    state: &ApiState,
    request: BrokerRequest,
) -> Result<serde_json::Value, ApiError> {
    let broker = state.broker.clone().ok_or(ApiError::BrokerUnavailable)?;
    let tracker = state.blocking_tasks.clone();
    let value = tokio::task::spawn_blocking(move || {
        let _guard = tracker.start();
        broker.request(&request)
    })
    .await
    .map_err(|_| ApiError::BrokerUnavailable)?
    .map_err(|error| match error {
        BrokerClientError::Rejected(message) => ApiError::BrokerRejected(message),
        BrokerClientError::Unavailable | BrokerClientError::Protocol => ApiError::BrokerUnavailable,
    })?;
    Ok(value)
}

async fn game_hosting_readiness(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "games.view").await?;
    if state.broker.is_none() {
        return Ok((
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({
                "schema_version": 1,
                "availability": "unavailable",
                "available_features": [],
                "blockers": [
                    { "code": "verified_restore", "status": "required" },
                    { "code": "privileged_broker", "status": "required" },
                    { "code": "native_execution", "status": "required" }
                ],
                "collected_at_unix_ms": unix_timestamp_ms()
            })),
        )
            .into_response());
    }
    broker_json(&state, BrokerRequest::ServerManagerReadiness {}).await
}

async fn api_not_found() -> ApiError {
    ApiError::NotFound
}

#[derive(Debug)]
pub(crate) enum ApiError {
    AccountConflict,
    AccountRejected,
    ApplicationCapacityExhausted,
    AttemptRateLimited,
    AuthenticationRequired,
    AuthorizationDenied,
    BrokerRejected(String),
    BrokerUnavailable,
    CrossSiteRequest,
    CsrfRejected,
    CurrentPasswordRejected,
    HostBusy,
    HostUnavailable,
    InvalidHost,
    InvalidJson,
    InvalidOrigin,
    InvalidPreferences,
    InvalidServerAppearance,
    InvalidStorageAnalysisRequest,
    InvalidTerminalRequest,
    InvalidWeatherLocation,
    LoginRejected,
    NotFound,
    PayloadTooLarge,
    PasswordWorkersBusy,
    PreferenceWriteRateLimited,
    ServerAppearanceConflict,
    StorageAnalysisRateLimited,
    StrandBusy,
    StrandConflict,
    StrandRejected(String),
    TerminalCapacityExhausted,
    TerminalTicketRejected,
    TerminalUnavailable,
    WeatherBusy,
    WeatherLocationNotFound,
    WeatherUnavailable,
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
        let this = match self {
            Self::BrokerRejected(message) => {
                return (
                    StatusCode::BAD_REQUEST,
                    [(header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({
                        "code": "broker_operation_rejected",
                        "message": message,
                    })),
                )
                    .into_response();
            }
            Self::StrandRejected(message) => {
                return (
                    StatusCode::BAD_REQUEST,
                    [(header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({
                        "code": "strand_rejected",
                        "message": message,
                    })),
                )
                    .into_response();
            }
            other => other,
        };
        let clear_cookie = matches!(this, Self::AuthenticationRequired);
        let (status, problem, retry_after) = match this {
            Self::AccountConflict => (
                StatusCode::CONFLICT,
                ApiProblem {
                    code: "account_update_conflict",
                    message: "The requested login name is not available.",
                },
                None,
            ),
            Self::AccountRejected => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "account_update_rejected",
                    message: "The account changes did not meet Helix's security requirements.",
                },
                None,
            ),
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
            Self::BrokerUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiProblem {
                    code: "host_broker_unavailable",
                    message: "Host management is temporarily unavailable.",
                },
                Some(HeaderValue::from_static("2")),
            ),
            Self::BrokerRejected(_) => unreachable!("handled before the static problem match"),
            Self::StrandRejected(_) => unreachable!("handled before the static problem match"),
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
            Self::CurrentPasswordRejected => (
                StatusCode::UNAUTHORIZED,
                ApiProblem {
                    code: "current_password_rejected",
                    message: "The current password could not be verified.",
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
            Self::InvalidPreferences => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "invalid_dashboard_preferences",
                    message: "The dashboard preferences are invalid or exceed Helix's limits.",
                },
                None,
            ),
            Self::InvalidServerAppearance => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "invalid_server_appearance",
                    message: "Choose a supported preset or upload a valid PNG or JPEG icon.",
                },
                None,
            ),
            Self::InvalidStorageAnalysisRequest => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "invalid_storage_analysis_request",
                    message: "The storage analysis path or job id is invalid.",
                },
                None,
            ),
            Self::InvalidTerminalRequest => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "invalid_terminal_request",
                    message: "The requested terminal dimensions are outside the supported range.",
                },
                None,
            ),
            Self::InvalidWeatherLocation => (
                StatusCode::BAD_REQUEST,
                ApiProblem {
                    code: "invalid_weather_location",
                    message: "Enter a city, postal code, or city and region between 2 and 120 characters.",
                },
                None,
            ),
            Self::PreferenceWriteRateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                ApiProblem {
                    code: "preference_write_rate_limited",
                    message: "Dashboard preferences are being saved too quickly.",
                },
                Some(HeaderValue::from_static("60")),
            ),
            Self::ServerAppearanceConflict => (
                StatusCode::CONFLICT,
                ApiProblem {
                    code: "server_appearance_conflict",
                    message: "The server icon changed in another session. Refresh and try again.",
                },
                None,
            ),
            Self::StorageAnalysisRateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                ApiProblem {
                    code: "storage_analysis_rate_limited",
                    message: "Storage analysis requests are being sent too quickly.",
                },
                Some(HeaderValue::from_static("60")),
            ),
            Self::StrandBusy => (
                StatusCode::TOO_MANY_REQUESTS,
                ApiProblem {
                    code: "strand_busy",
                    message: "This Strand is over its call limit.",
                },
                Some(HeaderValue::from_static("60")),
            ),
            Self::StrandConflict => (
                StatusCode::CONFLICT,
                ApiProblem {
                    code: "strand_conflict",
                    message: "A different Strand is already installed with that slug.",
                },
                None,
            ),
            Self::TerminalCapacityExhausted => (
                StatusCode::TOO_MANY_REQUESTS,
                ApiProblem {
                    code: "terminal_capacity_exhausted",
                    message: "Too many terminal authorizations are already in progress.",
                },
                Some(HeaderValue::from_static("1")),
            ),
            Self::TerminalTicketRejected => (
                StatusCode::UNAUTHORIZED,
                ApiProblem {
                    code: "terminal_ticket_rejected",
                    message: "The one-time terminal authorization was rejected or expired.",
                },
                None,
            ),
            Self::TerminalUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiProblem {
                    code: "terminal_unavailable",
                    message: "The optional unprivileged host terminal is not available.",
                },
                Some(HeaderValue::from_static("2")),
            ),
            Self::WeatherBusy => (
                StatusCode::TOO_MANY_REQUESTS,
                ApiProblem {
                    code: "weather_capacity_exhausted",
                    message: "Weather lookup capacity is busy. Try again shortly.",
                },
                Some(HeaderValue::from_static("2")),
            ),
            Self::WeatherLocationNotFound => (
                StatusCode::NOT_FOUND,
                ApiProblem {
                    code: "weather_location_not_found",
                    message: "No matching weather location was found.",
                },
                None,
            ),
            Self::WeatherUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiProblem {
                    code: "weather_unavailable",
                    message: "The weather provider is temporarily unavailable.",
                },
                Some(HeaderValue::from_static("60")),
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
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                ApiProblem {
                    code: "payload_too_large",
                    message: "The request body exceeds the configured limit.",
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
fn private_test_directory(description: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap_or_else(|error| panic!("{description}: {error}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|error| panic!("secure {description}: {error}"));
    }
    directory
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

    const fn test_fixture(bytes: &'static [u8]) -> &'static str {
        match std::str::from_utf8(bytes) {
            Ok(value) => value,
            Err(_) => "invalid-test-fixture",
        }
    }

    const PASSWORD: &str = test_fixture(&[
        86, 55, 33, 113, 117, 97, 114, 116, 122, 45, 77, 101, 116, 101, 111, 114, 35, 50, 57,
    ]);

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
        let data = private_test_directory("temporary data directory");
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

    fn post_raw(uri: &str, body: &'static str, peer_octet: u8) -> HttpRequest<Body> {
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
            .body(Body::from(body))
            .expect("request")
    }

    fn put_json(uri: &str, value: &Value, peer_octet: u8) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("PUT")
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

    fn put_raw(uri: &str, body: &'static str, peer_octet: u8) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("PUT")
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .extension(ConnectInfo(SocketAddr::from((
                [127, 0, 0, peer_octet],
                41000,
            ))))
            .body(Body::from(body))
            .expect("request")
    }

    fn delete_json(uri: &str, peer_octet: u8) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("DELETE")
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .extension(ConnectInfo(SocketAddr::from((
                [127, 0, 0, peer_octet],
                41000,
            ))))
            .body(Body::empty())
            .expect("request")
    }

    fn delete_body_json(uri: &str, value: &Value, peer_octet: u8) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("DELETE")
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
        claim_owner_with_password(context, bootstrap, PASSWORD).await
    }

    async fn claim_owner_with_password(
        context: &TestApp,
        bootstrap: &str,
        password: &str,
    ) -> AuthClient {
        let fixation = "helix_session=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let request = with_cookie(
            post_json(
                "/api/v1/setup/owner",
                &json!({
                    "bootstrapToken": bootstrap,
                    "loginName": "owner",
                    "displayName": "Rique\u{301}",
                    "password": password
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

    fn dashboard_preferences_with_serialized_size(target: usize) -> DashboardPreferences {
        let home_widgets: Vec<_> = (0..5)
            .map(|index| HomeWidgetPreference {
                id: format!("note-{index}"),
                kind: HomeWidgetKind::Note,
                size: HomeWidgetSize::Full,
                height: HomeWidgetHeight::Tall,
                title: format!("Note {index}"),
                content: "x".repeat(7_200),
                url: String::new(),
                color: String::new(),
                icon: String::new(),
            })
            .collect();
        let mut preferences = DashboardPreferences {
            navigation_order: vec![
                PrimaryDashboardSection::Overview,
                PrimaryDashboardSection::Home,
                PrimaryDashboardSection::Storage,
                PrimaryDashboardSection::Network,
                PrimaryDashboardSection::Host,
                PrimaryDashboardSection::Security,
                PrimaryDashboardSection::Terminal,
                PrimaryDashboardSection::Servers,
                PrimaryDashboardSection::Hooks,
                PrimaryDashboardSection::Strands,
                PrimaryDashboardSection::Globe,
            ],
            metrics_refresh_ms: 1_000,
            home_widgets: home_widgets.clone(),
            home_templates: vec![HomeTemplatePreference {
                id: "home-main".to_owned(),
                name: "Mainx".to_owned(),
                accent: "#d7f64d".to_owned(),
                widgets: home_widgets,
            }],
            active_home_id: "home-main".to_owned(),
            colors: DashboardColorPreferences::default(),
            servers_enabled: true,
            hidden_pages: default_hidden_pages(),
        };
        let initial = serde_json::to_string(&preferences).expect("serialize preferences");
        let excess = initial
            .len()
            .checked_sub(target)
            .expect("fixture must start above the requested size");
        let paired_reduction = excess / 2;
        let last = preferences.home_widgets.last_mut().expect("last widget");
        let keep = last
            .content
            .len()
            .checked_sub(paired_reduction)
            .expect("target fits within one widget adjustment");
        last.content.truncate(keep);
        preferences.home_templates[0]
            .widgets
            .last_mut()
            .expect("template widget")
            .content
            .truncate(keep);
        if excess % 2 == 1 {
            preferences.home_templates[0].name.pop();
        }
        assert_eq!(
            serde_json::to_string(&preferences)
                .expect("serialize adjusted preferences")
                .len(),
            target
        );
        preferences
    }

    #[test]
    fn legacy_dashboard_navigation_adds_hooks_without_reordering_existing_pages() {
        let mut preferences = DashboardPreferences::default();
        preferences
            .navigation_order
            .retain(|section| *section != PrimaryDashboardSection::Hooks);
        let original = preferences.navigation_order.clone();

        reconcile_legacy_home_preferences(&mut preferences);

        assert_eq!(
            &preferences.navigation_order[..original.len()],
            original.as_slice()
        );
        assert_eq!(
            preferences.navigation_order.last(),
            Some(&PrimaryDashboardSection::Hooks)
        );
        assert!(validate_dashboard_preferences(&preferences).is_ok());
    }

    #[test]
    fn legacy_dashboard_navigation_adds_globe_without_reordering_existing_pages() {
        let mut preferences = DashboardPreferences::default();
        preferences
            .navigation_order
            .retain(|section| *section != PrimaryDashboardSection::Globe);
        let original = preferences.navigation_order.clone();

        reconcile_legacy_home_preferences(&mut preferences);

        assert_eq!(
            &preferences.navigation_order[..original.len()],
            original.as_slice()
        );
        assert_eq!(
            preferences.navigation_order.last(),
            Some(&PrimaryDashboardSection::Globe)
        );
        assert!(
            preferences
                .hidden_pages
                .contains(&PrimaryDashboardSection::Globe)
        );
        assert!(validate_dashboard_preferences(&preferences).is_ok());
    }

    #[test]
    fn missing_hidden_pages_defaults_to_globe_only() {
        let mut value = serde_json::to_value(DashboardPreferences::default()).expect("value");
        value.as_object_mut().expect("object").remove("hiddenPages");
        let parsed: DashboardPreferences =
            serde_json::from_value(value).expect("hidden_pages default");
        assert_eq!(parsed.hidden_pages, vec![PrimaryDashboardSection::Globe]);
    }

    #[test]
    fn legacy_dashboard_navigation_inserts_security_before_terminal() {
        let mut preferences = DashboardPreferences::default();
        preferences
            .navigation_order
            .retain(|section| *section != PrimaryDashboardSection::Security);
        reconcile_legacy_home_preferences(&mut preferences);
        let security = preferences
            .navigation_order
            .iter()
            .position(|section| *section == PrimaryDashboardSection::Security);
        let terminal = preferences
            .navigation_order
            .iter()
            .position(|section| *section == PrimaryDashboardSection::Terminal);
        assert!(security.is_some());
        assert_eq!(security.unwrap() + 1, terminal.unwrap());
        assert!(validate_dashboard_preferences(&preferences).is_ok());
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
            "/api/v1/games/readiness",
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
        assert!(
            me["user"]["capabilities"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item == "system.view"))
        );
        assert!(me["expiresAtUnixMs"].as_i64().is_some());

        for route in [
            "/api/v1/health",
            "/api/v1/system/overview",
            "/api/v1/games/readiness",
        ] {
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
        assert!(
            login["user"]["capabilities"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item == "system.view"))
        );
        assert!(login["csrfToken"].as_str().is_some());
    }

    #[tokio::test]
    async fn terminal_status_is_explicit_when_the_optional_host_service_is_absent() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        let response = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/terminal/status"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("terminal status response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["availability"], "unavailable");
        assert_eq!(body["reauthenticationRequired"], true);
        assert_eq!(body["shellPrivilege"], "linux_user");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_ticket_requires_current_password_and_stays_in_a_scoped_cookie() {
        use std::os::unix::net::UnixListener;

        let terminal_directory = private_test_directory("temporary terminal directory");
        let terminal_socket = terminal_directory.path().join("terminal.sock");
        let _listener = UnixListener::bind(&terminal_socket).expect("bind terminal socket");
        let context = test_app_with_state(DatabaseStatus::Ok, move |state| {
            state
                .with_terminal_socket(terminal_socket)
                .expect("configure terminal socket")
        })
        .await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        let wrong_password = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/terminal/ticket",
                        &json!({
                            "currentPassword": "definitely-wrong",
                            "columns": 120,
                            "rows": 32
                        }),
                        31,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("wrong terminal password response");
        assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);
        assert!(!wrong_password.headers().contains_key(header::SET_COOKIE));
        assert_eq!(
            response_json(wrong_password).await["code"],
            "current_password_rejected"
        );

        let response = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/terminal/ticket",
                        &json!({
                            "currentPassword": PASSWORD,
                            "columns": 120,
                            "rows": 32
                        }),
                        31,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("terminal ticket response");
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        let ticket_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("terminal ticket cookie")
            .to_str()
            .expect("terminal ticket cookie text")
            .to_owned();
        assert!(ticket_cookie.starts_with("helix_terminal_ticket="));
        assert!(ticket_cookie.contains("HttpOnly"));
        assert!(ticket_cookie.contains("SameSite=Strict"));
        assert!(ticket_cookie.contains("Path=/api/v1/terminal/connect"));
        assert!(ticket_cookie.contains("Max-Age=30"));
        let body = response_json(response).await;
        assert_eq!(body["connectPath"], "/api/v1/terminal/connect");
        assert_eq!(body["subprotocol"], "helix-terminal-v1");
        assert!(body.get("ticket").is_none());
        assert!(!body.to_string().contains("helix_terminal_ticket"));
    }

    #[tokio::test]
    async fn owner_can_replace_login_and_password_with_current_password_proof() {
        const REPLACEMENT_PASSWORD: &str = test_fixture(&[
            71, 56, 33, 99, 111, 98, 97, 108, 116, 45, 72, 111, 114, 105, 122, 111, 110, 35, 53, 52,
        ]);

        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        let wrong_current = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/auth/account",
                        &json!({
                            "currentPassword": "wrong-password",
                            "loginName": "rique",
                            "displayName": "Rique",
                            "newPassword": REPLACEMENT_PASSWORD
                        }),
                        31,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("wrong-current-password response");
        assert_eq!(wrong_current.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response_json(wrong_current).await["code"],
            "current_password_rejected"
        );

        let still_current = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/auth/me"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("session after rejected update");
        assert_eq!(still_current.status(), StatusCode::OK);

        let updated = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/auth/account",
                        &json!({
                            "currentPassword": PASSWORD,
                            "loginName": "rique",
                            "displayName": "Rique",
                            "newPassword": REPLACEMENT_PASSWORD
                        }),
                        32,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("account update response");
        assert_eq!(updated.status(), StatusCode::NO_CONTENT);
        assert!(
            updated
                .headers()
                .get(header::SET_COOKIE)
                .is_some_and(|value| {
                    value.as_bytes().windows(9).any(|part| part == b"Max-Age=0")
                })
        );

        let revoked = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/auth/me"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("revoked old session response");
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

        let old_login = context
            .app
            .clone()
            .oneshot(post_json(
                "/api/v1/auth/login",
                &json!({"loginName": "owner", "password": PASSWORD}),
                33,
            ))
            .await
            .expect("old login response");
        assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED);

        let new_login = context
            .app
            .oneshot(post_json(
                "/api/v1/auth/login",
                &json!({"loginName": "rique", "password": REPLACEMENT_PASSWORD}),
                34,
            ))
            .await
            .expect("new login response");
        assert_eq!(new_login.status(), StatusCode::OK);
        let body = response_json(new_login).await;
        assert_eq!(body["user"]["loginName"], "rique");
        assert_eq!(body["user"]["displayName"], "Rique");
    }

    #[tokio::test]
    async fn account_password_proofs_are_worker_bounded_throttled_and_audited() {
        let workers = Arc::new(Semaphore::new(1));
        let workers_for_state = Arc::clone(&workers);
        let context = test_app_with_state(DatabaseStatus::Ok, move |state| {
            state.with_password_workers(workers_for_state)
        })
        .await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let held_worker = Arc::clone(&workers)
            .acquire_owned()
            .await
            .expect("hold password worker");
        let request = || {
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/auth/account",
                        &json!({
                            "currentPassword": "wrong-password",
                            "loginName": "owner",
                            "displayName": "Riqué"
                        }),
                        41,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            )
        };

        let busy = context
            .app
            .clone()
            .oneshot(request())
            .await
            .expect("busy worker response");
        assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response_json(busy).await["code"],
            "password_capacity_exhausted"
        );
        drop(held_worker);

        for _ in 0..5 {
            let rejected = context
                .app
                .clone()
                .oneshot(request())
                .await
                .expect("rejected proof response");
            assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                response_json(rejected).await["code"],
                "current_password_rejected"
            );
        }
        let peer_cycled = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/auth/account",
                        &json!({
                            "currentPassword": "wrong-password",
                            "loginName": "owner",
                            "displayName": "Riqué"
                        }),
                        42,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("rate-limited proof response");
        assert_eq!(peer_cycled.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response_json(peer_cycled).await["code"],
            "too_many_attempts"
        );

        let now = i64::try_from(unix_timestamp_ms()).expect("current time fits i64");
        let credential = context
            .databases
            .state()
            .credential_by_login("owner", now)
            .expect("credential lookup")
            .expect("owner credential");
        assert_eq!(credential.failed_login_count, 0);
        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        let audit_rows = connection
            .query_row(
                "SELECT count(*) FROM audit_events
                 WHERE action = 'account.owner_update_password_rejected'
                       AND outcome = 'denied'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("account denial audit count");
        assert_eq!(audit_rows, 5);
    }

    #[tokio::test]
    async fn identity_only_changes_revalidate_the_verified_password_context() {
        const CURRENT_PASSWORD: &str =
            test_fixture(&[99, 111, 98, 97, 108, 116, 45, 115, 107, 121, 45, 57, 50]);

        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner_with_password(&context, &bootstrap, CURRENT_PASSWORD).await;
        let conflicting = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/auth/account",
                        &json!({
                            "currentPassword": CURRENT_PASSWORD,
                            "loginName": CURRENT_PASSWORD,
                            "displayName": "Riqué"
                        }),
                        43,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("conflicting identity response");
        assert_eq!(conflicting.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(conflicting).await["code"],
            "account_update_rejected"
        );

        let still_authenticated = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/auth/me"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("session after rejected identity");
        assert_eq!(still_authenticated.status(), StatusCode::OK);

        let safe_change = context
            .app
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/auth/account",
                        &json!({
                            "currentPassword": CURRENT_PASSWORD,
                            "loginName": "new-owner",
                            "displayName": "Riqué"
                        }),
                        44,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("safe identity response");
        assert_eq!(safe_change.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn weather_requires_authentication_and_rejects_invalid_queries_without_fetching() {
        let context = test_app(DatabaseStatus::Ok).await;
        let response = context
            .app
            .clone()
            .oneshot(get("/api/v1/weather?location=Denver&unit=fahrenheit"))
            .await
            .expect("unauthenticated weather response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let response = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    get("/api/v1/weather?location=Denver&unit=kelvin"),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("invalid weather query response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(response).await["code"],
            "invalid_weather_location"
        );

        let response = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    get("/api/v1/weather?location=Denver%0AColorado&unit=fahrenheit"),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("control-character location response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(response).await["code"],
            "invalid_weather_location"
        );
    }

    #[tokio::test]
    async fn weather_has_a_nonqueueing_worker_limit() {
        let context = test_app_with_state(DatabaseStatus::Ok, |mut state| {
            state.weather_workers = Arc::new(Semaphore::new(0));
            state
        })
        .await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let response = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    get("/api/v1/weather?location=Denver&unit=fahrenheit"),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("busy weather response");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response_json(response).await["code"],
            "weather_capacity_exhausted"
        );
    }

    #[tokio::test]
    async fn dashboard_preferences_are_persistent_validated_and_revision_guarded() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        let initial = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/settings/preferences"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("initial preferences response");
        assert_eq!(initial.status(), StatusCode::OK);
        let initial = response_json(initial).await;
        assert_eq!(initial["revision"], 0);
        assert_eq!(initial["preferences"]["metricsRefreshMs"], 5_000);

        let preferences = json!({
            "navigationOrder": ["home", "overview", "servers", "storage", "network", "host"],
            "metricsRefreshMs": 2_000,
            "homeWidgets": [
                {
                    "id": "clock",
                    "kind": "clock",
                    "size": "compact",
                    "title": "Right now",
                    "content": "",
                    "url": ""
                },
                {
                    "id": "plex",
                    "kind": "shortcut",
                    "size": "wide",
                    "title": "Plex",
                    "content": "Media",
                    "url": "https://example.com/plex"
                }
            ]
        });
        let saved = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/settings/preferences",
                        &json!({"expectedRevision": 0, "preferences": preferences}),
                        35,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("saved preferences response");
        assert_eq!(saved.status(), StatusCode::OK);
        let saved = response_json(saved).await;
        assert_eq!(saved["revision"], 1);
        assert_eq!(saved["preferences"]["navigationOrder"][0], "home");
        assert!(saved["updatedAtUnixMs"].as_i64().is_some());

        let stale = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/settings/preferences",
                        &json!({"expectedRevision": 0, "preferences": preferences}),
                        36,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("stale preferences response");
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        let stale = response_json(stale).await;
        assert_eq!(stale["code"], "preferences_revision_conflict");
        assert_eq!(stale["current"]["revision"], 1);

        let mut invalid = preferences;
        invalid["homeWidgets"][1]["url"] = json!("javascript:alert(1)");
        let rejected = context
            .app
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/settings/preferences",
                        &json!({"expectedRevision": 1, "preferences": invalid}),
                        37,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("invalid preferences response");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(rejected).await["code"],
            "invalid_dashboard_preferences"
        );
    }

    #[tokio::test]
    async fn dashboard_preference_revision_and_serialized_size_boundaries_are_api_errors() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let defaults = serde_json::to_value(DashboardPreferences::default())
            .expect("serialize default preferences");

        for revision in [json!(-1), json!(i64::MAX)] {
            let rejected = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(
                        put_json(
                            "/api/v1/settings/preferences",
                            &json!({
                                "expectedRevision": revision,
                                "preferences": defaults
                            }),
                            51,
                        ),
                        &client.cookie,
                    ),
                    &client.csrf,
                ))
                .await
                .expect("invalid revision response");
            assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                response_json(rejected).await["code"],
                "invalid_dashboard_preferences"
            );
        }

        let maximum = dashboard_preferences_with_serialized_size(65_536);
        let accepted = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/settings/preferences",
                        &json!({
                            "expectedRevision": 0,
                            "preferences": serde_json::to_value(maximum)
                                .expect("serialize maximum preferences")
                        }),
                        52,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("maximum-size response");
        assert_eq!(accepted.status(), StatusCode::OK);

        let now = i64::try_from(unix_timestamp_ms()).expect("current time fits i64");
        let owner = context
            .databases
            .state()
            .credential_by_login("owner", now)
            .expect("owner lookup")
            .expect("owner credential");
        let stored = context
            .databases
            .state()
            .user_preferences(&owner.user_id)
            .expect("stored preferences")
            .expect("preferences record");
        assert_eq!(stored.revision, 1);
        assert_eq!(stored.preferences_json.len(), 65_536);

        let oversized = dashboard_preferences_with_serialized_size(65_537);
        let rejected = context
            .app
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/settings/preferences",
                        &json!({
                            "expectedRevision": 1,
                            "preferences": serde_json::to_value(oversized)
                                .expect("serialize oversized preferences")
                        }),
                        53,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("oversized response");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(rejected).await["code"],
            "invalid_dashboard_preferences"
        );
    }

    #[tokio::test]
    async fn malformed_preference_json_is_mapped_only_after_header_and_auth_checks() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        let mut invalid_origin = with_csrf(
            with_cookie(
                put_raw("/api/v1/settings/preferences", "{", 54),
                &client.cookie,
            ),
            &client.csrf,
        );
        invalid_origin.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("http://not-localhost"),
        );
        let invalid_origin = context
            .app
            .clone()
            .oneshot(invalid_origin)
            .await
            .expect("invalid-origin response");
        assert_eq!(invalid_origin.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(invalid_origin).await["code"],
            "invalid_origin"
        );

        let unauthenticated = context
            .app
            .clone()
            .oneshot(put_raw("/api/v1/settings/preferences", "{", 55))
            .await
            .expect("unauthenticated malformed response");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response_json(unauthenticated).await["code"],
            "authentication_required"
        );

        let malformed = context
            .app
            .oneshot(with_csrf(
                with_cookie(
                    put_raw("/api/v1/settings/preferences", "{", 56),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("authenticated malformed response");
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response_json(malformed).await["code"], "invalid_json");
    }

    #[tokio::test]
    async fn game_hosting_readiness_is_truthful_and_contains_no_fake_instances() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let response = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/games/readiness"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("game readiness response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        let body = response_json(response).await;
        assert_eq!(body["schema_version"], 1);
        assert_eq!(body["availability"], "unavailable");
        assert_eq!(body["available_features"], json!([]));
        assert_eq!(
            body["blockers"],
            json!([
                { "code": "verified_restore", "status": "required" },
                { "code": "privileged_broker", "status": "required" },
                { "code": "native_execution", "status": "required" }
            ])
        );
        assert!(body["collected_at_unix_ms"].as_i64().is_some());
        assert!(body.get("instances").is_none());
    }

    #[tokio::test]
    async fn typed_host_control_and_history_routes_enforce_authentication_and_csrf() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        for uri in [
            "/api/v1/host/integration",
            "/api/v1/host/reboot/preflight",
            "/api/v1/hooks",
            "/api/v1/hooks/tailscale/install/preflight",
            "/api/v1/hooks/jobs/8953dc16-3891-42bf-802f-711b3ba2965a",
            "/api/v1/servers/helix:test/logs/history?lines=500",
        ] {
            let response = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(get(uri), &client.cookie),
                    &client.csrf,
                ))
                .await
                .expect("typed read route response");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{uri}");
            assert_eq!(
                response_json(response).await["code"],
                "host_broker_unavailable"
            );
        }

        let missing_csrf = context
            .app
            .clone()
            .oneshot(with_cookie(
                put_json(
                    "/api/v1/host/integration/start-on-boot",
                    &json!({"enabled": true}),
                    41,
                ),
                &client.cookie,
            ))
            .await
            .expect("missing CSRF response");
        assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);
        assert_eq!(response_json(missing_csrf).await["code"], "csrf_rejected");

        let unauthenticated_malformed = context
            .app
            .clone()
            .oneshot(put_raw("/api/v1/host/integration/start-on-boot", "{", 48))
            .await
            .expect("unauthenticated malformed response");
        assert_eq!(unauthenticated_malformed.status(), StatusCode::UNAUTHORIZED);
        let authenticated_malformed = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_raw("/api/v1/host/integration/start-on-boot", "{", 49),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("authenticated malformed response");
        assert_eq!(authenticated_malformed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(authenticated_malformed).await["code"],
            "invalid_json"
        );

        let start_on_boot = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/host/integration/start-on-boot",
                        &json!({"enabled": true}),
                        42,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("start-on-boot route response");
        assert_eq!(start_on_boot.status(), StatusCode::SERVICE_UNAVAILABLE);

        let hook_without_csrf = context
            .app
            .clone()
            .oneshot(with_cookie(
                post_json(
                    "/api/v1/hooks/plex/actions",
                    &json!({"action": "restart"}),
                    45,
                ),
                &client.cookie,
            ))
            .await
            .expect("missing hook CSRF response");
        assert_eq!(hook_without_csrf.status(), StatusCode::FORBIDDEN);

        let malformed_hook = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_raw("/api/v1/hooks/plex/actions", "{", 46),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("malformed hook response");
        assert_eq!(malformed_hook.status(), StatusCode::BAD_REQUEST);

        let hook_action = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/hooks/plex/actions",
                        &json!({"action": "restart"}),
                        47,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("hook action response");
        assert_eq!(hook_action.status(), StatusCode::SERVICE_UNAVAILABLE);

        let hook_install_without_csrf = context
            .app
            .clone()
            .oneshot(with_cookie(
                post_json(
                    "/api/v1/hooks/tailscale/install",
                    &json!({
                        "confirmation": "tailscale",
                        "repository_change_acknowledged": true
                    }),
                    50,
                ),
                &client.cookie,
            ))
            .await
            .expect("missing hook install CSRF response");
        assert_eq!(hook_install_without_csrf.status(), StatusCode::FORBIDDEN);

        let hook_install = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/hooks/tailscale/install",
                        &json!({
                            "confirmation": "tailscale",
                            "repository_change_acknowledged": true
                        }),
                        51,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("hook install route response");
        assert_eq!(hook_install.status(), StatusCode::SERVICE_UNAVAILABLE);

        let reboot = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/host/reboot",
                        &json!({
                            "confirmation_hostname": "test-host",
                            "delay_seconds": 30,
                            "disruption_acknowledged": true
                        }),
                        43,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("schedule reboot route response");
        assert_eq!(reboot.status(), StatusCode::SERVICE_UNAVAILABLE);

        let recurring_reboot = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/host/reboot/recurring",
                        &json!({
                            "weekdays": ["monday", "wednesday", "friday"],
                            "hour": 5,
                            "minute": 30,
                            "timezone": "America/Denver",
                            "confirmation_hostname": "test-host",
                            "disruption_acknowledged": true
                        }),
                        50,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("recurring reboot route response");
        assert_eq!(recurring_reboot.status(), StatusCode::SERVICE_UNAVAILABLE);

        let remove_recurring_reboot = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    delete_body_json(
                        "/api/v1/host/reboot/recurring",
                        &json!({"confirmation_hostname": "test-host"}),
                        51,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("remove recurring reboot route response");
        assert_eq!(
            remove_recurring_reboot.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        let cancel = context
            .app
            .oneshot(with_csrf(
                with_cookie(
                    delete_json(
                        "/api/v1/host/reboot/12345678-1234-4234-8234-123456789abc",
                        44,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("cancel reboot route response");
        assert_eq!(cancel.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn network_firewall_and_package_routes_enforce_typed_auth_and_csrf_ordering() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        for uri in [
            "/api/v1/network/inventory",
            "/api/v1/network/globe",
            "/api/v1/system/packages",
        ] {
            let response = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(get(uri), &client.cookie),
                    &client.csrf,
                ))
                .await
                .expect("infrastructure read response");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        }

        let missing_csrf = context
            .app
            .clone()
            .oneshot(with_cookie(
                post_json(
                    "/api/v1/network/firewall/rules",
                    &json!({
                        "name": "Minecraft",
                        "description": "Survival server",
                        "protocol": "tcp",
                        "port_start": 25565,
                        "port_end": 25565
                    }),
                    61,
                ),
                &client.cookie,
            ))
            .await
            .expect("missing firewall CSRF response");
        assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);
        assert_eq!(response_json(missing_csrf).await["code"], "csrf_rejected");

        let unauthenticated_malformed = context
            .app
            .clone()
            .oneshot(post_raw("/api/v1/network/firewall/rules", "{", 62))
            .await
            .expect("unauthenticated malformed firewall response");
        assert_eq!(unauthenticated_malformed.status(), StatusCode::UNAUTHORIZED);

        let authenticated_malformed = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_raw("/api/v1/network/firewall/rules", "{", 63),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("authenticated malformed firewall response");
        assert_eq!(authenticated_malformed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(authenticated_malformed).await["code"],
            "invalid_json"
        );

        let create = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/network/firewall/rules",
                        &json!({
                            "name": "Minecraft",
                            "description": "Survival server",
                            "protocol": "tcp",
                            "port_start": 25565,
                            "port_end": 25565
                        }),
                        64,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("firewall create response");
        assert_eq!(create.status(), StatusCode::SERVICE_UNAVAILABLE);

        let leftover_without_csrf = context
            .app
            .clone()
            .oneshot(with_cookie(
                post_json(
                    "/api/v1/network/amp-router-forwards/release",
                    &json!({
                        "port": 25566,
                        "confirmation": "REMOVE AMP FORWARD 25566"
                    }),
                    67,
                ),
                &client.cookie,
            ))
            .await
            .expect("missing leftover AMP CSRF response");
        assert_eq!(leftover_without_csrf.status(), StatusCode::FORBIDDEN);

        let leftover_release = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/network/amp-router-forwards/release",
                        &json!({
                            "port": 25566,
                            "confirmation": "REMOVE AMP FORWARD 25566"
                        }),
                        67,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("leftover AMP forward release response");
        assert_eq!(leftover_release.status(), StatusCode::SERVICE_UNAVAILABLE);

        for request in [
            with_csrf(
                with_cookie(
                    delete_json(
                        "/api/v1/network/firewall/rules/8953dc16-3891-42bf-802f-711b3ba2965a",
                        65,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/network/firewall/rules/8953dc16-3891-42bf-802f-711b3ba2965a/restore",
                        &json!({}),
                        66,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
        ] {
            let response = context
                .app
                .clone()
                .oneshot(request)
                .await
                .expect("firewall lifecycle response");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }

        let package_without_csrf = context
            .app
            .clone()
            .oneshot(with_cookie(
                post_json("/api/v1/system/packages/refresh", &json!({}), 68),
                &client.cookie,
            ))
            .await
            .expect("missing package CSRF response");
        assert_eq!(package_without_csrf.status(), StatusCode::FORBIDDEN);

        let malformed_apply = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_raw("/api/v1/system/packages/apply", "{", 69),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("malformed package apply response");
        assert_eq!(malformed_apply.status(), StatusCode::BAD_REQUEST);

        for request in [
            with_csrf(
                with_cookie(
                    post_json("/api/v1/system/packages/refresh", &json!({}), 70),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/system/packages/apply",
                        &json!({
                            "packages": [{
                                "name": "openssl",
                                "installed_version": "3.0.1",
                                "candidate_version": "3.0.2"
                            }],
                            "confirmation": "APPLY 1 UPDATE",
                            "disruption_acknowledged": true
                        }),
                        71,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    get("/api/v1/system/packages/jobs/12345678-1234-4234-8234-123456789abc"),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json("/api/v1/system/helix/check", &json!({}), 72),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/system/helix/apply",
                        &json!({
                            "target_tag": "v1.0.1",
                            "confirmation": "UPDATE HELIX",
                            "disruption_acknowledged": true
                        }),
                        73,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
        ] {
            let response = context
                .app
                .clone()
                .oneshot(request)
                .await
                .expect("package mutation route response");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
    }

    #[tokio::test]
    async fn network_and_package_reads_require_their_exact_capabilities() {
        let context = test_app(DatabaseStatus::Ok).await;
        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        connection
            .execute(
                "DELETE FROM role_capabilities
                 WHERE capability IN (
                    'network.firewall.read',
                    'network.firewall.write',
                    'system.packages.read',
                    'system.packages.write'
                 )",
                [],
            )
            .expect("remove infrastructure capabilities");
        drop(connection);
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        for request in [
            with_csrf(
                with_cookie(get("/api/v1/network/inventory"), &client.cookie),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(get("/api/v1/system/packages"), &client.cookie),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/network/firewall/rules",
                        &json!({
                            "name": "Minecraft",
                            "description": "Survival server",
                            "protocol": "tcp",
                            "port_start": 25565,
                            "port_end": 25565
                        }),
                        67,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json("/api/v1/system/packages/refresh", &json!({}), 68),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/system/packages/apply",
                        &json!({
                            "packages": [],
                            "confirmation": "",
                            "disruption_acknowledged": false
                        }),
                        69,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json("/api/v1/system/helix/check", &json!({}), 70),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/system/helix/apply",
                        &json!({
                            "target_tag": "v1.0.1",
                            "confirmation": "UPDATE HELIX",
                            "disruption_acknowledged": true
                        }),
                        71,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
        ] {
            let response = context
                .app
                .clone()
                .oneshot(request)
                .await
                .expect("infrastructure capability response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                response_json(response).await["code"],
                "authorization_denied"
            );
        }
    }

    #[tokio::test]
    async fn typed_host_control_routes_deny_sessions_without_required_capabilities() {
        let context = test_app(DatabaseStatus::Ok).await;
        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        connection
            .execute("DELETE FROM role_capabilities", [])
            .expect("remove capability grants");
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        for request in [
            with_csrf(
                with_cookie(get("/api/v1/host/integration"), &client.cookie),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    get("/api/v1/hooks/tailscale/install/preflight"),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/hooks/tailscale/install",
                        &json!({
                            "confirmation": "tailscale",
                            "repository_change_acknowledged": true
                        }),
                        44,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    get("/api/v1/servers/helix:test/logs/history?lines=20"),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/host/integration/start-on-boot",
                        &json!({"enabled": false}),
                        45,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/host/reboot",
                        &json!({
                            "confirmation_hostname": "test-host",
                            "delay_seconds": 30,
                            "disruption_acknowledged": true
                        }),
                        46,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    delete_json(
                        "/api/v1/host/reboot/12345678-1234-4234-8234-123456789abc",
                        47,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
            with_csrf(
                with_cookie(
                    put_json(
                        "/api/v1/host/reboot/recurring",
                        &json!({
                            "weekdays": ["sunday"],
                            "hour": 4,
                            "minute": 0,
                            "timezone": "America/Denver",
                            "confirmation_hostname": "test-host",
                            "disruption_acknowledged": true
                        }),
                        48,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ),
        ] {
            let response = context
                .app
                .clone()
                .oneshot(request)
                .await
                .expect("capability denial response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                response_json(response).await["code"],
                "authorization_denied"
            );
        }
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

        let oversized = HttpRequest::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
            .body(Body::from(format!(
                r#"{{"loginName":"nobody","password":"{}"}}"#,
                "x".repeat(API_BODY_LIMIT_BYTES)
            )))
            .expect("request");
        let oversized = context
            .app
            .clone()
            .oneshot(oversized)
            .await
            .expect("response");
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(response_json(oversized).await["code"], "payload_too_large");

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

        let first = context
            .app
            .clone()
            .oneshot(post_json(
                "/api/v1/auth/login",
                &json!({"loginName": "owner", "password": "wrong-password"}),
                1,
            ))
            .await
            .expect("wrong-password response");
        assert_eq!(first.status(), StatusCode::UNAUTHORIZED);
        assert!(!first.headers().contains_key(header::SET_COOKIE));
        let expected = response_json(first).await;

        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        let delayed_until = i64::try_from(helix_core::unix_timestamp_ms())
            .unwrap_or(i64::MAX)
            .saturating_add(60_000);
        connection
            .execute(
                "UPDATE users SET login_not_before_unix_ms = ?1 WHERE login_name = 'owner'",
                [delayed_until],
            )
            .expect("keep the correct-password case inside the login delay");

        let attempts = [
            ("owner", PASSWORD, 2_u8),
            ("unknown", "wrong-password", 3_u8),
            ("NotCanonical", "wrong-password", 4_u8),
        ];
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
            assert_eq!(body, expected);
        }

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
        assert_eq!(response_json(disabled).await, expected);
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
            .clone()
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
    async fn file_and_server_reads_require_their_explicit_capabilities() {
        let context = test_app(DatabaseStatus::Ok).await;
        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        connection
            .execute(
                "DELETE FROM role_capabilities
                 WHERE capability IN ('storage.files.read', 'games.view')",
                [],
            )
            .expect("remove explicit read capabilities");
        drop(connection);
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        for uri in [
            "/api/v1/files?path=/",
            "/api/v1/servers",
            "/api/v1/servers/minecraft/versions?software=paper",
            "/api/v1/servers/inventory-health",
            "/api/v1/servers/example/marketplace/search?query=world",
            "/api/v1/servers/example/marketplace/projects/1bokaNcj",
            "/api/v1/servers/minecraft/modpacks/search?query=adventure",
            "/api/v1/servers/minecraft/modpacks/projects/1bokaNcj",
        ] {
            let response = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(get(uri), &client.cookie),
                    &client.csrf,
                ))
                .await
                .expect("read authorization response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
            assert_eq!(
                response_json(response).await["code"],
                "authorization_denied"
            );
        }
    }

    #[tokio::test]
    async fn file_uploads_require_storage_or_game_capabilities() {
        let context = test_app(DatabaseStatus::Ok).await;
        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        connection
            .execute(
                "DELETE FROM role_capabilities
                 WHERE capability IN ('storage.files.manage', 'games.manage')",
                [],
            )
            .expect("remove upload capabilities");
        drop(connection);
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        for (uri, body) in [
            (
                "/api/v1/files/upload/begin",
                json!({
                    "target": { "kind": "directory", "parent": "/" },
                    "name": "note.txt",
                    "expected_size": 12
                }),
            ),
            (
                "/api/v1/files/upload/begin",
                json!({
                    "target": { "kind": "custom_jar" },
                    "name": "server.jar",
                    "expected_size": 32768
                }),
            ),
            (
                "/api/v1/files/upload/chunk",
                json!({
                    "upload_id": "00000000-0000-4000-8000-000000000001",
                    "purpose": "storage",
                    "offset": 0,
                    "data_base64": "QQ=="
                }),
            ),
            (
                "/api/v1/files/upload/finish",
                json!({
                    "upload_id": "00000000-0000-4000-8000-000000000001",
                    "purpose": "custom_jar"
                }),
            ),
        ] {
            let response = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(post_json(uri, &body, 81), &client.cookie),
                    &client.csrf,
                ))
                .await
                .expect("upload authorization response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
            assert_eq!(
                response_json(response).await["code"],
                "authorization_denied"
            );
        }
    }

    #[tokio::test]
    async fn marketplace_images_use_session_auth_without_a_header_images_cannot_send() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;

        let unauthenticated = context
            .app
            .clone()
            .oneshot(get(
                "/api/v1/marketplace/modrinth/image?path=invalid-provider-path",
            ))
            .await
            .expect("unauthenticated marketplace image response");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let authenticated_without_csrf = context
            .app
            .clone()
            .oneshot(with_cookie(
                get("/api/v1/marketplace/modrinth/image?path=invalid-provider-path"),
                &client.cookie,
            ))
            .await
            .expect("authenticated marketplace image response");
        assert_eq!(authenticated_without_csrf.status(), StatusCode::NOT_FOUND);

        let forbidden_context = test_app(DatabaseStatus::Ok).await;
        let connection = rusqlite::Connection::open(
            forbidden_context
                .data
                .path()
                .join("state")
                .join("helix-state.db"),
        )
        .expect("open state database");
        connection
            .execute(
                "DELETE FROM role_capabilities WHERE capability = 'games.view'",
                [],
            )
            .expect("remove games view capability");
        drop(connection);
        let forbidden_bootstrap = install_bootstrap(&forbidden_context);
        let forbidden_client = claim_owner(&forbidden_context, &forbidden_bootstrap).await;
        let forbidden = forbidden_context
            .app
            .clone()
            .oneshot(with_cookie(
                get("/api/v1/marketplace/modrinth/image?path=invalid-provider-path"),
                &forbidden_client.cookie,
            ))
            .await
            .expect("marketplace image capability response");
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn marketplace_install_requires_manage_capability_and_maps_json_after_auth() {
        let context = test_app(DatabaseStatus::Ok).await;
        let unauthenticated = HttpRequest::builder()
            .method("POST")
            .uri("/api/v1/servers/example/marketplace/install")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
            .body(Body::from("{"))
            .expect("malformed unauthenticated request");
        let response = context
            .app
            .clone()
            .oneshot(unauthenticated)
            .await
            .expect("unauthenticated response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        connection
            .execute(
                "DELETE FROM role_capabilities WHERE capability = 'games.manage'",
                [],
            )
            .expect("remove server management capability");
        drop(connection);
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let response = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/servers/example/marketplace/install",
                        &json!({"project_id": "1bokaNcj", "version_id": null}),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("marketplace authorization response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(response).await["code"],
            "authorization_denied"
        );

        let response = context
            .app
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/servers/minecraft/modpacks",
                        &json!({
                            "name": "Adventure",
                            "memory_mb": 6144,
                            "max_players": 20,
                            "game_port": 25565,
                            "start_on_boot": true,
                            "eula_accepted": true,
                            "project_id": "1bokaNcj",
                            "version_id": "abcdef12"
                        }),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("modpack create authorization response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(response).await["code"],
            "authorization_denied"
        );
    }

    #[tokio::test]
    async fn server_console_maps_json_only_after_headers_and_authentication() {
        let context = test_app(DatabaseStatus::Ok).await;
        let response = context
            .app
            .clone()
            .oneshot(post_raw("/api/v1/servers/example/console", "{", 70))
            .await
            .expect("unauthenticated malformed console response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let response = context
            .app
            .oneshot(with_csrf(
                with_cookie(
                    post_raw("/api/v1/servers/example/console", "{", 71),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("authenticated malformed console response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response_json(response).await["code"], "invalid_json");
    }

    #[tokio::test]
    async fn server_mutations_map_json_only_after_headers_and_authentication() {
        let context = test_app(DatabaseStatus::Ok).await;
        let paths = [
            "/api/v1/servers/example/settings",
            "/api/v1/servers/example/actions",
            "/api/v1/servers/minecraft",
        ];
        for (index, path) in paths.iter().enumerate() {
            let response = context
                .app
                .clone()
                .oneshot(post_raw(path, "{", 80 + u8::try_from(index).unwrap()))
                .await
                .expect("unauthenticated malformed server mutation response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        }

        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        for (index, path) in paths.iter().enumerate() {
            let response = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(
                        post_raw(path, "{", 90 + u8::try_from(index).unwrap()),
                        &client.cookie,
                    ),
                    &client.csrf,
                ))
                .await
                .expect("authenticated malformed server mutation response");
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response_json(response).await["code"], "invalid_json");
        }
    }

    #[tokio::test]
    async fn storage_analysis_requires_its_explicit_capability() {
        let context = test_app(DatabaseStatus::Ok).await;
        let connection =
            rusqlite::Connection::open(context.data.path().join("state").join("helix-state.db"))
                .expect("open state database");
        connection
            .execute(
                "DELETE FROM role_capabilities WHERE capability = 'storage.analyze'",
                [],
            )
            .expect("remove storage analysis capability");
        drop(connection);
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let response = context
            .app
            .oneshot(with_csrf(
                with_cookie(
                    post_json("/api/v1/storage/analysis", &json!({"path": "/srv"}), 1),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("storage authorization response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(response).await["code"],
            "authorization_denied"
        );
    }

    #[test]
    fn storage_analysis_job_ids_are_strict_opaque_uuids() {
        assert!(valid_storage_analysis_job_id(
            "12345678-1234-4234-8234-123456789abc"
        ));
        for invalid in [
            "",
            "12345678-1234-4234-8234-123456789abc/extra",
            "12345678123442348234123456789abc",
            "zzzzzzzz-1234-4234-8234-123456789abc",
            "12345678-1234-4234-8234-123456789ABC",
        ] {
            assert!(!valid_storage_analysis_job_id(invalid), "{invalid}");
        }
    }

    #[tokio::test]
    async fn storage_analysis_maps_json_only_after_headers_and_authentication() {
        let context = test_app(DatabaseStatus::Ok).await;
        let unauthenticated = HttpRequest::builder()
            .method("POST")
            .uri("/api/v1/storage/analysis")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
            .body(Body::from("{"))
            .expect("malformed unauthenticated request");
        let response = context
            .app
            .clone()
            .oneshot(unauthenticated)
            .await
            .expect("unauthenticated response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let malformed = HttpRequest::builder()
            .method("POST")
            .uri("/api/v1/storage/analysis")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
            .body(Body::from("{"))
            .expect("malformed authenticated request");
        let response = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(malformed, &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("malformed response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response_json(response).await["code"], "invalid_json");

        let invalid_path = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/storage/analysis",
                        &json!({"path": "../outside", "mode": "quick"}),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("invalid path response");
        assert_eq!(invalid_path.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(invalid_path).await["code"],
            "invalid_storage_analysis_request"
        );

        let oversized = HttpRequest::builder()
            .method("POST")
            .uri("/api/v1/storage/analysis")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
            .body(Body::from(
                json!({"path": format!("/{}", "x".repeat(9_000)), "mode": "quick"}).to_string(),
            ))
            .expect("oversized analysis request");
        let oversized = context
            .app
            .oneshot(with_csrf(
                with_cookie(oversized, &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("oversized response");
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn storage_analysis_routes_are_bounded_before_broker_dispatch() {
        const SESSION_START_BUDGET: usize = 4;

        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        for _ in 0..SESSION_START_BUDGET {
            let response = context
                .app
                .clone()
                .oneshot(with_csrf(
                    with_cookie(
                        post_json(
                            "/api/v1/storage/analysis",
                            &json!({"path": "/srv", "mode": "quick"}),
                            1,
                        ),
                        &client.cookie,
                    ),
                    &client.csrf,
                ))
                .await
                .expect("bounded analysis start");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
        let limited = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/storage/analysis",
                        &json!({"path": "/srv", "mode": "quick"}),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("limited analysis start");
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            limited.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("60"))
        );
        assert_eq!(
            response_json(limited).await["code"],
            "storage_analysis_rate_limited"
        );

        let job_id = "12345678-1234-4234-8234-123456789abc";
        let mut status_request = with_csrf(
            with_cookie(
                get(&format!("/api/v1/storage/analysis/{job_id}")),
                &client.cookie,
            ),
            &client.csrf,
        );
        status_request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))));
        let status_response = context
            .app
            .clone()
            .oneshot(status_request)
            .await
            .expect("analysis status");
        let status_code = status_response.status();
        let status_body = response_json(status_response).await;
        assert_eq!(
            status_code,
            StatusCode::SERVICE_UNAVAILABLE,
            "{status_body}"
        );
        let cancel = with_csrf(
            with_cookie(
                delete_json(&format!("/api/v1/storage/analysis/{job_id}"), 1),
                &client.cookie,
            ),
            &client.csrf,
        );
        assert_eq!(
            context
                .app
                .oneshot(cancel)
                .await
                .expect("analysis cancel")
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
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

    #[tokio::test]
    async fn ui_strands_can_be_installed_enabled_and_asked_for_metrics() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("strands")
            .join("system-health");
        let bytes = helix_strand_kit::pack_strand_project(&root).expect("pack example");
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);

        let listed = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(get("/api/v1/strands"), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("list empty");
        assert_eq!(listed.status(), StatusCode::OK);
        assert_eq!(response_json(listed).await["strands"], json!([]));

        let installed = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        "/api/v1/strands",
                        &json!({
                            "source": "upload",
                            "filename": "system-health.strand.zip",
                            "bytesBase64": encoded
                        }),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("install");
        assert_eq!(installed.status(), StatusCode::CREATED);
        let installed = response_json(installed).await;
        assert_eq!(installed["slug"], "system-health");
        assert_eq!(installed["enabled"], false);
        let strand_id = installed["id"].as_str().expect("id").to_owned();

        let enabled = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        &format!("/api/v1/strands/{strand_id}"),
                        &json!({ "enabled": true }),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("enable");
        assert_eq!(enabled.status(), StatusCode::OK);
        assert_eq!(response_json(enabled).await["enabled"], true);

        let called = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        &format!("/api/v1/strands/{strand_id}/host"),
                        &json!({ "method": "metrics.snapshot", "params": {} }),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("host call");
        assert_eq!(called.status(), StatusCode::OK);
        let snapshot = response_json(called).await;
        assert!(snapshot["helixVersion"].as_str().is_some());
        assert!(snapshot["memoryTotalBytes"].as_u64().is_some());

        let ui = context
            .app
            .clone()
            .oneshot(with_cookie(
                get(&format!("/api/v1/strands/{strand_id}/files/ui/index.html")),
                &client.cookie,
            ))
            .await
            .expect("ui file");
        assert_eq!(ui.status(), StatusCode::OK);
        let csp = ui
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .expect("csp")
            .to_str()
            .expect("csp ascii");
        assert!(csp.contains("unsafe-inline"));
        assert!(csp.contains("frame-ancestors 'self'"));

        let disabled = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        &format!("/api/v1/strands/{strand_id}"),
                        &json!({ "enabled": false }),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("disable");
        assert_eq!(disabled.status(), StatusCode::OK);

        let denied = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        &format!("/api/v1/strands/{strand_id}/host"),
                        &json!({ "method": "metrics.snapshot", "params": {} }),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("disabled host call");
        assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response_json(denied).await["code"], "strand_rejected");

        let hidden = context
            .app
            .clone()
            .oneshot(with_cookie(
                get(&format!("/api/v1/strands/{strand_id}/files/ui/index.html")),
                &client.cookie,
            ))
            .await
            .expect("disabled ui file");
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn https_strands_reject_private_targets_and_remember_updates() {
        let context = test_app(DatabaseStatus::Ok).await;
        let bootstrap = install_bootstrap(&context);
        let client = claim_owner(&context, &bootstrap).await;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("strands")
            .join("https-probe");
        let bytes = helix_strand_kit::pack_strand_project(&root).expect("pack https-probe");
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        let source = json!({
            "source": "upload",
            "filename": "https-probe.strand.zip",
            "bytesBase64": encoded
        });

        let inspected = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json("/api/v1/strands/inspect", &source, 1),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("inspect");
        assert_eq!(inspected.status(), StatusCode::OK);
        let inspected = response_json(inspected).await;
        assert_eq!(inspected["alreadyInstalled"], false);
        assert_eq!(inspected["slug"], "https-probe");

        let installed = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(post_json("/api/v1/strands", &source, 1), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("install https-probe");
        assert_eq!(installed.status(), StatusCode::CREATED);
        let installed = response_json(installed).await;
        let strand_id = installed["id"].as_str().expect("id").to_owned();

        let again = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json("/api/v1/strands/inspect", &source, 1),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("inspect existing");
        assert_eq!(response_json(again).await["alreadyInstalled"], true);

        let enabled = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    put_json(
                        &format!("/api/v1/strands/{strand_id}"),
                        &json!({ "enabled": true }),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("enable https-probe");
        assert_eq!(enabled.status(), StatusCode::OK);

        let private_target = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(
                    post_json(
                        &format!("/api/v1/strands/{strand_id}/host"),
                        &json!({
                            "method": "net.fetch",
                            "params": { "method": "GET", "url": "https://127.0.0.1/" }
                        }),
                        1,
                    ),
                    &client.cookie,
                ),
                &client.csrf,
            ))
            .await
            .expect("ssrf host call");
        assert_eq!(private_target.status(), StatusCode::BAD_REQUEST);

        let updated = context
            .app
            .clone()
            .oneshot(with_csrf(
                with_cookie(post_json("/api/v1/strands", &source, 1), &client.cookie),
                &client.csrf,
            ))
            .await
            .expect("reinstall");
        assert_eq!(updated.status(), StatusCode::CREATED);
        assert_eq!(response_json(updated).await["enabled"], false);
    }
}
