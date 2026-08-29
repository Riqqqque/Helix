# Release Process

Helix uses semantic versions. `vMAJOR.MINOR.PATCH` GitHub tags publish a
digest-pinned source archive for the private-LAN updater. A Git tag, green
build, or public repository is not by itself a public-internet support promise.

## Private-LAN numbered releases

A `vX.Y.Z` tag on `main` after a green CI run publishes `helix-source-X.Y.Z.tar.gz`
and `SHA256SUMS`. The dashboard Update Helix control uses those assets. This is
the 1.0 private-LAN channel.

Before creating that tag:

1. verify the `AGPL-3.0-or-later` license text and manifest metadata are present
   in the exact release tree;
2. start from a clean, reviewed commit with a green protected CI run;
3. pass the full Rust and frontend suites using locked dependencies;
4. pass RustSec, dependency-policy, npm, secret, and shell analysis;
5. update `CHANGELOG.md`, `PROGRESS.md`, compatibility claims, and recovery
   instructions without hiding waivers or failed results;
6. review the final tree and Git history for secrets and private host data.

Do not tag a prerelease identifier (`-alpha`, `-rc`) if you want the in-app
updater to accept it. The updater only applies stable `vMAJOR.MINOR.PATCH` tags.

## Public-internet release prerequisites

The following remain gates for calling Helix a supported public-internet
product. Missing evidence stays recorded in `PROGRESS.md`; it is not turned
into a caveat that pretends the gate passed.

1. verify the `AGPL-3.0-or-later` license text and manifest metadata are present
   in the exact release tree;
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

Release assets are built from the tag in GitHub Actions. Publish the source
archive, SHA-256 checksum, and GitHub build-provenance attestation together.
Checksums detect changed bytes; provenance connects the artifact to its source
and workflow. Neither is a security audit. The in-app updater requires the
source archive and `SHA256SUMS`; it does not run `git pull`.

Never upload an artifact built from an uncommitted local tree.

## Verification

After publication, download the public asset into a clean directory, verify its
checksum, and confirm the repository landing page, security reporting, wiki,
tag, release notes, and artifact links work while signed out.

If a public-internet gate cannot run, keep that claim out of the notes. A
private-LAN numbered tag may still ship when the updater assets are digest-pinned
and the changelog is honest about remaining gates.
