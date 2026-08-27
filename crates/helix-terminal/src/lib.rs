//! Length-bounded protocol shared by Helix and the unprivileged PTY daemon.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
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
}
