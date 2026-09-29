# Hytale

Helix runs the official Hytale dedicated server in an isolated container on
Java 25, with sign-in handled for you, CurseForge mods, console access, backups,
and automatic updates. Create one from **Servers → New server → Hytale**.

## First start: two sign-ins

Hytale ties every server to a Hytale account, so the first start asks you to sign
in twice. Helix shows each link as soon as Hytale asks for it — in the create
progress and as a **Sign in to Hytale** banner on the server page.

1. **Download.** The official Hytale Downloader signs in to fetch the server
   files. Open the link, sign in, and confirm the code. The download continues on
   its own.
2. **Server.** Once the server boots, Helix runs `/auth login device` for you.
   Sign in again with the new link. Helix then runs
   `/auth persistence Encrypted`, so the server stays signed in across restarts.

Helix only shows links that point to `https://…hytale.com`. Codes expire after a
few minutes; Helix retries the download with a fresh code, or you can run
`/auth login device` from the Console tab yourself.

The downloader credentials and the server's encrypted sign-in stay in the
server's own folder (`.helix/downloader/` and `auth.enc`) and are included in its
backups.

## Letting players join

Hytale uses QUIC, so players connect over **UDP** only, on port 5520 by default.
The Ports card shows the game port as **UDP (QUIC)**. For play outside your
network, forward that UDP port in your router, or choose **Public** when you
create the server so Helix opens the host firewall for it.

Server name, player limit, and password are written into `config.json` at each
start. Other `config.json` settings are left alone; edit them from **Files**
while the server is stopped.

Extra ports for web maps or APIs that a mod opens can be added on the Ports card.
Minecraft presets such as Simple Voice Chat are not offered, because they do not
apply to Hytale.

## Mods

Hytale has one first-party mod API and no loader to install.

- **Mods tab.** Search CurseForge, Hytale's official mod platform, then choose
  **Install**. Helix picks the newest release, installs required dependencies,
  checks every file against CurseForge's SHA-1, and writes it to `mods/`. This
  needs a CurseForge API key in **Settings → Catalogs**.
- **Your own files.** Upload `.jar` plugins or `.zip` packs into `mods/` from the
  Files tab.

Restart the server to load new mods, then check the Logs tab to confirm they
loaded.

## Console

The Console tab sends commands to the server, for example `/who` or
`/auth status`. Hytale has no RCON, so replies appear in the log instead of next
to the command.

## Updates

On every start, Helix asks the Hytale Downloader for the latest release and
updates the server files when a newer one exists. A failed update keeps the
installed version running. To stay on a version, set `"auto_update": false` in
`hytale.json` from Files.

## Backups

Hytale servers use the same backups as every Helix server: **Back up now**, keep
rules, recoverable deletion, and restore. See
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace#backups).

More detail lives in
[`docs/HYTALE.md`](https://github.com/Riqqqque/Helix/blob/main/docs/HYTALE.md).
