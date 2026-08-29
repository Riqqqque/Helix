use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs},
    time::Duration,
};
use thiserror::Error;

const USER_AGENT: &str = concat!(
    "Helix/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Riqqqque/Helix)"
);
const MAX_REDIRECTS: u32 = 0;

#[derive(Debug, Error)]
pub(crate) enum StrandNetError {
    #[error("the URL is not an allowed HTTPS target")]
    Denied,
    #[error("the HTTPS origin is not in this Strand's allowlist")]
    OriginDenied,
    #[error("the HTTPS request failed")]
    Unavailable,
    #[error("the HTTPS response was too large or invalid")]
    InvalidResponse,
}

pub(crate) struct HttpsRequest<'a> {
    pub url: &'a str,
    pub method: &'a str,
    pub headers: &'a [(String, String)],
    pub body: Option<&'a [u8]>,
    pub timeout: Duration,
    pub max_response_bytes: u64,
    pub allowed_origins: Option<&'a [String]>,
}

pub(crate) struct HttpsResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

pub(crate) fn fetch_https(request: HttpsRequest<'_>) -> Result<HttpsResponse, StrandNetError> {
    let parsed = parse_https_url(request.url)?;
    if let Some(allowed) = request.allowed_origins
        && !origin_allowed(&parsed.origin, allowed)
    {
        return Err(StrandNetError::OriginDenied);
    }
    reject_blocked_host(&parsed.host, parsed.port)?;
    let method = request.method.to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "HEAD" | "POST" | "PUT" | "DELETE") {
        return Err(StrandNetError::Denied);
    }
    if request.body.is_some() && matches!(method.as_str(), "GET" | "HEAD") {
        return Err(StrandNetError::Denied);
    }
    if request.timeout > Duration::from_secs(30) {
        return Err(StrandNetError::Denied);
    }
    let agent = ureq::Agent::from(
        ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(MAX_REDIRECTS)
            .timeout_global(Some(request.timeout.max(Duration::from_millis(10))))
            .user_agent(USER_AGENT)
            .build(),
    );
    let apply_headers = |mut builder: ureq::RequestBuilder<ureq::typestate::WithoutBody>| {
        for (name, value) in request.headers {
            if !allowed_request_header(name) {
                return Err(StrandNetError::Denied);
            }
            builder = builder.header(name.as_str(), value.as_str());
        }
        Ok(builder)
    };
    let mut response = match method.as_str() {
        "GET" => apply_headers(agent.get(request.url))?.call(),
        "HEAD" => apply_headers(agent.head(request.url))?.call(),
        "DELETE" => apply_headers(agent.delete(request.url))?.call(),
        "POST" | "PUT" => {
            let mut builder = if method == "POST" {
                agent.post(request.url)
            } else {
                agent.put(request.url)
            };
            for (name, value) in request.headers {
                if !allowed_request_header(name) {
                    return Err(StrandNetError::Denied);
                }
                builder = builder.header(name.as_str(), value.as_str());
            }
            match request.body {
                Some(body) => builder.send(body),
                None => builder.send_empty(),
            }
        }
        _ => return Err(StrandNetError::Denied),
    }
    .map_err(|_| StrandNetError::Unavailable)?;
    let status = u16::from(response.status());
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    if content_type.len() > 128
        || content_type
            .bytes()
            .any(|byte| !(0x20..=0x7e).contains(&byte))
    {
        return Err(StrandNetError::InvalidResponse);
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(request.max_response_bytes)
        .read_to_vec()
        .map_err(|_| StrandNetError::InvalidResponse)?;
    Ok(HttpsResponse {
        status,
        content_type,
        body,
    })
}

struct ParsedHttps {
    host: String,
    port: u16,
    origin: String,
}

fn parse_https_url(value: &str) -> Result<ParsedHttps, StrandNetError> {
    if value.len() > 2_048 || value.contains('\0') {
        return Err(StrandNetError::Denied);
    }
    let uri: axum::http::Uri = value.parse().map_err(|_| StrandNetError::Denied)?;
    if uri.scheme_str() != Some("https") {
        return Err(StrandNetError::Denied);
    }
    let authority = uri.authority().ok_or(StrandNetError::Denied)?;
    if authority.as_str().contains('@') {
        return Err(StrandNetError::Denied);
    }
    let host = authority.host();
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return Err(StrandNetError::Denied);
    }
    let port = authority.port_u16().unwrap_or(443);
    if port == 0 {
        return Err(StrandNetError::Denied);
    }
    let origin = if port == 443 {
        format!("https://{host}")
    } else {
        format!("https://{host}:{port}")
    };
    Ok(ParsedHttps {
        host: host.to_owned(),
        port,
        origin,
    })
}

fn origin_allowed(origin: &str, allowed: &[String]) -> bool {
    allowed
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(origin))
}

fn reject_blocked_host(host: &str, port: u16) -> Result<(), StrandNetError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_is_blocked(ip) {
            return Err(StrandNetError::Denied);
        }
        return Ok(());
    }
    let lookup = format!("{host}:{port}");
    let addresses = lookup
        .to_socket_addrs()
        .map_err(|_| StrandNetError::Unavailable)?;
    let mut resolved = false;
    for address in addresses {
        resolved = true;
        if ip_is_blocked(address.ip()) {
            return Err(StrandNetError::Denied);
        }
    }
    if resolved {
        Ok(())
    } else {
        Err(StrandNetError::Unavailable)
    }
}

fn ip_is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(address) => ipv4_is_blocked(address),
        IpAddr::V6(address) => ipv6_is_blocked(address),
    }
}

fn ipv4_is_blocked(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || address.is_documentation()
        || octets[0] == 0
        || octets[0] == 100 && octets[1] & 0b1100_0000 == 64
        || octets[0] == 169 && octets[1] == 254
        || octets[0] == 192 && octets[1] == 0 && octets[2] == 0
        || octets[0] == 198 && (octets[1] == 18 || octets[1] == 19)
}

fn ipv6_is_blocked(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return ipv4_is_blocked(mapped);
    }
    address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (address.segments()[0] & 0xfe00) == 0xfc00
        || (address.segments()[0] & 0xffc0) == 0xfe80
        || address.segments()[0] == 0x2001 && address.segments()[1] == 0xdb8
}

fn allowed_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept" | "accept-language" | "authorization" | "content-type"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_metadata_targets_are_denied() {
        assert!(parse_https_url("http://example.com/x").is_err());
        assert!(parse_https_url("https://user:pass@example.com/x").is_err());
        assert!(reject_blocked_host("127.0.0.1", 443).is_err());
        assert!(reject_blocked_host("10.0.0.8", 443).is_err());
        assert!(reject_blocked_host("169.254.169.254", 80).is_err());
        assert!(reject_blocked_host("::1", 443).is_err());
        assert!(ipv4_is_blocked(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(!ipv4_is_blocked(Ipv4Addr::new(1, 1, 1, 1)));
        assert!(origin_allowed(
            "https://api.open-meteo.com",
            &["https://api.open-meteo.com".to_owned()]
        ));
        assert!(!origin_allowed(
            "https://evil.example",
            &["https://api.open-meteo.com".to_owned()]
        ));
    }
}
