//! Length-bounded protocol shared by Helix and the unprivileged PTY daemon.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_TERMINAL_INPUT_BYTES: usize = 32 * 1024;
pub const MIN_TERMINAL_COLUMNS: u16 = 20;
pub const MAX_TERMINAL_COLUMNS: u16 = 400;
pub const MIN_TERMINAL_ROWS: u16 = 5;
pub const MAX_TERMINAL_ROWS: u16 = 200;

pub mod kind {
    pub const CLIENT_OPEN: u8 = 1;
    pub const CLIENT_INPUT: u8 = 2;
    pub const CLIENT_RESIZE: u8 = 3;
    pub const CLIENT_CLOSE: u8 = 4;
    pub const SERVER_READY: u8 = 101;
    pub const SERVER_OUTPUT: u8 = 102;
    pub const SERVER_EXIT: u8 = 103;
    pub const SERVER_ERROR: u8 = 104;
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalDimensions {
    pub columns: u16,
    pub rows: u16,
}

impl TerminalDimensions {
    pub fn validate(self) -> Result<Self, ProtocolError> {
        if !(MIN_TERMINAL_COLUMNS..=MAX_TERMINAL_COLUMNS).contains(&self.columns)
            || !(MIN_TERMINAL_ROWS..=MAX_TERMINAL_ROWS).contains(&self.rows)
        {
            return Err(ProtocolError::InvalidDimensions);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenRequest {
    pub protocol_version: u8,
    pub dimensions: TerminalDimensions,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyResponse {
    pub protocol_version: u8,
    pub user: String,
    pub shell: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExitResponse {
    pub exit_code: u32,
    pub signal: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ProtocolError {
    #[error("terminal frame is empty or exceeds the protocol limit")]
    InvalidFrameLength,
    #[error("terminal frame payload is invalid")]
    InvalidPayload,
    #[error("terminal dimensions are outside the supported range")]
    InvalidDimensions,
}

pub fn encode_frame(kind: u8, payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let frame_length = payload
        .len()
        .checked_add(1)
        .filter(|length| *length <= MAX_FRAME_BYTES)
        .ok_or(ProtocolError::InvalidFrameLength)?;
    let frame_length =
        u32::try_from(frame_length).map_err(|_| ProtocolError::InvalidFrameLength)?;
    let mut encoded = Vec::with_capacity(payload.len().saturating_add(5));
    encoded.extend_from_slice(&frame_length.to_be_bytes());
    encoded.push(kind);
    encoded.extend_from_slice(payload);
    Ok(encoded)
}

pub fn decode_frame_length(header: [u8; 4]) -> Result<usize, ProtocolError> {
    let length = usize::try_from(u32::from_be_bytes(header))
        .map_err(|_| ProtocolError::InvalidFrameLength)?;
    if !(1..=MAX_FRAME_BYTES).contains(&length) {
        return Err(ProtocolError::InvalidFrameLength);
    }
    Ok(length)
}

pub fn encode_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = serde_json::to_vec(value).map_err(|_| ProtocolError::InvalidPayload)?;
    if payload.len().saturating_add(1) > MAX_FRAME_BYTES {
        return Err(ProtocolError::InvalidFrameLength);
    }
    Ok(payload)
}

pub fn decode_json<T: DeserializeOwned>(payload: &[u8]) -> Result<T, ProtocolError> {
    serde_json::from_slice(payload).map_err(|_| ProtocolError::InvalidPayload)
}

pub fn encode_resize(dimensions: TerminalDimensions) -> Result<[u8; 4], ProtocolError> {
    let dimensions = dimensions.validate()?;
    let mut payload = [0_u8; 4];
    payload[..2].copy_from_slice(&dimensions.columns.to_be_bytes());
    payload[2..].copy_from_slice(&dimensions.rows.to_be_bytes());
    Ok(payload)
}

pub fn decode_resize(payload: &[u8]) -> Result<TerminalDimensions, ProtocolError> {
    let [column_high, column_low, row_high, row_low] = payload else {
        return Err(ProtocolError::InvalidPayload);
    };
    TerminalDimensions {
        columns: u16::from_be_bytes([*column_high, *column_low]),
        rows: u16::from_be_bytes([*row_high, *row_low]),
    }
    .validate()
}

/// Interactive login flags for `/bin/bash`, matching a normal SSH tty.
pub const LOGIN_SHELL_ARGS: &[&str] = &["-i", "-l"];
pub const DEFAULT_CHILD_LANG: &str = "C.UTF-8";

/// Prefer the daemon account's `HOME` so `~`, profile, and Tab completion match a real login.
pub fn child_home(daemon_home: Option<&Path>, working_directory: &Path) -> PathBuf {
    match daemon_home {
        Some(home) if is_usable_home(home) => home.to_path_buf(),
        _ => working_directory.to_path_buf(),
    }
}

fn is_usable_home(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return false;
    }
    path.is_absolute() || path.has_root()
}

/// Copy a host `LANG` only when it is a UTF-8 locale Helix can safely set.
pub fn child_lang(host_lang: Option<&str>) -> String {
    match host_lang {
        Some(value) if is_safe_utf8_locale(value) => value.to_owned(),
        _ => DEFAULT_CHILD_LANG.to_owned(),
    }
}

fn is_safe_utf8_locale(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 || !bytes.is_ascii() {
        return false;
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'@'))
    {
        return false;
    }
    matches!(
        value.rsplit_once('.').map(|(_, encoding)| encoding),
        Some("UTF-8" | "utf-8" | "UTF8" | "utf8")
    )
}

/// Copy `TZ` when it looks like an IANA name or POSIX offset, never a path.
pub fn child_tz(host_tz: Option<&str>) -> Option<&str> {
    let value = host_tz?;
    if value.is_empty() || value.len() > 64 || !value.is_ascii() {
        return None;
    }
    if value.starts_with('/') || value.contains("..") {
        return None;
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'/' | b'+'))
    {
        return None;
    }
    Some(value)
}

/// Accept only the conventional `/run/user/<uid>` runtime dir for this account.
pub fn child_xdg_runtime_dir(uid: u32, host_value: Option<&str>) -> Option<String> {
    if uid == 0 {
        return None;
    }
    let expected = format!("/run/user/{uid}");
    match host_value {
        Some(value) if value == expected => Some(expected),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_big_endian_and_length_bounded() {
        assert_eq!(
            encode_frame(kind::CLIENT_INPUT, b"ls\r").unwrap(),
            [0, 0, 0, 4, 2, b'l', b's', b'\r']
        );
        assert_eq!(decode_frame_length([0, 0, 0, 4]).unwrap(), 4);
        assert_eq!(
            decode_frame_length([0, 0, 0, 0]),
            Err(ProtocolError::InvalidFrameLength)
        );
        assert_eq!(
            encode_frame(kind::CLIENT_INPUT, &vec![0; MAX_FRAME_BYTES]),
            Err(ProtocolError::InvalidFrameLength)
        );
    }

    #[test]
    fn dimensions_round_trip_and_reject_extremes() {
        let dimensions = TerminalDimensions {
            columns: 132,
            rows: 42,
        };
        assert_eq!(
            decode_resize(&encode_resize(dimensions).unwrap()).unwrap(),
            dimensions
        );
        assert_eq!(
            TerminalDimensions {
                columns: 0,
                rows: 42
            }
            .validate(),
            Err(ProtocolError::InvalidDimensions)
        );
    }

    #[test]
    fn open_payload_rejects_unknown_fields() {
        let invalid =
            br#"{"protocol_version":1,"dimensions":{"columns":80,"rows":24},"command":"id"}"#;
        assert_eq!(
            decode_json::<OpenRequest>(invalid),
            Err(ProtocolError::InvalidPayload)
        );
    }

    #[test]
    fn login_shell_is_interactive_and_login() {
        assert_eq!(LOGIN_SHELL_ARGS, ["-i", "-l"]);
    }

    #[test]
    fn child_home_prefers_absolute_daemon_home() {
        let working = Path::new("/var/tmp/helix-cwd");
        assert_eq!(
            child_home(Some(Path::new("/home/rique")), working),
            PathBuf::from("/home/rique")
        );
        assert_eq!(
            child_home(Some(Path::new("relative-home")), working),
            working.to_path_buf()
        );
        assert_eq!(child_home(None, working), working.to_path_buf());
        assert_eq!(
            child_home(Some(Path::new("/tmp/../etc")), working),
            working.to_path_buf()
        );
    }

    #[test]
    fn child_lang_allows_utf8_locales_only() {
        assert_eq!(child_lang(Some("en_US.UTF-8")), "en_US.UTF-8");
        assert_eq!(child_lang(Some("C.utf8")), "C.utf8");
        assert_eq!(child_lang(Some("C")), DEFAULT_CHILD_LANG);
        assert_eq!(child_lang(Some("en_US.ISO-8859-1")), DEFAULT_CHILD_LANG);
        assert_eq!(
            child_lang(Some("en_US.UTF-8/../../etc/passwd")),
            DEFAULT_CHILD_LANG
        );
        assert_eq!(child_lang(None), DEFAULT_CHILD_LANG);
    }

    #[test]
    fn child_tz_rejects_paths_and_junk() {
        assert_eq!(child_tz(Some("America/Denver")), Some("America/Denver"));
        assert_eq!(child_tz(Some("UTC")), Some("UTC"));
        assert_eq!(child_tz(Some("GMT+6")), Some("GMT+6"));
        assert_eq!(child_tz(Some("/etc/localtime")), None);
        assert_eq!(child_tz(Some("../zoneinfo/UTC")), None);
        assert_eq!(child_tz(Some("UTC;id")), None);
        assert_eq!(child_tz(None), None);
    }

    #[test]
    fn child_xdg_runtime_dir_is_uid_specific() {
        assert_eq!(
            child_xdg_runtime_dir(1_001, Some("/run/user/1001")).as_deref(),
            Some("/run/user/1001")
        );
        assert_eq!(child_xdg_runtime_dir(1_001, Some("/run/user/0")), None);
        assert_eq!(child_xdg_runtime_dir(0, Some("/run/user/0")), None);
        assert_eq!(child_xdg_runtime_dir(1_001, None), None);
    }
}
