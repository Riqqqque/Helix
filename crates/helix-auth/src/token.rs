use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

pub const OPAQUE_TOKEN_BYTES: usize = 32;
pub const OPAQUE_TOKEN_ENCODED_LEN: usize = 43;

/// Purpose used to domain-separate token verifier hashes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenDomain {
    Bootstrap,
    Session,
    Csrf,
}

impl TokenDomain {
    const fn separator(self) -> &'static [u8] {
        match self {
            Self::Bootstrap => b"helix/bootstrap/v1\0",
            Self::Session => b"helix/session/v1\0",
            Self::Csrf => b"helix/csrf/v1\0",
        }
    }
}

/// A 256-bit opaque bearer token.
///
/// This type intentionally implements neither `Debug`, `Display`, nor `Clone`.
/// Its bytes are zeroized on drop.
pub struct OpaqueToken(Zeroizing<[u8; OPAQUE_TOKEN_BYTES]>);

impl OpaqueToken {
    pub fn generate() -> Result<Self, TokenError> {
        let mut bytes = Zeroizing::new([0_u8; OPAQUE_TOKEN_BYTES]);
        getrandom::fill(bytes.as_mut()).map_err(|_| TokenError::RandomSource)?;
        Ok(Self(bytes))
    }

    pub fn from_encoded(encoded: &str) -> Result<Self, TokenError> {
        if encoded.len() != OPAQUE_TOKEN_ENCODED_LEN {
            return Err(TokenError::InvalidEncoding);
        }

        let mut bytes = Zeroizing::new([0_u8; OPAQUE_TOKEN_BYTES]);
        let written = URL_SAFE_NO_PAD
            .decode_slice(encoded, bytes.as_mut())
            .map_err(|_| TokenError::InvalidEncoding)?;
        if written != OPAQUE_TOKEN_BYTES {
            return Err(TokenError::InvalidEncoding);
        }

        let canonical = Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_ref()));
        if canonical.as_str() != encoded {
            return Err(TokenError::InvalidEncoding);
        }

        Ok(Self(bytes))
    }

    #[must_use]
    pub fn encode(&self) -> EncodedToken {
        EncodedToken(Zeroizing::new(URL_SAFE_NO_PAD.encode(self.0.as_ref())))
    }

    #[must_use]
    pub fn verification_hash(&self, domain: TokenDomain) -> TokenHash {
        let mut digest = Sha256::new();
        digest.update(domain.separator());
        digest.update(self.0.as_ref());
        let output = digest.finalize();
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&output);
        TokenHash(Zeroizing::new(bytes))
    }
}

/// URL-safe, unpadded bearer-token representation.
///
/// This type intentionally implements neither `Debug`, `Display`, nor `Clone`.
/// Use `expose_secret` only at a transport boundary; its allocation is zeroized
/// on drop.
pub struct EncodedToken(Zeroizing<String>);

impl EncodedToken {
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.as_str()
    }
}

/// Domain-separated SHA-256 token verifier suitable for restricted storage.
///
/// This type intentionally implements neither `Debug` nor `Display` and
/// zeroizes its bytes on drop.
pub struct TokenHash(Zeroizing<[u8; 32]>);

impl TokenHash {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TokenError {
    #[error("the operating-system random source is unavailable")]
    RandomSource,
    #[error("opaque tokens must use the canonical 43-character base64url form")]
    InvalidEncoding,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_round_trip_through_the_canonical_encoding() {
        let token = OpaqueToken::generate().expect("generate token");
        let encoded = token.encode();
        assert_eq!(encoded.expose_secret().len(), OPAQUE_TOKEN_ENCODED_LEN);
        assert!(
            encoded
                .expose_secret()
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );

        let decoded = OpaqueToken::from_encoded(encoded.expose_secret()).expect("decode token");
        assert_eq!(
            token.verification_hash(TokenDomain::Session).as_bytes(),
            decoded.verification_hash(TokenDomain::Session).as_bytes()
        );
    }

    #[test]
    fn decoder_rejects_padding_wrong_lengths_and_non_url_safe_bytes() {
        for invalid in [
            "short",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA+",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA/",
        ] {
            assert_eq!(
                OpaqueToken::from_encoded(invalid)
                    .err()
                    .expect("invalid token"),
                TokenError::InvalidEncoding
            );
        }
    }

    #[test]
    fn verification_hashes_are_domain_separated_and_deterministic() {
        let token = OpaqueToken::from_encoded("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
            .expect("fixed valid token");
        let bootstrap = token.verification_hash(TokenDomain::Bootstrap);
        let session = token.verification_hash(TokenDomain::Session);
        let csrf = token.verification_hash(TokenDomain::Csrf);
        let session_again = token.verification_hash(TokenDomain::Session);

        assert_ne!(bootstrap.as_bytes(), session.as_bytes());
        assert_ne!(bootstrap.as_bytes(), csrf.as_bytes());
        assert_ne!(session.as_bytes(), csrf.as_bytes());
        assert_eq!(session.as_bytes(), session_again.as_bytes());
    }

    #[test]
    fn independently_generated_tokens_are_distinct() {
        let first = OpaqueToken::generate().expect("first token");
        let second = OpaqueToken::generate().expect("second token");

        assert_ne!(
            first.verification_hash(TokenDomain::Session).as_bytes(),
            second.verification_hash(TokenDomain::Session).as_bytes()
        );
    }
}
