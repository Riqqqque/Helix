use serde_json::Value;

use crate::{GameCreateSpec, GameKind};

use super::{
    dont_starve_together, factorio, project_zomboid, rust, satisfactory, seven_days_to_die,
    sons_of_the_forest, vintage_story,
};

#[derive(Clone, Copy)]
pub(crate) struct PortSlot {
    /// Offset from the game port when ports are auto-allocated as a block.
    pub offset: u16,
    pub tcp: bool,
    pub udp: bool,
}

impl PortSlot {
    pub(crate) const fn game(offset: u16, tcp: bool, udp: bool) -> Self {
        Self { offset, tcp, udp }
    }
}

pub(crate) struct GameDef {
    pub kind: GameKind,
    pub slug: &'static str,
    pub display: &'static str,
    pub runtime_image: &'static str,
    /// Bundled Dockerfile contents (written to the build staging dir).
    pub dockerfile: &'static str,
    /// Bundled entrypoint contents.
    pub entrypoint: &'static str,
    pub artifact: &'static str,
    pub memory: (u32, u32),
    /// Supported player-count range; recorded per game but not enforced yet.
    #[allow(dead_code)]
    pub players: (u16, u16),
    /// Migration defaults: (memory_mb, max_players).
    pub defaults: (u32, u16),
    /// Required host ports, in manifest order: slot 0 -> game_port, slot 1 -> query_port, slot 2 -> rcon_port.
    pub slots: &'static [PortSlot],
    /// Extra slot consumed when `GameCreateSpec::caves` is set (Don't Starve Together).
    pub caves_slot: Option<PortSlot>,
    /// Settings file name under /data; entries become HELIX_* container env vars.
    pub settings_file: &'static str,
    /// Paths that must exist under /data once the runtime has installed the server.
    pub install_markers: &'static [&'static str],
    /// Directories the runtime owns under /data; used by the repair path.
    pub data_dirs: &'static [&'static str],
    /// Default automatic port pool.
    pub pool: (u16, u16),
    /// Build the /data settings document; `generated` is a fresh random secret.
    pub create_settings: fn(&GameCreateSpec, &str) -> Value,
}

pub(crate) fn game_def(kind: GameKind) -> Option<&'static GameDef> {
    match kind {
        GameKind::Satisfactory => Some(&satisfactory::DEF),
        GameKind::ProjectZomboid => Some(&project_zomboid::DEF),
        GameKind::SevenDaysToDie => Some(&seven_days_to_die::DEF),
        GameKind::Rust => Some(&rust::DEF),
        GameKind::SonsOfTheForest => Some(&sons_of_the_forest::DEF),
        GameKind::Factorio => Some(&factorio::DEF),
        GameKind::DontStarveTogether => Some(&dont_starve_together::DEF),
        GameKind::VintageStory => Some(&vintage_story::DEF),
        _ => None,
    }
}

/// Ports a managed game needs, in slot order. `caves` only applies to
/// Don't Starve Together.
pub(crate) fn slot_layout(def: &GameDef, caves: bool) -> Vec<PortSlot> {
    let mut slots = def.slots.to_vec();
    if caves && let Some(slot) = def.caves_slot {
        slots.push(slot);
    }
    slots
}

pub(crate) fn settings_value(settings: &Value, key: &str) -> Option<String> {
    match settings.get(key)? {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

pub(crate) fn env_name_for_setting(key: &str) -> String {
    format!("HELIX_{}", key.to_uppercase())
}
