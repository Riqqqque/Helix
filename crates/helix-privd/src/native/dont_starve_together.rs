use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../dontstarvetogether/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../dontstarvetogether/entrypoint.sh");

const SLOTS: &[PortSlot] = &[PortSlot::game(0, false, true)];
const CAVES_SLOT: PortSlot = PortSlot::game(1, false, true);

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::DontStarveTogether,
    slug: "dont_starve_together",
    display: "Don't Starve Together",
    runtime_image: "helix-dont-starve-together-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "steam://343050",
    memory: (1_024, 8_192),
    players: (1, 64),
    defaults: (2_048, 8),
    slots: SLOTS,
    caves_slot: Some(CAVES_SLOT),
    settings_file: "dont_starve_together.json",
    install_markers: &["server/bin64/dontstarve_dedicated_server_nullrenderer_x64"],
    data_dirs: &["server", "DoNotStarveTogether", "steamcmd"],
    pool: (10_999, 11_050),
    create_settings,
};

fn create_settings(spec: &GameCreateSpec, _generated: &str) -> Value {
    let mut settings = json!({
        "caves": spec.caves,
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
    if let Some(token) = spec
        .cluster_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        settings["cluster_token"] = Value::from(token);
    }
    settings
}
