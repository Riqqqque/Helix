# Building Strands

A Strand is a Helix extension you can pack as a zip and drop onto a dashboard.
It can be a page, a Home widget, a vacuum/battery/cloud probe, or any other
HTTPS JSON integration you write. Helix does not run arbitrary native code or
root shells; it runs isolated UI plus declared host calls.

## Install someone else's Strand

1. Get a `.strand.zip` from the author (email, git, USB — Helix has no store).
2. Open **Strands** in the dashboard.
3. Drop the zip, choose a file, or paste an `https://` zip URL.
4. Read the capability list. `helix:net.https` shows the exact origins.
5. Install (starts disabled), then **Enable** if you accept those calls.
6. Open the Strand, or add it to Home if it declared `helix:ui.widget`.
7. **Export zip** to share your copy.

Unsigned zips are owner-authorized, not scanned. Disable or remove a Strand to
revoke its host calls immediately.

## Build one

```text
helixctl strand new vacuum-status --name "Vacuum status" --publisher "Your name"
helixctl strand check vacuum-status
helixctl strand pack vacuum-status -o vacuum-status.strand.zip
```

Default `--kind ui-only` is the installable format (`helix.strand/1`). Portable
Wasm (`--kind portable`) still validates preview metadata only.

The generator never overwrites an existing path and does not need a running
Helix.

```text
vacuum-status/
├── strand.toml
├── README.md
├── .gitignore
└── ui/
    ├── index.html
    ├── helix.js
    └── style.css
```

`ui/helix.js` talks to Helix through the parent page. The iframe is sandboxed
(`allow-scripts` only, no same-origin cookies) and cannot `fetch` Helix APIs
directly.

## Host calls

Start with zero capabilities. Add only what the UI needs:

| Capability | Host method | What it does |
| --- | --- | --- |
| `helix:metrics.read` | `helix.metrics.snapshot()` | Bounded CPU/memory/uptime |
| `helix:storage.kv` | `helix.storage.get/set/remove/list` | This Strand's key/value only |
| `helix:net.https` | `helix.net.fetch({ method, url, headers, body })` | HTTPS through Helix, origin allowlist |
| `helix:ui.page` | — | Declare a full-page surface; any enabled Strand can still be opened |
| `helix:ui.widget` | — | Offer this Strand as a Home widget |

`helix:net.https` requires `origins = ["https://api.example.com"]` and
`limits.outbound_requests_per_minute` of at least 1. Helix blocks loopback,
link-local, private, CGNAT, and metadata addresses. There is no ambient socket,
no `helix-privd`, and no filesystem outside this KV namespace.

Copy [https-probe](https://github.com/Riqqqque/Helix/tree/main/examples/strands/https-probe)
and change the origin/URL for a Dyson cloud API, a printer, or anything else
that speaks HTTPS JSON.

## What this version does not do

- Portable Wasm / `helix-strandd` execution
- Native sidecars (those stay high-trust and separately reviewed)
- A Helix-operated catalog, signatures, or auto-update
- Root, Docker, files, terminal, or secret-store access

Read the
[complete author guide](https://github.com/Riqqqque/Helix/blob/main/docs/STRAND-DEVELOPMENT.md)
and the
[system-health example](https://github.com/Riqqqque/Helix/tree/main/examples/strands/system-health).
