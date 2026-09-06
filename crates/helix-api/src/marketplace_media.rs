use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use thiserror::Error;

const MODRINTH_ORIGIN: &str = "https://cdn.modrinth.com";
const CURSEFORGE_ORIGIN: &str = "https://media.forgecdn.net";
const USER_AGENT: &str = "Helix/0.1 (+https://github.com/Riqqqque/Helix)";
const MAX_IMAGE_BYTES: u64 = 512 * 1024;
const MAX_CACHE_ENTRIES: usize = 64;
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy)]
pub(crate) enum MarketplaceImageOrigin {
    Modrinth,
    Curseforge,
}

impl MarketplaceImageOrigin {
    fn cdn(self) -> &'static str {
        match self {
            Self::Modrinth => MODRINTH_ORIGIN,
            Self::Curseforge => CURSEFORGE_ORIGIN,
        }
    }

    fn cache_prefix(self) -> &'static str {
        match self {
            Self::Modrinth => "mr",
            Self::Curseforge => "cf",
        }
    }
}

#[derive(Clone)]
pub(crate) struct MarketplaceImage {
    pub(crate) content_type: &'static str,
    pub(crate) body: Arc<[u8]>,
}

#[derive(Clone)]
struct CachedImage {
    inserted_at: Instant,
    image: MarketplaceImage,
}

#[derive(Debug, Error)]
pub(crate) enum MarketplaceMediaError {
    #[error("invalid marketplace image path")]
    InvalidPath,
    #[error("marketplace image provider unavailable")]
    ProviderUnavailable,
    #[error("marketplace image response invalid")]
    InvalidResponse,
}

static CACHE: OnceLock<Mutex<HashMap<String, CachedImage>>> = OnceLock::new();
static HTTP_AGENT: OnceLock<ureq::Agent> = OnceLock::new();

pub(crate) fn image(
    origin: MarketplaceImageOrigin,
    path: &str,
) -> Result<MarketplaceImage, MarketplaceMediaError> {
    validate_path(origin, path)?;
    let cache_key = format!("{}:{path}", origin.cache_prefix());
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache
        .lock()
        .map_err(|_| MarketplaceMediaError::ProviderUnavailable)?
        .get(&cache_key)
        .filter(|entry| entry.inserted_at.elapsed() < CACHE_TTL)
        .cloned()
    {
        return Ok(cached.image);
    }

    let agent = HTTP_AGENT.get_or_init(|| {
        ureq::Agent::from(
            ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(8)))
                .user_agent(USER_AGENT)
                .build(),
        )
    });
    let url = format!("{}{path}", origin.cdn());
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|_| MarketplaceMediaError::ProviderUnavailable)?;
    if response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length == 0 || length > MAX_IMAGE_BYTES)
    {
        return Err(MarketplaceMediaError::InvalidResponse);
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_IMAGE_BYTES.saturating_add(1))
        .read_to_vec()
        .map_err(|_| MarketplaceMediaError::ProviderUnavailable)?;
    if body.is_empty() || u64::try_from(body.len()).unwrap_or(u64::MAX) > MAX_IMAGE_BYTES {
        return Err(MarketplaceMediaError::InvalidResponse);
    }
    let content_type =
        detected_content_type(&body).ok_or(MarketplaceMediaError::InvalidResponse)?;
    let image = MarketplaceImage {
        content_type,
        body: Arc::from(body),
    };

    let mut entries = cache
        .lock()
        .map_err(|_| MarketplaceMediaError::ProviderUnavailable)?;
    entries.retain(|_, value| value.inserted_at.elapsed() < CACHE_TTL);
    if entries.len() >= MAX_CACHE_ENTRIES
        && let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, value)| value.inserted_at)
            .map(|(key, _)| key.clone())
    {
        entries.remove(&oldest);
    }
    entries.insert(
        cache_key,
        CachedImage {
            inserted_at: Instant::now(),
            image: image.clone(),
        },
    );
    Ok(image)
}

pub(crate) fn validate_path(
    origin: MarketplaceImageOrigin,
    path: &str,
) -> Result<(), MarketplaceMediaError> {
    let prefix = match origin {
        MarketplaceImageOrigin::Modrinth => "/data/",
        MarketplaceImageOrigin::Curseforge => "/avatars/",
    };
    if path.len() < 8
        || path.len() > 512
        || !path.starts_with(prefix)
        || path.contains(['?', '#', '\\', '%'])
        || path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
        || ![".png", ".jpg", ".jpeg", ".webp", ".gif"]
            .iter()
            .any(|extension| path.to_ascii_lowercase().ends_with(extension))
    {
        return Err(MarketplaceMediaError::InvalidPath);
    }
    Ok(())
}

fn detected_content_type(body: &[u8]) -> Option<&'static str> {
    if body.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if body.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if body.starts_with(b"GIF87a") || body.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if body.len() >= 12 && body.starts_with(b"RIFF") && &body[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_paths_cannot_escape_the_exact_modrinth_cdn_shape() {
        assert!(validate_path(MarketplaceImageOrigin::Modrinth, "/data/AANobbMI/icon.png").is_ok());
        assert!(
            validate_path(
                MarketplaceImageOrigin::Modrinth,
                "/data/AANobbMI/images/example_1.webp"
            )
            .is_ok()
        );
        for path in [
            "https://cdn.modrinth.com/data/a/icon.png",
            "/data/../icon.png",
            "/data/a/icon.svg",
            "/data/a/icon.png?redirect=https://example.test",
            "/other/a/icon.png",
        ] {
            assert!(
                validate_path(MarketplaceImageOrigin::Modrinth, path).is_err(),
                "{path}"
            );
        }
        assert!(
            validate_path(
                MarketplaceImageOrigin::Curseforge,
                "/avatars/12/345/icon.png"
            )
            .is_ok()
        );
        assert!(
            validate_path(
                MarketplaceImageOrigin::Curseforge,
                "/data/AANobbMI/icon.png"
            )
            .is_err()
        );
    }

    #[test]
    fn image_type_comes_from_magic_bytes_not_remote_headers() {
        assert_eq!(
            detected_content_type(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
            Some("image/png")
        );
        assert_eq!(detected_content_type(b"GIF89a"), Some("image/gif"));
        assert_eq!(detected_content_type(b"<svg><script>"), None);
    }
}
