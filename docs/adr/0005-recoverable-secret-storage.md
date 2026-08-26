# ADR 0005: Recoverable-secret envelope storage

- Status: Accepted
- Date: 2026-08-25
- Decision owners: Helix maintainers

## Problem

Helix must retain credentials such as RCON, backup-provider, notification, and
third-party API secrets without placing plaintext in SQLite, configuration,
ordinary backups, logs, command lines, or HTTP responses. Database disclosure
must not disclose recoverable secrets, and moving encrypted fields between rows
or revisions must fail authentication. The design also needs a master-key
identity and format that can later support deliberate rotation and independent
recovery without pretending those operational workflows exist today.

## Considered options

### Encrypt each value directly with the installation key

This is simple, but rotation requires decrypting and re-encrypting every secret
payload. It also couples data encryption to the installation key and leaves no
separate revision for rewrapping key material.

### AES-256-GCM envelopes

AES-GCM is established and widely accelerated, but a repeated 96-bit nonce
under the same installation key is catastrophic. Helix does not need hardware
interoperability at this boundary, and a 192-bit random nonce gives more margin
for a service generating nonces from the operating-system CSPRNG.

### XChaCha20-Poly1305 envelopes

XChaCha20-Poly1305 provides authenticated encryption with a 256-bit key and a
192-bit nonce. RustCrypto exposes an allocation-capable, in-place API and a
zeroizing cipher-key feature without a native library dependency.

## Decision

Use a per-record envelope built from XChaCha20-Poly1305:

1. generate a fresh 32-byte data-encryption key (DEK), 24-byte data nonce, and
   24-byte wrapping nonce from the operating-system random source;
2. encrypt the bounded plaintext with the DEK and record-data associated data;
3. encrypt the DEK with the active installation master key and DEK-wrap
   associated data; and
4. commit ciphertext, wrapped DEK, nonces, format metadata, and monotonic
   revisions in one state-database transaction.

Plaintext must contain 1 through 65,536 bytes. XChaCha20-Poly1305's 16-byte tag
makes stored ciphertext 17 through 65,552 bytes. A wrapped 32-byte DEK is always
48 bytes.

Recoverable-secret storage enters the state schema at version 3.
`master_key_versions` stores key identity,
version, lifecycle status, algorithm/format metadata, and an authenticated
check record, but never key bytes. A unique active slot permits at most one
active key. `secret_records` stores stable identity, data and wrap revisions,
and encrypted material. SQL constraints, foreign keys, unique logical
identity, immutable-identity triggers, and monotonic-revision triggers provide
defense in depth around the repository API.

### Master-key credential

The portable credential is exactly 76 bytes:

```text
"HLXKEY01" || installation_uuid[16] || key_uuid[16]
             || key_version_u32_be || master_key[32]
```

Decoding rejects every other length or magic value, nil identifiers, version
zero, and an all-zero key. The credential and plaintext types are non-cloneable,
non-serializable, redacted in `Debug`, and zeroized on drop. The crate accepts
or emits only an in-memory `SecretValue`; it does not read paths, environment
variables, CLI arguments, or configuration.

On the first open of an uninitialized state database, the supplied credential
creates one active master-key row with an encrypted fixed check value. Every
later open must match the stored installation ID, key ID, version, and check
record. A wrong key, missing active key, unsupported format, or authentication
failure fails closed. This implements initial key creation and verification,
not rotation.

### Canonical associated data

Associated data is a versioned binary encoding, never JSON or a locale-sensitive
string. Every encoding starts with `HELIX-AAD`, one format byte, and one kind
byte. Integers use big-endian encoding, UUIDs use their 16 raw bytes, and text
uses a two-byte big-endian length followed by validated lowercase ASCII bytes.

- master-key check AAD binds installation UUID, key UUID, and key version;
- record-data AAD binds installation UUID, record UUID, complete logical
  identity, and data revision; and
- DEK-wrap AAD binds the same installation and record identity plus wrap
  revision, master-key UUID, and key version.

The data AAD deliberately excludes the wrapping key and wrap revision. A future
rotation can therefore authenticate and rewrap a DEK without re-encrypting the
secret payload. Golden-byte tests freeze all three encodings.

### API boundary

`helix-secrets` exposes put, metadata, closure-only read, revision-CAS replace,
and revision-CAS delete operations. It has no HTTP or serialization dependency.
The read callback scopes the crate-owned decrypted buffer and ensures that
buffer is zeroized, but its generic result deliberately permits a caller to
copy or derive a value. Every caller remains responsible for minimizing any
copy's lifetime and keeping it out of diagnostics and transport responses.

## Threat boundary

This design protects confidentiality when the state database or ordinary
database backup is disclosed without the external credential. AEAD and bound
metadata detect encrypted-field tampering, row substitution, and revision
manipulation before plaintext is released. Database constraints reject many
malformed states before cryptography runs.

It does not protect secrets from unrestricted root, a compromised Helix
process, code executing inside the process, a debugger or memory dump with key
access, a compromised kernel/random source, or a caller that copies plaintext.
It does not make a stolen host safe without host encryption, nor does it make
an ordinary database backup recoverable after the external key is lost.

## Operational work deliberately deferred

- systemd `LoadCredential=`/`LoadCredentialEncrypted=` provisioning and package
  wiring on supported Ubuntu releases;
- protected fallback-file creation, ownership, symlink handling, upgrade, and
  restore behavior;
- staged/active/retiring/retired transition APIs and crash-safe bulk DEK
  rewrapping;
- independent recovery-key enrollment, recovery envelopes, export, and restore;
- TPM2-bound credentials; and
- daemon/API integration and end-to-end leak testing.

None of those items is implied by the portable crate or the reserved lifecycle
columns. Until credential provisioning is wired, `helixd` cannot honestly use
this store for production secrets.

## Dependency and compatibility decision

The workspace pins `chacha20poly1305` 0.11.0 and `secrecy` 0.10.3, and uses
`getrandom` 0.4.3 plus `zeroize` 1.9.0. Those crates and the locked RustCrypto
transitives declare minimum Rust versions no later than 1.85, below the
workspace's Rust 1.88 floor. The crypto path uses RustCrypto's current
`AeadInOut` API and enables cipher-key zeroization.

Primary implementation references checked on 2026-08-25:

- [RustCrypto `chacha20poly1305` 0.11.0](https://docs.rs/chacha20poly1305/0.11.0/chacha20poly1305/)
- [RustCrypto `aead` 0.6.1](https://docs.rs/aead/0.6.1/aead/trait.AeadInOut.html)
- [`secrecy` 0.10.3](https://docs.rs/secrecy/0.10.3/secrecy/)
- [`getrandom` 0.4.3](https://docs.rs/getrandom/0.4.3/getrandom/)
- [systemd credentials](https://systemd.io/CREDENTIALS/)

## Validation status

Portable unit tests on Windows cover strict credential decoding and redaction,
master-key check reopening and wrong-key failure, the plaintext bound, put/read,
revision-CAS replace/delete, nonce non-repetition across generated records,
ciphertext and wrapped-DEK tampering, encrypted row substitution, revision
manipulation, stable AAD bytes, plaintext-canary absence from the database/WAL/
SHM artifacts that exist during the test, master-key absence from those
artifacts, and a verified SQLite backup. State tests cover a verified v2
snapshot before v3 migration, refusal to migrate an invalid v2 source, and
startup rejection when required v3 semantics are removed.

The Windows development host passed the workspace check and 152-test suite with
both Rust 1.94.1 and the declared Rust 1.88.0 minimum. Hosted Ubuntu 24.04 also
passed the target-conditioned suite with Rust 1.88.0 and stable.

One scoped hosted Ubuntu package lifecycle validated selected filesystem modes,
the dedicated systemd identity and hardening, forced-crash restart, clean
stop/start, and a verified database-only backup. It did not validate production
master-key delivery, systemd credential availability/lifetime, core-dump policy,
cross-version package or schema upgrades, power-loss durability, disk-full
behavior, or backup restore. Those broader Linux release gates remain open.
