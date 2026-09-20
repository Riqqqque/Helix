use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../sevendays/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../sevendays/entrypoint.sh");

const SLOTS: &[PortSlot] = &[
    PortSlot::game(0, true, true),
    PortSlot::game(1, false, true),
    PortSlot::game(2, false, true),
];

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::SevenDaysToDie,
    slug: "seven_days_to_die",
    display: "7 Days to Die",
    runtime_image: "helix-seven-days-to-die-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "steam://294420",
    memory: (6_144, 49_152),
    players: (1, 64),
    defaults: (12_288, 8),
    slots: SLOTS,
    caves_slot: None,
    settings_file: "seven_days_to_die.json",
    install_markers: &["server/7DaysToDieServer.x86_64"],
    data_dirs: &["server", "config", "logs", "steamcmd"],
    pool: (26_900, 26_965),
    create_settings,
};

fn create_settings(spec: &GameCreateSpec, _generated: &str) -> Value {
    let mut settings = json!({
        "list_on_browser": spec.list_on_browser,
    });
    if let Some(password) = spec
        .server_password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        settings["server_password"] = Value::from(password);
    }
    if let Some(world_name) = spec
        .world_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        settings["world_name"] = Value::from(world_name);
    }
    if let Some(seed) = spec.world_seed {
        settings["world_seed"] = Value::from(seed);
    }
    settings
}
