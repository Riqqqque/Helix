use crate::{DisplayName, LoginName};
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

pub const MIN_PASSWORD_CODE_POINTS: usize = 15;
pub const MAX_PASSWORD_CODE_POINTS: usize = 256;
pub const MAX_PASSWORD_BYTES: usize = 1024;

pub const ARGON2_MEMORY_KIB: u32 = 19 * 1024;
pub const ARGON2_TIME_COST: u32 = 2;
pub const ARGON2_PARALLELISM: u32 = 1;
pub const ARGON2_OUTPUT_BYTES: usize = 32;
pub const PASSWORD_POLICY_VERSION: u32 = 1;

const ARGON2_SALT_BYTES: usize = 16;
const MAX_STORED_PHC_BYTES: usize = 256;
// Keep these verification floors pinned to the oldest supported policy when
// hashing parameters increase, so a successful login can upgrade an older PHC.
const MIN_VERIFY_MEMORY_KIB: u32 = 19 * 1024;
const MIN_VERIFY_TIME_COST: u32 = 2;
const MIN_VERIFY_PARALLELISM: u32 = 1;
const MAX_VERIFY_MEMORY_KIB: u32 = 64 * 1024;
const MAX_VERIFY_TIME_COST: u32 = 8;
const MAX_VERIFY_PARALLELISM: u32 = 4;
const MIN_VERIFY_SALT_BYTES: usize = 16;
const MAX_VERIFY_SALT_BYTES: usize = 32;

// This small built-in set catches obvious defaults even if the maintained
// compromised-password source is unavailable. It is intentionally not
// represented as a complete breach corpus.
const COMMON_PASSWORD_KEYS: &[&str] = &[
    "000000000000000",
    "111111111111111",
    "123456789012345",
    "aaaaaaaaaaaaaaa",
    "adminadminadmin",
    "administrator123",
    "asdfghjklasdfgh",
    "changemechangeme",
    "correcthorsebatterystaple",
    "dragon123456789",
    "helixhelixhelix",
    "iloveyouiloveyou",
    "letmeinletmeinletmein",
    "monkeymonkeymonkey",
    "password123456",
    "passwordpassword",
    "qwerty123456789",
    "qwertyqwertyqwerty",
    "qwertyuiopasdfgh",
    "welcome123456789",
];

/// A validated, NFC-normalized password.
///
/// This type intentionally implements neither `Debug`, `Display`, nor `Clone`.
/// Its allocation is zeroized on drop.
pub struct ValidatedPassword(PasswordInput);

impl ValidatedPassword {
    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// Bounded, NFC-normalized secret input used only to verify an existing hash.
///
/// This deliberately does not apply the prospective-password blocklist. A
/// policy update must not prevent an existing password from authenticating so
/// it can be replaced or rehashed. `hash_password` accepts only
/// `ValidatedPassword`, not this type.
pub struct PasswordInput(Zeroizing<String>);

impl PasswordInput {
    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// An Argon2 PHC string intended for restricted database storage.
///
/// PHC strings are password verifiers and must not enter ordinary logs. This
/// type intentionally implements neither `Debug` nor `Display` and zeroizes its
/// allocation on drop.
pub struct PasswordPhc(Zeroizing<String>);

impl PasswordPhc {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Account-specific terms which must not appear in a prospective password.
pub struct PasswordContext<'a> {
    login_name: &'a LoginName,
    display_name: &'a DisplayName,
    additional_terms: &'a [&'a str],
}

impl<'a> PasswordContext<'a> {
    #[must_use]
    pub const fn new(login_name: &'a LoginName, display_name: &'a DisplayName) -> Self {
        Self {
            login_name,
            display_name,
            additional_terms: &[],
        }
    }

    #[must_use]
    pub const fn with_additional_terms(mut self, terms: &'a [&'a str]) -> Self {
        self.additional_terms = terms;
        self
    }
}

/// Adapter for a maintained compromised-password source.
///
/// The checker receives the NFC-normalized plaintext candidate. Implementations
/// must not log, retain, or transmit it except through a separately reviewed
/// privacy-preserving breach-check protocol.
pub trait CompromisedPasswordChecker {
    fn is_compromised(&self, normalized_password: &str) -> bool;
}

impl<F> CompromisedPasswordChecker for F
where
    F: Fn(&str) -> bool,
{
    fn is_compromised(&self, normalized_password: &str) -> bool {
        self(normalized_password)
    }
}

struct NoAdditionalCompromisedPasswords;

impl CompromisedPasswordChecker for NoAdditionalCompromisedPasswords {
    fn is_compromised(&self, _normalized_password: &str) -> bool {
        false
    }
}

pub fn validate_password(
    candidate: String,
    context: &PasswordContext<'_>,
) -> Result<ValidatedPassword, PasswordValidationError> {
    validate_password_with_checker(candidate, context, &NoAdditionalCompromisedPasswords)
}

pub fn validate_password_with_checker<C>(
    candidate: String,
    context: &PasswordContext<'_>,
    compromised_checker: &C,
) -> Result<ValidatedPassword, PasswordValidationError>
where
    C: CompromisedPasswordChecker + ?Sized,
{
    let normalized = normalize_password_for_verification(candidate)?;
    let code_points = normalized.0.chars().count();

    if code_points < MIN_PASSWORD_CODE_POINTS {
        return Err(PasswordValidationError::TooShort);
    }

    let comparison_key = Zeroizing::new(context_key(normalized.0.as_str()));
    if context_term_matches(comparison_key.as_str(), "helix")
        || context_term_matches(comparison_key.as_str(), context.login_name.as_str())
        || context_term_matches(comparison_key.as_str(), context.display_name.as_str())
        || context
            .additional_terms
            .iter()
            .any(|term| context_term_matches(comparison_key.as_str(), term))
    {
        return Err(PasswordValidationError::ContextSpecific);
    }

    if COMMON_PASSWORD_KEYS.contains(&comparison_key.as_str())
        || compromised_checker.is_compromised(normalized.0.as_str())
    {
        return Err(PasswordValidationError::Compromised);
    }

    Ok(ValidatedPassword(normalized))
}

/// Normalize and bound a password submitted for verification without applying
/// the current prospective-password blocklist or minimum length.
pub fn normalize_password_for_verification(
    candidate: String,
) -> Result<PasswordInput, PasswordInputError> {
    let candidate = Zeroizing::new(candidate);
    if candidate.len() > MAX_PASSWORD_BYTES {
        return Err(PasswordInputError::TooManyBytes);
    }
    let normalized = Zeroizing::new(candidate.as_str().nfc().collect::<String>());
    if normalized.len() > MAX_PASSWORD_BYTES {
        return Err(PasswordInputError::TooManyBytes);
    }
    if normalized.chars().count() > MAX_PASSWORD_CODE_POINTS {
        return Err(PasswordInputError::TooManyCodePoints);
    }

    Ok(PasswordInput(normalized))
}

fn context_key(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn context_term_matches(candidate_key: &str, term: &str) -> bool {
    let term_key = context_key(term);
    if term_key.is_empty() {
        return false;
    }

    if term_key.chars().count() < 4 {
        candidate_key == term_key
    } else {
        candidate_key.contains(&term_key)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PasswordValidationError {
    #[error("passwords must contain at least 15 Unicode code points")]
    TooShort,
    #[error("passwords must not contain account- or installation-specific terms")]
    ContextSpecific,
    #[error("the password appears in the common or compromised password policy")]
    Compromised,
    #[error(transparent)]
    InvalidInput(#[from] PasswordInputError),
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PasswordInputError {
    #[error("passwords must not exceed 256 Unicode code points")]
    TooManyCodePoints,
    #[error("passwords must not exceed 1024 UTF-8 bytes after NFC normalization")]
    TooManyBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PasswordHashParameters {
    pub memory_kib: u32,
    pub time_cost: u32,
    pub parallelism: u32,
    pub output_bytes: usize,
}

pub fn hash_password(password: &ValidatedPassword) -> Result<PasswordPhc, PasswordHashError> {
    hash_normalized_password(password.as_bytes())
}

/// Rehash an already verified password with the current Argon2 policy.
///
/// Callers must invoke this only after `verify_password` returned `Ok(true)` for
/// the same `PasswordInput` and stored PHC. The resulting verifier and policy
/// version should replace the old values with a compare-and-swap update. New
/// account and password-change flows must use `hash_password` with a
/// `ValidatedPassword` so they cannot bypass the prospective-password policy.
pub fn rehash_verified_password(
    verified_password: &PasswordInput,
) -> Result<PasswordPhc, PasswordHashError> {
    hash_normalized_password(verified_password.as_bytes())
}

fn hash_normalized_password(password: &[u8]) -> Result<PasswordPhc, PasswordHashError> {
    let mut salt_bytes = Zeroizing::new([0_u8; ARGON2_SALT_BYTES]);
    getrandom::fill(salt_bytes.as_mut()).map_err(|_| PasswordHashError::RandomSource)?;
    let salt = SaltString::encode_b64(salt_bytes.as_ref())
        .map_err(|_| PasswordHashError::HashingFailed)?;
    let argon2 = policy_argon2()?;
    let phc = argon2
        .hash_password(password, &salt)
        .map_err(|_| PasswordHashError::HashingFailed)?;

    Ok(PasswordPhc(Zeroizing::new(phc.to_string())))
}

pub fn inspect_password_hash(
    stored_phc: &str,
) -> Result<PasswordHashParameters, StoredPasswordHashError> {
    Ok(parse_stored_hash(stored_phc)?.parameters)
}

/// Determine whether an accepted stored verifier differs from the current
/// hashing policy. The same strict parser and resource ceilings used by
/// verification run before any comparison. Call this only after successful
/// password verification, then replace the PHC and policy version with a
/// compare-and-swap update so concurrent logins cannot overwrite newer state.
pub fn password_needs_rehash(
    stored_phc: &str,
    stored_policy_version: u32,
) -> Result<bool, StoredPasswordHashError> {
    let parameters = parse_stored_hash(stored_phc)?.parameters;
    Ok(stored_policy_version != PASSWORD_POLICY_VERSION
        || parameters.memory_kib != ARGON2_MEMORY_KIB
        || parameters.time_cost != ARGON2_TIME_COST
        || parameters.parallelism != ARGON2_PARALLELISM
        || parameters.output_bytes != ARGON2_OUTPUT_BYTES)
}

pub fn verify_password(
    password: &PasswordInput,
    stored_phc: &str,
) -> Result<bool, PasswordHashError> {
    let stored = parse_stored_hash(stored_phc)?;
    match Argon2::default().verify_password(password.as_bytes(), &stored.parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(_) => Err(PasswordHashError::VerificationFailed),
    }
}

fn policy_argon2() -> Result<Argon2<'static>, PasswordHashError> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_TIME_COST,
        ARGON2_PARALLELISM,
        Some(ARGON2_OUTPUT_BYTES),
    )
    .map_err(|_| PasswordHashError::InvalidPolicy)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

struct ValidatedStoredHash<'a> {
    parsed: PasswordHash<'a>,
    parameters: PasswordHashParameters,
}

fn parse_stored_hash(stored_phc: &str) -> Result<ValidatedStoredHash<'_>, StoredPasswordHashError> {
    if stored_phc.len() > MAX_STORED_PHC_BYTES {
        return Err(StoredPasswordHashError::TooLong);
    }

    let parsed = PasswordHash::new(stored_phc).map_err(|_| StoredPasswordHashError::Malformed)?;
    if parsed.algorithm.as_str() != "argon2id" {
        return Err(StoredPasswordHashError::WrongAlgorithm);
    }
    if parsed.version != Some(u32::from(Version::V0x13)) {
        return Err(StoredPasswordHashError::WrongVersion);
    }
    if parsed.params.iter().count() != 3
        || parsed.params.get_decimal("m").is_none()
        || parsed.params.get_decimal("t").is_none()
        || parsed.params.get_decimal("p").is_none()
    {
        return Err(StoredPasswordHashError::UnexpectedParameters);
    }

    let params = Params::try_from(&parsed).map_err(|_| StoredPasswordHashError::Malformed)?;
    if !params.keyid().is_empty() || !params.data().is_empty() {
        return Err(StoredPasswordHashError::UnexpectedParameters);
    }
    if !(MIN_VERIFY_MEMORY_KIB..=MAX_VERIFY_MEMORY_KIB).contains(&params.m_cost()) {
        return Err(StoredPasswordHashError::MemoryCostOutOfRange);
    }
    if !(MIN_VERIFY_TIME_COST..=MAX_VERIFY_TIME_COST).contains(&params.t_cost()) {
        return Err(StoredPasswordHashError::TimeCostOutOfRange);
    }
    if !(MIN_VERIFY_PARALLELISM..=MAX_VERIFY_PARALLELISM).contains(&params.p_cost()) {
        return Err(StoredPasswordHashError::ParallelismOutOfRange);
    }

    let salt = parsed.salt.ok_or(StoredPasswordHashError::MissingSalt)?;
    let mut decoded_salt = [0_u8; 64];
    let decoded_salt = salt
        .decode_b64(&mut decoded_salt)
        .map_err(|_| StoredPasswordHashError::Malformed)?;
    if !(MIN_VERIFY_SALT_BYTES..=MAX_VERIFY_SALT_BYTES).contains(&decoded_salt.len()) {
        return Err(StoredPasswordHashError::WrongSaltLength);
    }

    let output_bytes = parsed
        .hash
        .as_ref()
        .ok_or(StoredPasswordHashError::MissingOutput)?
        .len();
    if output_bytes != ARGON2_OUTPUT_BYTES {
        return Err(StoredPasswordHashError::WrongOutputLength);
    }

    Ok(ValidatedStoredHash {
        parsed,
        parameters: PasswordHashParameters {
            memory_kib: params.m_cost(),
            time_cost: params.t_cost(),
            parallelism: params.p_cost(),
            output_bytes,
        },
    })
}

#[derive(Debug, Error)]
pub enum PasswordHashError {
    #[error("the operating-system random source is unavailable")]
    RandomSource,
    #[error("the Argon2 password policy is invalid")]
    InvalidPolicy,
    #[error("password hashing failed")]
    HashingFailed,
    #[error("stored password hash verification failed")]
    VerificationFailed,
    #[error(transparent)]
    StoredHash(#[from] StoredPasswordHashError),
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum StoredPasswordHashError {
    #[error("stored password PHC string exceeds the parser limit")]
    TooLong,
    #[error("stored password PHC string is malformed")]
    Malformed,
    #[error("stored password hash is not Argon2id")]
    WrongAlgorithm,
    #[error("stored password hash is not Argon2 version 19")]
    WrongVersion,
    #[error("stored password hash has missing or unexpected parameters")]
    UnexpectedParameters,
    #[error("stored password hash memory cost is outside the verification policy")]
    MemoryCostOutOfRange,
    #[error("stored password hash time cost is outside the verification policy")]
    TimeCostOutOfRange,
    #[error("stored password hash parallelism is outside the verification policy")]
    ParallelismOutOfRange,
    #[error("stored password hash has no salt")]
    MissingSalt,
    #[error("stored password hash salt length is outside the verification policy")]
    WrongSaltLength,
    #[error("stored password hash has no output")]
    MissingOutput,
    #[error("stored password hash output must be exactly 32 bytes")]
    WrongOutputLength,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> (LoginName, DisplayName) {
        (
            LoginName::parse("rique01").expect("login name"),
            DisplayName::parse("Rique").expect("display name"),
        )
    }

    fn valid_password(value: &str) -> ValidatedPassword {
        let (login, display) = context();
        validate_password(value.to_owned(), &PasswordContext::new(&login, &display))
            .expect("valid password")
    }

    #[test]
    fn password_length_and_byte_limits_apply_after_nfc_normalization() {
        let (login, display) = context();
        let policy = PasswordContext::new(&login, &display);

        assert!(validate_password("cobalt-sky-927!".to_owned(), &policy).is_ok());
        assert_eq!(
            validate_password("cobalt-sky-927".to_owned(), &policy)
                .err()
                .expect("too short"),
            PasswordValidationError::TooShort
        );
        assert!(validate_password("🦀".repeat(MAX_PASSWORD_CODE_POINTS), &policy).is_ok());
        assert_eq!(
            validate_password("🦀".repeat(MAX_PASSWORD_CODE_POINTS + 1), &policy)
                .err()
                .expect("too many bytes"),
            PasswordValidationError::InvalidInput(PasswordInputError::TooManyBytes)
        );
        assert_eq!(
            validate_password("a".repeat(MAX_PASSWORD_CODE_POINTS + 1), &policy)
                .err()
                .expect("too many code points"),
            PasswordValidationError::InvalidInput(PasswordInputError::TooManyCodePoints)
        );
    }

    #[test]
    fn password_policy_has_no_character_class_rule() {
        let (login, display) = context();
        let policy = PasswordContext::new(&login, &display);

        assert!(validate_password("all lowercase words are valid".to_owned(), &policy).is_ok());
        assert!(validate_password("🦀".repeat(MIN_PASSWORD_CODE_POINTS), &policy).is_ok());
    }

    #[test]
    fn password_is_nfc_normalized() {
        let (login, display) = context();
        let policy = PasswordContext::new(&login, &display);
        let decomposed = "e\u{301}".repeat(MIN_PASSWORD_CODE_POINTS);
        let password = validate_password(decomposed, &policy).expect("normalized password");

        assert_eq!(password.0.0.chars().count(), MIN_PASSWORD_CODE_POINTS);
        assert!(unicode_normalization::is_nfc(password.0.0.as_str()));
    }

    #[test]
    fn context_specific_passwords_are_rejected() {
        let (login, display) = context();
        let extra = ["minecraft"];
        let policy = PasswordContext::new(&login, &display).with_additional_terms(&extra);

        for candidate in [
            "unique-rique01-passphrase",
            "this contains RIQUE inside",
            "helix control panel password",
            "minecraft administration phrase",
        ] {
            assert_eq!(
                validate_password(candidate.to_owned(), &policy)
                    .err()
                    .expect("context term"),
                PasswordValidationError::ContextSpecific
            );
        }
    }

    #[test]
    fn common_and_external_compromised_passwords_are_rejected() {
        let (login, display) = context();
        let policy = PasswordContext::new(&login, &display);

        assert_eq!(
            validate_password("correct horse battery staple".to_owned(), &policy)
                .err()
                .expect("well-known phrase"),
            PasswordValidationError::Compromised
        );
        assert_eq!(
            validate_password("PASSWORD-123456".to_owned(), &policy)
                .err()
                .expect("canonical common password"),
            PasswordValidationError::Compromised
        );
        assert_eq!(
            validate_password("a".repeat(MIN_PASSWORD_CODE_POINTS), &policy)
                .err()
                .expect("obvious repeated password"),
            PasswordValidationError::Compromised
        );
        let checker = |candidate: &str| candidate == "a private breached password";
        assert_eq!(
            validate_password_with_checker(
                "a private breached password".to_owned(),
                &policy,
                &checker,
            )
            .err()
            .expect("external blocklist"),
            PasswordValidationError::Compromised
        );
    }

    #[test]
    fn verification_input_does_not_reapply_prospective_password_policy() {
        assert!(
            normalize_password_for_verification("correct horse battery staple".to_owned()).is_ok()
        );
        assert!(normalize_password_for_verification("short".to_owned()).is_ok());
    }

    #[test]
    fn verified_blocklisted_password_can_be_rehashed_after_successful_login() {
        const BLOCKLISTED: &str = "correct horse battery staple";
        let (login, display) = context();
        let policy = PasswordContext::new(&login, &display);
        assert_eq!(
            validate_password(BLOCKLISTED.to_owned(), &policy)
                .err()
                .expect("new password policy rejects it"),
            PasswordValidationError::Compromised
        );

        let login_input = normalize_password_for_verification(BLOCKLISTED.to_owned())
            .expect("bounded login input");
        let salt = SaltString::encode_b64(&[7_u8; ARGON2_SALT_BYTES]).expect("test salt");
        let existing_phc = Zeroizing::new(
            policy_argon2()
                .expect("policy Argon2")
                .hash_password(login_input.as_bytes(), &salt)
                .expect("existing hash")
                .to_string(),
        );

        assert!(
            verify_password(&login_input, existing_phc.as_str())
                .expect("successful existing-password verification")
        );
        assert!(
            password_needs_rehash(existing_phc.as_str(), PASSWORD_POLICY_VERSION - 1)
                .expect("old policy version")
        );

        let upgraded = rehash_verified_password(&login_input).expect("rehash verified password");
        assert!(verify_password(&login_input, upgraded.as_str()).expect("verify upgraded hash"));
        assert!(
            !password_needs_rehash(upgraded.as_str(), PASSWORD_POLICY_VERSION)
                .expect("current rehash policy")
        );
    }

    #[test]
    fn generated_phc_has_the_exact_argon2id_policy_and_verifies() {
        let password = valid_password("an unrelated strong passphrase");
        let password_input =
            normalize_password_for_verification("an unrelated strong passphrase".to_owned())
                .expect("password input");
        let wrong_password_input =
            normalize_password_for_verification("another unrelated passphrase".to_owned())
                .expect("wrong password input");
        let phc = hash_password(&password).expect("hash password");

        assert!(phc.as_str().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert_eq!(
            inspect_password_hash(phc.as_str()).expect("inspect generated PHC"),
            PasswordHashParameters {
                memory_kib: ARGON2_MEMORY_KIB,
                time_cost: ARGON2_TIME_COST,
                parallelism: ARGON2_PARALLELISM,
                output_bytes: ARGON2_OUTPUT_BYTES,
            }
        );
        assert!(verify_password(&password_input, phc.as_str()).expect("verify correct password"));
        assert!(
            !verify_password(&wrong_password_input, phc.as_str()).expect("reject wrong password")
        );
    }

    #[test]
    fn password_hashes_use_distinct_random_salts() {
        let password = valid_password("an unrelated strong passphrase");
        let first = hash_password(&password).expect("first hash");
        let second = hash_password(&password).expect("second hash");

        assert_ne!(first.as_str(), second.as_str());
    }

    #[test]
    fn rehash_decision_checks_policy_version_and_exact_current_parameters() {
        let password = valid_password("an unrelated strong passphrase");
        let current = hash_password(&password).expect("current hash");
        assert!(
            !password_needs_rehash(current.as_str(), PASSWORD_POLICY_VERSION)
                .expect("current policy")
        );
        assert!(
            password_needs_rehash(current.as_str(), PASSWORD_POLICY_VERSION - 1)
                .expect("old policy version")
        );
        assert!(
            password_needs_rehash(current.as_str(), PASSWORD_POLICY_VERSION + 1)
                .expect("different policy version")
        );

        const SALT: &str = "AAAAAAAAAAAAAAAAAAAAAA";
        const OUTPUT: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let accepted_non_current = format!("$argon2id$v=19$m=24576,t=3,p=2${SALT}${OUTPUT}");
        assert!(
            password_needs_rehash(&accepted_non_current, PASSWORD_POLICY_VERSION)
                .expect("accepted non-current parameters")
        );
    }

    #[test]
    fn rehash_decision_rejects_malformed_and_over_ceiling_phcs() {
        assert_eq!(
            password_needs_rehash("not-a-phc", PASSWORD_POLICY_VERSION).expect_err("malformed PHC"),
            StoredPasswordHashError::Malformed
        );

        const SALT: &str = "AAAAAAAAAAAAAAAAAAAAAA";
        const OUTPUT: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let excessive = format!("$argon2id$v=19$m=1048576,t=2,p=1${SALT}${OUTPUT}");
        assert_eq!(
            password_needs_rehash(&excessive, PASSWORD_POLICY_VERSION).expect_err("excessive PHC"),
            StoredPasswordHashError::MemoryCostOutOfRange
        );
    }

    #[test]
    fn extreme_or_wrong_phc_metadata_is_rejected_before_verification() {
        const SALT: &str = "AAAAAAAAAAAAAAAAAAAAAA";
        const OUTPUT: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

        let cases = [
            (
                format!("$argon2id$v=19$m=1048576,t=2,p=1${SALT}${OUTPUT}"),
                StoredPasswordHashError::MemoryCostOutOfRange,
            ),
            (
                format!("$argon2id$v=19$m=19456,t=99,p=1${SALT}${OUTPUT}"),
                StoredPasswordHashError::TimeCostOutOfRange,
            ),
            (
                format!("$argon2id$v=19$m=19456,t=2,p=99${SALT}${OUTPUT}"),
                StoredPasswordHashError::ParallelismOutOfRange,
            ),
            (
                format!("$argon2i$v=19$m=19456,t=2,p=1${SALT}${OUTPUT}"),
                StoredPasswordHashError::WrongAlgorithm,
            ),
            (
                format!("$argon2id$v=16$m=19456,t=2,p=1${SALT}${OUTPUT}"),
                StoredPasswordHashError::WrongVersion,
            ),
        ];

        for (phc, expected) in cases {
            assert_eq!(
                inspect_password_hash(&phc).expect_err("unsafe PHC must fail"),
                expected
            );
        }
    }

    #[test]
    fn phc_requires_exact_parameters_salt_and_output() {
        const SALT: &str = "AAAAAAAAAAAAAAAAAAAAAA";
        const OUTPUT_16_BYTES: &str = "AAAAAAAAAAAAAAAAAAAAAA";
        const OUTPUT_32_BYTES: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

        assert_eq!(
            inspect_password_hash(&format!(
                "$argon2id$v=19$m=19456,t=2,p=1,data=AA${SALT}${OUTPUT_32_BYTES}"
            ))
            .expect_err("extra parameter"),
            StoredPasswordHashError::UnexpectedParameters
        );
        assert_eq!(
            inspect_password_hash(&format!(
                "$argon2id$v=19$m=19456,t=2,p=1${SALT}${OUTPUT_16_BYTES}"
            ))
            .expect_err("short output"),
            StoredPasswordHashError::WrongOutputLength
        );
        assert_eq!(
            inspect_password_hash(&format!(
                "$argon2id$v=19$m=19456,t=2,p=1$AAAA${OUTPUT_32_BYTES}"
            ))
            .expect_err("short salt"),
            StoredPasswordHashError::WrongSaltLength
        );
    }
}
