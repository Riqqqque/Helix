use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../sonsoftheforest/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../sonsoftheforest/entrypoint.sh");

const SLOTS: &[PortSlot] = &[
    PortSlot::game(0, false, true),
    PortSlot::game(1, false, true),
    PortSlot::game(2, false, true),
];

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::SonsOfTheForest,
    slug: "sons_of_the_forest",
    display: "Sons of the Forest",
    runtime_image: "helix-sons-of-the-forest-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "steam://2465200",
    memory: (6_144, 32_768),
    players: (1, 8),
    defaults: (8_192, 8),
    slots: SLOTS,
    caves_slot: None,
    settings_file: "sons_of_the_forest.json",
    install_markers: &["server/SonsOfTheForestDS.exe"],
    data_dirs: &["server", "userdata", "wine", "steamcmd"],
    pool: (8_766, 8_840),
    create_settings,
};

fn create_settings(spec: &GameCreateSpec, _generated: &str) -> Value {
    let mut settings = json!({});
    if let Some(password) = spec
        .server_password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        settings["server_password"] = Value::from(password);
    }
    settings
}
