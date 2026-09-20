use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../vintagestory/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../vintagestory/entrypoint.sh");

const SLOTS: &[PortSlot] = &[PortSlot::game(0, true, false)];

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::VintageStory,
    slug: "vintage_story",
    display: "Vintage Story",
    runtime_image: "helix-vintage-story-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "vintagestory://server",
    memory: (2_048, 32_768),
    players: (1, 64),
    defaults: (6_144, 16),
    slots: SLOTS,
    caves_slot: None,
    settings_file: "vintage_story.json",
    install_markers: &["server/VintagestoryServer.dll"],
    data_dirs: &["server", "data"],
    pool: (42_420, 42_470),
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
