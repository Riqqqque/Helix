//! Platform-neutral copy rules for moving a game folder into a new Helix server.
//!
//! Helix never rewrites AMP or Pterodactyl files. This module only decides what
//! a copy would include, which Minecraft software to run, and how to merge
//! `server.properties` onto Helix-owned ports and RCON.

use crate::{GameKind, MinecraftSoftware, TerrariaSoftware};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const MAX_MIGRATE_PATH_BYTES: usize = 4_096;
pub const MAX_MIGRATE_FILES: usize = 250_000;
pub const MAX_MIGRATE_BYTES: u64 = 128 * 1024 * 1024 * 1024;
pub const MAX_MIGRATE_DEPTH: usize = 24;

const HELIX_PROPERTY_KEYS: &[&str] = &[
    "server-port",
    "query.port",
    "rcon.port",
    "rcon.password",
    "enable-rcon",
    "enable-query",
    "max-players",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyDecision {
    Copy,
    Skip,
    MergeProperties,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MappedMinecraftSoftware {
    pub software: MinecraftSoftware,
    pub copy_server_jar: bool,
    pub warning: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MinecraftVersionChoice {
    pub version: String,
    pub used_latest: bool,
}

#[must_use]
pub fn map_amp_minecraft_software(raw: &str) -> Result<MappedMinecraftSoftware, String> {
    let normalized = raw.trim().to_ascii_lowercase().replace([' ', '-', '.'], "_");
    let mapped = match normalized.as_str() {
        "official" | "vanilla" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Vanilla,
            copy_server_jar: false,
            warning: None,
        },
        "paper" | "paper_spigot" | "paperspigot" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Paper,
            copy_server_jar: false,
            warning: None,
        },
        "purpur" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Purpur,
            copy_server_jar: false,
            warning: None,
        },
        "folia" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Folia,
            copy_server_jar: false,
            warning: None,
        },
        "leaves" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Leaves,
            copy_server_jar: false,
            warning: None,
        },
        "fabric" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Fabric,
            copy_server_jar: false,
            warning: None,
        },
        "forge" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Forge,
            copy_server_jar: false,
            warning: None,
        },
        "neoforge" => MappedMinecraftSoftware {
            software: MinecraftSoftware::NeoForge,
            copy_server_jar: false,
            warning: None,
        },
        "quilt" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Quilt,
            copy_server_jar: false,
            warning: None,
        },
        "pufferfish" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Pufferfish,
            copy_server_jar: false,
            warning: None,
        },
        "spigot" | "bukkit" | "craftbukkit" | "craft_bukkit" => MappedMinecraftSoftware {
            software: MinecraftSoftware::Paper,
            copy_server_jar: false,
            warning: Some(
                "AMP is Spigot/Bukkit. Helix will run Paper and copy plugins, worlds, and configs.",
            ),
        },
        "mohist" | "magma" | "arclight" | "banner" | "catserver" | "custom" | "unknown" => {
            MappedMinecraftSoftware {
                software: MinecraftSoftware::Custom,
                copy_server_jar: true,
                warning: Some(
                    "Helix does not publish this loader. It will copy the existing server JAR as a custom server.",
                ),
            }
        }
        other if other.contains("bedrock")
            || other.contains("pocket")
            || other.contains("nukkit")
            || other.contains("geyser") && !other.contains("paper") =>
        {
            return Err(
                "Helix native Minecraft is Java Edition only. Bedrock/PocketMine stays in AMP or Pterodactyl."
                    .to_owned(),
            );
        }
        _ => MappedMinecraftSoftware {
            software: MinecraftSoftware::Custom,
            copy_server_jar: true,
            warning: Some(
                "Helix could not map this AMP server type. It will copy the existing server JAR as a custom server.",
            ),
        },
    };
    Ok(mapped)
}

#[must_use]
pub fn amp_software_is_known_hybrid(raw: &str) -> bool {
    let normalized = raw.trim().to_ascii_lowercase().replace([' ', '-', '.'], "_");
    matches!(
        normalized.as_str(),
        "mohist" | "magma" | "arclight" | "banner" | "catserver" | "custom" | "unknown"
    )
}

pub fn resolve_minecraft_software(
    raw: &str,
    root: &Path,
) -> Result<MappedMinecraftSoftware, String> {
    let mapped = map_amp_minecraft_software(raw)?;
    if mapped.copy_server_jar && !amp_software_is_known_hybrid(raw) {
        let detected = detect_minecraft_software_from_root(root)?;
        if !detected.copy_server_jar {
            return Ok(detected);
        }
    }
    Ok(mapped)
}

#[must_use]
pub fn source_looks_live(status: &str) -> bool {
    matches!(
        status,
        "online" | "idle" | "starting" | "stopping" | "updating"
    )
}

#[must_use]
pub fn minecraft_version_for_create(raw: &str) -> MinecraftVersionChoice {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("latest")
        || trimmed.eq_ignore_ascii_case("managed by amp")
        || trimmed.eq_ignore_ascii_case("latest version")
    {
        return MinecraftVersionChoice {
            version: "latest".to_owned(),
            used_latest: true,
        };
    }
    if trimmed.len() <= 64
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
    {
        MinecraftVersionChoice {
            version: trimmed.to_owned(),
            used_latest: false,
        }
    } else {
        MinecraftVersionChoice {
            version: "latest".to_owned(),
            used_latest: true,
        }
    }
}

#[must_use]
pub fn java_version_from_amp(raw: Option<&str>) -> u16 {
    let Some(raw) = raw else {
        return 21;
    };
    let digits: String = raw.chars().filter(char::is_ascii_digit).take(2).collect();
    match digits.parse::<u16>() {
        Ok(8 | 11) => 17,
        Ok(value) if (17..=25).contains(&value) => value,
        _ => 21,
    }
}

#[must_use]
pub fn looks_like_minecraft_root(names: &[impl AsRef<str>]) -> bool {
    names.iter().any(|name| {
        matches!(
            name.as_ref(),
            "server.properties"
                | "plugins"
                | "mods"
                | "world"
                | "level.dat"
                | "spigot.yml"
                | "paper.yml"
                | "bukkit.yml"
                | "fabric"
                | "eula.txt"
        )
    })
}

#[must_use]
pub fn looks_like_vrising_root(names: &[impl AsRef<str>]) -> bool {
    names.iter().any(|name| {
        let lower = name.as_ref().to_ascii_lowercase();
        lower == "serverhostsettings.json"
            || lower == "save-data"
            || lower == "savedata"
            || lower == "saves"
            || lower.ends_with(".sav")
    })
}

#[must_use]
pub fn looks_like_bedrock_root(names: &[impl AsRef<str>]) -> bool {
    let lower: Vec<String> = names
        .iter()
        .map(|name| name.as_ref().to_ascii_lowercase())
        .collect();
    let has = |needle: &str| lower.iter().any(|name| name == needle);
    let contains = |needle: &str| lower.iter().any(|name| name.contains(needle));
    contains("bedrock_server")
        || contains("pocketmine")
        || contains("nukkit")
        || has("permissions.json")
        || (has("allowlist.json") && !has("plugins") && !has("mods"))
}

#[must_use]
pub fn looks_like_valheim_root(names: &[impl AsRef<str>]) -> bool {
    names.iter().any(|name| {
        let lower = name.as_ref().to_ascii_lowercase();
        lower == "valheim_server"
            || lower.starts_with("valheim_server")
            || lower.ends_with(".fwl")
            || lower == "start_server.sh"
            || (lower == "worlds" || lower == "worlds_local")
    })
}

#[must_use]
pub fn looks_like_terraria_root(names: &[impl AsRef<str>]) -> bool {
    names.iter().any(|name| {
        let lower = name.as_ref().to_ascii_lowercase();
        lower.ends_with(".wld")
            || lower == "serverconfig.txt"
            || lower == "tmodloader"
            || lower.contains("terraria")
    })
}

#[must_use]
pub fn detect_game_from_names(names: &[impl AsRef<str>]) -> Option<GameKind> {
    if looks_like_bedrock_root(names) {
        return None;
    }
    if looks_like_minecraft_root(names) {
        return Some(GameKind::Minecraft);
    }
    if looks_like_vrising_root(names) {
        return Some(GameKind::VRising);
    }
    if looks_like_terraria_root(names) {
        return Some(GameKind::Terraria);
    }
    if looks_like_valheim_root(names) {
        return Some(GameKind::Valheim);
    }
    None
}

#[must_use]
pub fn minecraft_software_label(software: MinecraftSoftware) -> &'static str {
    match software {
        MinecraftSoftware::Custom => "Custom",
        MinecraftSoftware::Vanilla => "Vanilla",
        MinecraftSoftware::Paper => "Paper",
        MinecraftSoftware::Purpur => "Purpur",
        MinecraftSoftware::Folia => "Folia",
        MinecraftSoftware::Leaves => "Leaves",
        MinecraftSoftware::Fabric => "Fabric",
        MinecraftSoftware::NeoForge => "NeoForge",
        MinecraftSoftware::Forge => "Forge",
        MinecraftSoftware::Quilt => "Quilt",
        MinecraftSoftware::Pufferfish => "Pufferfish",
    }
}

#[must_use]
pub fn detect_minecraft_software_from_names(names: &[impl AsRef<str>]) -> MappedMinecraftSoftware {
    let lower: Vec<String> = names
        .iter()
        .map(|name| name.as_ref().to_ascii_lowercase())
        .collect();
    let has = |needle: &str| lower.iter().any(|name| name == needle);
    let contains = |needle: &str| lower.iter().any(|name| name.contains(needle));
    let has_plugins = has("plugins");
    let has_mods = has("mods");

    if contains("mohist")
        || contains("magma")
        || contains("arclight")
        || contains("banner")
        || contains("catserver")
        || (has_plugins && has_mods)
    {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Custom,
            copy_server_jar: true,
            warning: Some(
                "This folder looks like a hybrid or unpublished loader. Helix will copy the existing server JAR as a custom server.",
            ),
        };
    }
    if has(".fabric") || has("fabric-server-launch.jar") || has("fabric") {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Fabric,
            copy_server_jar: false,
            warning: None,
        };
    }
    if has("quilt-server-launch.jar") || has(".quilt") {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Quilt,
            copy_server_jar: false,
            warning: None,
        };
    }
    if contains("neoforge") {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::NeoForge,
            copy_server_jar: false,
            warning: None,
        };
    }
    if has("libraries")
        && (has("unix_args.txt")
            || has("user_jvm_args.txt")
            || has("run.sh")
            || has("run.bat")
            || contains("forge"))
    {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Forge,
            copy_server_jar: false,
            warning: None,
        };
    }
    if has("purpur.yml") || has("purpur.jar") {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Purpur,
            copy_server_jar: false,
            warning: None,
        };
    }
    if contains("pufferfish") {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Pufferfish,
            copy_server_jar: false,
            warning: None,
        };
    }
    if contains("folia") {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Folia,
            copy_server_jar: false,
            warning: None,
        };
    }
    if contains("leaves") {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Leaves,
            copy_server_jar: false,
            warning: None,
        };
    }
    if has("paper.yml")
        || has("paper-global.yml")
        || has("paper.jar")
        || has("config") && has_plugins
    {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Paper,
            copy_server_jar: false,
            warning: None,
        };
    }
    if has_plugins && has("spigot.yml") {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Paper,
            copy_server_jar: false,
            warning: Some(
                "This folder looks like Spigot/Bukkit. Helix will run Paper and copy plugins, worlds, and configs.",
            ),
        };
    }
    if has_plugins {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Paper,
            copy_server_jar: false,
            warning: None,
        };
    }
    if has_mods {
        return MappedMinecraftSoftware {
            software: MinecraftSoftware::Fabric,
            copy_server_jar: false,
            warning: Some(
                "This folder has mods and no plugins. Helix will run Fabric unless you pick a different loader.",
            ),
        };
    }
    MappedMinecraftSoftware {
        software: MinecraftSoftware::Vanilla,
        copy_server_jar: false,
        warning: None,
    }
}

pub fn detect_minecraft_software_from_root(root: &Path) -> Result<MappedMinecraftSoftware, String> {
    Ok(detect_minecraft_software_from_names(&directory_names(root)?))
}

#[must_use]
pub fn detect_terraria_software_from_names(names: &[impl AsRef<str>]) -> TerrariaSoftware {
    if names.iter().any(|name| {
        let lower = name.as_ref().to_ascii_lowercase();
        lower.contains("tmod") || lower == "mods" || lower.ends_with(".tmod")
    }) {
        TerrariaSoftware::Tmodloader
    } else {
        TerrariaSoftware::Vanilla
    }
}

pub fn detect_terraria_software(root: &Path) -> Result<TerrariaSoftware, String> {
    let names = directory_names(root)?;
    if detect_terraria_software_from_names(&names) == TerrariaSoftware::Tmodloader {
        return Ok(TerrariaSoftware::Tmodloader);
    }
    let mods = root.join("mods");
    if is_real_dir(&mods) {
        for entry in read_real_dir(&mods)? {
            if entry
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".tmod"))
            {
                return Ok(TerrariaSoftware::Tmodloader);
            }
        }
    }
    Ok(TerrariaSoftware::Vanilla)
}

#[must_use]
pub fn classify_minecraft_entry(name: &str) -> CopyDecision {
    let file_name = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_lowercase();
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        return CopyDecision::Skip;
    }
    if file_name == "server.properties" {
        return CopyDecision::MergeProperties;
    }
    if file_name.ends_with(".kvp")
        || file_name.starts_with("amp_")
        || matches!(
            file_name.as_str(),
            "logs"
                | "crash-reports"
                | "crash_reports"
                | "cache"
                | ".cache"
                | "tmp"
                | "temp"
                | "dumps"
                | ".ampdata"
                | "backups"
                | "backup"
                | "java"
                | "jre"
                | "jdk"
                | ".mixin.out"
                | "session.lock"
                | "eula.txt"
                | "unix_args.txt"
                | "user_jvm_args.txt"
                | "run.sh"
                | "run.bat"
                | "start.sh"
                | "start.bat"
                | ".helix-ready"
        )
        || file_name.ends_with(".log")
        || file_name.ends_with(".log.gz")
        || file_name.ends_with(".tmp")
        || file_name.ends_with(".lck")
    {
        return CopyDecision::Skip;
    }
    CopyDecision::Copy
}

#[must_use]
pub fn minecraft_root_jar_name(name: &str) -> bool {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file_name.eq_ignore_ascii_case("server.jar")
        || file_name.eq_ignore_ascii_case("fabric-server-launch.jar")
        || file_name.eq_ignore_ascii_case("paper.jar")
        || file_name.eq_ignore_ascii_case("purpur.jar")
}

#[must_use]
pub fn should_copy_minecraft_relative(relative: &str, copy_server_jar: bool) -> CopyDecision {
    if relative.contains("..") {
        return CopyDecision::Skip;
    }
    let segments: Vec<&str> = relative
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return CopyDecision::Skip;
    }
    for (index, segment) in segments.iter().enumerate() {
        match classify_minecraft_entry(segment) {
            CopyDecision::Skip => return CopyDecision::Skip,
            CopyDecision::MergeProperties if index + 1 == segments.len() => {
                return CopyDecision::MergeProperties;
            }
            CopyDecision::MergeProperties => return CopyDecision::Skip,
            CopyDecision::Copy => {}
        }
    }
    if !copy_server_jar
        && matches!(
            segments[0].to_ascii_lowercase().as_str(),
            "libraries" | "versions" | ".fabric"
        )
    {
        return CopyDecision::Skip;
    }
    if !copy_server_jar && segments.len() == 1 && minecraft_root_jar_name(segments[0]) {
        return CopyDecision::Skip;
    }
    CopyDecision::Copy
}

#[must_use]
pub fn overlay_relative_for_game(game: GameKind, relative: &str) -> Option<String> {
    if relative.contains("..") {
        return None;
    }
    let normalized = relative.replace('\\', "/");
    let first = normalized.split('/').find(|segment| !segment.is_empty())?;
    match game {
        GameKind::Minecraft => {
            if relative.contains("..") {
                None
            } else {
                Some(normalized)
            }
        }
        GameKind::VRising => {
            let lower = first.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "wine"
                    | "steamcmd"
                    | "steamapps"
                    | "server"
                    | "logs"
                    | "vrising.exe"
                    | "vrising-server.exe"
                    | "vrising-server"
            ) || first.ends_with(".kvp")
            {
                return None;
            }
            if lower == "save" {
                return Some(normalized);
            }
            if lower == "saves" {
                return Some(match normalized.split_once('/') {
                    Some((_, rest)) => format!("save/Saves/{rest}"),
                    None => "save/Saves".to_owned(),
                });
            }
            if lower == "save-data" || lower == "savedata" {
                return Some(match normalized.split_once('/') {
                    Some((_, rest)) => format!("save/{rest}"),
                    None => "save".to_owned(),
                });
            }
            if lower == "serverhostsettings.json"
                || normalized.to_ascii_lowercase().contains("serverhostsettings.json")
            {
                return Some("save/Settings/ServerHostSettings.json".to_owned());
            }
            if lower == "settings" {
                return Some(match normalized.split_once('/') {
                    Some((_, rest)) => format!("save/Settings/{rest}"),
                    None => "save/Settings".to_owned(),
                });
            }
            None
        }
        GameKind::Valheim => {
            let lower = first.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "steamcmd" | "server" | "logs" | "valheim_server" | "valheim_server.x86_64"
            ) || first.ends_with(".kvp")
            {
                return None;
            }
            if lower == "worlds" || lower == "worlds_local" {
                return Some(normalized);
            }
            if lower == "plugins" {
                return Some(normalized);
            }
            if lower == "bepinex" {
                if let Some((_, rest)) = normalized.split_once('/')
                    && rest.to_ascii_lowercase().starts_with("plugins")
                {
                    return Some(rest.to_owned());
                }
                return None;
            }
            if lower.ends_with(".fwl") || lower.ends_with(".db") || lower.ends_with(".db.old") {
                return Some(format!("worlds/{first}"));
            }
            None
        }
        GameKind::Terraria => {
            let lower = first.to_ascii_lowercase();
            if matches!(lower.as_str(), "steamcmd" | "server" | "logs") || first.ends_with(".kvp")
            {
                return None;
            }
            if lower == "worlds" || lower == "mods" {
                return Some(normalized);
            }
            if lower.ends_with(".wld") || lower.ends_with(".wld.bak") {
                return Some(format!("worlds/{first}"));
            }
            if lower.ends_with(".tmod") {
                return Some(format!("mods/{first}"));
            }
            None
        }
    }
}

#[must_use]
pub fn merge_server_properties(
    helix: &str,
    source: &str,
    game_port: u16,
    rcon_port: u16,
    rcon_password: &str,
    max_players: u16,
) -> String {
    let mut values = BTreeMap::new();
    for line in helix.lines().chain(source.lines()) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.len() > 128 {
            continue;
        }
        values.insert(key.to_owned(), value.trim().to_owned());
    }
    values.insert("server-port".to_owned(), game_port.to_string());
    values.insert("query.port".to_owned(), game_port.to_string());
    values.insert("rcon.port".to_owned(), rcon_port.to_string());
    values.insert("rcon.password".to_owned(), rcon_password.to_owned());
    values.insert("enable-rcon".to_owned(), "true".to_owned());
    values.insert("enable-query".to_owned(), "true".to_owned());
    values.insert("max-players".to_owned(), max_players.to_string());
    let mut body = String::from("# Managed by Helix after a copy from another manager\n");
    for (key, value) in values {
        if HELIX_PROPERTY_KEYS.contains(&key.as_str())
            || key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            body.push_str(&key);
            body.push('=');
            body.push_str(&sanitize_property_value(&value));
            body.push('\n');
        }
    }
    body
}

#[must_use]
pub fn merge_vrising_host_settings(helix: Value, source_json: &str) -> Value {
    let Ok(Value::Object(source)) = serde_json::from_str::<Value>(source_json) else {
        return helix;
    };
    let mut out = helix;
    for key in ["SaveName", "Password", "Description", "GameSettingsPreset"] {
        if let Some(Value::String(text)) = source.get(key)
            && text.len() <= 256
            && !text.contains('\0')
            && !text.contains('\n')
            && !text.contains('\r')
        {
            out[key] = json!(text);
        }
    }
    out
}

fn sanitize_property_value(value: &str) -> String {
    value
        .chars()
        .filter(|ch| *ch != '\0' && *ch != '\n' && *ch != '\r')
        .take(512)
        .collect()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OverlayReport {
    pub files: usize,
    pub bytes: u64,
    pub skipped: usize,
    pub copies: Vec<String>,
    pub skips: Vec<String>,
}

const NESTED_GAME_ROOTS: &[&str] = &[
    "Minecraft",
    "minecraft",
    "server",
    "data",
    "SaveData",
    "save-data",
    "saves",
    "Saves",
    "VRising",
    "vrising",
    "Valheim",
    "valheim",
    "Terraria",
    "terraria",
    "tModLoader",
    "Worlds",
    "worlds",
    "worlds_local",
];

pub fn find_game_root(start: &Path) -> Result<(GameKind, PathBuf), String> {
    let start_names = directory_names(start)?;
    if looks_like_bedrock_root(&start_names) {
        return Err(
            "Helix native Minecraft is Java Edition only. Bedrock/PocketMine stays in AMP or Pterodactyl."
                .to_owned(),
        );
    }
    if let Some(kind) = detect_game_from_names(&start_names) {
        return Ok((kind, start.to_path_buf()));
    }
    for child in NESTED_GAME_ROOTS {
        let path = start.join(child);
        if !is_real_dir(&path) {
            continue;
        }
        let names = directory_names(&path)?;
        if looks_like_bedrock_root(&names) {
            return Err(
                "Helix native Minecraft is Java Edition only. Bedrock/PocketMine stays in AMP or Pterodactyl."
                    .to_owned(),
            );
        }
        if let Some(kind) = detect_game_from_names(&names) {
            return Ok((kind, path));
        }
    }
    let mut checked = 0usize;
    for entry in read_real_dir(start)? {
        if checked >= 64 {
            break;
        }
        checked = checked.saturating_add(1);
        if !is_real_dir(&entry) {
            continue;
        }
        let names = directory_names(&entry).unwrap_or_default();
        if looks_like_bedrock_root(&names) {
            continue;
        }
        if let Some(kind) = detect_game_from_names(&names) {
            return Ok((kind, entry));
        }
    }
    Err("Helix could not see a Minecraft, V Rising, Valheim, or Terraria folder there. Pick the folder that has the world, plugins, mods, or save files.".to_owned())
}

pub fn scan_overlay(
    game: GameKind,
    root: &Path,
    copy_server_jar: bool,
) -> Result<OverlayReport, String> {
    walk_overlay(game, root, None, copy_server_jar)
}

pub fn apply_overlay(
    game: GameKind,
    source: &Path,
    destination: &Path,
    copy_server_jar: bool,
) -> Result<OverlayReport, String> {
    if !is_real_dir(destination) {
        return Err("the Helix server directory is missing".to_owned());
    }
    walk_overlay(game, source, Some(destination), copy_server_jar)
}

pub fn read_source_properties(root: &Path) -> Option<String> {
    let path = root.join("server.properties");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 512 * 1024 {
        return None;
    }
    fs::read_to_string(&path).ok()
}

pub fn find_minecraft_server_jar(root: &Path) -> Result<PathBuf, String> {
    for name in ["server.jar", "paper.jar", "purpur.jar", "fabric-server-launch.jar"] {
        let path = root.join(name);
        if is_real_file(&path) {
            return Ok(path);
        }
    }
    let mut found = None;
    for entry in read_real_dir(root)? {
        let Some(name) = entry.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".jar") {
            continue;
        }
        if !is_real_file(&entry) {
            continue;
        }
        if found.is_some() {
            return Err(
                "that folder has more than one server JAR. Rename the one Helix should copy to server.jar."
                    .to_owned(),
            );
        }
        found = Some(entry);
    }
    found.ok_or_else(|| {
        "Helix could not find a server JAR to copy. Put one named server.jar in that folder.".to_owned()
    })
}

pub fn ensure_named_save(
    directory: &Path,
    primary_stem: &str,
    extension: &str,
    companions: &[&str],
) -> Result<Option<String>, String> {
    if !is_real_dir(directory) {
        return Ok(None);
    }
    let primary = directory.join(format!("{primary_stem}.{extension}"));
    if is_real_file(&primary) {
        return Ok(None);
    }
    let suffix = format!(".{extension}");
    let mut best: Option<(PathBuf, u64, String)> = None;
    for entry in read_real_dir(directory)? {
        let Some(name) = entry.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(&suffix) || lower.ends_with(".bak") {
            continue;
        }
        if !is_real_file(&entry) {
            continue;
        }
        let metadata = fs::symlink_metadata(&entry)
            .map_err(|_| format!("could not read {name} from the copied worlds"))?;
        let stem = name
            .rsplit_once('.')
            .map(|(stem, _)| stem.to_owned())
            .unwrap_or_else(|| name.to_owned());
        if best
            .as_ref()
            .is_none_or(|(_, size, _)| metadata.len() > *size)
        {
            best = Some((entry, metadata.len(), stem));
        }
    }
    let Some((source, _, stem)) = best else {
        return Ok(None);
    };
    fs::copy(&source, &primary)
        .map_err(|_| "could not promote the copied world to the Helix default name".to_owned())?;
    for companion in companions {
        let from = directory.join(format!("{stem}.{companion}"));
        let to = directory.join(format!("{primary_stem}.{companion}"));
        if is_real_file(&from) && !is_real_file(&to) {
            fs::copy(&from, &to).map_err(|_| {
                "could not promote the copied world sidecar files to the Helix default name"
                    .to_owned()
            })?;
        }
    }
    Ok(Some(stem))
}

fn is_real_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
}

fn walk_overlay(
    game: GameKind,
    source: &Path,
    destination: Option<&Path>,
    copy_server_jar: bool,
) -> Result<OverlayReport, String> {
    let mut report = OverlayReport::default();
    let mut stack = vec![(source.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        if depth > MAX_MIGRATE_DEPTH {
            return Err("that folder is nested too deeply to copy".to_owned());
        }
        for entry in read_real_dir(&dir)? {
            let name = entry
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| "a file name in that folder is not usable".to_owned())?
                .to_owned();
            if name == "." || name == ".." {
                continue;
            }
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let metadata = fs::symlink_metadata(&entry)
                .map_err(|_| format!("could not read {relative} from the source folder"))?;
            if metadata.file_type().is_symlink() {
                record_skip(&mut report, &relative);
                continue;
            }
            if metadata.is_dir() {
                match overlay_decision(game, &relative, copy_server_jar) {
                    CopyDecision::Skip => {
                        record_skip(&mut report, &relative);
                        continue;
                    }
                    CopyDecision::MergeProperties | CopyDecision::Copy => {
                        if let Some(destination) = destination
                            && let Some(mapped) = overlay_relative_for_game(game, &relative)
                        {
                            fs::create_dir_all(destination.join(mapped)).map_err(|_| {
                                format!("could not create {relative} in the Helix server")
                            })?;
                        }
                        stack.push((entry, relative, depth.saturating_add(1)));
                    }
                }
                continue;
            }
            if !metadata.is_file() {
                record_skip(&mut report, &relative);
                continue;
            }
            match overlay_decision(game, &relative, copy_server_jar) {
                CopyDecision::Skip | CopyDecision::MergeProperties => {
                    record_skip(&mut report, &relative);
                }
                CopyDecision::Copy => {
                    let mapped = overlay_relative_for_game(game, &relative).ok_or_else(|| {
                        format!("Helix refused to copy {relative}")
                    })?;
                    report.files = report.files.saturating_add(1);
                    report.bytes = report.bytes.saturating_add(metadata.len());
                    if report.files > MAX_MIGRATE_FILES {
                        return Err("that folder has too many files to copy".to_owned());
                    }
                    if report.bytes > MAX_MIGRATE_BYTES {
                        return Err("that folder is larger than 128 GiB".to_owned());
                    }
                    if report.copies.len() < 24 {
                        report.copies.push(relative.clone());
                    }
                    if let Some(destination) = destination {
                        let target = destination.join(&mapped);
                        if let Some(parent) = target.parent() {
                            fs::create_dir_all(parent).map_err(|_| {
                                format!("could not create {relative} in the Helix server")
                            })?;
                        }
                        if target.exists() && minecraft_root_jar_name(&mapped) && !copy_server_jar {
                            continue;
                        }
                        fs::copy(&entry, &target).map_err(|_| {
                            format!("could not copy {relative} into the Helix server")
                        })?;
                    }
                }
            }
        }
    }
    Ok(report)
}

fn overlay_decision(game: GameKind, relative: &str, copy_server_jar: bool) -> CopyDecision {
    match game {
        GameKind::Minecraft => should_copy_minecraft_relative(relative, copy_server_jar),
        GameKind::VRising | GameKind::Valheim | GameKind::Terraria => {
            if overlay_relative_for_game(game, relative).is_some() {
                CopyDecision::Copy
            } else {
                CopyDecision::Skip
            }
        }
    }
}

fn record_skip(report: &mut OverlayReport, relative: &str) {
    report.skipped = report.skipped.saturating_add(1);
    if report.skips.len() < 16 {
        report.skips.push(relative.to_owned());
    }
}

fn directory_names(path: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in read_real_dir(path)? {
        if let Some(name) = entry.file_name().and_then(|value| value.to_str()) {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

fn read_real_dir(path: &Path) -> Result<Vec<PathBuf>, String> {
    if !is_real_dir(path) {
        return Err("the selected path is not a directory".to_owned());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|_| "could not read that folder".to_owned())? {
        let entry = entry.map_err(|_| "could not read that folder".to_owned())?;
        entries.push(entry.path());
    }
    Ok(entries)
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn maps_amp_paper_and_refuses_bedrock() {
        assert_eq!(
            map_amp_minecraft_software("Paper_Spigot").unwrap().software,
            MinecraftSoftware::Paper
        );
        assert_eq!(
            map_amp_minecraft_software("Spigot").unwrap().software,
            MinecraftSoftware::Paper
        );
        assert!(map_amp_minecraft_software("Spigot").unwrap().warning.is_some());
        assert!(map_amp_minecraft_software("Mohist").unwrap().copy_server_jar);
        assert!(map_amp_minecraft_software("Bedrock").is_err());
        assert!(source_looks_live("online"));
        assert!(source_looks_live("updating"));
        assert!(!source_looks_live("offline"));
        assert!(!source_looks_live("manager_stopped"));
    }

    #[test]
    fn latest_is_used_for_amp_placeholder_versions() {
        assert!(minecraft_version_for_create("Managed by AMP").used_latest);
        assert_eq!(minecraft_version_for_create("1.21.8").version, "1.21.8");
        assert_eq!(java_version_from_amp(Some("Java 21")), 21);
        assert_eq!(java_version_from_amp(Some("8")), 17);
    }

    #[test]
    fn skips_amp_metadata_and_logs() {
        assert_eq!(
            classify_minecraft_entry("MinecraftModule.kvp"),
            CopyDecision::Skip
        );
        assert_eq!(classify_minecraft_entry("logs"), CopyDecision::Skip);
        assert_eq!(
            classify_minecraft_entry("server.properties"),
            CopyDecision::MergeProperties
        );
        assert_eq!(classify_minecraft_entry("plugins"), CopyDecision::Copy);
        assert_eq!(classify_minecraft_entry("world"), CopyDecision::Copy);
        assert_eq!(
            should_copy_minecraft_relative("server.jar", false),
            CopyDecision::Skip
        );
        assert_eq!(
            should_copy_minecraft_relative("server.jar", true),
            CopyDecision::Copy
        );
        assert_eq!(
            should_copy_minecraft_relative("mods/fabric-api.jar", false),
            CopyDecision::Copy
        );
        assert_eq!(
            should_copy_minecraft_relative("../escape", false),
            CopyDecision::Skip
        );
    }

    #[test]
    fn detects_minecraft_loader_from_ordinary_folder_names() {
        assert_eq!(
            detect_minecraft_software_from_names(&["plugins", "paper.yml", "world"]).software,
            MinecraftSoftware::Paper
        );
        assert_eq!(
            detect_minecraft_software_from_names(&["mods", ".fabric", "fabric-server-launch.jar"])
                .software,
            MinecraftSoftware::Fabric
        );
        assert_eq!(
            detect_minecraft_software_from_names(&["libraries", "unix_args.txt", "mods"]).software,
            MinecraftSoftware::Forge
        );
        assert!(
            detect_minecraft_software_from_names(&["plugins", "mods", "server.jar"]).copy_server_jar
        );
        assert_eq!(
            detect_minecraft_software_from_names(&["server.properties", "world"]).software,
            MinecraftSoftware::Vanilla
        );
        assert_eq!(
            detect_terraria_software_from_names(&["world.wld", "tModLoader"]),
            TerrariaSoftware::Tmodloader
        );
        assert_eq!(
            detect_terraria_software_from_names(&["world.wld", "serverconfig.txt"]),
            TerrariaSoftware::Vanilla
        );
    }

    #[test]
    fn detects_each_supported_game_from_folder_names() {
        assert_eq!(
            detect_game_from_names(&["server.properties", "plugins"]),
            Some(GameKind::Minecraft)
        );
        assert_eq!(
            detect_game_from_names(&["ServerHostSettings.json", "saves"]),
            Some(GameKind::VRising)
        );
        assert_eq!(
            detect_game_from_names(&["world.wld", "serverconfig.txt"]),
            Some(GameKind::Terraria)
        );
        assert_eq!(
            detect_game_from_names(&["world.fwl", "worlds"]),
            Some(GameKind::Valheim)
        );
    }

    #[test]
    fn maps_non_minecraft_saves_into_helix_layout() {
        assert_eq!(
            overlay_relative_for_game(GameKind::VRising, "saves").as_deref(),
            Some("save/Saves")
        );
        assert_eq!(
            overlay_relative_for_game(GameKind::VRising, "saves/world1/Session.sav").as_deref(),
            Some("save/Saves/world1/Session.sav")
        );
        assert_eq!(
            overlay_relative_for_game(GameKind::VRising, "save-data/Saves/world1/Session.sav")
                .as_deref(),
            Some("save/Saves/world1/Session.sav")
        );
        assert_eq!(
            overlay_relative_for_game(GameKind::VRising, "save/Saves/world1/Session.sav").as_deref(),
            Some("save/Saves/world1/Session.sav")
        );
        assert_eq!(
            overlay_relative_for_game(GameKind::Valheim, "worlds/Midgard.fwl").as_deref(),
            Some("worlds/Midgard.fwl")
        );
        assert_eq!(
            overlay_relative_for_game(GameKind::Valheim, "worlds_local/Midgard.fwl").as_deref(),
            Some("worlds_local/Midgard.fwl")
        );
        assert_eq!(
            overlay_relative_for_game(GameKind::Terraria, "MyWorld.wld").as_deref(),
            Some("worlds/MyWorld.wld")
        );
        assert_eq!(
            overlay_relative_for_game(GameKind::VRising, "wine/drive_c/windows"),
            None
        );
    }

    #[test]
    fn merged_properties_keep_helix_ports_and_source_world() {
        let merged = merge_server_properties(
            "server-port=25570\nrcon.port=1\nrcon.password=secret\nmax-players=20\n",
            "server-port=25565\nlevel-name=skyblock\nmotd=From AMP\nwhite-list=true\n",
            25_571,
            4_000,
            "helix-rcon",
            30,
        );
        assert!(merged.contains("server-port=25571"));
        assert!(merged.contains("rcon.password=helix-rcon"));
        assert!(merged.contains("level-name=skyblock"));
        assert!(merged.contains("motd=From AMP"));
        assert!(merged.contains("white-list=true"));
        assert!(merged.contains("max-players=30"));
        assert!(!merged.contains("25565"));
    }

    #[test]
    fn copies_a_minecraft_tree_and_skips_amp_files() {
        let root = tempfile::tempdir().expect("temp");
        let game = root.path().join("Minecraft");
        fs::create_dir_all(game.join("world")).unwrap();
        fs::create_dir_all(game.join("plugins")).unwrap();
        fs::create_dir_all(game.join("logs")).unwrap();
        fs::write(game.join("server.properties"), "level-name=world\nserver-port=25565\n").unwrap();
        fs::write(game.join("world").join("level.dat"), b"world").unwrap();
        fs::write(game.join("plugins").join("WorldGuard.jar"), b"plugin").unwrap();
        fs::write(game.join("logs").join("latest.log"), b"log").unwrap();
        fs::write(root.path().join("MinecraftModule.kvp"), "Minecraft.ServerType=Paper\n").unwrap();
        let (kind, found) = find_game_root(root.path()).expect("detect");
        assert_eq!(kind, GameKind::Minecraft);
        assert_eq!(found, game);
        let dest = tempfile::tempdir().expect("dest");
        let report = apply_overlay(kind, &found, dest.path(), false).expect("copy");
        assert!(report.files >= 2);
        assert!(dest.path().join("world").join("level.dat").is_file());
        assert!(dest.path().join("plugins").join("WorldGuard.jar").is_file());
        assert!(!dest.path().join("logs").join("latest.log").exists());
        assert!(!dest.path().join("server.properties").exists());
        assert!(!dest.path().join("MinecraftModule.kvp").exists());
    }

    #[test]
    fn promotes_a_copied_world_to_the_helix_default_name() {
        let dir = tempfile::tempdir().expect("temp");
        fs::write(dir.path().join("Skyblock.wld"), b"world-bytes-here").unwrap();
        fs::write(dir.path().join("Skyblock.wld.bak"), b"bak").unwrap();
        let stem = ensure_named_save(dir.path(), "world", "wld", &["wld.bak"])
            .expect("promote")
            .expect("found");
        assert_eq!(stem, "Skyblock");
        assert!(dir.path().join("world.wld").is_file());
        assert!(dir.path().join("world.wld.bak").is_file());
        assert!(dir.path().join("Skyblock.wld").is_file());
        assert!(ensure_named_save(dir.path(), "world", "wld", &["wld.bak"])
            .expect("second")
            .is_none());
    }

    #[test]
    fn unknown_amp_type_uses_the_folder_loader_when_it_is_obvious() {
        let root = tempfile::tempdir().expect("temp");
        fs::create_dir_all(root.path().join("plugins")).unwrap();
        fs::write(root.path().join("paper.yml"), "settings: {}\n").unwrap();
        let mapped = resolve_minecraft_software("Minecraft", root.path()).expect("detect");
        assert_eq!(mapped.software, MinecraftSoftware::Paper);
        assert!(!mapped.copy_server_jar);
        let hybrid = resolve_minecraft_software("Mohist", root.path()).expect("hybrid");
        assert!(hybrid.copy_server_jar);
    }

    #[test]
    fn amp_vrising_instance_root_is_not_treated_as_the_save_folder() {
        let root = tempfile::tempdir().expect("temp");
        let wine = root.path().join("wine");
        let game = root.path().join("VRising");
        fs::create_dir_all(wine.join("drive_c")).unwrap();
        fs::create_dir_all(game.join("save-data").join("Saves").join("world1")).unwrap();
        fs::create_dir_all(game.join("save-data").join("Settings")).unwrap();
        fs::write(
            game.join("save-data")
                .join("Settings")
                .join("ServerHostSettings.json"),
            r#"{"SaveName":"world1","Password":"secret"}"#,
        )
        .unwrap();
        fs::write(
            game.join("save-data").join("Saves").join("world1").join("Session.sav"),
            b"save",
        )
        .unwrap();
        let (kind, found) = find_game_root(root.path()).expect("detect");
        assert_eq!(kind, GameKind::VRising);
        assert_eq!(found, game);
        let dest = tempfile::tempdir().expect("dest");
        fs::create_dir_all(dest.path().join("save").join("Settings")).unwrap();
        apply_overlay(kind, &found, dest.path(), false).expect("copy");
        assert!(dest
            .path()
            .join("save")
            .join("Saves")
            .join("world1")
            .join("Session.sav")
            .is_file());
        assert!(!dest.path().join("wine").exists());
        let merged = merge_vrising_host_settings(
            json!({"Name":"Helix","Port":9876,"SaveName":"world1","Password":""}),
            r#"{"SaveName":"castle","Password":"keep-me","Port":9871}"#,
        );
        assert_eq!(merged["SaveName"], "castle");
        assert_eq!(merged["Password"], "keep-me");
        assert_eq!(merged["Port"], 9876);
        assert_eq!(merged["Name"], "Helix");
    }

    #[test]
    fn bedrock_folders_are_refused() {
        let root = tempfile::tempdir().expect("temp");
        fs::write(root.path().join("server.properties"), "server-name=Bedrock\n").unwrap();
        fs::write(root.path().join("permissions.json"), "[]").unwrap();
        fs::write(root.path().join("bedrock_server"), b"bin").unwrap();
        assert!(find_game_root(root.path()).is_err());
        assert_eq!(
            detect_game_from_names(&["server.properties", "permissions.json", "bedrock_server"]),
            None
        );
    }
}
