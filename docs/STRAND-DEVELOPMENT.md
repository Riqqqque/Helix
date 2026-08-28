# Building a Strand

## What is available today

A **Strand** is Helix's extension unit: isolated dashboard HTML plus declared
host calls. UI-only packages using `helix.strand/1` can be packed as a zip,
reviewed, installed, enabled, shared, and opened as a page or Home widget.

Portable Wasm and native sidecars are still not a runtime. Do not ship a
pretend ABI for those classes.

## Create, pack, and install

```text
helixctl strand new vacuum-status --name "Vacuum status" --publisher "Your name"
helixctl strand check vacuum-status
helixctl strand pack vacuum-status -o vacuum-status.strand.zip
helixctl strand inspect vacuum-status.strand.zip
```

Install the zip from the dashboard **Strands** page (upload or https URL).
Installing stores the package disabled. Enabling is a second owner action after
the capability list is on screen.

From a source checkout:

```text
cargo run --locked -p helixctl -- strand new vacuum-status --name "Vacuum status"
cargo run --locked -p helixctl -- strand pack vacuum-status -o vacuum-status.strand.zip
```

`--kind ui-only` is the default and the only installable class. `--kind portable`
still writes `helix.strand/preview-1` metadata for a future Wasm host.

The destination must not exist. Strand tooling does not need a running daemon.

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

## Manifest (`helix.strand/1`)

`strand.toml` is strict TOML. Unknown fields fail closed.

| Field | Purpose |
| --- | --- |
| `id` | Permanent canonical UUID |
| `slug` | Lowercase machine name |
| `name` / `description` / `publisher` / `license` | Review identity |
| `version` | Semantic Versioning |
| `kind` | `ui-only` for installable packages |
| `capabilities` | Named host calls with reasons; `origins` for HTTPS |
| `compatibility.helix` | Bounded Helix range |
| `compatibility.host_api` | `1` for this installable generation |
| `limits` | Memory, timeout, KV, outbound request, log ceilings |
| `ui.entry` | HTML file under `ui/`, usually `ui/index.html` |

```toml
[[capabilities]]
name = "helix:net.https"
reason = "Read the vacuum cloud API battery and docking state."
optional = false
origins = ["https://api.example-vacuum.com"]
```

Allowed installable capabilities: `helix:metrics.read`, `helix:storage.kv`,
`helix:net.https`, `helix:ui.page`, `helix:ui.widget`.

## Host SDK

`ui/helix.js` posts messages to the Helix parent. The iframe is sandboxed
without `allow-same-origin`, so it cannot read Helix cookies or `fetch` the API.

```javascript
const snapshot = await helix.metrics.snapshot();
await helix.storage.set("last", JSON.stringify(snapshot));
const response = await helix.net.fetch({
  method: "GET",
  url: "https://api.example-vacuum.com/v1/devices",
  headers: { authorization: "Bearer …" },
});
```

Keep secrets in Strand KV (an input in the Strand's own UI) or typed by the
owner in that UI. Helix does not give Strands the installation secret store.

## Examples

- [system-health](../examples/strands/system-health) — metrics page/widget
- [https-probe](../examples/strands/https-probe) — allowlisted HTTPS + KV

Security contract: [Strand Extension Security and Runtime](STRANDS.md).
