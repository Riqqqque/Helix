# Building Strands

A Strand is a future Helix extension: a page, widget, integration, health check,
notification or backup provider, game tool, automation action, API route, or
data provider.

The current Strand Kit creates and validates **preview metadata only**. Helix
cannot install or run a Strand yet.

```text
helixctl strand new system-health --name "System Health" --publisher "Your name"
helixctl strand check system-health
```

The generated `strand.toml` starts with no permissions. It records a stable
UUID, Semantic Version, bounded Helix compatibility range, package kind,
publisher, license, capability reasons, and requested resource ceilings.
Validation is strict, size-bounded, non-executing, and safe to use in CI.

The generator stages the complete project before publishing it and refuses to
overwrite an existing path. It does not need a running Helix installation.

Portable and UI-only preview projects are available. The convenience command
does not generate trusted native sidecars because those require a separate
high-trust review.

Read the
[complete author guide](https://github.com/Riqqqque/Helix/blob/main/docs/STRAND-DEVELOPMENT.md)
and the
[reference project](https://github.com/Riqqqque/Helix/tree/main/examples/strands/system-health).
