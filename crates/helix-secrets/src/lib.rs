//! Authenticated envelope encryption for recoverable Helix secrets.
//!
//! This crate deliberately owns only the portable cryptographic boundary. The
//! caller must acquire the encoded master-key credential through a platform
//! trust mechanism and pass it in memory; credentials are never read from
//! environment variables, CLI arguments, or Helix configuration here.

use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce, aead::AeadInOut};
use helix_state::{
    EncryptedSecretWrite, InstallMasterKeyInput, InstallMasterKeyOutcome, MasterKeyRecord,
    SecretRecordMetadata, StateDatabase, StateError, StoredSecretRecord,
};
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox};
use std::fmt;
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

pub const MAX_SECRET_PLAINTEXT_BYTES: usize = 65_536;

const ALGORITHM: &str = "xchacha20poly1305";
const ENVELOPE_FORMAT_VERSION: i64 = 1;
const CREDENTIAL_MAGIC: &[u8; 8] = b"HLXKEY01";
const CREDENTIAL_ENCODED_LEN: usize = 8 + 16 + 16 + 4 + 32;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const MASTER_KEY_CHECK: &[u8] = b"helix-master-key-check-v1";
const AAD_DOMAIN: &[u8] = b"HELIX-AAD";
const AAD_FORMAT_VERSION: u8 = 1;
const AAD_KIND_MASTER_CHECK: u8 = 1;
const AAD_KIND_RECORD_DATA: u8 = 2;
const AAD_KIND_DEK_WRAP: u8 = 3;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error("the operating system random source failed")]
    RandomSource,
    #[error("invalid encoded master-key credential: {0}")]
    InvalidCredential(&'static str),
    #[error("invalid secret identity: {0}")]
    InvalidIdentity(&'static str),
    #[error("the master-key credential belongs to a different Helix installation")]
    InstallationMismatch,
    #[error("the master-key credential was rejected")]
    MasterKeyRejected,
    #[error("the state database has master-key history but no active key")]
    MissingActiveMasterKey,
    #[error("the active master key changed after the secret store was opened")]
    MasterKeyLifecycleChanged,
    #[error("unsupported or malformed encrypted record: {0}")]
    InvalidEnvelope(&'static str),
    #[error("secret record authentication failed")]
    RecordAuthenticationFailed,
    #[error("secret plaintext must contain between 1 and 65536 bytes")]
    InvalidPlaintextLength,
}

/// A plaintext value which is redacted in diagnostics and zeroized on drop.
///
/// The type intentionally has no `Clone`, `Display`, or serialization support.
pub struct SecretValue(SecretBox<Vec<u8>>);

impl SecretValue {
    #[must_use]
    pub fn new(value: Vec<u8>) -> Self {
        Self(SecretBox::new(Box::new(value)))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.expose_secret().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.expose_secret().is_empty()
    }

    pub fn with_secret<T>(&self, use_secret: impl FnOnce(&[u8]) -> T) -> T {
        use_secret(self.0.expose_secret())
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

/// The portable, versioned credential supplied by a platform trust boundary.
///
/// The encoded form is `HLXKEY01 || installation UUID || key UUID || key
/// version (u32 BE) || 32-byte key`. It is intended to be carried as opaque
/// bytes by a credential manager, not stored beside the database.
pub struct MasterKeyCredential {
    installation_id: Uuid,
    key_id: Uuid,
    key_version: u32,
    key: SecretBox<[u8; KEY_LEN]>,
}

impl MasterKeyCredential {
    pub fn generate(installation_id: Uuid, key_version: u32) -> Result<Self, SecretError> {
        if installation_id.is_nil() {
            return Err(SecretError::InvalidCredential(
                "installation identifier must not be nil",
            ));
        }
        if key_version == 0 {
            return Err(SecretError::InvalidCredential(
                "key version must be positive",
            ));
        }

        let mut key = SecretBox::<[u8; KEY_LEN]>::default();
        getrandom::fill(key.expose_secret_mut()).map_err(|_| SecretError::RandomSource)?;
        if key.expose_secret().iter().all(|byte| *byte == 0) {
            return Err(SecretError::RandomSource);
        }
        Ok(Self {
            installation_id,
            key_id: random_uuid_v4()?,
            key_version,
            key,
        })
    }

    pub fn decode(encoded: SecretValue) -> Result<Self, SecretError> {
        let mut key = SecretBox::<[u8; KEY_LEN]>::default();
        let (installation_id, key_id, key_version) = encoded.with_secret(|bytes| {
            if bytes.len() != CREDENTIAL_ENCODED_LEN {
                return Err(SecretError::InvalidCredential(
                    "unexpected credential length",
                ));
            }
            if &bytes[..CREDENTIAL_MAGIC.len()] != CREDENTIAL_MAGIC {
                return Err(SecretError::InvalidCredential(
                    "unsupported credential format",
                ));
            }

            let installation_id = Uuid::from_slice(&bytes[8..24])
                .map_err(|_| SecretError::InvalidCredential("invalid installation identifier"))?;
            let key_id = Uuid::from_slice(&bytes[24..40])
                .map_err(|_| SecretError::InvalidCredential("invalid key identifier"))?;
            let key_version = u32::from_be_bytes(
                bytes[40..44]
                    .try_into()
                    .map_err(|_| SecretError::InvalidCredential("invalid key version"))?,
            );
            if installation_id.is_nil() || key_id.is_nil() || key_version == 0 {
                return Err(SecretError::InvalidCredential(
                    "identifiers and key version must be non-zero",
                ));
            }
            if bytes[44..76].iter().all(|byte| *byte == 0) {
                return Err(SecretError::InvalidCredential(
                    "master key must not be all zeroes",
                ));
            }
            key.expose_secret_mut().copy_from_slice(&bytes[44..76]);
            Ok((installation_id, key_id, key_version))
        })?;

        Ok(Self {
            installation_id,
            key_id,
            key_version,
            key,
        })
    }

    #[must_use]
    pub fn encode(&self) -> SecretValue {
        let mut encoded = Vec::with_capacity(CREDENTIAL_ENCODED_LEN);
        encoded.extend_from_slice(CREDENTIAL_MAGIC);
        encoded.extend_from_slice(self.installation_id.as_bytes());
        encoded.extend_from_slice(self.key_id.as_bytes());
        encoded.extend_from_slice(&self.key_version.to_be_bytes());
        encoded.extend_from_slice(self.key.expose_secret());
        SecretValue::new(encoded)
    }

    #[must_use]
    pub fn installation_id(&self) -> Uuid {
        self.installation_id
    }

    #[must_use]
    pub fn key_id(&self) -> Uuid {
        self.key_id
    }

    #[must_use]
    pub fn key_version(&self) -> u32 {
        self.key_version
    }
}

impl fmt::Debug for MasterKeyCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MasterKeyCredential")
            .field("installation_id", &self.installation_id)
            .field("key_id", &self.key_id)
            .field("key_version", &self.key_version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretIdentity {
    secret_type: String,
    scope_type: String,
    scope_id: Uuid,
    purpose: String,
}

impl SecretIdentity {
    pub fn new(
        secret_type: impl Into<String>,
        scope_type: impl Into<String>,
        scope_id: Uuid,
        purpose: impl Into<String>,
    ) -> Result<Self, SecretError> {
        let identity = Self {
            secret_type: secret_type.into(),
            scope_type: scope_type.into(),
            scope_id,
            purpose: purpose.into(),
        };
        identity.validate()?;
        Ok(identity)
    }

    #[must_use]
    pub fn secret_type(&self) -> &str {
        &self.secret_type
    }

    #[must_use]
    pub fn scope_type(&self) -> &str {
        &self.scope_type
    }

    #[must_use]
    pub fn scope_id(&self) -> Uuid {
        self.scope_id
    }

    #[must_use]
    pub fn purpose(&self) -> &str {
        &self.purpose
    }

    fn validate(&self) -> Result<(), SecretError> {
        validate_identifier("secret type", &self.secret_type, 64)?;
        validate_identifier("scope type", &self.scope_type, 32)?;
        if self.scope_id.is_nil() {
            return Err(SecretError::InvalidIdentity("scope identifier is nil"));
        }
        validate_identifier("purpose", &self.purpose, 128)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretMetadata {
    pub id: Uuid,
    pub identity: SecretIdentity,
    pub revision: u64,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

/// A store bound to one validated master-key credential.
///
/// Plaintext retrieval runs inside a callback while this crate owns and
/// zeroizes the decrypted buffer. The generic callback result is intentionally
/// useful, but a caller can explicitly copy or serialize bytes and remains
/// responsible for any such value.
pub struct SecretStore<'a> {
    state: &'a StateDatabase,
    installation_id: Uuid,
    credential: MasterKeyCredential,
}

impl<'a> SecretStore<'a> {
    pub fn open(
        state: &'a StateDatabase,
        credential: MasterKeyCredential,
    ) -> Result<Self, SecretError> {
        let installation_id = parse_canonical_uuid(
            state.installation_id(),
            SecretError::InvalidEnvelope("invalid installation identifier in state"),
        )?;
        if credential.installation_id != installation_id {
            return Err(SecretError::InstallationMismatch);
        }

        let store = Self {
            state,
            installation_id,
            credential,
        };
        store.initialize_or_verify_master_key()?;
        Ok(store)
    }

    pub fn put(
        &self,
        identity: &SecretIdentity,
        plaintext: SecretValue,
    ) -> Result<SecretMetadata, SecretError> {
        identity.validate()?;
        validate_plaintext(&plaintext)?;
        self.require_credential_is_active()?;

        let id = random_uuid_v4()?;
        let encrypted = self.encrypt_record(id, identity, 1, 1, &plaintext)?;
        let id_string = id.hyphenated().to_string();
        let scope_id = identity.scope_id().hyphenated().to_string();
        let master_key_id = self.credential.key_id.hyphenated().to_string();
        let metadata = self.state.insert_secret_record(EncryptedSecretWrite {
            id: &id_string,
            secret_type: identity.secret_type(),
            scope_type: identity.scope_type(),
            scope_id: &scope_id,
            purpose: identity.purpose(),
            revision: 1,
            wrap_revision: 1,
            master_key_id: &master_key_id,
            algorithm: ALGORITHM,
            format_version: ENVELOPE_FORMAT_VERSION,
            data_nonce: &encrypted.data_nonce,
            ciphertext: &encrypted.ciphertext,
            dek_wrap_nonce: &encrypted.dek_wrap_nonce,
            wrapped_dek: &encrypted.wrapped_dek,
        })?;
        metadata_from_state(metadata)
    }

    pub fn metadata(&self, id: Uuid) -> Result<SecretMetadata, SecretError> {
        let record = self
            .state
            .load_secret_record(&id.hyphenated().to_string())?
            .ok_or(StateError::SecretNotFound)?;
        validate_record(&record)?;
        metadata_from_record(&record)
    }

    pub fn with_secret<T>(
        &self,
        id: Uuid,
        use_secret: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, SecretError> {
        self.require_credential_is_active()?;
        let record = self
            .state
            .load_secret_record(&id.hyphenated().to_string())?
            .ok_or(StateError::SecretNotFound)?;
        let context = validate_record(&record)?;
        if context.master_key_id != self.credential.key_id {
            return Err(SecretError::MasterKeyRejected);
        }

        let wrap_aad = dek_wrap_aad(self.installation_id, &context, self.credential.key_version);
        let unwrapped = decrypt_to_zeroizing(
            self.credential.key.expose_secret(),
            &context.dek_wrap_nonce,
            &wrap_aad,
            &record.wrapped_dek,
        )
        .map_err(|_| SecretError::RecordAuthenticationFailed)?;
        if unwrapped.len() != KEY_LEN {
            return Err(SecretError::InvalidEnvelope("unwrapped DEK length"));
        }
        let mut dek = SecretBox::<[u8; KEY_LEN]>::default();
        dek.expose_secret_mut().copy_from_slice(&unwrapped);

        let data_aad = record_data_aad(self.installation_id, &context);
        let plaintext = decrypt_to_zeroizing(
            dek.expose_secret(),
            &context.data_nonce,
            &data_aad,
            &record.ciphertext,
        )
        .map_err(|_| SecretError::RecordAuthenticationFailed)?;
        if plaintext.is_empty() || plaintext.len() > MAX_SECRET_PLAINTEXT_BYTES {
            return Err(SecretError::InvalidEnvelope("plaintext length"));
        }
        Ok(use_secret(&plaintext))
    }

    pub fn replace(
        &self,
        id: Uuid,
        expected_revision: u64,
        plaintext: SecretValue,
    ) -> Result<SecretMetadata, SecretError> {
        validate_plaintext(&plaintext)?;
        let expected_revision_i64 = i64_from_revision(expected_revision)?;
        self.require_credential_is_active()?;
        let current = self
            .state
            .load_secret_record(&id.hyphenated().to_string())?
            .ok_or(StateError::SecretNotFound)?;
        let context = validate_record(&current)?;
        if context.revision != expected_revision {
            return Err(StateError::SecretRevisionConflict {
                expected: expected_revision_i64,
                actual: current.revision,
            }
            .into());
        }

        let revision = expected_revision
            .checked_add(1)
            .ok_or(SecretError::InvalidEnvelope("record revision overflow"))?;
        let wrap_revision = context
            .wrap_revision
            .checked_add(1)
            .ok_or(SecretError::InvalidEnvelope("wrap revision overflow"))?;
        let encrypted =
            self.encrypt_record(id, &context.identity, revision, wrap_revision, &plaintext)?;
        let id_string = id.hyphenated().to_string();
        let scope_id = context.identity.scope_id().hyphenated().to_string();
        let master_key_id = self.credential.key_id.hyphenated().to_string();
        let metadata = self.state.replace_secret_record(
            expected_revision_i64,
            EncryptedSecretWrite {
                id: &id_string,
                secret_type: context.identity.secret_type(),
                scope_type: context.identity.scope_type(),
                scope_id: &scope_id,
                purpose: context.identity.purpose(),
                revision: i64_from_revision(revision)?,
                wrap_revision: i64_from_revision(wrap_revision)?,
                master_key_id: &master_key_id,
                algorithm: ALGORITHM,
                format_version: ENVELOPE_FORMAT_VERSION,
                data_nonce: &encrypted.data_nonce,
                ciphertext: &encrypted.ciphertext,
                dek_wrap_nonce: &encrypted.dek_wrap_nonce,
                wrapped_dek: &encrypted.wrapped_dek,
            },
        )?;
        metadata_from_state(metadata)
    }

    pub fn delete(&self, id: Uuid, expected_revision: u64) -> Result<SecretMetadata, SecretError> {
        self.require_credential_is_active()?;
        let deleted = self.state.delete_secret_record(
            &id.hyphenated().to_string(),
            i64_from_revision(expected_revision)?,
        )?;
        metadata_from_state(deleted)
    }

    fn initialize_or_verify_master_key(&self) -> Result<(), SecretError> {
        if let Some(active) = self.state.active_master_key_record()? {
            return self.verify_master_key_record(&active);
        }
        if self.state.master_key_record_count()? != 0 {
            return Err(SecretError::MissingActiveMasterKey);
        }

        let check_nonce = random_array::<NONCE_LEN>()?;
        let check_aad = master_check_aad(
            self.installation_id,
            self.credential.key_id,
            self.credential.key_version,
        );
        let check_ciphertext = encrypt_to_vec(
            self.credential.key.expose_secret(),
            &check_nonce,
            &check_aad,
            MASTER_KEY_CHECK,
        )?;
        let master_key_id = self.credential.key_id.hyphenated().to_string();
        let outcome = self
            .state
            .install_initial_master_key(InstallMasterKeyInput {
                id: &master_key_id,
                key_version: i64::from(self.credential.key_version),
                algorithm: ALGORITHM,
                format_version: ENVELOPE_FORMAT_VERSION,
                check_nonce: &check_nonce,
                check_ciphertext: &check_ciphertext,
            })?;
        if outcome == InstallMasterKeyOutcome::Installed {
            return Ok(());
        }

        let active = self
            .state
            .active_master_key_record()?
            .ok_or(SecretError::MissingActiveMasterKey)?;
        self.verify_master_key_record(&active)
    }

    fn verify_master_key_record(&self, record: &MasterKeyRecord) -> Result<(), SecretError> {
        let record_id = parse_canonical_uuid(
            &record.id,
            SecretError::InvalidEnvelope("invalid master-key identifier"),
        )?;
        let record_version = u32::try_from(record.key_version)
            .map_err(|_| SecretError::InvalidEnvelope("invalid master-key version"))?;
        if record.algorithm != ALGORITHM || record.format_version != ENVELOPE_FORMAT_VERSION {
            return Err(SecretError::InvalidEnvelope("master-key envelope format"));
        }
        if record_id != self.credential.key_id || record_version != self.credential.key_version {
            return Err(SecretError::MasterKeyRejected);
        }
        let nonce = nonce_from_slice(&record.check_nonce, "master-key check nonce")?;
        if record.check_ciphertext.len() != MASTER_KEY_CHECK.len() + TAG_LEN {
            return Err(SecretError::InvalidEnvelope(
                "master-key check ciphertext length",
            ));
        }
        let aad = master_check_aad(self.installation_id, record_id, record_version);
        let check = decrypt_to_zeroizing(
            self.credential.key.expose_secret(),
            &nonce,
            &aad,
            &record.check_ciphertext,
        )
        .map_err(|_| SecretError::MasterKeyRejected)?;
        if check.as_slice() != MASTER_KEY_CHECK {
            return Err(SecretError::MasterKeyRejected);
        }
        Ok(())
    }

    fn require_credential_is_active(&self) -> Result<(), SecretError> {
        let active = self
            .state
            .active_master_key_record()?
            .ok_or(SecretError::MasterKeyLifecycleChanged)?;
        let id = Uuid::parse_str(&active.id)
            .map_err(|_| SecretError::InvalidEnvelope("invalid active master-key identifier"))?;
        let version = u32::try_from(active.key_version)
            .map_err(|_| SecretError::InvalidEnvelope("invalid active master-key version"))?;
        if id == self.credential.key_id
            && version == self.credential.key_version
            && active.algorithm == ALGORITHM
            && active.format_version == ENVELOPE_FORMAT_VERSION
        {
            Ok(())
        } else {
            Err(SecretError::MasterKeyLifecycleChanged)
        }
    }

    fn encrypt_record(
        &self,
        id: Uuid,
        identity: &SecretIdentity,
        revision: u64,
        wrap_revision: u64,
        plaintext: &SecretValue,
    ) -> Result<EncryptedRecord, SecretError> {
        let mut dek = SecretBox::<[u8; KEY_LEN]>::default();
        getrandom::fill(dek.expose_secret_mut()).map_err(|_| SecretError::RandomSource)?;
        let data_nonce = random_array::<NONCE_LEN>()?;
        let dek_wrap_nonce = random_array::<NONCE_LEN>()?;
        let context = RecordContext {
            id,
            identity: identity.clone(),
            revision,
            wrap_revision,
            master_key_id: self.credential.key_id,
            data_nonce,
            dek_wrap_nonce,
        };
        let data_aad = record_data_aad(self.installation_id, &context);
        let ciphertext = plaintext.with_secret(|value| {
            encrypt_to_vec(dek.expose_secret(), &data_nonce, &data_aad, value)
        })?;
        let wrap_aad = dek_wrap_aad(self.installation_id, &context, self.credential.key_version);
        let wrapped_dek = encrypt_to_vec(
            self.credential.key.expose_secret(),
            &dek_wrap_nonce,
            &wrap_aad,
            dek.expose_secret(),
        )?;
        Ok(EncryptedRecord {
            data_nonce,
            ciphertext,
            dek_wrap_nonce,
            wrapped_dek,
        })
    }
}

struct EncryptedRecord {
    data_nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
    dek_wrap_nonce: [u8; NONCE_LEN],
    wrapped_dek: Vec<u8>,
}

struct RecordContext {
    id: Uuid,
    identity: SecretIdentity,
    revision: u64,
    wrap_revision: u64,
    master_key_id: Uuid,
    data_nonce: [u8; NONCE_LEN],
    dek_wrap_nonce: [u8; NONCE_LEN],
}

fn validate_record(record: &StoredSecretRecord) -> Result<RecordContext, SecretError> {
    let id = parse_canonical_uuid(
        &record.id,
        SecretError::InvalidEnvelope("invalid record identifier"),
    )?;
    let scope_id = parse_canonical_uuid(
        &record.scope_id,
        SecretError::InvalidEnvelope("invalid scope identifier"),
    )?;
    let master_key_id = parse_canonical_uuid(
        &record.master_key_id,
        SecretError::InvalidEnvelope("invalid record master-key identifier"),
    )?;
    let identity = SecretIdentity::new(
        record.secret_type.clone(),
        record.scope_type.clone(),
        scope_id,
        record.purpose.clone(),
    )
    .map_err(|_| SecretError::InvalidEnvelope("invalid record identity"))?;
    let revision = revision_from_i64(record.revision, "record revision")?;
    let wrap_revision = revision_from_i64(record.wrap_revision, "wrap revision")?;
    if record.algorithm != ALGORITHM || record.format_version != ENVELOPE_FORMAT_VERSION {
        return Err(SecretError::InvalidEnvelope("record envelope format"));
    }
    if !(1 + TAG_LEN..=MAX_SECRET_PLAINTEXT_BYTES + TAG_LEN).contains(&record.ciphertext.len()) {
        return Err(SecretError::InvalidEnvelope("record ciphertext length"));
    }
    if record.wrapped_dek.len() != KEY_LEN + TAG_LEN {
        return Err(SecretError::InvalidEnvelope("wrapped DEK length"));
    }
    let data_nonce = nonce_from_slice(&record.data_nonce, "record data nonce")?;
    let dek_wrap_nonce = nonce_from_slice(&record.dek_wrap_nonce, "record DEK-wrap nonce")?;
    Ok(RecordContext {
        id,
        identity,
        revision,
        wrap_revision,
        master_key_id,
        data_nonce,
        dek_wrap_nonce,
    })
}

fn metadata_from_state(metadata: SecretRecordMetadata) -> Result<SecretMetadata, SecretError> {
    let id = parse_canonical_uuid(
        &metadata.id,
        SecretError::InvalidEnvelope("invalid record identifier"),
    )?;
    let scope_id = parse_canonical_uuid(
        &metadata.scope_id,
        SecretError::InvalidEnvelope("invalid scope identifier"),
    )?;
    let identity = SecretIdentity::new(
        metadata.secret_type,
        metadata.scope_type,
        scope_id,
        metadata.purpose,
    )
    .map_err(|_| SecretError::InvalidEnvelope("invalid record identity"))?;
    Ok(SecretMetadata {
        id,
        identity,
        revision: revision_from_i64(metadata.revision, "record revision")?,
        created_at_unix_ms: metadata.created_at_unix_ms,
        updated_at_unix_ms: metadata.updated_at_unix_ms,
    })
}

fn metadata_from_record(record: &StoredSecretRecord) -> Result<SecretMetadata, SecretError> {
    metadata_from_state(record.metadata())
}

fn validate_identifier(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), SecretError> {
    if !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        Ok(())
    } else {
        Err(SecretError::InvalidIdentity(field))
    }
}

fn validate_plaintext(plaintext: &SecretValue) -> Result<(), SecretError> {
    if plaintext.is_empty() || plaintext.len() > MAX_SECRET_PLAINTEXT_BYTES {
        Err(SecretError::InvalidPlaintextLength)
    } else {
        Ok(())
    }
}

fn random_array<const N: usize>() -> Result<[u8; N], SecretError> {
    let mut value = [0_u8; N];
    getrandom::fill(&mut value).map_err(|_| SecretError::RandomSource)?;
    Ok(value)
}

fn random_uuid_v4() -> Result<Uuid, SecretError> {
    let mut bytes = random_array::<16>()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

fn encrypt_to_vec(
    key: &[u8],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, SecretError> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| SecretError::InvalidEnvelope("encryption key length"))?;
    let nonce = XNonce::from(*nonce);
    let mut buffer = Zeroizing::new(Vec::with_capacity(plaintext.len() + TAG_LEN));
    buffer.extend_from_slice(plaintext);
    cipher
        .encrypt_in_place(&nonce, aad, &mut *buffer)
        .map_err(|_| SecretError::InvalidEnvelope("AEAD encryption failed"))?;
    Ok(std::mem::take(&mut *buffer))
}

fn decrypt_to_zeroizing(
    key: &[u8],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, SecretError> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| SecretError::InvalidEnvelope("decryption key length"))?;
    let nonce = XNonce::from(*nonce);
    let mut buffer = Zeroizing::new(Vec::with_capacity(ciphertext.len()));
    buffer.extend_from_slice(ciphertext);
    cipher
        .decrypt_in_place(&nonce, aad, &mut *buffer)
        .map_err(|_| SecretError::RecordAuthenticationFailed)?;
    Ok(buffer)
}

fn master_check_aad(installation_id: Uuid, key_id: Uuid, key_version: u32) -> Vec<u8> {
    let mut aad = aad_prefix(AAD_KIND_MASTER_CHECK);
    aad.extend_from_slice(installation_id.as_bytes());
    aad.extend_from_slice(key_id.as_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    aad
}

fn record_data_aad(installation_id: Uuid, context: &RecordContext) -> Vec<u8> {
    let mut aad = aad_prefix(AAD_KIND_RECORD_DATA);
    append_record_identity(&mut aad, installation_id, context);
    aad.extend_from_slice(&context.revision.to_be_bytes());
    aad
}

fn dek_wrap_aad(installation_id: Uuid, context: &RecordContext, key_version: u32) -> Vec<u8> {
    let mut aad = aad_prefix(AAD_KIND_DEK_WRAP);
    append_record_identity(&mut aad, installation_id, context);
    aad.extend_from_slice(&context.wrap_revision.to_be_bytes());
    aad.extend_from_slice(context.master_key_id.as_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    aad
}

fn aad_prefix(kind: u8) -> Vec<u8> {
    let mut aad = Vec::with_capacity(128);
    aad.extend_from_slice(AAD_DOMAIN);
    aad.push(AAD_FORMAT_VERSION);
    aad.push(kind);
    aad
}

fn append_record_identity(aad: &mut Vec<u8>, installation_id: Uuid, context: &RecordContext) {
    aad.extend_from_slice(installation_id.as_bytes());
    aad.extend_from_slice(context.id.as_bytes());
    append_field(aad, context.identity.secret_type());
    append_field(aad, context.identity.scope_type());
    aad.extend_from_slice(context.identity.scope_id().as_bytes());
    append_field(aad, context.identity.purpose());
}

fn append_field(aad: &mut Vec<u8>, value: &str) {
    let length = u16::try_from(value.len()).expect("validated identity field length");
    aad.extend_from_slice(&length.to_be_bytes());
    aad.extend_from_slice(value.as_bytes());
}

fn nonce_from_slice(value: &[u8], field: &'static str) -> Result<[u8; NONCE_LEN], SecretError> {
    value
        .try_into()
        .map_err(|_| SecretError::InvalidEnvelope(field))
}

fn parse_canonical_uuid(value: &str, error: SecretError) -> Result<Uuid, SecretError> {
    let parsed = Uuid::parse_str(value).map_err(|_| error)?;
    if parsed.hyphenated().to_string() == value && !parsed.is_nil() {
        Ok(parsed)
    } else {
        Err(SecretError::InvalidEnvelope("non-canonical UUID"))
    }
}

fn revision_from_i64(value: i64, field: &'static str) -> Result<u64, SecretError> {
    u64::try_from(value)
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or(SecretError::InvalidEnvelope(field))
}

fn i64_from_revision(value: u64) -> Result<i64, SecretError> {
    i64::try_from(value)
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or(SecretError::InvalidEnvelope(
            "record revision is outside the supported range",
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_state::DatabaseSet;
    use rusqlite::{Connection, params};
    use std::{collections::HashSet, fs, path::Path};

    fn private_test_directory(description: &str) -> tempfile::TempDir {
        let directory =
            tempfile::tempdir().unwrap_or_else(|error| panic!("{description}: {error}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
                .unwrap_or_else(|error| panic!("secure {description}: {error}"));
        }
        directory
    }

    fn setup() -> (tempfile::TempDir, DatabaseSet, MasterKeyCredential) {
        let temp = private_test_directory("temporary directory");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("open databases");
        let installation_id =
            Uuid::parse_str(databases.state().installation_id()).expect("installation identifier");
        let credential =
            MasterKeyCredential::generate(installation_id, 1).expect("generate credential");
        (temp, databases, credential)
    }

    fn identity(purpose: &str) -> SecretIdentity {
        SecretIdentity::new("api-token", "node", Uuid::new_v4(), purpose).expect("valid identity")
    }

    fn copied_secret(value: &SecretValue) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(value.with_secret(<[u8]>::to_vec))
    }

    fn assert_authentication_failure(store: &SecretStore<'_>, id: Uuid) {
        assert!(matches!(
            store.with_secret(id, |_| ()),
            Err(SecretError::RecordAuthenticationFailed)
        ));
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|candidate| candidate == needle)
    }

    fn hex(value: &[u8]) -> String {
        value.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn credentials_are_strict_versioned_and_redacted() {
        let installation_id = Uuid::new_v4();
        let credential =
            MasterKeyCredential::generate(installation_id, 7).expect("generate credential");
        let expected_key_id = credential.key_id();
        let encoded = credential.encode();

        assert_eq!(encoded.len(), CREDENTIAL_ENCODED_LEN);
        assert!(encoded.with_secret(|bytes| bytes[44..].iter().any(|byte| *byte != 0)));
        assert_eq!(format!("{encoded:?}"), "SecretValue([REDACTED])");
        let diagnostic = format!("{credential:?}");
        assert!(diagnostic.contains("[REDACTED]"));

        let mut round_trip = copied_secret(&encoded);
        let decoded =
            MasterKeyCredential::decode(SecretValue::new(std::mem::take(&mut *round_trip)))
                .expect("decode credential");
        assert_eq!(decoded.installation_id(), installation_id);
        assert_eq!(decoded.key_id(), expected_key_id);
        assert_eq!(decoded.key_version(), 7);

        let mut invalid_magic = copied_secret(&encoded);
        invalid_magic[0] ^= 1;
        assert!(matches!(
            MasterKeyCredential::decode(SecretValue::new(std::mem::take(&mut *invalid_magic))),
            Err(SecretError::InvalidCredential(_))
        ));

        let mut zero_key = copied_secret(&encoded);
        zero_key[44..].fill(0);
        assert!(matches!(
            MasterKeyCredential::decode(SecretValue::new(std::mem::take(&mut *zero_key))),
            Err(SecretError::InvalidCredential(_))
        ));

        assert!(matches!(
            MasterKeyCredential::decode(SecretValue::new(vec![0; 12])),
            Err(SecretError::InvalidCredential(_))
        ));
    }

    #[test]
    fn put_read_replace_and_delete_are_revision_checked() {
        let (_temp, databases, credential) = setup();
        let store = SecretStore::open(databases.state(), credential).expect("open store");
        let identity = identity("control-plane");
        let created = store
            .put(&identity, SecretValue::new(b"first-secret".to_vec()))
            .expect("put secret");

        assert_eq!(created.identity, identity);
        assert_eq!(created.revision, 1);
        assert_eq!(store.metadata(created.id).expect("metadata"), created);
        let observed = store
            .with_secret(created.id, <[u8]>::to_vec)
            .expect("read secret");
        assert_eq!(observed, b"first-secret");

        assert!(matches!(
            store.replace(
                created.id,
                0,
                SecretValue::new(b"invalid-revision".to_vec())
            ),
            Err(SecretError::InvalidEnvelope(_))
        ));
        assert!(matches!(
            store.replace(created.id, 2, SecretValue::new(b"wrong-revision".to_vec())),
            Err(SecretError::State(StateError::SecretRevisionConflict {
                expected: 2,
                actual: 1
            }))
        ));
        let replaced = store
            .replace(created.id, 1, SecretValue::new(b"second-secret".to_vec()))
            .expect("replace secret");
        assert_eq!(replaced.revision, 2);
        assert_eq!(
            store
                .with_secret(created.id, <[u8]>::to_vec)
                .expect("read replacement"),
            b"second-secret"
        );
        assert!(matches!(
            store.delete(created.id, 1),
            Err(SecretError::State(StateError::SecretRevisionConflict {
                expected: 1,
                actual: 2
            }))
        ));
        assert_eq!(
            store.delete(created.id, 2).expect("delete secret"),
            replaced
        );
        assert!(matches!(
            store.metadata(created.id),
            Err(SecretError::State(StateError::SecretNotFound))
        ));
    }

    #[test]
    fn plaintext_length_is_bounded_before_storage() {
        let (_temp, databases, credential) = setup();
        let store = SecretStore::open(databases.state(), credential).expect("open store");
        let identity = identity("bounded");

        assert!(matches!(
            store.put(&identity, SecretValue::new(Vec::new())),
            Err(SecretError::InvalidPlaintextLength)
        ));
        assert!(matches!(
            store.put(
                &identity,
                SecretValue::new(vec![7; MAX_SECRET_PLAINTEXT_BYTES + 1])
            ),
            Err(SecretError::InvalidPlaintextLength)
        ));

        let maximum = store
            .put(
                &identity,
                SecretValue::new(vec![9; MAX_SECRET_PLAINTEXT_BYTES]),
            )
            .expect("store maximum plaintext");
        assert_eq!(
            store
                .with_secret(maximum.id, <[u8]>::len)
                .expect("read maximum plaintext"),
            MAX_SECRET_PLAINTEXT_BYTES
        );
    }

    #[test]
    fn generated_record_nonces_do_not_repeat() {
        let (_temp, databases, credential) = setup();
        let store = SecretStore::open(databases.state(), credential).expect("open store");
        let mut nonces = HashSet::new();

        for sequence in 0_u64..64 {
            let record = store
                .put(
                    &identity(&format!("nonce-{sequence}")),
                    SecretValue::new(sequence.to_be_bytes().to_vec()),
                )
                .expect("put secret");
            let stored = databases
                .state()
                .load_secret_record(&record.id.hyphenated().to_string())
                .expect("load record")
                .expect("stored record");
            assert!(nonces.insert(stored.data_nonce));
            assert!(nonces.insert(stored.dek_wrap_nonce));
        }
        assert_eq!(nonces.len(), 128);
    }

    #[test]
    fn persisted_master_key_check_rejects_wrong_key_material() {
        let (_temp, databases, credential) = setup();
        let encoded = credential.encode();
        SecretStore::open(databases.state(), credential).expect("initialize master key");
        assert_eq!(
            databases
                .state()
                .master_key_record_count()
                .expect("key count"),
            1
        );

        let mut correct_bytes = copied_secret(&encoded);
        let correct =
            MasterKeyCredential::decode(SecretValue::new(std::mem::take(&mut *correct_bytes)))
                .expect("decode correct credential");
        SecretStore::open(databases.state(), correct).expect("verify persisted check");

        let mut wrong_bytes = copied_secret(&encoded);
        wrong_bytes[CREDENTIAL_ENCODED_LEN - 1] ^= 1;
        let wrong =
            MasterKeyCredential::decode(SecretValue::new(std::mem::take(&mut *wrong_bytes)))
                .expect("decode structurally valid wrong credential");
        assert!(matches!(
            SecretStore::open(databases.state(), wrong),
            Err(SecretError::MasterKeyRejected)
        ));
    }

    #[test]
    fn ciphertext_and_wrapped_dek_tampering_are_detected() {
        let (_temp, databases, credential) = setup();
        let store = SecretStore::open(databases.state(), credential).expect("open store");
        let ciphertext_record = store
            .put(&identity("ciphertext"), SecretValue::new(b"alpha".to_vec()))
            .expect("put ciphertext record");
        let wrap_record = store
            .put(
                &identity("wrapped-dek"),
                SecretValue::new(b"bravo".to_vec()),
            )
            .expect("put wrapped-DEK record");
        let connection = Connection::open(databases.state().path()).expect("open raw state");

        let mut ciphertext: Vec<u8> = connection
            .query_row(
                "SELECT ciphertext FROM secret_records WHERE id = ?1",
                [ciphertext_record.id.hyphenated().to_string()],
                |row| row.get(0),
            )
            .expect("read ciphertext");
        ciphertext[0] ^= 1;
        connection
            .execute(
                "UPDATE secret_records SET ciphertext = ?1 WHERE id = ?2",
                params![ciphertext, ciphertext_record.id.hyphenated().to_string()],
            )
            .expect("tamper ciphertext");

        let mut wrapped_dek: Vec<u8> = connection
            .query_row(
                "SELECT wrapped_dek FROM secret_records WHERE id = ?1",
                [wrap_record.id.hyphenated().to_string()],
                |row| row.get(0),
            )
            .expect("read wrapped DEK");
        wrapped_dek[0] ^= 1;
        connection
            .execute(
                "UPDATE secret_records SET wrapped_dek = ?1 WHERE id = ?2",
                params![wrapped_dek, wrap_record.id.hyphenated().to_string()],
            )
            .expect("tamper wrapped DEK");

        assert_authentication_failure(&store, ciphertext_record.id);
        assert_authentication_failure(&store, wrap_record.id);
    }

    #[test]
    fn encrypted_material_cannot_be_swapped_between_rows() {
        let (_temp, databases, credential) = setup();
        let store = SecretStore::open(databases.state(), credential).expect("open store");
        let first = store
            .put(&identity("swap-a"), SecretValue::new(b"same1".to_vec()))
            .expect("put first secret");
        let second = store
            .put(&identity("swap-b"), SecretValue::new(b"same2".to_vec()))
            .expect("put second secret");
        let first_record = databases
            .state()
            .load_secret_record(&first.id.hyphenated().to_string())
            .expect("load first")
            .expect("first record");
        let second_record = databases
            .state()
            .load_secret_record(&second.id.hyphenated().to_string())
            .expect("load second")
            .expect("second record");

        let mut connection = Connection::open(databases.state().path()).expect("open raw state");
        let transaction = connection.transaction().expect("swap transaction");
        transaction
            .execute(
                "UPDATE secret_records
                 SET data_nonce = ?1, ciphertext = ?2,
                     dek_wrap_nonce = ?3, wrapped_dek = ?4
                 WHERE id = ?5",
                params![
                    second_record.data_nonce,
                    second_record.ciphertext,
                    second_record.dek_wrap_nonce,
                    second_record.wrapped_dek,
                    first.id.hyphenated().to_string()
                ],
            )
            .expect("replace first material");
        transaction
            .execute(
                "UPDATE secret_records
                 SET data_nonce = ?1, ciphertext = ?2,
                     dek_wrap_nonce = ?3, wrapped_dek = ?4
                 WHERE id = ?5",
                params![
                    first_record.data_nonce,
                    first_record.ciphertext,
                    first_record.dek_wrap_nonce,
                    first_record.wrapped_dek,
                    second.id.hyphenated().to_string()
                ],
            )
            .expect("replace second material");
        transaction.commit().expect("commit swap");

        assert_authentication_failure(&store, first.id);
        assert_authentication_failure(&store, second.id);
    }

    #[test]
    fn revision_manipulation_is_authenticated() {
        let (_temp, databases, credential) = setup();
        let store = SecretStore::open(databases.state(), credential).expect("open store");
        let record = store
            .put(
                &identity("revision"),
                SecretValue::new(b"revision-bound".to_vec()),
            )
            .expect("put secret");
        let connection = Connection::open(databases.state().path()).expect("open raw state");
        connection
            .execute(
                "UPDATE secret_records SET revision = revision + 1 WHERE id = ?1",
                [record.id.hyphenated().to_string()],
            )
            .expect("manipulate revision");

        assert_authentication_failure(&store, record.id);
    }

    #[test]
    fn plaintext_canary_is_absent_from_database_sidecars_and_backup() {
        let (temp, databases, credential) = setup();
        let encoded_credential = credential.encode();
        let master_key =
            encoded_credential.with_secret(|bytes| Zeroizing::new(bytes[44..].to_vec()));
        let store = SecretStore::open(databases.state(), credential).expect("open store");
        let canary = b"HELIX-PLAINTEXT-CANARY-4f9b72d180cf";
        let record = store
            .put(&identity("canary"), SecretValue::new(canary.to_vec()))
            .expect("put canary");
        let stored = databases
            .state()
            .load_secret_record(&record.id.hyphenated().to_string())
            .expect("load record")
            .expect("stored record");
        assert!(!format!("{stored:?}").contains("PLAINTEXT-CANARY"));

        let database_path = databases.state().path();
        for path in [
            database_path.to_path_buf(),
            Path::new(&format!("{}-wal", database_path.display())).to_path_buf(),
            Path::new(&format!("{}-shm", database_path.display())).to_path_buf(),
        ] {
            if path.exists() {
                let raw = fs::read(&path).expect("read database artifact");
                assert!(!contains_bytes(&raw, canary), "canary appeared in {path:?}");
                assert!(
                    !contains_bytes(&raw, &master_key),
                    "master key appeared in {path:?}"
                );
            }
        }

        let backup = temp.path().join("backups").join("state-current.db");
        databases
            .state()
            .backup_to(&backup)
            .expect("create verified backup");
        let raw_backup = fs::read(&backup).expect("read backup");
        assert!(!contains_bytes(&raw_backup, canary));
        assert!(!contains_bytes(&raw_backup, &master_key));
        let backup_connection =
            Connection::open_with_flags(&backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("open backup read-only");
        let backup_count: i64 = backup_connection
            .query_row("SELECT count(*) FROM secret_records", [], |row| row.get(0))
            .expect("count backup secrets");
        let integrity: String = backup_connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .expect("backup integrity");
        assert_eq!(backup_count, 1);
        assert_eq!(integrity, "ok");
    }

    #[test]
    fn aad_encodings_have_stable_golden_bytes() {
        let installation_id = Uuid::from_u128(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff);
        let record_id = Uuid::from_u128(0x1021_3243_5465_7687_98a9_babb_dcdd_edef);
        let scope_id = Uuid::from_u128(0xfedc_ba98_7654_3210_0123_4567_89ab_cdef);
        let master_key_id = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let context = RecordContext {
            id: record_id,
            identity: SecretIdentity::new("token", "node", scope_id, "control").expect("identity"),
            revision: 0x0102_0304_0506_0708,
            wrap_revision: 0x1112_1314_1516_1718,
            master_key_id,
            data_nonce: [0; NONCE_LEN],
            dek_wrap_nonce: [0; NONCE_LEN],
        };

        assert_eq!(
            hex(&record_data_aad(installation_id, &context)),
            "48454c49582d4141440102\
             00112233445566778899aabbccddeeff\
             102132435465768798a9babbdcddedef\
             0005746f6b656e\
             00046e6f6465\
             fedcba98765432100123456789abcdef\
             0007636f6e74726f6c\
             0102030405060708"
                .replace([' ', '\n'], "")
        );
        assert_eq!(
            hex(&dek_wrap_aad(installation_id, &context, 0xa1b2_c3d4)),
            "48454c49582d4141440103\
             00112233445566778899aabbccddeeff\
             102132435465768798a9babbdcddedef\
             0005746f6b656e\
             00046e6f6465\
             fedcba98765432100123456789abcdef\
             0007636f6e74726f6c\
             1112131415161718\
             0123456789abcdef0123456789abcdef\
             a1b2c3d4"
                .replace([' ', '\n'], "")
        );
        assert_eq!(
            hex(&master_check_aad(
                installation_id,
                master_key_id,
                0xa1b2_c3d4
            )),
            "48454c49582d4141440101\
             00112233445566778899aabbccddeeff\
             0123456789abcdef0123456789abcdef\
             a1b2c3d4"
                .replace([' ', '\n'], "")
        );
    }
}
