# Pumpkin

Pumpkin is a Minecraft server written in Rust. It is not a Java server or a mod
loader, and it speaks both Java and Bedrock. Choose **Servers → New server →
Minecraft → Pumpkin**.

Helix downloads a versioned Pumpkin release from the publisher, checks its
SHA-256 digest and that the executable matches the host's CPU, and runs it as an
unprivileged user in a resource-limited container. No Java is needed. Linux
x86-64 and ARM64 release assets are supported.

## Versions and compatibility

The version list shows published Pumpkin release tags, such as
`0.1.0-dev+26.2-26.45` (Java 26.2 and Bedrock 1.26.45), not Minecraft versions.
Moving nightly builds are left out on purpose. Pumpkin is still under heavy
development; a working server does not mean full vanilla behavior.

Pumpkin has its own plugin API. Paper and Bukkit JARs, and Fabric, Quilt, Forge,
or NeoForge mods and packs, are not Pumpkin add-ons. So Helix offers no
marketplace for Pumpkin, and
[Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers)
refuses Pumpkin servers on either side. Put trusted Pumpkin plugins built for the
exact release, OS, and CPU into the server through Files. PatchBukkit is a
separate compatibility bridge and is not a promise that Bukkit plugins work;
Helix does not install it.

Do not swap an existing world into Pumpkin in place. Test a copy from a backup on
a separate Pumpkin server first. Helix refuses automatic conversion of AMP
servers, worlds, or modpacks to Pumpkin: being able to read a world says nothing
about plugin, block, entity, or gameplay compatibility.

The integration was checked against `0.1.0-dev+26.2-26.45` on Linux x86-64. See
[`docs/PUMPKIN-VALIDATION.md`](https://github.com/Riqqqque/Helix/blob/main/docs/PUMPKIN-VALIDATION.md)
for what was and was not tested.

## Java and Bedrock ports

Java uses the Minecraft TCP port. Bedrock uses NetherNet on a **separate port,
both TCP and UDP**. Set it when you create the server, or leave it blank and Helix
reserves a second free number from the Minecraft port pool. Helix checks both
protocols and existing Helix and AMP claims. RCON is published only on the host's
loopback address, never as a game port.

For LAN play, use the addresses on Overview. For internet play, forward Java TCP
and Bedrock TCP and UDP to the host yourself. Behind NAT, Bedrock also needs
`external_ip` under `[networking.bedrock.nethernet]` in `pumpkin.toml` set to your
public address. CGNAT can still block inbound players. Helix does not change your
router and has not tested a connection from the internet.

Bedrock players sign in with Xbox and get a `.` in front of their name so they do
not collide with Java accounts. Use that prefixed name in whitelist and operator
commands. Older Bedrock protocols are not supported automatically. Geyser is not
needed for Pumpkin's own Bedrock support.

## Controls

Start, Stop, Restart, Kill, start after boot, memory and CPU limits, status,
console history, Files, icons, backups, restore, and removal work like other
Helix servers. See
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace).
A few things differ from Java Minecraft:

- **Ready** means the Java status ping answers and the Bedrock port answers a
  Bedrock ping.
- **Stop** stops the container cleanly with no forced-kill deadline, but Helix
  does not send `save-all flush` first.
- **Memory** is the container's hard limit. There is no Java heap and no extra
  headroom on top of it.

## Settings

The Settings tab edits `pumpkin.toml`. Helix keeps settings it does not know
about, saves a copy of the previous file, and rejects a save if the file changed
since you opened it. Most changes need a restart. The form covers MOTD, player
limit, game mode, difficulty, view and simulation distance, online mode, PvP, and
the whitelist. Changing the Java port or memory recreates the container; the
Bedrock port stays the same. Do not change Helix-managed listener addresses in
Files without matching container changes.

Idle-kick time, allow flight, and spawn protection are disabled in the form,
because this Pumpkin release has no matching settings. Use Files for anything
else. The settings editor may rewrite TOML comments and formatting; the previous
file is kept as its backup copy. For the in-game server icon use Pumpkin's
`icon.png`; the server card picture in Helix is separate.

## Backups

Pumpkin servers are not backed up live. **Back up now** on a running Pumpkin
server stops it, archives the folder, and starts it again. Backups leave out the
same regenerable folders as Java Minecraft (`logs/`, `crash-reports/`, `cache/`,
and the Files trash). See
[Backups](https://github.com/Riqqqque/Helix/wiki/Backups).

## Updates and recovery

**Advanced → Server software** offers two choices for Pumpkin:

- **Change release** moves to a newer published release.
- **Repair files** downloads the exact release you run again.

Helix downloads and verifies the release before touching the server. A release
whose files match what you run already changes nothing. Otherwise Helix stops the
server, makes a full backup, swaps the executable, and recreates the container. A
running server must pass the startup check, or Helix restores the full backup,
including the world and settings. A stopped server stays stopped and is not
checked until you start it.

A release with an older Java Minecraft version than the one you run is refused,
like any Minecraft downgrade. A newer one is allowed, so test it on a copy first:
Helix does not convert worlds, install nightlies, or promise plugin
compatibility.

## References

- [Pumpkin releases](https://github.com/Pumpkin-MC/Pumpkin/releases)
- [Configuration](https://docs.pumpkinmc.org/config/basic)
- [Bedrock configuration in the checked release](https://github.com/Pumpkin-MC/Pumpkin/blob/0.1.0-dev%2B26.2-26.45/crates/pumpkin-config/src/networking/bedrock.rs)
- [Plugin development and API stability](https://docs.pumpkinmc.org/plugin-dev/introduction)
