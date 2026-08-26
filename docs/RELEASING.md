# Release Process

Helix uses semantic versions and starts with prerelease versions. A Git tag,
green build, or public repository is not by itself a supported release.

## Release prerequisites

Before creating a tag or GitHub release:

1. select and record a project license;
2. start from a clean, reviewed commit with a green protected CI run;
3. pass the full Rust and frontend suites using locked dependencies;
4. pass RustSec, dependency-policy, npm, secret, and shell analysis;
5. build the Linux bundle on the declared supported Ubuntu release;
6. pass clean install, owner claim, authenticated API, upgrade, rollback,
   tamper rejection, uninstall, permissions, and service-lifecycle tests;
7. complete the migration, corruption, low-disk, interrupted-operation, and
   restore tests relevant to the release;
8. record reference performance and accessibility evidence;
9. update `CHANGELOG.md`, `PROGRESS.md`, compatibility claims, and recovery
   instructions without hiding waivers or failed results;
10. review the final tree and Git history for secrets and private host data.

## Artifact policy

Release assets are built from the tag in GitHub Actions. Publish the Linux
archive, SHA-256 checksum, and GitHub build-provenance attestation together.
Checksums detect changed bytes; provenance connects the artifact to its source
and workflow. Neither is a security audit.

The release notes must state the supported OS/architecture, configuration and
schema compatibility, required restart, rollback limits, known issues, and exact
verification command. Never upload an artifact built from an uncommitted local
tree.

## Verification

After publication, download the public asset into a clean directory, verify its
checksum and provenance, extract it, and repeat the supported installation and
readiness flow. Confirm that the repository landing page, security reporting,
wiki, tag, release notes, and artifact links work while signed out.

If any required gate cannot run, keep the version unreleased and record the
blocker in `PROGRESS.md`; do not turn missing evidence into a release-note caveat.
