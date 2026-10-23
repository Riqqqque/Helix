# Building Strands

A **Strand** is a small extension you install from a zip file. It adds a page,
a Home widget, or both, made of plain HTML, CSS, and JavaScript. It runs in a
locked-down frame and can only use the host calls its manifest declares:
bounded host metrics, its own key/value store, and HTTPS requests to origins
you approve.

A Strand cannot reach the root broker, the file system, Docker, the terminal,
or Helix's secret store. Helix does not run native code or Wasm from a Strand.

## Install someone else's Strand

1. Get a `.strand.zip` from its author. Helix has no store; share it however
   you like.
2. Open **Strands** in the dashboard.
3. Drop the zip on the page, choose the file, or paste an `https://` link to
   the zip and choose **Review URL**.
4. Read the review: name, publisher, version, digest, files, and **Requested
   host calls** with each reason. For `helix:net.https` the exact origins are
   listed.
5. Choose **Install disabled**. Nothing runs yet.
6. Choose **Enable** when you accept those host calls.
7. Choose **Open**, or add it to Home as a **Strand** widget if it declared
   `helix:ui.widget`.

Other actions on an installed Strand:

| Action | Effect |
| --- | --- |
| **Disable** | Stops its host calls and its page at once. Its data stays. |
| **Export zip** | Downloads the stored package so you can share it. |
| **Remove** | Deletes the package and its key/value data. |
| Install the same ID again | Shows **Replace and disable**. The new zip replaces the old one, key/value data is kept, and the Strand stays disabled until you enable it again. |

Installing, enabling, and removing need the `system.settings.write`
capability. Packages are not signed or scanned; installing one is your
decision as the owner.

## Create a project

`helixctl` builds and checks Strands without a running Helix:

```bash
helixctl strand new vacuum-status --name "Vacuum status" --publisher "Your name"
helixctl strand check vacuum-status
helixctl strand pack vacuum-status -o vacuum-status.strand.zip
helixctl strand inspect vacuum-status.strand.zip
```

From a source checkout, use `cargo run --locked -p helixctl -- strand ...`.

`strand new` options:

| Option | Default |
| --- | --- |
| `--name` | The slug in title case |
| `--publisher` | `Local developer`. Set your own name; it is shown during review. |
| `--license` | `AGPL-3.0-or-later` |
| `--kind` | `ui-only`, the only kind you can install. `portable` writes preview metadata for a future Wasm host and cannot be packed. |
| destination | A folder named after the slug. It must not exist yet; `new` never overwrites. |

The generated project:

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

`ui/helix.js` is the host SDK. Keep it; your page talks to Helix through it.

## The manifest

`strand.toml` is strict TOML. Unknown fields, missing required fields, and
out-of-range values all fail `strand check` and install. The manifest file may
be up to 64 KiB.

### Top-level fields

| Field | Rule |
| --- | --- |
| `schema` | `"helix.strand/1"` for installable packages. |
| `id` | A UUID in lowercase hyphenated form, not all zeros. It is the Strand's permanent identity; keep it across versions. |
| `slug` | 2 to 48 characters: lowercase letters and digits in segments joined by single hyphens, starting with a letter. |
| `name` | 1 to 80 characters. |
| `version` | Semantic Versioning, for example `0.1.0`. |
| `description` | 1 to 240 characters. |
| `license` | 1 to 96 characters, usually an SPDX expression. |
| `publisher` | 1 to 120 characters. |
| `kind` | `"ui-only"`. |

Text fields may not have leading or trailing spaces, control characters, or
bidirectional formatting characters.

### `[[capabilities]]`

Each entry asks for one host call. Up to 32 entries, no duplicates.

| Field | Rule |
| --- | --- |
| `name` | One of the names below. |
| `reason` | 8 to 200 characters, shown to the owner during review. |
| `optional` | `true` or `false`. Defaults to `false`. |
| `origins` | Only for `helix:net.https`, and required there. 1 to 8 exact origins like `https://api.example.com` or `https://api.example.com:8443`. No path, query, user info, or explicit `:443`. |

| Capability | Host method | What it allows |
| --- | --- | --- |
| `helix:metrics.read` | `helix.metrics.snapshot()` | Helix version, uptime, CPU percent (can be `null`) and core count, memory and swap use, sample time. |
| `helix:storage.kv` | `helix.storage.get / set / remove / list` | Read and write this Strand's own key/value store. |
| `helix:net.https` | `helix.net.fetch(request)` | HTTPS requests to the listed origins, sent by Helix. |
| `helix:ui.page` | none | Declares a full page. Any enabled Strand can still be opened. |
| `helix:ui.widget` | none | Lets you add the Strand to Home as a widget. |

### `[compatibility]`

| Field | Rule |
| --- | --- |
| `helix` | A version range with both a lower and an upper bound, for example `">=1.0.0, <2.0.0"`. Install fails if the running Helix is outside it. |
| `host_api` | `"1"`. |

### `[limits]`

| Field | Allowed range | Used for |
| --- | --- | --- |
| `memory_mib` | 8 to 512 | Declared budget. |
| `timeout_ms` | 10 to 30,000 | Timeout for each HTTPS request. |
| `concurrent_calls` | 1 to 32 | Declared budget. |
| `queue_depth` | 1 to 1,024 | Declared budget. |
| `storage_mib` | 0 to 4,096 | Declared budget. |
| `outbound_requests_per_minute` | 0 to 600 | Rate limit for `net.fetch`. Must be at least 1 when you request `helix:net.https`. |
| `log_kib_per_minute` | 1 to 1,024 | Declared budget. |

Fields marked "declared budget" are validated and shown in review. This
version enforces the HTTPS timeout and rate limit, plus the fixed key/value
limits below.

### `[ui]`

| Field | Rule |
| --- | --- |
| `entry` | An `.html` file under `ui/`, usually `ui/index.html`. Required for `helix.strand/1`. |

## Host calls from your page

```javascript
const metrics = await helix.metrics.snapshot();

await helix.storage.set("last", JSON.stringify(metrics));
const saved = await helix.storage.get("last"); // { value: "..." } or { value: null }
const keys = await helix.storage.list();       // { keys: [...] }
await helix.storage.remove("last");

const response = await helix.net.fetch({
  method: "GET",
  url: "https://api.example.com/v1/status",
  headers: { authorization: "Bearer <token>" },
});
// { status, contentType, encoding: "utf8" | "base64", body }
```

Each call returns a promise. The SDK rejects it after 30 seconds with no
answer, or when Helix refuses the call (for example, the capability is
missing or the Strand is disabled).

### `net.fetch` rules

- Methods: `GET`, `HEAD`, `POST`, `PUT`, `DELETE`. No body on `GET` or
  `HEAD`.
- The URL must match one of the declared origins.
- Redirects are not followed.
- Loopback, link-local, private, CGNAT, and cloud metadata addresses are
  refused.
- The response body is capped at 256 KiB. Text comes back as `utf8`; anything
  else as `base64`.
- The timeout is `limits.timeout_ms`.
- Calls beyond `outbound_requests_per_minute` in any 60-second window are
  refused.

### Key/value limits

| Limit | Value |
| --- | --- |
| Keys per Strand | 256 |
| Key length | 128 characters |
| Value size | 8 KiB |
| Total per Strand | 1 MiB |

If your Strand needs a token for an outside API, have the owner type it into
your page and keep it in the key/value store, as the `https-probe` example
does. Helix does not give Strands access to its own secrets.

## The frame and sandbox

- Your entry page loads in an `<iframe sandbox="allow-scripts">` with no
  `allow-same-origin`. Its origin is opaque, so it cannot read Helix cookies,
  browser storage, or call the Helix API.
- Files are served with this Content Security Policy:

  ```text
  default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'none'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'
  ```

  `connect-src 'none'` blocks `fetch`, XHR, and WebSockets from the frame.
  All network access goes through `helix.net.fetch`.
- `helix.js` posts each call to the parent page. The parent checks that the
  message came from this exact frame, then sends it to Helix with the owner's
  session. Helix checks the Strand is enabled and holds the capability on
  every call.
- As a widget, the page loads with `#helix-widget` in the URL. Use it to
  switch to a compact layout:

  ```javascript
  document.documentElement.classList.toggle("is-widget", location.hash === "#helix-widget");
  ```

## Packaging rules

| Limit | Value |
| --- | --- |
| Zip size | 8 MiB |
| Unpacked size | 16 MiB |
| Files | 64 |
| Single file | 512 KiB |
| Installed Strands | 32 |
| Download from URL | 20 seconds, `https://` only |

Only `strand.toml` and files under `ui/` with these extensions are allowed:
`css`, `html`, `ico`, `jpeg`, `jpg`, `js`, `json`, `md`, `png`, `svg`, `txt`,
`webp`, `woff2`. Symbolic links, encrypted entries, and setuid bits are
rejected.

## A complete example

This is the
[https-probe](https://github.com/Riqqqque/Helix/tree/main/examples/strands/https-probe)
manifest. It calls one HTTPS API and remembers the last answer.

```toml
schema = "helix.strand/1"
id = "c3e0d2a1-8f44-4b9e-9c11-6a7b5d4e3f20"
slug = "https-probe"
name = "HTTPS Probe"
version = "0.1.0"
description = "Calls one allowlisted HTTPS JSON API through Helix and remembers the last response."
license = "AGPL-3.0-or-later"
publisher = "Helix example"
kind = "ui-only"

[[capabilities]]
name = "helix:net.https"
reason = "Fetch a public weather JSON sample so other HTTPS APIs can copy the pattern."
optional = false
origins = ["https://api.open-meteo.com"]

[[capabilities]]
name = "helix:storage.kv"
reason = "Remember the last successful probe in this Strand's own key/value store."
optional = false

[[capabilities]]
name = "helix:ui.page"
reason = "Open this Strand as a page in the Strands workspace."
optional = false

[[capabilities]]
name = "helix:ui.widget"
reason = "Optionally pin this Strand as a Home widget."
optional = false

[compatibility]
helix = ">=1.0.0, <2.0.0"
host_api = "1"

[limits]
memory_mib = 32
timeout_ms = 8000
concurrent_calls = 2
queue_depth = 32
storage_mib = 1
outbound_requests_per_minute = 12
log_kib_per_minute = 64

[ui]
entry = "ui/index.html"
```

A minimal `ui/index.html` that uses one capability:

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Host load</title>
<link rel="stylesheet" href="style.css">
</head>
<body>
<main>
<h1>Host load</h1>
<p id="status">Reading…</p>
</main>
<script src="helix.js"></script>
<script>
helix.metrics.snapshot().then(function (m) {
  var cpu = m.cpuPercent === null ? "unknown" : m.cpuPercent.toFixed(1) + "%";
  document.getElementById("status").textContent =
    "CPU " + cpu + " across " + m.logicalCores + " cores";
}).catch(function (error) {
  document.getElementById("status").textContent = error.message;
});
</script>
</body>
</html>
```

That page needs only `helix:metrics.read` (and `helix:ui.widget` if you want
it on Home). Start every Strand with no capabilities and add only what the
page uses. The
[system-health](https://github.com/Riqqqque/Helix/tree/main/examples/strands/system-health)
example is a fuller metrics page and widget.

## Not in this version

- Portable Wasm Strands and the `helix-strandd` host.
- Native sidecars.
- Signatures, a Helix-run catalog, or automatic updates.
- Access to root, Docker, files, the terminal, or Helix secrets.

## Related pages

- [Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home)
- [API and Integrations](https://github.com/Riqqqque/Helix/wiki/API-and-Integrations)
- [Strand security contract](https://github.com/Riqqqque/Helix/blob/main/docs/STRANDS.md)
- [Strand author guide](https://github.com/Riqqqque/Helix/blob/main/docs/STRAND-DEVELOPMENT.md)
