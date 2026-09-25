# Hytale servers

Helix runs the official Hytale dedicated server in an isolated container on
Java 25. Create one from **Servers → New server → Hytale**.

## First setup: two sign-ins

Hytale ties servers to a Hytale account, so the first start asks you to sign
in twice. Helix shows each link as soon as Hytale asks for it — in the create
progress and as a **Sign in to Hytale** banner on the server page.

1. **Download.** The official Hytale Downloader signs in to fetch the server
   files. Open the link, sign in, and confirm the code. The download then
   continues on its own.
2. **Server.** Once the server boots, Helix runs `/auth login device` for you.
   Sign in again with the new link. Helix then runs
   `/auth persistence Encrypted`, so the server stays signed in across restarts.

Links are only shown when they are `https://…hytale.com` addresses. Codes
expire after a few minutes; if one does, Helix retries the download with a fresh
code, or you can run `/auth login device` from the Console tab.

The downloader credentials and the server's encrypted sign-in stay in the
server's data folder (`.helix/downloader/` and `auth.enc`). They are included in
Helix backups.

## Playing

Hytale uses QUIC, so players connect over **UDP** only (default port 5520). For
play outside your network, forward that UDP port or choose **Public** when
creating the server.

Server name, player limit and password are written into `config.json` at each
start. Other `config.json` settings are left as they are; edit them from Files
while the server is stopped.

## Mods

Hytale has one first-party mod API and no loader to install.

- **Mods tab.** Search CurseForge (Hytale's official mod platform), then
  Install. Helix picks the newest release, installs required dependencies, checks
  every file against CurseForge's SHA-1, and writes it to `mods/`. It needs a
  CurseForge API key in **Settings → Catalogs**.
- **Your own files.** Upload `.jar` plugins or `.zip` packs into `mods/` from
  the Files tab.

Restart the server to load new mods, then check the Logs tab to confirm they
loaded.

## Console

The Console tab sends commands to the server (for example `/who`,
`/auth status`). Hytale has no RCON, so the reply appears in the log rather
than next to the command.

## Updates

On every start Helix asks the Hytale Downloader for the latest release and
updates the server files if a newer one exists. A failed update keeps the
installed version running. To pin a version, set `"auto_update": false` in
`hytale.json` from Files.
