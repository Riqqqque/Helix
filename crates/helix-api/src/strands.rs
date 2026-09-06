use crate::{
    ApiError, ApiState, auth,
    strand_net::{self, HttpsRequest, StrandNetError},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path as RoutePath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use helix_core::unix_timestamp_ms;
use helix_state::{
    StateError, StrandInstallInput, StrandKvEntry, StrandOrigin, StrandPackageSummary,
};
use helix_strand_kit::{
    CapabilityRequest, UnpackedStrand, content_type_for_asset, unpack_strand_package,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const STRAND_API_BODY_LIMIT_BYTES: usize = 12 * 1024 * 1024;
const MAX_UPLOAD_FILENAME: usize = 128;
const MAX_HOST_REQUEST_BODY: usize = 64 * 1024;
const MAX_NET_RESPONSE_BYTES: u64 = 256 * 1024;
const UI_CSP: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'none'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'";

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/strands", get(list_strands).post(install_strand))
        .route("/strands/inspect", post(inspect_strand))
        .route(
            "/strands/{strand_id}",
            put(set_strand_enabled).delete(delete_strand),
        )
        .route("/strands/{strand_id}/package", get(download_strand_package))
        .route("/strands/{strand_id}/host", post(strand_host_call))
        .route(
            "/strands/{strand_id}/files/{*asset}",
            get(serve_strand_file),
        )
        .layer(DefaultBodyLimit::max(STRAND_API_BODY_LIMIT_BYTES))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StrandCapabilityView {
    name: String,
    reason: String,
    optional: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    origins: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StrandSummaryView {
    id: String,
    slug: String,
    name: String,
    version: String,
    description: String,
    license: String,
    publisher: String,
    kind: String,
    enabled: bool,
    origin: String,
    origin_detail: String,
    digest_sha256: String,
    ui_entry: String,
    capabilities: Vec<StrandCapabilityView>,
    limits: Value,
    package_bytes: i64,
    installed_at_unix_ms: i64,
    updated_at_unix_ms: i64,
    has_page: bool,
    has_widget: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase")]
enum StrandPackageSource {
    Upload {
        filename: String,
        #[serde(rename = "bytesBase64")]
        bytes_base64: String,
    },
    Url {
        url: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StrandEnableBody {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StrandHostCallBody {
    method: String,
    #[serde(default)]
    params: Value,
}

async fn list_strands(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    let databases = Arc::clone(&state.databases);
    let packages = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases.state().list_strand_packages()
    })
    .await?;
    let strands = packages
        .into_iter()
        .map(summary_view)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "strands": strands })),
    ))
}

async fn inspect_strand(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<StrandPackageSource>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let Json(source) = body.map_err(auth::map_json_rejection)?;
    let (unpacked, _) = load_unpacked_source(&state, source).await?;
    let strand_id = unpacked.manifest.id.hyphenated().to_string();
    let databases = Arc::clone(&state.databases);
    let existing = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases.state().strand_package(&strand_id)
    })
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "id": unpacked.manifest.id.hyphenated().to_string(),
            "slug": unpacked.manifest.slug,
            "name": unpacked.manifest.name,
            "version": unpacked.manifest.version.to_string(),
            "description": unpacked.manifest.description,
            "license": unpacked.manifest.license,
            "publisher": unpacked.manifest.publisher,
            "kind": unpacked.manifest.kind.to_string(),
            "digestSha256": unpacked.digest_sha256,
            "uiEntry": unpacked.ui_entry(),
            "capabilities": unpacked.manifest.capabilities,
            "limits": unpacked.manifest.limits,
            "alreadyInstalled": existing.is_some(),
            "installedVersion": existing.as_ref().map(|record| record.summary.version.clone()),
            "installedDigestSha256": existing.as_ref().map(|record| record.summary.digest_sha256.clone()),
            "files": unpacked.assets.iter().map(|asset| json!({
                "path": asset.path,
                "bytes": asset.bytes.len(),
            })).collect::<Vec<_>>(),
        })),
    ))
}

async fn install_strand(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<StrandPackageSource>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let Json(source) = body.map_err(auth::map_json_rejection)?;
    let (origin, origin_detail) = match &source {
        StrandPackageSource::Upload { filename, .. } => (StrandOrigin::Upload, filename.clone()),
        StrandPackageSource::Url { url } => (StrandOrigin::Url, url.clone()),
    };
    let (unpacked, package_bytes) = load_unpacked_source(&state, source).await?;
    let capabilities_json = serde_json::to_string(&unpacked.manifest.capabilities)
        .map_err(|_| ApiError::ServiceUnavailable)?;
    let limits_json = serde_json::to_string(&unpacked.manifest.limits)
        .map_err(|_| ApiError::ServiceUnavailable)?;
    let input_id = unpacked.manifest.id.hyphenated().to_string();
    let slug = unpacked.manifest.slug.clone();
    let name = unpacked.manifest.name.clone();
    let version = unpacked.manifest.version.to_string();
    let description = unpacked.manifest.description.clone();
    let license = unpacked.manifest.license.clone();
    let publisher = unpacked.manifest.publisher.clone();
    let digest = unpacked.digest_sha256.clone();
    let ui_entry = unpacked.ui_entry().to_owned();
    let now = i64::try_from(unix_timestamp_ms()).unwrap_or(i64::MAX);
    let databases = Arc::clone(&state.databases);
    let origin_detail_owned = truncate_origin_detail(&origin_detail);
    let summary = tokio::task::spawn_blocking(move || {
        databases
            .state()
            .install_strand_package(StrandInstallInput {
                id: &input_id,
                slug: &slug,
                name: &name,
                version: &version,
                description: &description,
                license: &license,
                publisher: &publisher,
                origin,
                origin_detail: &origin_detail_owned,
                digest_sha256: &digest,
                ui_entry: &ui_entry,
                capabilities_json: &capabilities_json,
                limits_json: &limits_json,
                package_bytes: &package_bytes,
                now_unix_ms: now,
            })
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable)?
    .map_err(map_strand_state)?;
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(summary_view(summary)?),
    )
        .into_response())
}

async fn set_strand_enabled(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(strand_id): RoutePath<String>,
    body: Result<Json<StrandEnableBody>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    let now = i64::try_from(unix_timestamp_ms()).unwrap_or(i64::MAX);
    let databases = Arc::clone(&state.databases);
    let summary = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases
            .state()
            .set_strand_enabled(&strand_id, body.enabled, now)
    })
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(summary_view(summary)?),
    ))
}

async fn delete_strand(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(strand_id): RoutePath<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.settings.write").await?;
    let databases = Arc::clone(&state.databases);
    let deleted = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases.state().delete_strand_package(&strand_id)
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

async fn download_strand_package(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(strand_id): RoutePath<String>,
) -> Result<Response, ApiError> {
    auth::require_capability(&state, &headers, "system.view").await?;
    let databases = Arc::clone(&state.databases);
    let record = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases.state().strand_package(&strand_id)
    })
    .await?
    .ok_or(ApiError::NotFound)?;
    let filename = format!("{}.strand.zip", record.summary.slug);
    let mut response = Response::new(Body::from(record.package_bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok(response)
}

async fn serve_strand_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath((strand_id, asset)): RoutePath<(String, String)>,
) -> Result<Response, ApiError> {
    auth::require_capability_without_csrf(&state, &headers, "system.view").await?;
    let databases = Arc::clone(&state.databases);
    let record = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases.state().strand_package(&strand_id)
    })
    .await?
    .ok_or(ApiError::NotFound)?;
    if !record.summary.enabled {
        return Err(ApiError::NotFound);
    }
    let unpacked = unpack_strand_package(&record.package_bytes).map_err(map_kit_error)?;
    let path = if asset.is_empty() {
        unpacked.ui_entry().to_owned()
    } else if asset.starts_with("ui/") {
        asset
    } else {
        format!("ui/{asset}")
    };
    let Some(file) = unpacked.asset(&path) else {
        return Err(ApiError::NotFound);
    };
    let content_type = content_type_for_asset(&file.path);
    let mut response = Response::new(Body::from(file.bytes.clone()));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(UI_CSP),
    );
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("SAMEORIGIN"),
    );
    Ok(response)
}

async fn strand_host_call(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RoutePath(strand_id): RoutePath<String>,
    body: Result<Json<StrandHostCallBody>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    auth::validate_post_headers(&headers)?;
    auth::require_capability(&state, &headers, "system.view").await?;
    let Json(body) = body.map_err(auth::map_json_rejection)?;
    let databases = Arc::clone(&state.databases);
    let strand_id_for_load = strand_id.clone();
    let record = auth::run_blocking_state(&state.blocking_tasks, move || {
        databases.state().strand_package(&strand_id_for_load)
    })
    .await?
    .ok_or(ApiError::NotFound)?;
    if !record.summary.enabled {
        return Err(ApiError::StrandRejected(
            "Enable this Strand before it can make host calls.".to_owned(),
        ));
    }
    let capabilities = parse_capabilities(&record.summary.capabilities_json)?;
    let limits: helix_strand_kit::ResourceLimits =
        serde_json::from_str(&record.summary.limits_json)
            .map_err(|_| ApiError::ServiceUnavailable)?;
    let _slot = acquire_call_slot(&record.summary.id, limits.concurrent_calls)?;
    let result =
        dispatch_host_call(&state, &record.summary.id, &capabilities, &limits, body).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(result)))
}

async fn dispatch_host_call(
    state: &ApiState,
    strand_id: &str,
    capabilities: &[CapabilityRequest],
    limits: &helix_strand_kit::ResourceLimits,
    body: StrandHostCallBody,
) -> Result<Value, ApiError> {
    match body.method.as_str() {
        "metrics.snapshot" => {
            require_named_capability(capabilities, "helix:metrics.read")?;
            let permit = state.host.try_acquire().ok_or(ApiError::HostBusy)?;
            let snapshot = tokio::task::spawn_blocking(move || permit.snapshot())
                .await
                .map_err(|_| ApiError::HostUnavailable)?
                .map_err(|_| ApiError::HostUnavailable)?;
            Ok(json!({
                "helixVersion": snapshot.helix_version,
                "uptimeSeconds": snapshot.uptime_seconds,
                "cpuPercent": snapshot.cpu.usage_percent,
                "logicalCores": snapshot.cpu.logical_cores,
                "memoryUsedBytes": snapshot.memory.used_bytes,
                "memoryTotalBytes": snapshot.memory.total_bytes,
                "swapUsedBytes": snapshot.swap.used_bytes,
                "swapTotalBytes": snapshot.swap.total_bytes,
                "collectedAtUnixMs": snapshot.collected_at_unix_ms,
            }))
        }
        "storage.get" => {
            require_named_capability(capabilities, "helix:storage.kv")?;
            let key = param_string(&body.params, "key")?;
            let databases = Arc::clone(&state.databases);
            let strand_id = strand_id.to_owned();
            let entry = auth::run_blocking_state(&state.blocking_tasks, move || {
                databases.state().strand_kv_get(&strand_id, &key)
            })
            .await?;
            Ok(json!({ "value": entry.map(|entry| entry.value) }))
        }
        "storage.set" => {
            require_named_capability(capabilities, "helix:storage.kv")?;
            let key = param_string(&body.params, "key")?;
            let value = param_string(&body.params, "value")?;
            let now = i64::try_from(unix_timestamp_ms()).unwrap_or(i64::MAX);
            let databases = Arc::clone(&state.databases);
            let strand_id = strand_id.to_owned();
            let entry = tokio::task::spawn_blocking(move || {
                databases
                    .state()
                    .strand_kv_set(&strand_id, &key, &value, now)
            })
            .await
            .map_err(|_| ApiError::ServiceUnavailable)?
            .map_err(map_strand_state)?;
            Ok(kv_json(entry))
        }
        "storage.delete" => {
            require_named_capability(capabilities, "helix:storage.kv")?;
            let key = param_string(&body.params, "key")?;
            let databases = Arc::clone(&state.databases);
            let strand_id = strand_id.to_owned();
            let deleted = auth::run_blocking_state(&state.blocking_tasks, move || {
                databases.state().strand_kv_delete(&strand_id, &key)
            })
            .await?;
            Ok(json!({ "deleted": deleted }))
        }
        "storage.list" => {
            require_named_capability(capabilities, "helix:storage.kv")?;
            let databases = Arc::clone(&state.databases);
            let strand_id = strand_id.to_owned();
            let entries = auth::run_blocking_state(&state.blocking_tasks, move || {
                databases.state().strand_kv_list(&strand_id)
            })
            .await?;
            Ok(json!({
                "keys": entries.into_iter().map(|entry| entry.key).collect::<Vec<_>>()
            }))
        }
        "net.fetch" => {
            let capability = require_named_capability(capabilities, "helix:net.https")?;
            if !take_outbound_token(strand_id, limits.outbound_requests_per_minute) {
                return Err(ApiError::StrandBusy);
            }
            let url = param_string(&body.params, "url")?;
            let method = body
                .params
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .to_owned();
            let headers = parse_request_headers(body.params.get("headers"))?;
            let request_body = parse_optional_body(body.params.get("body"))?;
            let timeout = Duration::from_millis(u64::from(limits.timeout_ms.max(10)));
            let origins = capability.origins.clone();
            let fetched = tokio::task::spawn_blocking(move || {
                strand_net::fetch_https(HttpsRequest {
                    url: &url,
                    method: &method,
                    headers: &headers,
                    body: request_body.as_deref(),
                    timeout,
                    max_response_bytes: MAX_NET_RESPONSE_BYTES,
                    allowed_origins: Some(&origins),
                })
            })
            .await
            .map_err(|_| ApiError::ServiceUnavailable)?
            .map_err(map_net_error)?;
            let (body, encoding) = if let Ok(text) = std::str::from_utf8(&fetched.body) {
                (text.to_owned(), "utf8")
            } else {
                (STANDARD.encode(&fetched.body), "base64")
            };
            Ok(json!({
                "status": fetched.status,
                "contentType": fetched.content_type,
                "encoding": encoding,
                "body": body,
            }))
        }
        _ => Err(ApiError::StrandRejected(
            "Unknown Strand host method.".to_owned(),
        )),
    }
}

async fn load_unpacked_source(
    state: &ApiState,
    source: StrandPackageSource,
) -> Result<(UnpackedStrand, Vec<u8>), ApiError> {
    let bytes = match source {
        StrandPackageSource::Upload {
            filename,
            bytes_base64,
        } => {
            validate_upload_filename(&filename)?;
            STANDARD.decode(bytes_base64.trim()).map_err(|_| {
                ApiError::StrandRejected("Strand zip is not valid base64.".to_owned())
            })?
        }
        StrandPackageSource::Url { url } => {
            let fetched = tokio::task::spawn_blocking(move || {
                strand_net::fetch_https(HttpsRequest {
                    url: &url,
                    method: "GET",
                    headers: &[],
                    body: None,
                    timeout: Duration::from_secs(20),
                    max_response_bytes: helix_strand_kit::MAX_PACKAGE_BYTES,
                    allowed_origins: None,
                })
            })
            .await
            .map_err(|_| ApiError::ServiceUnavailable)?
            .map_err(map_net_error)?;
            if fetched.status != 200 {
                return Err(ApiError::StrandRejected(
                    "Strand download URL did not return HTTP 200.".to_owned(),
                ));
            }
            fetched.body
        }
    };
    let _ = state;
    let unpacked = unpack_strand_package(&bytes).map_err(map_kit_error)?;
    unpacked
        .manifest
        .ensure_helix_compatible(helix_core::VERSION)
        .map_err(map_kit_error)?;
    Ok((unpacked, bytes))
}

fn summary_view(summary: StrandPackageSummary) -> Result<StrandSummaryView, ApiError> {
    let capabilities = parse_capabilities(&summary.capabilities_json)?;
    let limits: Value =
        serde_json::from_str(&summary.limits_json).map_err(|_| ApiError::ServiceUnavailable)?;
    let has_page = capabilities
        .iter()
        .any(|capability| capability.name == "helix:ui.page");
    let has_widget = capabilities
        .iter()
        .any(|capability| capability.name == "helix:ui.widget");
    Ok(StrandSummaryView {
        id: summary.id,
        slug: summary.slug,
        name: summary.name,
        version: summary.version,
        description: summary.description,
        license: summary.license,
        publisher: summary.publisher,
        kind: summary.kind,
        enabled: summary.enabled,
        origin: summary.origin.as_str().to_owned(),
        origin_detail: summary.origin_detail,
        digest_sha256: summary.digest_sha256,
        ui_entry: summary.ui_entry,
        capabilities: capabilities
            .into_iter()
            .map(|capability| StrandCapabilityView {
                name: capability.name,
                reason: capability.reason,
                optional: capability.optional,
                origins: capability.origins,
            })
            .collect(),
        limits,
        package_bytes: summary.package_bytes_len,
        installed_at_unix_ms: summary.installed_at_unix_ms,
        updated_at_unix_ms: summary.updated_at_unix_ms,
        has_page,
        has_widget,
    })
}

fn parse_capabilities(json: &str) -> Result<Vec<CapabilityRequest>, ApiError> {
    serde_json::from_str(json).map_err(|_| ApiError::ServiceUnavailable)
}

fn require_named_capability<'a>(
    capabilities: &'a [CapabilityRequest],
    name: &str,
) -> Result<&'a CapabilityRequest, ApiError> {
    capabilities
        .iter()
        .find(|capability| capability.name == name)
        .ok_or_else(|| ApiError::StrandRejected(format!("This Strand was not granted {name}.")))
}

fn param_string(params: &Value, key: &str) -> Result<String, ApiError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::StrandRejected(format!("Host call is missing {key}.")))
}

fn parse_request_headers(value: Option<&Value>) -> Result<Vec<(String, String)>, ApiError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::StrandRejected("Request headers must be an object.".to_owned()))?;
    if object.len() > 8 {
        return Err(ApiError::StrandRejected(
            "Too many HTTPS request headers.".to_owned(),
        ));
    }
    let mut headers = Vec::new();
    for (name, value) in object {
        let value = value.as_str().ok_or_else(|| {
            ApiError::StrandRejected("HTTPS header values must be strings.".to_owned())
        })?;
        if name.len() > 64 || value.len() > 4_096 {
            return Err(ApiError::StrandRejected(
                "An HTTPS header is too large.".to_owned(),
            ));
        }
        headers.push((name.clone(), value.to_owned()));
    }
    Ok(headers)
}

fn parse_optional_body(value: Option<&Value>) -> Result<Option<Vec<u8>>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let text = value
        .as_str()
        .ok_or_else(|| ApiError::StrandRejected("HTTPS bodies must be strings.".to_owned()))?;
    if text.len() > MAX_HOST_REQUEST_BODY {
        return Err(ApiError::StrandRejected(
            "HTTPS request body exceeds the Strand limit.".to_owned(),
        ));
    }
    Ok(Some(text.as_bytes().to_vec()))
}

fn kv_json(entry: StrandKvEntry) -> Value {
    json!({
        "key": entry.key,
        "value": entry.value,
        "updatedAtUnixMs": entry.updated_at_unix_ms,
    })
}

fn validate_upload_filename(filename: &str) -> Result<(), ApiError> {
    if filename.is_empty()
        || filename.len() > MAX_UPLOAD_FILENAME
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
    {
        return Err(ApiError::StrandRejected(
            "Choose a zip filename without path separators.".to_owned(),
        ));
    }
    Ok(())
}

fn truncate_origin_detail(value: &str) -> String {
    value.chars().take(512).collect()
}

struct StrandCallGuard {
    strand_id: String,
}

impl Drop for StrandCallGuard {
    fn drop(&mut self) {
        let mut slots = call_slots()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(count) = slots.get_mut(&self.strand_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                slots.remove(&self.strand_id);
            }
        }
    }
}

fn call_slots() -> &'static Mutex<HashMap<String, u16>> {
    static SLOTS: OnceLock<Mutex<HashMap<String, u16>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn acquire_call_slot(strand_id: &str, max: u16) -> Result<StrandCallGuard, ApiError> {
    if max == 0 {
        return Err(ApiError::StrandBusy);
    }
    let mut slots = call_slots()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let count = slots.entry(strand_id.to_owned()).or_insert(0);
    if *count >= max {
        return Err(ApiError::StrandBusy);
    }
    *count = count.saturating_add(1);
    Ok(StrandCallGuard {
        strand_id: strand_id.to_owned(),
    })
}

fn take_outbound_token(strand_id: &str, per_minute: u16) -> bool {
    if per_minute == 0 {
        return false;
    }
    static WINDOW: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();
    let mut windows = WINDOW
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    let stamps = windows.entry(strand_id.to_owned()).or_default();
    stamps.retain(|stamp| now.duration_since(*stamp) < Duration::from_secs(60));
    if stamps.len() >= usize::from(per_minute) {
        return false;
    }
    stamps.push(now);
    true
}

fn map_kit_error(error: helix_strand_kit::StrandKitError) -> ApiError {
    ApiError::StrandRejected(error.to_string())
}

fn map_net_error(error: StrandNetError) -> ApiError {
    match error {
        StrandNetError::Denied | StrandNetError::OriginDenied => {
            ApiError::StrandRejected(error.to_string())
        }
        StrandNetError::Unavailable => {
            ApiError::StrandRejected("Helix could not complete that HTTPS request.".to_owned())
        }
        StrandNetError::InvalidResponse => {
            ApiError::StrandRejected("The HTTPS response was too large or invalid.".to_owned())
        }
    }
}

fn map_strand_state(error: StateError) -> ApiError {
    match error {
        StateError::StrandConflict => ApiError::StrandConflict,
        StateError::StrandNotFound => ApiError::NotFound,
        StateError::StrandQuotaExceeded => {
            ApiError::StrandRejected("This Strand hit a storage or package quota.".to_owned())
        }
        StateError::InvalidStrandInput(message) => ApiError::StrandRejected(message.to_owned()),
        _ => {
            tracing::error!(error = %error, "strand state operation failed");
            ApiError::ServiceUnavailable
        }
    }
}
