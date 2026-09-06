use std::{fmt, str::FromStr};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

pub const LOGIN_NAME_MIN_BYTES: usize = 3;
pub const LOGIN_NAME_MAX_BYTES: usize = 64;
pub const DISPLAY_NAME_MAX_CODE_POINTS: usize = 128;
pub const DISPLAY_NAME_MAX_BYTES: usize = 512;

/// Canonical account name used for lookup and uniqueness.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LoginName(String);

impl LoginName {
    pub fn parse(input: &str) -> Result<Self, LoginNameError> {
        if !(LOGIN_NAME_MIN_BYTES..=LOGIN_NAME_MAX_BYTES).contains(&input.len()) {
            return Err(LoginNameError::InvalidLength);
        }

        if !input.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        }) {
            return Err(LoginNameError::InvalidCharacter);
        }

        let first = input.as_bytes()[0];
        let last = input.as_bytes()[input.len() - 1];
        if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
            return Err(LoginNameError::InvalidBoundary);
        }

        Ok(Self(input.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for LoginName {
    type Err = LoginNameError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for LoginName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum LoginNameError {
    #[error("login names must contain between 3 and 64 ASCII bytes")]
    InvalidLength,
    #[error("login names may contain only lowercase ASCII letters, digits, '.', '_', and '-'")]
    InvalidCharacter,
    #[error("login names must begin and end with an ASCII letter or digit")]
    InvalidBoundary,
}

/// User-facing name stored in Unicode NFC.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DisplayName(String);

impl DisplayName {
    pub fn parse(input: &str) -> Result<Self, DisplayNameError> {
        if input.len() > DISPLAY_NAME_MAX_BYTES {
            return Err(DisplayNameError::TooManyBytes);
        }
        let normalized: String = input.nfc().collect();
        let code_points = normalized.chars().count();

        if code_points == 0 || code_points > DISPLAY_NAME_MAX_CODE_POINTS {
            return Err(DisplayNameError::InvalidLength);
        }
        if normalized.len() > DISPLAY_NAME_MAX_BYTES {
            return Err(DisplayNameError::TooManyBytes);
        }
        if normalized.trim() != normalized {
            return Err(DisplayNameError::SurroundingWhitespace);
        }
        if normalized.chars().any(char::is_control) {
            return Err(DisplayNameError::ControlCharacter);
        }

        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DisplayName {
    type Err = DisplayNameError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DisplayNameError {
    #[error("display names must contain between 1 and 128 Unicode code points")]
    InvalidLength,
    #[error("display names must not exceed 512 UTF-8 bytes")]
    TooManyBytes,
    #[error("display names must not begin or end with whitespace")]
    SurroundingWhitespace,
    #[error("display names must not contain control characters")]
    ControlCharacter,
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_normalization::is_nfc;

    #[test]
    fn login_name_accepts_only_the_canonical_ascii_form() {
        let login = LoginName::parse("rique_01").expect("valid login name");
        assert_eq!(login.as_str(), "rique_01");

        assert_eq!(
            LoginName::parse("Rique").expect_err("uppercase is not canonical"),
            LoginNameError::InvalidCharacter
        );
        assert_eq!(
            LoginName::parse("riqué").expect_err("Unicode is not accepted"),
            LoginNameError::InvalidCharacter
        );
        assert_eq!(
            LoginName::parse("_rique").expect_err("separator boundary"),
            LoginNameError::InvalidBoundary
        );
    }

    #[test]
    fn login_name_enforces_length_boundaries() {
        assert!(LoginName::parse("abc").is_ok());
        assert!(LoginName::parse(&"a".repeat(LOGIN_NAME_MAX_BYTES)).is_ok());
        assert_eq!(
            LoginName::parse("ab").expect_err("too short"),
            LoginNameError::InvalidLength
        );
        assert_eq!(
            LoginName::parse(&"a".repeat(LOGIN_NAME_MAX_BYTES + 1)).expect_err("too long"),
            LoginNameError::InvalidLength
        );
    }

    #[test]
    fn display_name_is_normalized_to_nfc() {
        let display = DisplayName::parse("Rique\u{301}").expect("valid display name");
        assert!(is_nfc(display.as_str()));
        assert_eq!(display.as_str(), "Riqu\u{e9}");
    }

    #[test]
    fn display_name_rejects_ambiguous_boundaries_and_controls() {
        assert_eq!(
            DisplayName::parse(" Rique").expect_err("leading space"),
            DisplayNameError::SurroundingWhitespace
        );
        assert_eq!(
            DisplayName::parse("Rique\nAdmin").expect_err("control character"),
            DisplayNameError::ControlCharacter
        );
        assert_eq!(
            DisplayName::parse("").expect_err("empty display name"),
            DisplayNameError::InvalidLength
        );
    }
}
