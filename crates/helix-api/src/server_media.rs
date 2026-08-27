use crate::ApiError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use helix_state::{
    MAX_SERVER_ICON_BYTES, ServerAppearanceSummary, ServerAppearanceUpdate, ServerIconPreset,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const MAX_ENCODED_ICON_BYTES: usize = 700 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ServerAppearanceBody {
    Preset {
        expected_revision: i64,
        preset: ApiServerIconPreset,
    },
    Custom {
        expected_revision: i64,
        content_type: String,
        image_base64: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApiServerIconPreset {
    Grass,
    Portal,
    Crystal,
    Fortress,
    Ember,
    Ocean,
}

impl From<ApiServerIconPreset> for ServerIconPreset {
    fn from(value: ApiServerIconPreset) -> Self {
        match value {
            ApiServerIconPreset::Grass => Self::Grass,
            ApiServerIconPreset::Portal => Self::Portal,
            ApiServerIconPreset::Crystal => Self::Crystal,
            ApiServerIconPreset::Fortress => Self::Fortress,
            ApiServerIconPreset::Ember => Self::Ember,
            ApiServerIconPreset::Ocean => Self::Ocean,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClearServerAppearanceBody {
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerAppearanceImageQuery {
    pub revision: i64,
}

pub(crate) enum OwnedServerAppearanceUpdate {
    Preset(ServerIconPreset),
    Custom {
        content_type: &'static str,
        image_bytes: Vec<u8>,
        width: u16,
        height: u16,
    },
}

impl OwnedServerAppearanceUpdate {
    pub(crate) fn as_borrowed(&self) -> ServerAppearanceUpdate<'_> {
        match self {
            Self::Preset(preset) => ServerAppearanceUpdate::Preset(*preset),
            Self::Custom {
                content_type,
                image_bytes,
                width,
                height,
            } => ServerAppearanceUpdate::Custom {
                content_type,
                image_bytes,
                width: *width,
                height: *height,
            },
        }
    }
}

pub(crate) fn validate_server_id(server_id: &str) -> Result<(), ApiError> {
    let valid_prefix = server_id.starts_with("helix:") || server_id.starts_with("amp:");
    if valid_prefix
        && (7..=165).contains(&server_id.len())
        && server_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(ApiError::InvalidServerAppearance)
    }
}

pub(crate) fn validate_appearance_body(
    body: ServerAppearanceBody,
) -> Result<(i64, OwnedServerAppearanceUpdate), ApiError> {
    match body {
        ServerAppearanceBody::Preset {
            expected_revision,
            preset,
        } => {
            validate_revision(expected_revision)?;
            Ok((
                expected_revision,
                OwnedServerAppearanceUpdate::Preset(preset.into()),
            ))
        }
        ServerAppearanceBody::Custom {
            expected_revision,
            content_type,
            image_base64,
        } => {
            validate_revision(expected_revision)?;
            if image_base64.is_empty()
                || image_base64.len() > MAX_ENCODED_ICON_BYTES
                || !image_base64
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
            {
                return Err(ApiError::InvalidServerAppearance);
            }
            let image_bytes = STANDARD
                .decode(image_base64.as_bytes())
                .map_err(|_| ApiError::InvalidServerAppearance)?;
            if image_bytes.len() > MAX_SERVER_ICON_BYTES {
                return Err(ApiError::InvalidServerAppearance);
            }
            let (detected_type, width, height) = validated_image_metadata(&image_bytes)?;
            if content_type != detected_type {
                return Err(ApiError::InvalidServerAppearance);
            }
            Ok((
                expected_revision,
                OwnedServerAppearanceUpdate::Custom {
                    content_type: detected_type,
                    image_bytes,
                    width,
                    height,
                },
            ))
        }
    }
}

fn validate_revision(revision: i64) -> Result<(), ApiError> {
    if revision < 0 {
        Err(ApiError::InvalidServerAppearance)
    } else {
        Ok(())
    }
}

pub(crate) fn appearance_json(summary: Option<&ServerAppearanceSummary>) -> Value {
    let Some(summary) = summary else {
        return json!({ "kind": "default", "revision": 0 });
    };
    if let Some(preset) = summary.preset {
        return json!({
            "kind": "preset",
            "revision": summary.revision,
            "preset": preset.as_str(),
            "updated_at_unix_ms": summary.updated_at_unix_ms,
        });
    }
    json!({
        "kind": "custom",
        "revision": summary.revision,
        "content_type": summary.content_type,
        "width": summary.width,
        "height": summary.height,
        "updated_at_unix_ms": summary.updated_at_unix_ms,
        "image_url": format!(
            "/api/v1/servers/{}/appearance/image?revision={}",
            summary.server_id, summary.revision
        ),
    })
}

pub(crate) fn attach_appearances(
    servers: &mut Value,
    appearances: &[ServerAppearanceSummary],
) -> Result<(), ()> {
    let servers = servers.as_array_mut().ok_or(())?;
    let appearances = appearances
        .iter()
        .map(|appearance| (appearance.server_id.as_str(), appearance))
        .collect::<HashMap<_, _>>();
    for server in servers {
        let object = server.as_object_mut().ok_or(())?;
        let id = object.get("id").and_then(Value::as_str).ok_or(())?;
        object.insert(
            "appearance".to_owned(),
            appearance_json(appearances.get(id).copied()),
        );
    }
    Ok(())
}

pub(crate) fn server_list_contains(servers: &Value, instance_id: &str) -> bool {
    servers.as_array().is_some_and(|servers| {
        servers.iter().any(|server| {
            server
                .as_object()
                .and_then(|server| server.get("id"))
                .and_then(Value::as_str)
                == Some(instance_id)
        })
    })
}

fn validated_image_metadata(image: &[u8]) -> Result<(&'static str, u16, u16), ApiError> {
    let (content_type, width, height) = if image.starts_with(b"\x89PNG\r\n\x1a\n") {
        if image.len() < 24 || &image[12..16] != b"IHDR" {
            return Err(ApiError::InvalidServerAppearance);
        }
        let width = u32::from_be_bytes(
            image[16..20]
                .try_into()
                .map_err(|_| ApiError::InvalidServerAppearance)?,
        );
        let height = u32::from_be_bytes(
            image[20..24]
                .try_into()
                .map_err(|_| ApiError::InvalidServerAppearance)?,
        );
        ("image/png", width, height)
    } else if image.starts_with(&[0xff, 0xd8]) {
        let (width, height) = jpeg_dimensions(image)?;
        ("image/jpeg", u32::from(width), u32::from(height))
    } else {
        return Err(ApiError::InvalidServerAppearance);
    };
    if !(32..=2_048).contains(&width) || !(32..=2_048).contains(&height) {
        return Err(ApiError::InvalidServerAppearance);
    }
    Ok((
        content_type,
        u16::try_from(width).map_err(|_| ApiError::InvalidServerAppearance)?,
        u16::try_from(height).map_err(|_| ApiError::InvalidServerAppearance)?,
    ))
}

fn jpeg_dimensions(image: &[u8]) -> Result<(u16, u16), ApiError> {
    let mut offset = 2_usize;
    while offset < image.len() {
        if image[offset] != 0xff {
            return Err(ApiError::InvalidServerAppearance);
        }
        while offset < image.len() && image[offset] == 0xff {
            offset += 1;
        }
        let marker = *image.get(offset).ok_or(ApiError::InvalidServerAppearance)?;
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        let length_bytes = image
            .get(offset..offset + 2)
            .ok_or(ApiError::InvalidServerAppearance)?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2
            || offset
                .checked_add(length)
                .is_none_or(|end| end > image.len())
        {
            return Err(ApiError::InvalidServerAppearance);
        }
        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) {
            if length < 7 {
                return Err(ApiError::InvalidServerAppearance);
            }
            let height = u16::from_be_bytes([image[offset + 3], image[offset + 4]]);
            let width = u16::from_be_bytes([image[offset + 5], image[offset + 6]]);
            return Ok((width, height));
        }
        offset += length;
    }
    Err(ApiError::InvalidServerAppearance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_metadata_uses_magic_bytes_and_bounded_dimensions() {
        let mut png = vec![
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0,
            0, 2, 0, 0, 0, 1, 0,
        ];
        png.resize(64, 0);
        assert_eq!(
            validated_image_metadata(&png).ok(),
            Some(("image/png", 512, 256))
        );
        png[16..20].copy_from_slice(&4_096_u32.to_be_bytes());
        assert!(validated_image_metadata(&png).is_err());
        assert!(validated_image_metadata(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_err());
    }

    #[test]
    fn server_list_appearance_merge_is_total_and_same_origin() {
        let mut servers = json!([{ "id": "helix:server", "name": "Server" }]);
        let appearances = vec![ServerAppearanceSummary {
            server_id: "helix:server".to_owned(),
            revision: 3,
            preset: None,
            content_type: Some("image/png".to_owned()),
            width: Some(128),
            height: Some(128),
            updated_at_unix_ms: 1,
        }];
        attach_appearances(&mut servers, &appearances).expect("attach appearance");
        assert_eq!(
            servers[0]["appearance"]["image_url"],
            "/api/v1/servers/helix:server/appearance/image?revision=3"
        );
        assert!(server_list_contains(&servers, "helix:server"));
        assert!(!server_list_contains(&servers, "amp:missing"));
    }
}
