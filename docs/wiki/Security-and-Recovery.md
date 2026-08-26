# Security and Recovery

## Current boundary

Helix rejects non-loopback binds. Remote access, TLS termination, trusted proxy
metadata, MFA, and public setup are not implemented or supported.

The current owner flow uses:

- a random, single-use, 15-minute local bootstrap capability;
- canonical owner identity and Argon2id password hashing;
- random revocable sessions stored only as verification hashes;
- `HttpOnly`, host-only, `SameSite=Strict` cookies;
- a session-bound CSRF proof required even for protected reads;
- bounded password work, login budgets, sessions, requests, and host sampling;
- generic authentication failures and secret-free audit details.

Authentication/session audit retention is fixed and bounded: Helix protects the
newest 1,024 events, applies a 90-day window beyond that floor, targets no more
than 50,000 rows, and deletes at most 256 rows in one audited-write or startup
transaction. Ordinary updates/deletes remain blocked. Export, holds, hash
chaining, and off-host evidence are not implemented yet.

## Data and recovery

Critical state and replaceable metrics are separate. Existing-schema migrations
create a no-clobber verified source snapshot before mutation. Startup validates
recovery state before deleting transient evidence. A clean-shutdown marker is
written only after HTTP draining and tracked blocking work finishes within the
shared 20-second deadline. A forced exit leaves the marker unclean.

`helixctl backup-state` creates a verified online critical-state snapshot. A
cleanup warning identifies a verified `.partial-*` hard-link residue; it does
not mean the published destination failed. A verified restore command is not
implemented yet, so do not use Helix as the only copy of important data.

Read the full [security model](https://github.com/Riqqqque/Helix/blob/main/docs/SECURITY.md)
and [recovery contract](https://github.com/Riqqqque/Helix/blob/main/docs/RECOVERY.md).

Report vulnerabilities through
[GitHub private vulnerability reporting](https://github.com/Riqqqque/Helix/security/advisories/new),
never a public issue.
