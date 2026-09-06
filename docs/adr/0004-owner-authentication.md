# ADR 0004: Owner password and session authentication

- Status: Accepted
- Date: 2026-08-25
- Amended: 2026-09-03
- Decision owners: Helix maintainers

## Problem

Helix needs a safe first-owner flow and browser authentication before it can expose detailed host information or any mutating control. The design has to survive concurrent setup attempts, support later MFA and granular roles, avoid bearer tokens in browser storage, and keep password work from exhausting the async runtime.

## Considered options

### Signed stateless browser tokens

JWT-style tokens avoid a lookup but make immediate revocation, role changes, idle expiration, and incident response harder. They also encourage putting authorization state into a client-held object. Helix already has a local critical-state database, so the avoided lookup is not valuable here.

### Generic framework session storage

A generic key/value session bag is convenient, but it obscures the security schema and tends to accumulate unrelated state. Helix needs explicit token hashing, authentication-version invalidation, absolute and idle expiry, CSRF binding, and audit behavior.

### Opaque server-side sessions

Random opaque tokens can be stored only as hashes, revoked immediately, and joined against current user and role state. The database remains authoritative and the cookie contains no claims.

## Decision

Use opaque server-side sessions and a single-use local bootstrap capability.

- `helixctl` creates 32 random bootstrap bytes from the operating-system CSPRNG. It prints the encoded token once and stores only a domain-separated SHA-256 hash, a short expiry, and consumption state.
- Owner creation performs a second no-owner, token, expiry, and consumption check inside one immediate SQLite transaction. The transaction inserts the owner, role assignment, session, and audit record and consumes the token atomically.
- Passwords use Argon2id version 19 PHC strings. The initial policy is 19 MiB, two iterations, one lane, and a 32-byte output. Parameters are versioned and re-benchmarked on supported Ubuntu hardware before public release.
- A password used as the only factor must contain at least 13 Unicode code points. Helix accepts at least 64 and initially caps input at 256 code points and 1024 UTF-8 bytes to bound work. It imposes no character-class rule or periodic rotation. The current production path rejects a small built-in set of obvious defaults. The authentication crate has a pluggable compromised-password checker, but a maintained blocklist source and its offline/update/failure policy remain a public-release gate. NIST recommends 15 characters for password-only authentication; operators can still choose that stronger length without Helix forcing a credential change during adoption.
- A stored PHC string is parsed and checked against explicit algorithm, version, output, memory, iteration, and lane ceilings before verification. Database tampering must not turn login into an unbounded allocation.
- Password hashing and verification run on bounded blocking workers behind a small semaphore. Unknown and disabled login names follow a dummy Argon2 path, and source/login rate limits run before expensive work.
- A browser session uses a separate 32-byte random token. Only its domain-separated hash is stored. Sessions have server-enforced idle and absolute deadlines by default (30 minutes idle, eight hours absolute) and capture the user's `auth_version`; password, status, and authorization changes invalidate prior versions. Settings can disable idle and absolute expiry for this private-LAN console; CSRF pairing, logout, and credential-change revocation stay required. Persistent cookies are still capped at 400 days.
- Normal post-convergence state retains at most 64 session rows per user. Each login performs at most one 256-row repair batch and returns a generic retryable 503 while a larger imported set converges. At the cap, eviction, credential rehash/reset, random token creation, insertion, stale pruning, and success audit commit atomically.
- Loopback HTTP uses an `HttpOnly`, host-only, `SameSite=Strict`, `Path=/` cookie with no `Domain`. HTTPS deployment will use the `__Host-` prefix and `Secure` only after the TLS or trusted-proxy boundary is implemented and verified.
- Every protected route requires both the session cookie and the session-bound synchronizer token in `X-Helix-CSRF`, including protected reads. CSRF tokens are random and stored server-side only as hashes. The frontend stores only that proof in origin-scoped browser storage so the same Helix origin can restore across reloads; normal idle and absolute deadlines remain server-enforced. Mutations additionally require an exact allowed `Origin` and same-origin Fetch Metadata when supplied.
- Reload restoration never mints a proof from the cookie. The frontend must already have the matching saved proof, and `/auth/me` revalidates both parts. Logout, account changes, authentication rejection, and malformed saved state clear the saved proof. The rotation route still requires the current synchronizer token.
- Synchronizer rotation is compare-and-swap, so concurrent requests using one old proof cannot both commit valid replacements.
- This stronger local rule is intentional: cookies are scoped by host, not TCP port, so another service on `localhost` can receive `helix_session`. It still cannot use that cookie against Helix without the current CSRF proof. Persistent browser storage is scoped by origin, including the port, so a service on another port cannot read the saved proof. Same-origin script compromise remains able to act as the signed-in user, so CSP and frontend supply-chain controls remain part of this boundary. TLS deployment must revisit cookie names, `Secure`, trusted proxy behavior, and origin policy as one reviewed boundary.
- Authorization is server-side and default-deny. The owner role is assigned explicit capabilities; the frontend capability list is presentation data, not authority.
- Session, role, bootstrap, and authentication events are append-only audit records. Passwords, PHC strings, raw tokens, cookies, and authorization headers never enter the audit detail or ordinary logs.

## Consequences

Every authenticated request requires one bounded local lookup on the successful path, which is acceptable for a local SQLite control plane and gives immediate revocation. A failed CSRF check performs one additional proof-free lookup only to preserve the useful distinction between an invalid session (`401`) and an invalid proof (`403`). State migration and login code are more explicit than a generic session library, but their invariants can be tested directly.

Reloading no longer changes authentication state: both normal and extended sessions can restore only while the server-side session is current. Helix accepts the browser-storage exposure of the non-bearer CSRF half so it can preserve that behavior without weakening the two-part request check or exposing the `HttpOnly` session cookie.

The design is not a claim of NIST conformance. MFA, password blocklist operations, TLS, recovery authentication, trusted proxies, credential breach response, and independent review remain separate gates.

## Validation required

- competing bootstrap claims result in exactly one owner, one initial session, and one committed success audit record;
- raw bootstrap, session, and CSRF tokens never appear in SQLite or logs;
- malformed and extreme PHC strings are rejected before Argon2 allocation;
- password work never runs on a Tokio core thread and never exceeds the configured concurrency;
- unknown, disabled, delayed, and incorrect-password responses are generic and rate limited;
- session idle, absolute, revoke, and `auth_version` boundaries pass with a controlled clock;
- oversized imported session sets converge in fixed batches, maintenance responses do not consume password-failure budgets, and RNG/audit/commit failures roll back cap eviction and credential changes;
- missing, wrong, duplicate, stale-after-rotation, cross-session, cross-origin, and cross-site CSRF requests fail, including protected reads and the rotation route;
- a loopback cookie without its matching proof cannot read protected data or rotate a replacement proof;
- persistent restoration succeeds only for a non-expiring session with both valid proofs, and stale, malformed, revoked, or newly expiring saved proofs are discarded;
- every protected capability defaults to denied when a mapping is missing;
- Linux permissions, TLS cookie behavior, reverse-proxy handling, and brute-force behavior pass dedicated tests.

## Current external basis

- OWASP Password Storage Cheat Sheet, checked 2026-08-25: Argon2id minimum 19 MiB, two iterations, one lane.
- NIST SP 800-63B-4, published July 2025 and checked 2026-08-25: 15-character minimum for password-only authentication, at least 64-character support, no composition rule, blocklist checking, and salted work-factor hashing.
- RustCrypto `argon2` 0.5.3 documentation, checked 2026-08-25: Argon2id v19 PHC password-hash support and Rust 1.65 minimum.

These are implementation inputs, not frozen product assumptions. Revalidate them when the policy or dependency changes.
