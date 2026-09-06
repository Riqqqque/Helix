# Pumpkin

Choose **Servers → New server → Minecraft → Pumpkin**. Pumpkin is a native Rust
server, not a Java server or mod loader. Helix downloads a versioned publisher
release, verifies its SHA-256 digest and Linux executable architecture, and runs
it as an unprivileged user in a resource-limited container. No Java installation
is needed. Linux x86-64 and ARM64 release assets are supported.

## Versions and compatibility

The selector lists Pumpkin release tags, not arbitrary Minecraft versions.
The initial integration targets `0.1.0-dev+26.2-26.45`: Java 26.2 and Bedrock
1.26.45. Moving nightly builds are deliberately excluded. The project is still
under heavy development; a working server does not mean complete vanilla parity.

Pumpkin has its own plugin API. Paper/Bukkit JARs and Fabric, Quilt, Forge, or
NeoForge mods and packs are not native Pumpkin add-ons. Helix therefore does not
offer its Java plugin/mod marketplace for Pumpkin. Use **Files** for trusted
Pumpkin plugins compiled for the exact release, OS, and CPU architecture.
PatchBukkit is a separate compatibility bridge, not a blanket guarantee that
existing Bukkit plugins work. Helix does not install that bridge automatically.

Do not replace an existing server's world in place. Test a copy from a backup
on a separate Pumpkin server first. Automatic AMP/world/modpack conversion to
Pumpkin is intentionally rejected; world-format support is not proof of plugin,
block, entity, or gameplay compatibility.

## Java and Bedrock ports

Java uses the selected Minecraft TCP port. This release's Bedrock transport is
NetherNet, with a **separate TCP and UDP port**. Set that port during creation,
or leave it blank to reserve a second free number from the Minecraft port pool.
Helix checks both protocols and existing Helix/AMP claims. RCON is separate and
published only on host loopback, never as a public game port.

For LAN play, use the addresses shown on Overview. For internet play, manually
forward Java TCP and Bedrock TCP/UDP to this host. Behind NAT, Bedrock also needs
`external_ip` under `[networking.bedrock.nethernet]` in `pumpkin.toml` set to the
correct public IP. CGNAT can still prevent inbound access. Helix does not change
routers or claim to have tested a connection from the internet.

Bedrock accounts keep Xbox authentication enabled and use a `.` name prefix to
avoid collisions with Java accounts. Whitelist/operator commands must use the
actual prefixed Bedrock name. Older Bedrock protocols are not automatically
supported, and Geyser is not required for Pumpkin's native transport.

## Controls and settings

Start, stop, restart, force-stop, start-on-boot, CPU/memory limits, status,
persistent console history, file management, icons, backups, restore, and server
removal use Helix's normal native-server controls. The memory selection is a
container limit, not a JVM heap reservation.

Settings edit `pumpkin.toml`, preserve unknown configuration values, keep a copy
of the previous file, and reject stale edits. Most changes require a restart.
Changing the Java game port or memory recreates the container with its updated
limits/publication; the reserved Bedrock port stays unchanged. Do not change
managed listener addresses in Files without matching container publication.

Idle-kick time, allow-flight, and spawn protection are disabled in the standard
settings form because this release does not expose matching configuration fields.
Use Files for other Pumpkin settings. TOML formatting/comments may be rewritten
by the structured settings editor; the original file is retained in its backup.
For the in-game favicon use Pumpkin's `icon.png`; Helix's server-card image is
separate. The console uses private RCON and survives closing the dashboard.

## Updates and recovery

**Update** checks the latest versioned publisher release. An unchanged checksum
does nothing. A changed build is downloaded and verified before stopping the
server; Helix then makes a full stopped-server backup before activation. A
running server must pass startup checks or its previous binary is restored.
The full backup remains available from **Backups → Restore** for world/config
recovery. A stopped server stays stopped; its later startup is not prevalidated.

Updates that change the Java Minecraft version are blocked. Test that new
release separately before migrating a backup. Helix does not silently change
world versions, install nightlies, or promise third-party plugin compatibility.

## References

- [Pumpkin releases](https://github.com/Pumpkin-MC/Pumpkin/releases)
- [Configuration](https://docs.pumpkinmc.org/config/basic)
- [Pinned release Bedrock configuration](https://github.com/Pumpkin-MC/Pumpkin/blob/0.1.0-dev%2B26.2-26.45/crates/pumpkin-config/src/networking/bedrock.rs)
- [Plugin development and API stability](https://docs.pumpkinmc.org/plugin-dev/introduction)
