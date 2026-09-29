//! Copies chosen content from one Helix Minecraft server onto another, for example
//! promoting a finished test setup onto the public server.
//!
//! The source is packed inside a throwaway container that runs as the source's own
//! user with no network, so links planted in the source can never reach host files.
//! The target gets a verified backup, is stopped, patched with symlink-safe writes,
//! restarted, and checked; any failure restores the backup.

use super::*;
use helix_privd::{ServerTransferSpec, TransferPart, validate_transfer_parts};
use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::OwnedFd;

/// Largest packed transfer; worlds can be big, but this keeps a mistake from filling the disk.
const MAX_TRANSFER_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_TRANSFER_FILES: usize = 400_000;
const MAX_TRANSFER_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_JAR_METADATA_BYTES: u64 = 256 * 1024;
const MAX_LISTED_ITEMS: usize = 40;

/// A staged file: path relative to the server folder, its staged copy, and its size.
type StagedFile = (String, PathBuf, u64);
type StagedPart = (TransferPart, Vec<StagedFile>);

/// Keys that describe *which* server this is. A transfer never changes them on the target.
const TARGET_IDENTITY_KEYS: &[&str] = &[
    "server-ip",
    "server-port",
    "query.port",
    "rcon.port",
    "rcon.password",
    "enable-rcon",
    "enable-query",
    "level-name",
    "level-seed",
    "max-players",
    "motd",
    "white-list",
    "enforce-whitelist",
];

/// Plugin data never copied by the default Plugins part: databases, logs, caches, and
/// per-player files that would overwrite the target's real players.
const PLUGIN_DATA_DIRS: &[&str] = &[
    "userdata",
    "playerdata",
    "players",
    "data",
    "logs",
    "cache",
    "web",
    "backups",
    "backup",
    "tiles",
    "maps",
    "database",
    "storage",
    "libs",
    "lib",
    "tmp",
    "temp",
];
const PLUGIN_DATA_FILES: &[&str] = &[
    "*.db",
    "*.db-journal",
    "*.db-wal",
    "*.db-shm",
    "*.sqlite",
    "*.sqlite3",
    "*.sqlite-*",
    "*.mv.db",
    "*.trace.db",
    "*.log",
    "*.log.gz",
    "*.lck",
    "*.lock",
];
const CONFIG_FILES: &[&str] = &[
    "bukkit.yml",
    "spigot.yml",
    "commands.yml",
    "permissions.yml",
    "help.yml",
    "paper.yml",
    "purpur.yml",
    "pufferfish.yml",
    "leaves.yml",
];
const PLAYER_LIST_FILES: &[&str] = &[
    "whitelist.json",
    "ops.json",
    "banned-players.json",
    "banned-ips.json",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Family {
    Plugins,
    Fabric,
    Forge,
    NeoForge,
    Vanilla,
    Custom,
    Pumpkin,
}

fn family(software: MinecraftSoftware) -> Family {
    match software {
        MinecraftSoftware::Paper
        | MinecraftSoftware::Purpur
        | MinecraftSoftware::Folia
        | MinecraftSoftware::Leaves
        | MinecraftSoftware::Pufferfish => Family::Plugins,
        MinecraftSoftware::Fabric | MinecraftSoftware::Quilt => Family::Fabric,
        MinecraftSoftware::Forge => Family::Forge,
        MinecraftSoftware::NeoForge => Family::NeoForge,
        MinecraftSoftware::Vanilla => Family::Vanilla,
        MinecraftSoftware::Custom => Family::Custom,
        MinecraftSoftware::Pumpkin => Family::Pumpkin,
    }
}

fn part_name(part: TransferPart) -> &'static str {
    match part {
        TransferPart::Plugins => "plugins",
        TransferPart::PluginData => "plugin_data",
        TransferPart::Mods => "mods",
        TransferPart::Configs => "configs",
        TransferPart::ServerProperties => "server_properties",
        TransferPart::Datapacks => "datapacks",
        TransferPart::PlayerLists => "player_lists",
        TransferPart::Worlds => "worlds",
    }
}

/// Errors block the transfer; warnings are shown and the transfer may continue.
fn compatibility(
    source: &InstanceManifest,
    target: &InstanceManifest,
    parts: &[TransferPart],
) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if source.id == target.id {
        errors.push("choose a different server to copy to".to_owned());
    }
    if !source.is_minecraft() || !target.is_minecraft() {
        errors.push("transfers work between Helix Minecraft servers".to_owned());
        return (errors, warnings);
    }
    let (from, to) = (family(source.software), family(target.software));
    if from == Family::Pumpkin || to == Family::Pumpkin {
        errors.push(
            "Pumpkin servers use their own plugin format and cannot take part in a transfer"
                .to_owned(),
        );
    }
    let plugin_ready = |f: Family| matches!(f, Family::Plugins | Family::Custom);
    for part in parts {
        match part {
            TransferPart::Plugins | TransferPart::PluginData
                if !plugin_ready(from) || !plugin_ready(to) =>
            {
                errors.push(format!(
                    "plugins only move between plugin servers such as Paper; {} → {} cannot",
                    software_name(source.software),
                    software_name(target.software)
                ));
            }
            TransferPart::Mods if from != to && from != Family::Custom && to != Family::Custom => {
                errors.push(format!(
                    "{} mods do not load on {}",
                    software_name(source.software),
                    software_name(target.software)
                ));
            }
            TransferPart::Configs
                if from != to && from != Family::Custom && to != Family::Custom =>
            {
                errors.push(format!(
                    "{} configuration does not apply to {}",
                    software_name(source.software),
                    software_name(target.software)
                ));
            }
            TransferPart::Worlds | TransferPart::Datapacks
                if migrate_plan::minecraft_version_is_older(
                    &target.minecraft_version,
                    &source.minecraft_version,
                ) =>
            {
                errors.push(format!(
                    "the target runs Minecraft {}, older than the source's {}; worlds and datapacks would be downgraded. Update the target first",
                    target.minecraft_version, source.minecraft_version
                ));
            }
            _ => {}
        }
    }
    if source.minecraft_version != target.minecraft_version {
        warnings.push(format!(
            "the source runs Minecraft {} and the target {}; check that every plugin or mod supports the target version",
            source.minecraft_version, target.minecraft_version
        ));
    }
    if parts.contains(&TransferPart::PluginData) {
        warnings.push("Plugin data includes databases and player files; it replaces the target's copies, for example its CoreProtect history or LuckPerms data".to_owned());
    }
    if parts.contains(&TransferPart::Worlds) {
        warnings.push("Worlds replace the target's worlds completely. The target's backup is the only copy of its current worlds".to_owned());
    }
    if parts.contains(&TransferPart::PlayerLists) {
        warnings
            .push("Player lists replace the target's whitelist, operators, and bans".to_owned());
    }
    errors.dedup();
    (errors, warnings)
}

/// Shell script run inside the pack container. Paths and patterns are fixed text;
/// the only input is the validated level name passed as `$LEVEL`.
fn find_script(part: TransferPart) -> String {
    let prune_dirs = PLUGIN_DATA_DIRS
        .iter()
        .map(|dir| format!("-path 'plugins/*/{dir}'"))
        .collect::<Vec<_>>()
        .join(" -o ");
    let skip_files = PLUGIN_DATA_FILES
        .iter()
        .map(|pattern| format!("! -name '{pattern}'"))
        .collect::<Vec<_>>()
        .join(" ");
    match part {
        TransferPart::Plugins => format!(
            "find plugins \\( -path 'plugins/.paper-remapped' -o {prune_dirs} \\) -prune -o -type f {skip_files} -print0"
        ),
        TransferPart::PluginData => {
            "find plugins -path 'plugins/.paper-remapped' -prune -o -type f -print0".to_owned()
        }
        TransferPart::Mods => "find mods -maxdepth 1 -type f -name '*.jar' -print0".to_owned(),
        TransferPart::Configs => format!(
            "find {} config defaultconfigs -type f ! -name '*.log' -print0",
            CONFIG_FILES.join(" ")
        ),
        TransferPart::ServerProperties => "find server.properties -maxdepth 0 -type f -print0".to_owned(),
        TransferPart::Datapacks => "find \"$LEVEL/datapacks\" -type f -print0".to_owned(),
        TransferPart::PlayerLists => format!(
            "find {} -maxdepth 0 -type f -print0",
            PLAYER_LIST_FILES.join(" ")
        ),
        TransferPart::Worlds => "find \"$LEVEL\" \"${LEVEL}_nether\" \"${LEVEL}_the_end\" -type f ! -name session.lock -print0".to_owned(),
    }
}

fn valid_level_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn property(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        (name.trim() == key).then(|| value.trim().to_owned())
    })
}

/// Source gameplay settings on top of the target's file, keeping the target's identity keys.
fn merge_transfer_properties(target: &str, source: &str) -> String {
    let mut values = BTreeMap::new();
    let mut read = |text: &str, keep_identity: bool| {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if key.is_empty()
                || key.len() > 128
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            {
                continue;
            }
            if !keep_identity && TARGET_IDENTITY_KEYS.contains(&key) {
                continue;
            }
            let value: String = value
                .trim()
                .chars()
                .filter(|ch| !matches!(ch, '\0' | '\n' | '\r'))
                .take(512)
                .collect();
            values.insert(key.to_owned(), value);
        }
    };
    read(target, true);
    read(source, false);
    let mut body =
        String::from("# Managed by Helix; gameplay settings copied from another server\n");
    for (key, value) in values {
        body.push_str(&key);
        body.push('=');
        body.push_str(&value);
        body.push('\n');
    }
    body
}

/// The name a plugin or mod declares inside its JAR, so a newer build replaces an older one.
fn jar_identity(file: File) -> Option<String> {
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut text = |name: &str| -> Option<String> {
        let entry = archive.by_name(name).ok()?;
        if entry.size() > MAX_JAR_METADATA_BYTES {
            return None;
        }
        let mut body = String::new();
        entry
            .take(MAX_JAR_METADATA_BYTES)
            .read_to_string(&mut body)
            .ok()?;
        Some(body)
    };
    if let Some(yaml) = text("paper-plugin.yml").or_else(|| text("plugin.yml")) {
        return yaml_plugin_name(&yaml).map(|name| format!("plugin:{}", name.to_ascii_lowercase()));
    }
    if let Some(json) = text("fabric.mod.json") {
        return serde_json::from_str::<Value>(&json)
            .ok()?
            .get("id")?
            .as_str()
            .map(|id| format!("mod:{}", id.to_ascii_lowercase()));
    }
    if let Some(json) = text("quilt.mod.json") {
        return serde_json::from_str::<Value>(&json)
            .ok()?
            .pointer("/quilt_loader/id")?
            .as_str()
            .map(|id| format!("mod:{}", id.to_ascii_lowercase()));
    }
    let toml = text("META-INF/neoforge.mods.toml").or_else(|| text("META-INF/mods.toml"))?;
    toml_mod_id(&toml).map(|id| format!("mod:{}", id.to_ascii_lowercase()))
}

fn yaml_plugin_name(yaml: &str) -> Option<String> {
    yaml.lines().find_map(|line| {
        let value = line.strip_prefix("name:")?;
        let value = value.split('#').next()?.trim().trim_matches(['"', '\'']);
        (!value.is_empty() && value.len() <= 128).then(|| value.to_owned())
    })
}

fn toml_mod_id(toml: &str) -> Option<String> {
    toml.lines()
        .find_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            (key.trim() == "modId").then(|| value.trim().trim_matches(['"', '\'']).to_owned())
        })
        .filter(|id| !id.is_empty() && id.len() <= 128)
}

/// Opens (creating when missing) each directory of `relative` beneath `root`, never
/// following a link. The target is stopped while this runs.
fn open_dir_beneath(root: &OwnedFd, components: &[&str], create: bool) -> Result<OwnedFd, String> {
    let mut current = rustix::fs::openat(
        root,
        ".",
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| "the target server folder is unavailable".to_owned())?;
    for component in components {
        let open = |dir: &OwnedFd| {
            rustix::fs::openat(
                dir,
                *component,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
        };
        current = match open(&current) {
            Ok(next) => next,
            Err(rustix::io::Errno::NOENT) if create => {
                rustix::fs::mkdirat(&current, *component, rustix::fs::Mode::from_raw_mode(0o750))
                    .map_err(|_| format!("could not create {component} on the target"))?;
                open(&current).map_err(|_| format!("could not open {component} on the target"))?
            }
            Err(_) => {
                return Err(format!(
                    "{component} on the target is missing or is a link; nothing was written through it"
                ));
            }
        };
    }
    Ok(current)
}

fn safe_components(relative: &str) -> Option<Vec<&str>> {
    let parts = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    (!parts.is_empty()
        && parts
            .iter()
            .all(|part| *part != "." && *part != ".." && !part.contains('\0') && part.len() <= 255))
    .then_some(parts)
}

/// Regular files extracted for one part, relative to the source server folder.
fn staged_files(root: &Path) -> Result<Vec<StagedFile>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in
            fs::read_dir(&directory).map_err(|_| "could not read the staged copy".to_owned())?
        {
            let entry = entry.map_err(|_| "could not read the staged copy".to_owned())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| "could not inspect the staged copy".to_owned())?;
            if metadata.file_type().is_dir() {
                stack.push(path);
            } else if metadata.file_type().is_file() {
                let relative = path
                    .strip_prefix(root)
                    .ok()
                    .and_then(Path::to_str)
                    .ok_or_else(|| "a staged file has an unusable name".to_owned())?
                    .to_owned();
                if safe_components(&relative).is_some() {
                    files.push((relative, path, metadata.len()));
                }
            } else {
                // Links, sockets, and devices never reach the target.
                let _ = fs::remove_file(&path);
            }
            if files.len() > MAX_TRANSFER_FILES {
                return Err(format!(
                    "more than {MAX_TRANSFER_FILES} files; copy fewer parts at once"
                ));
            }
        }
    }
    files.sort();
    Ok(files)
}

impl NativeManager {
    fn transfer_manifests(
        &self,
        source_id: &str,
        target_id: &str,
    ) -> Result<(InstanceManifest, InstanceManifest), String> {
        let source = self.load_manifest(source_id.strip_prefix("helix:").unwrap_or(source_id))?;
        let target = self
            .load_manifest(target_id.strip_prefix("helix:").unwrap_or(target_id))
            .map_err(|_| "the target must be a Helix-managed server; AMP and imported servers cannot receive transfers".to_owned())?;
        Ok((source, target))
    }

    fn level_name(&self, manifest: &InstanceManifest) -> String {
        self.instance_path(&manifest.id)
            .ok()
            .and_then(|data| {
                read_small_regular_file(
                    &data.join("server.properties"),
                    MAX_PROPERTIES_BYTES,
                    "server settings",
                )
                .ok()
            })
            .and_then(|text| property(&text, "level-name"))
            .filter(|name| valid_level_name(name))
            .unwrap_or_else(|| "world".to_owned())
    }

    /// Runs `find` for one part inside a locked-down container as the source's user and
    /// either lists sizes or writes a tar of regular files into `out`.
    fn pack_part(
        &self,
        source: &InstanceManifest,
        part: TransferPart,
        level: &str,
        out: Option<&Path>,
    ) -> Result<String, String> {
        let data = self.instance_path(&source.id)?;
        let find = find_script(part);
        let script = match out {
            Some(_) => format!(
                "cd /data && {{ {find} 2>/dev/null; true; }} | tar --null --no-recursion -T - -cf /out/part.tar"
            ),
            None => format!(
                "cd /data && {{ {} 2>/dev/null; true; }} | tr '\\0' '\\n' | head -c 8388608",
                find.replace("-print0", "-printf '%s %p\\0'")
            ),
        };
        let mut args = vec![
            "run".to_owned(),
            "--rm".to_owned(),
            "--user".to_owned(),
            format!("{0}:{0}", source.run_uid),
            "--network".to_owned(),
            "none".to_owned(),
            "--read-only".to_owned(),
            "--cap-drop".to_owned(),
            "ALL".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges".to_owned(),
            "--pids-limit".to_owned(),
            "64".to_owned(),
            "--memory".to_owned(),
            "512m".to_owned(),
            "--mount".to_owned(),
            format!("type=bind,src={},dst=/data,readonly", data.display()),
            "--env".to_owned(),
            format!("LEVEL={level}"),
        ];
        if let Some(out) = out {
            args.push("--mount".to_owned());
            args.push(format!("type=bind,src={},dst=/out", out.display()));
        }
        args.extend([
            "--entrypoint".to_owned(),
            "sh".to_owned(),
            source.runtime_image.clone(),
            "-c".to_owned(),
            script,
        ]);
        self.docker_owned(&args, 60 * 60)
    }

    pub fn transfer_preflight(
        &self,
        source_id: &str,
        target_id: &str,
        parts: &[TransferPart],
    ) -> Result<Value, String> {
        validate_transfer_parts(parts)?;
        let (source, target) = self.transfer_manifests(source_id, target_id)?;
        let (errors, warnings) = compatibility(&source, &target, parts);
        let level = self.level_name(&source);
        let mut summary = serde_json::Map::new();
        let mut total_bytes = 0_u64;
        if errors.is_empty() {
            for part in parts {
                let listing = self.pack_part(&source, *part, &level, None)?;
                let (mut files, mut bytes) = (0_u64, 0_u64);
                let mut items = BTreeSet::new();
                for line in listing.lines() {
                    let Some((size, path)) = line.split_once(' ') else {
                        continue;
                    };
                    files += 1;
                    bytes = bytes.saturating_add(size.parse::<u64>().unwrap_or(0));
                    let shown = match part {
                        TransferPart::Plugins | TransferPart::PluginData | TransferPart::Mods => {
                            path.split('/').nth(1).unwrap_or(path)
                        }
                        _ => path,
                    };
                    if items.len() < MAX_LISTED_ITEMS {
                        items.insert(shown.to_owned());
                    }
                }
                total_bytes = total_bytes.saturating_add(bytes);
                summary.insert(
                    part_name(*part).to_owned(),
                    json!({"files": files, "bytes": bytes, "items": items}),
                );
            }
        }
        Ok(json!({
            "source": {"id": format!("helix:{}", source.id), "name": source.name, "software": source.software, "minecraft_version": source.minecraft_version},
            "target": {"id": format!("helix:{}", target.id), "name": target.name, "software": target.software, "minecraft_version": target.minecraft_version, "running": self.container_running(&target.container_name)},
            "parts": summary,
            "total_bytes": total_bytes,
            "errors": errors,
            "warnings": warnings,
            "notes": [
                "The source is only read; nothing on it changes.",
                "The target gets a full backup first, stops while files are copied, and starts again if it was running. If it does not start, Helix restores the backup.",
                "Files are added or replaced; nothing else on the target is deleted. A newer plugin or mod JAR replaces the older JAR with the same plugin or mod name.",
                "The target keeps its ports, RCON, world name, seed, player limit, MOTD, and whitelist setting."
            ]
        }))
    }

    pub fn transfer_content<F>(
        &self,
        source_id: &str,
        spec: &ServerTransferSpec,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        spec.validate()?;
        let (source, target) = self.transfer_manifests(source_id, &spec.target_id)?;
        if spec.confirmation_name != target.name {
            return Err("type the target server's exact name to confirm".to_owned());
        }
        let (errors, mut warnings) = compatibility(&source, &target, &spec.parts);
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        let _source_lock = self.begin_instance_operation(&source.id, "transfer")?;
        let _target_lock = self.begin_instance_operation(&target.id, "transfer")?;
        let source_level = self.level_name(&source);
        let target_level = self.level_name(&target);
        let stage = self
            .instance_root
            .join(".staging")
            .join(format!("transfer-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&stage)
            .map_err(|_| "could not create the transfer staging folder".to_owned())?;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the transfer staging folder".to_owned())?;
        let result = self.transfer_staged(
            &source,
            &target,
            spec,
            &source_level,
            &target_level,
            &stage,
            &mut warnings,
            &mut progress,
        );
        let _ = fs::remove_dir_all(&stage);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn transfer_staged<F>(
        &self,
        source: &InstanceManifest,
        target: &InstanceManifest,
        spec: &ServerTransferSpec,
        source_level: &str,
        target_level: &str,
        stage: &Path,
        warnings: &mut Vec<String>,
        progress: &mut F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        // 1. Pack and unpack each part into the broker-only staging folder.
        let mut staged: Vec<StagedPart> = Vec::new();
        let mut total = 0_u64;
        let count = u8::try_from(spec.parts.len()).unwrap_or(8).max(1);
        for (index, part) in spec.parts.iter().enumerate() {
            let step = u8::try_from(index).unwrap_or(0);
            progress(
                &format!(
                    "Reading {} from {}",
                    part_name(*part).replace('_', " "),
                    source.name
                ),
                5 + step * 30 / count,
            );
            let out = stage.join(format!("out-{}", part_name(*part)));
            let unpacked = stage.join(format!("files-{}", part_name(*part)));
            fs::create_dir(&out)
                .map_err(|_| "could not create the transfer staging folder".to_owned())?;
            fs::create_dir(&unpacked)
                .map_err(|_| "could not create the transfer staging folder".to_owned())?;
            fs::set_permissions(&unpacked, fs::Permissions::from_mode(0o700))
                .map_err(|_| "could not protect the transfer staging folder".to_owned())?;
            self.chown_instance(&out, source.run_uid)?;
            self.pack_part(source, *part, source_level, Some(&out))?;
            let archive = out.join("part.tar");
            let size = fs::symlink_metadata(&archive)
                .ok()
                .filter(|metadata| metadata.file_type().is_file())
                .map(|metadata| metadata.len())
                .ok_or_else(|| format!("could not read {} from the source", part_name(*part)))?;
            total = total.saturating_add(size);
            if total > MAX_TRANSFER_BYTES {
                return Err(
                    "this transfer is larger than 64 GiB; copy fewer parts at once".to_owned(),
                );
            }
            let free = rustix::fs::statvfs(&self.instance_root)
                .map(|stats| stats.f_bavail.saturating_mul(stats.f_frsize))
                .unwrap_or(0);
            if free < size.saturating_mul(2).saturating_add(1024 * 1024 * 1024) {
                return Err("there is not enough free disk space for this transfer and the target's safety backup".to_owned());
            }
            run_program(
                Path::new("/usr/bin/tar"),
                &[
                    "--extract".to_owned(),
                    "--file".to_owned(),
                    archive.to_string_lossy().into_owned(),
                    "--directory".to_owned(),
                    unpacked.to_string_lossy().into_owned(),
                    "--no-same-owner".to_owned(),
                    "--no-same-permissions".to_owned(),
                ],
                60 * 60,
            )?;
            let _ = fs::remove_dir_all(&out);
            let files = staged_files(&unpacked)?;
            if let Some((relative, _, _)) = files
                .iter()
                .find(|(_, _, size)| *size > MAX_TRANSFER_FILE_BYTES)
            {
                return Err(format!(
                    "{relative} is larger than 1 GiB and was not copied"
                ));
            }
            staged.push((*part, files));
        }
        let copied_files: usize = staged.iter().map(|(_, files)| files.len()).sum();
        if copied_files == 0 {
            return Err("the source has nothing to copy for the parts you chose".to_owned());
        }

        // 2. Safety backup, then stop the target.
        let data = self.instance_path(&target.id)?;
        let running = self.runtime_running_checked(target)?;
        progress(&format!("Backing up {}", target.name), 40);
        let backup = self.archive_data(target)?;
        progress(&format!("Stopping {}", target.name), 52);
        if let Err(error) = self.stop_runtime_for_files(target) {
            return Err(format!("{error}; nothing on the target changed"));
        }

        // 3. Apply, restoring the backup on any failure.
        progress(&format!("Copying into {}", target.name), 60);
        let applied = self.apply_transfer(target, &data, spec, &staged, source_level, target_level);
        let outcome = applied.and_then(|report| {
            self.chown_instance(&data, target.run_uid)?;
            self.protect_instance_artifacts(&data, target.run_uid)?;
            if running {
                progress(&format!("Starting {} and checking it", target.name), 85);
                self.restart_if_previously_running(target, true)?;
            }
            Ok(report)
        });
        match outcome {
            Ok(mut report) => {
                warnings.append(&mut report.warnings);
                let parts = staged
                    .iter()
                    .map(|(part, files)| {
                        (
                            part_name(*part).to_owned(),
                            json!({"files": files.len(), "bytes": files.iter().map(|(_, _, size)| size).sum::<u64>()}),
                        )
                    })
                    .collect::<serde_json::Map<_, _>>();
                Ok(json!({
                    "source_id": format!("helix:{}", source.id),
                    "target_id": format!("helix:{}", target.id),
                    "backup_id": backup_id_from_path(&backup),
                    "parts": parts,
                    "files_copied": copied_files,
                    "replaced_jars": report.replaced,
                    "removed_jars": report.removed,
                    "restarted": running,
                    "warnings": warnings,
                }))
            }
            Err(error) => {
                progress("Restoring the target's backup", 95);
                let restore = (|| {
                    self.stop_runtime_for_files(target)?;
                    self.restore_modpack_safety_backup(target, &data, &backup)?;
                    self.restart_if_previously_running(target, running)
                })();
                Err(match restore {
                    Ok(()) => format!(
                        "The transfer failed: {error}. {} was restored from its backup.",
                        target.name
                    ),
                    Err(restore) => format!(
                        "The transfer failed: {error}. Restoring the backup also needs attention: {restore}. Backup {} is in {}'s Backups.",
                        backup_id_from_path(&backup),
                        target.name
                    ),
                })
            }
        }
    }

    fn apply_transfer(
        &self,
        target: &InstanceManifest,
        data: &Path,
        spec: &ServerTransferSpec,
        staged: &[StagedPart],
        source_level: &str,
        target_level: &str,
    ) -> Result<ApplyReport, String> {
        let root = rustix::fs::open(
            data,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| "the target server folder is unavailable".to_owned())?;
        let mut report = ApplyReport::default();
        let rename_level = |relative: &str| -> String {
            for suffix in ["_nether", "_the_end", ""] {
                let from = format!("{source_level}{suffix}");
                if let Some(rest) = relative.strip_prefix(&format!("{from}/")) {
                    return format!("{target_level}{suffix}/{rest}");
                }
            }
            relative.to_owned()
        };
        for (part, files) in staged {
            match part {
                TransferPart::Worlds => {
                    let aside = format!(".helix-transfer-old-{}", Uuid::new_v4().simple());
                    let mut moved = false;
                    for suffix in ["", "_nether", "_the_end"] {
                        let name = format!("{target_level}{suffix}");
                        if files.iter().any(|(relative, _, _)| {
                            rename_level(relative).starts_with(&format!("{name}/"))
                        }) {
                            if !moved {
                                rustix::fs::mkdirat(
                                    &root,
                                    aside.as_str(),
                                    rustix::fs::Mode::from_raw_mode(0o700),
                                )
                                .map_err(|_| {
                                    "could not set the target's old worlds aside".to_owned()
                                })?;
                                moved = true;
                            }
                            match rustix::fs::renameat(
                                &root,
                                name.as_str(),
                                &root,
                                format!("{aside}/{name}").as_str(),
                            ) {
                                Ok(()) | Err(rustix::io::Errno::NOENT) => {}
                                Err(_) => {
                                    return Err(format!("could not set the target's {name} aside"));
                                }
                            }
                        }
                    }
                    self.write_staged(&root, files, rename_level, target.run_uid)?;
                    if moved {
                        let _ = fs::remove_dir_all(data.join(&aside));
                    }
                }
                TransferPart::Datapacks => {
                    self.write_staged(&root, files, rename_level, target.run_uid)?;
                }
                TransferPart::ServerProperties => {
                    let Some((_, path, _)) = files.first() else {
                        continue;
                    };
                    let source_text = read_small_regular_file(
                        path,
                        MAX_PROPERTIES_BYTES,
                        "copied server settings",
                    )?;
                    let target_text = read_small_regular_file(
                        &data.join("server.properties"),
                        MAX_PROPERTIES_BYTES,
                        "target server settings",
                    )
                    .unwrap_or_default();
                    let merged = merge_transfer_properties(&target_text, &source_text);
                    write_managed_file_at(
                        &root,
                        std::ffi::OsStr::new("server.properties"),
                        merged.as_bytes(),
                        0o640,
                        target.run_uid,
                        target.run_uid,
                    )?;
                }
                TransferPart::Plugins | TransferPart::PluginData | TransferPart::Mods => {
                    let folder = if *part == TransferPart::Mods {
                        "mods"
                    } else {
                        "plugins"
                    };
                    self.replace_matching_jars(
                        &root,
                        folder,
                        files,
                        spec.remove_missing_jars,
                        &mut report,
                    )?;
                    self.write_staged(
                        &root,
                        files,
                        |relative| relative.to_owned(),
                        target.run_uid,
                    )?;
                }
                TransferPart::Configs | TransferPart::PlayerLists => {
                    self.write_staged(
                        &root,
                        files,
                        |relative| relative.to_owned(),
                        target.run_uid,
                    )?;
                }
            }
        }
        Ok(report)
    }

    fn write_staged(
        &self,
        root: &OwnedFd,
        files: &[StagedFile],
        rename: impl Fn(&str) -> String,
        run_uid: u32,
    ) -> Result<(), String> {
        for (relative, path, _) in files {
            let destination = rename(relative);
            let components = safe_components(&destination)
                .ok_or_else(|| format!("{destination} has an unsafe name"))?;
            let (name, folders) = components
                .split_last()
                .ok_or_else(|| "empty path".to_owned())?;
            let directory = open_dir_beneath(root, folders, true)?;
            let content =
                fs::read(path).map_err(|_| format!("could not read the staged {relative}"))?;
            write_managed_file_at(
                &directory,
                std::ffi::OsStr::new(name),
                &content,
                0o640,
                run_uid,
                run_uid,
            )
            .map_err(|error| format!("{destination}: {error}"))?;
        }
        Ok(())
    }

    /// Removes target JARs that the incoming JARs replace (same plugin or mod name,
    /// different file name) and, when asked, JARs the source no longer has.
    fn replace_matching_jars(
        &self,
        root: &OwnedFd,
        folder: &str,
        files: &[StagedFile],
        remove_missing: bool,
        report: &mut ApplyReport,
    ) -> Result<(), String> {
        let incoming = files
            .iter()
            .filter_map(|(relative, path, _)| {
                let name = relative.strip_prefix(&format!("{folder}/"))?;
                (!name.contains('/') && name.to_ascii_lowercase().ends_with(".jar")).then(|| {
                    (
                        name.to_owned(),
                        File::open(path).ok().and_then(jar_identity),
                    )
                })
            })
            .collect::<Vec<_>>();
        if incoming.is_empty() {
            return Ok(());
        }
        let directory = match open_dir_beneath(root, &[folder], false) {
            Ok(directory) => directory,
            Err(_) => return Ok(()),
        };
        let incoming_names = incoming
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<BTreeSet<_>>();
        let incoming_ids = incoming
            .iter()
            .filter_map(|(_, id)| id.as_deref())
            .collect::<BTreeSet<_>>();
        let entries = rustix::fs::Dir::read_from(&directory)
            .map_err(|_| format!("could not list the target's {folder}"))?;
        let mut existing = Vec::new();
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.to_ascii_lowercase().ends_with(".jar")
                && matches!(
                    entry.file_type(),
                    rustix::fs::FileType::RegularFile | rustix::fs::FileType::Unknown
                )
            {
                existing.push(name);
            }
        }
        for name in existing {
            if incoming_names.contains(name.as_str()) {
                continue;
            }
            let identity = rustix::fs::openat(
                &directory,
                name.as_str(),
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .ok()
            .and_then(|fd| jar_identity(File::from(fd)));
            let replaced = identity
                .as_deref()
                .is_some_and(|id| incoming_ids.contains(id));
            if replaced || remove_missing {
                rustix::fs::unlinkat(&directory, name.as_str(), rustix::fs::AtFlags::empty())
                    .map_err(|_| format!("could not remove the old {folder}/{name}"))?;
                if replaced {
                    report.replaced.push(format!("{folder}/{name}"));
                } else {
                    report.removed.push(format!("{folder}/{name}"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct ApplyReport {
    replaced: Vec<String>,
    removed: Vec<String>,
    warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameplay_settings_move_but_the_target_keeps_its_identity() {
        let merged = merge_transfer_properties(
            "server-port=25565\nlevel-name=world\nmotd=Public\nmax-players=100\nrcon.password=target\ndifficulty=easy\nallow-flight=false\n",
            "server-port=25566\nlevel-name=test\nmotd=Test\nmax-players=5\nrcon.password=source\ndifficulty=hard\npvp=false\n",
        );
        for kept in [
            "server-port=25565",
            "level-name=world",
            "motd=Public",
            "max-players=100",
            "rcon.password=target",
        ] {
            assert!(merged.contains(kept), "{kept} in {merged}");
        }
        for copied in ["difficulty=hard", "pvp=false", "allow-flight=false"] {
            assert!(merged.contains(copied), "{copied} in {merged}");
        }
        assert!(!merged.contains("25566") && !merged.contains("source"));
    }

    #[test]
    fn plugin_and_mod_names_come_from_their_metadata() {
        assert_eq!(
            yaml_plugin_name("main: dev.X\nname: AllFather\nversion: 0.9.18\n").as_deref(),
            Some("AllFather")
        );
        assert_eq!(
            yaml_plugin_name("name: 'Simple Voice Chat' # comment\n").as_deref(),
            Some("Simple Voice Chat")
        );
        assert_eq!(yaml_plugin_name("  name: nested\n"), None);
        assert_eq!(
            toml_mod_id("[[mods]]\nmodId=\"sodium\"\n").as_deref(),
            Some("sodium")
        );
    }

    #[test]
    fn jar_identity_reads_plugin_yml_from_a_real_zip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AllFather-0.9.18.jar");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        zip.start_file("plugin.yml", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"name: AllFather\nversion: 0.9.18\n")
            .unwrap();
        zip.finish().unwrap();
        assert_eq!(
            jar_identity(File::open(&path).unwrap()).as_deref(),
            Some("plugin:allfather")
        );
    }

    #[test]
    fn default_plugin_copy_leaves_player_data_and_databases_behind() {
        let script = find_script(TransferPart::Plugins);
        for excluded in [
            "plugins/*/userdata",
            "plugins/*/playerdata",
            "*.db",
            "*.sqlite",
            "plugins/.paper-remapped",
        ] {
            assert!(script.contains(excluded), "{excluded}");
        }
        assert!(!find_script(TransferPart::PluginData).contains("*.db"));
        assert!(find_script(TransferPart::Worlds).contains("session.lock"));
    }

    #[test]
    fn level_names_and_paths_are_validated() {
        assert!(valid_level_name("world"));
        assert!(!valid_level_name("../etc"));
        assert!(!valid_level_name("world; rm -rf /"));
        assert!(safe_components("plugins/AllFather/config.yml").is_some());
        assert!(safe_components("plugins/../../etc/passwd").is_none());
    }
}
