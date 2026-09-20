use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../projectzomboid/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../projectzomboid/entrypoint.sh");

const SLOTS: &[PortSlot] = &[
    PortSlot::game(0, false, true),
    PortSlot::game(1, false, true),
];

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::ProjectZomboid,
    slug: "project_zomboid",
    display: "Project Zomboid",
    runtime_image: "helix-project-zomboid-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "steam://380870",
    memory: (4_096, 32_768),
    players: (1, 128),
    defaults: (8_192, 8),
    slots: SLOTS,
    caves_slot: None,
    settings_file: "project_zomboid.json",
    install_markers: &["server/start-server.sh"],
    data_dirs: &["server", "zomboid", "steamcmd"],
    pool: (16_261, 16_295),
    create_settings,
};

fn create_settings(spec: &GameCreateSpec, generated: &str) -> Value {
    let mut settings = json!({
        "admin_password": spec
            .admin_password
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(generated),
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
    settings
}
