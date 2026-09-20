use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../factorio/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../factorio/entrypoint.sh");

const SLOTS: &[PortSlot] = &[PortSlot::game(0, false, true)];

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::Factorio,
    slug: "factorio",
    display: "Factorio",
    runtime_image: "helix-factorio-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "factorio://headless",
    memory: (1_024, 16_384),
    players: (1, 255),
    defaults: (4_096, 16),
    slots: SLOTS,
    caves_slot: None,
    settings_file: "factorio.json",
    install_markers: &["server/factorio/bin/x64/factorio"],
    data_dirs: &["server", "config", "saves", "mods"],
    pool: (34_197, 34_250),
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
    settings
}
