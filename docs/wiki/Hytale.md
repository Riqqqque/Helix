# Hytale

Helix runs the official Hytale dedicated server in an isolated container on
Java 25. It handles the Hytale sign-ins, installs CurseForge mods, sends console
commands, updates the server on start, and backs it up like any other Helix
server. Create one from **Servers → New server → Hytale**.

## Creating the server

The create dialog asks for a name, memory (4–32 GiB, 6 GiB by default), an
optional CPU cap, the player limit (1–100), an optional server password, and the
game port (5520 by default, from the Hytale port pool 5520–5570).

The first time, Helix builds the `helix-hytale-runtime:1` image. The runtime then
downloads the official Hytale Downloader and, after you sign in, the server files.
Memory is the Java heap; the container is allowed about 1 GiB more for Java's own
overhead.

## First start: two sign-ins

Hytale ties every server to a Hytale account, so the first start asks you to sign
in twice. Helix shows each link as soon as Hytale asks for it, in the create
progress and as a **Sign in to Hytale** banner on the server page.

1. **Download.** The Hytale Downloader signs in to fetch the server files. Open
   the link, sign in, and confirm the code. The download continues on its own.
2. **Server.** When the server prints `Hytale Server Booted!`, Helix runs
   `/auth login device`. Sign in again with the new link. If your account has
   several profiles, Helix picks the first (`/auth select 1`). After the sign-in
   succeeds, Helix runs `/auth persistence Encrypted`, so the server stays signed
   in across restarts.

Helix only shows links that point to `https://hytale.com` or one of its
subdomains, and only short codes. Codes expire after a few minutes. Helix retries
the download with a fresh code, or you can run `/auth login device` from the
Console tab yourself.

The downloader's credentials stay in the server folder under `.helix/downloader/`
and the server's encrypted sign-in in `auth.enc`. Both are included in backups,
so a restored server does not need to sign in again.

## When the server counts as ready

Start, Restart, and deploys wait until the server is ready. For Hytale that is
when it prints `Hytale Server Booted!`. If a future Hytale release changes that
message, Helix treats a server that has stayed up for three minutes as ready.

To stop, Helix sends `/stop` through the console and gives the server up to 45
seconds to save and exit before Docker stops it.

## Letting players join

Hytale uses QUIC, so players connect over **UDP** only, on the game port. The
Ports card shows it as **UDP (QUIC)**. For play outside your network, forward that
UDP port in your router, or choose **Public** when you create the server so Helix
adds the host firewall rule (only when UFW is already active).

At every start Helix writes three values into `config.json`: the server name, the
player limit, and the password. Other keys in `config.json` are left alone. Edit
them from **Files** while the server is stopped. If `config.json` is not valid
JSON, Helix leaves it unchanged and says so in the log.

The Ports card can add extra ports for a web map or API that a mod opens. The
Minecraft presets, such as Simple Voice Chat, are not offered.

## Mods

Hytale has one first-party mod API and no loader to install.

- **Mods tab.** Search CurseForge, Hytale's official mod platform, and choose
  **Install**. Helix picks the newest release, installs required dependencies,
  checks every file against CurseForge's SHA-1, and writes it to `mods/`. This
  needs a CurseForge API key in **Settings → Catalogs**.
- **Your own files.** Upload `.jar` plugins or `.zip` packs into `mods/` from
  the Files tab, or through the server file API. Changes made through the API
  while the server runs are staged and installed on the next restart or deploy;
  see [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes).

Restart the server to load new mods, then check the Logs tab to confirm they
loaded.

## Console

The Console tab sends commands to the server, for example `who` or
`auth status`. Helix adds the leading `/` if you leave it out. A command is one
line under 512 bytes. Hytale has no RCON, so replies appear in the console output
instead of next to the command. Commands work only while the server runs.

## Updates

On every start, if `auto_update` is on, the runtime asks the Hytale Downloader for
the latest version on the selected patchline and installs it when it is newer than
the installed one. A failed update keeps the installed version running.
**Check for update** on Advanced restarts a running server so this check runs.

The settings live in `hytale.json` in the server folder:

| Key | Default | Meaning |
| --- | --- | --- |
| `auto_update` | `true` | Check for and install a newer server on each start |
| `patchline` | `release` | Which Hytale patchline the downloader uses |
| `server_password` | not set | The join password from the create dialog |

> [!IMPORTANT]
> Helix reads `hytale.json` when it creates the server's container, not on every
> start. After editing it, save a change that recreates the container, such as
> the memory, the CPU cap, or the ports, so the new values apply.

## Backups

**Back up now** on a running Hytale server stops it, archives the folder, and
starts it again. Live backups without a restart are only for Java Minecraft. A
Hytale backup includes the world (`universe/`), `mods/`, `config.json`,
`hytale.json`, the downloader credentials, and `auth.enc`. It leaves out the Files
trash. See [Backups](https://github.com/Riqqqque/Helix/wiki/Backups) for restore,
keep rules, and the trash.

More detail lives in
[`docs/HYTALE.md`](https://github.com/Riqqqque/Helix/blob/main/docs/HYTALE.md).
