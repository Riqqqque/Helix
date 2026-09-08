use crate::{ApiError, ApiState, auth};
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub(crate) const OPENAPI: &str = include_str!("../../../docs/openapi.json");

pub(crate) async fn discovery(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let session = auth::require_session(&state, &headers).await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "schema_version": 1,
            "api_version": "v1",
            "helix_version": env!("CARGO_PKG_VERSION"),
            "openapi_url": "/api/v1/openapi.json",
            "contract_scope": "server-management",
            "capabilities": session.capabilities,
            "authentication": {
                "mode": "session_cookie_and_csrf",
                "csrf_header": "X-Helix-CSRF",
                "mutation_origin_required": true,
                "delegated_tokens_supported": false,
                "per_server_credentials_supported": false
            },
            "conventions": {
                "server_identity": "id",
                "timestamps": "unix_milliseconds",
                "request_id_header": "X-Request-ID",
                "retry_after_header": "Retry-After",
                "automatic_mutation_retry_safe": false,
                "idempotency_keys_supported": false
            },
            "links": {
                "session": "/api/v1/auth/me",
                "servers": "/api/v1/servers",
                "server_readiness": "/api/v1/servers/manager/readiness",
                "server_capabilities_template": "/api/v1/servers/{instance_id}/capabilities",
                "server_files_template": "/api/v1/servers/{instance_id}/files",
                "job_template": "/api/v1/jobs/{job_id}"
            }
        })),
    ))
}

pub(crate) async fn openapi(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth::require_session(&state, &headers).await?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_TYPE, "application/json"),
        ],
        OPENAPI,
    ))
}

// Extractor, method, timeout, and panic responses otherwise bypass ApiError.
// Preserve their status and headers, but never expose framework error details.
pub(crate) async fn normalize_api_errors(request: Request, next: Next) -> Response {
    let is_api = request.uri().path() == "/api" || request.uri().path().starts_with("/api/");
    let response = next.run(request).await;
    if !is_api {
        return response;
    }
    normalize_error(response)
}

fn normalize_error(response: Response) -> Response {
    let status = response.status();
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| {
            value.to_str().is_ok_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
            })
        });
    if !(status.is_client_error() || status.is_server_error()) || is_json {
        return response;
    }
    let (code, message) = match status {
        StatusCode::BAD_REQUEST => ("invalid_request", "The request parameters are invalid."),
        StatusCode::NOT_FOUND => ("not_found", "The requested API resource was not found."),
        StatusCode::METHOD_NOT_ALLOWED => (
            "method_not_allowed",
            "This endpoint does not accept that HTTP method.",
        ),
        StatusCode::REQUEST_TIMEOUT => (
            "request_timeout",
            "The request timed out. A submitted operation may still be running; check its status before submitting again.",
        ),
        StatusCode::PAYLOAD_TOO_LARGE => (
            "payload_too_large",
            "The request exceeds this endpoint's size limit.",
        ),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => (
            "unsupported_media_type",
            "This endpoint requires a supported content type.",
        ),
        StatusCode::UNPROCESSABLE_ENTITY => {
            ("invalid_request", "The request could not be processed.")
        }
        _ => ("request_failed", "The request could not be completed."),
    };
    let (mut parts, _) = response.into_parts();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.remove(header::CONTENT_ENCODING);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    parts
        .headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let body = Json(json!({"code": code, "message": message}))
        .into_response()
        .into_body();
    Response::from_parts(parts, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeout_and_framework_errors_are_safe_json_and_keep_headers() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::PAYLOAD_TOO_LARGE,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            let response = (
                status,
                [(header::ALLOW, "GET"), (header::CONTENT_LENGTH, "6")],
                "secret",
            )
                .into_response();
            let response = normalize_error(response);
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::ALLOW], "GET");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(!response.headers().contains_key(header::CONTENT_LENGTH));
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert!(problem["code"].is_string());
            assert!(!String::from_utf8_lossy(&body).contains("secret"));
        }
    }

    #[test]
    fn successful_downloads_and_existing_problems_are_not_rewritten() {
        let download = (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            "file",
        )
            .into_response();
        assert_eq!(
            normalize_error(download).headers()[header::CONTENT_TYPE],
            "application/octet-stream"
        );
        let problem = ApiError::BrokerUnavailable.into_response();
        let response = normalize_error(problem);
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[header::RETRY_AFTER], "2");
    }
}
