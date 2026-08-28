# Getting Help

Helix is a private-alpha engineering preview. There is no production support
promise, response-time guarantee, compatibility commitment, supported
installer, or binary release yet. Public-internet exposure is unsupported.

## Before asking

Check these first:

- [Getting Started](docs/wiki/Getting-Started.md), including the Linux
  from-source install;
- [current verified status](PROGRESS.md);
- [known next work and release gates](NEXT.md);
- [private-LAN container boundaries](docs/CONTAINER-DEPLOYMENT.md);
- [development setup](docs/DEVELOPMENT.md);
- existing [issues](https://github.com/Riqqqque/Helix/issues) and
  [discussions](https://github.com/Riqqqque/Helix/discussions).

Use a GitHub Discussion for setup questions, design questions, or ideas that are
not yet a reproducible defect. Use the bug template for a reproducible problem.

Include the Helix version or commit, OS release, architecture, install method,
expected result, observed result, and the smallest sanitized log that explains
the failure. For broker-backed issues, also identify the affected surface
(storage, native server, AMP, network/UFW, host power, or package inventory) and
whether the broker, Docker, AMP, or UFW reported unavailable. Do not include
credentials, full configuration, private paths, or unrelated host inventory.

Never publish passwords, setup/session/CSRF tokens, recovery material, private
hostnames or addresses, storage roots, personal data, world content, console
history, or unredacted configuration.

## Security issues

Do not use an issue or discussion for a vulnerability. Follow
[the private reporting policy](SECURITY.md) instead.
