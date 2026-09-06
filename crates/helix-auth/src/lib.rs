//! Synchronous identity, password, and opaque-token primitives for Helix.
//!
//! This crate deliberately has no HTTP, database, or async-runtime dependency.
//! Callers are responsible for running password work on a bounded blocking
//! executor and for mapping detailed validation errors to safe API responses.

mod identity;
mod password;
mod token;

pub use identity::{
    DISPLAY_NAME_MAX_BYTES, DISPLAY_NAME_MAX_CODE_POINTS, DisplayName, DisplayNameError,
    LOGIN_NAME_MAX_BYTES, LOGIN_NAME_MIN_BYTES, LoginName, LoginNameError,
};
pub use password::{
    ARGON2_MEMORY_KIB, ARGON2_OUTPUT_BYTES, ARGON2_PARALLELISM, ARGON2_TIME_COST,
    CompromisedPasswordChecker, MAX_PASSWORD_BYTES, MAX_PASSWORD_CODE_POINTS,
    MIN_PASSWORD_CODE_POINTS, PASSWORD_POLICY_VERSION, PasswordContext, PasswordHashError,
    PasswordHashParameters, PasswordInput, PasswordInputError, PasswordPhc,
    PasswordValidationError, StoredPasswordHashError, ValidatedPassword, hash_password,
    inspect_password_hash, normalize_password_for_verification, password_needs_rehash,
    rehash_verified_password, validate_password, validate_password_with_checker,
    validate_verified_password_for_context, verify_password,
};
pub use token::{
    EncodedToken, OPAQUE_TOKEN_BYTES, OPAQUE_TOKEN_ENCODED_LEN, OpaqueToken, TokenDomain,
    TokenError, TokenHash,
};
