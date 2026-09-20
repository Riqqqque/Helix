use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../rust/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../rust/entrypoint.sh");

const SLOTS: &[PortSlot] = &[
    PortSlot::game(0, false, true),
    PortSlot::game(1, false, true),
    PortSlot::game(2, true, false),
];

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::Rust,
    slug: "rust",
    display: "Rust",
    runtime_image: "helix-rust-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "steam://258550",
    memory: (8_192, 65_536),
    players: (1, 500),
    defaults: (12_288, 50),
    slots: SLOTS,
    caves_slot: None,
    settings_file: "rust.json",
    install_markers: &["server/RustDedicated"],
    data_dirs: &["server", "logs", "steamcmd"],
    pool: (28_015, 28_090),
    create_settings,
};

fn create_settings(spec: &GameCreateSpec, generated: &str) -> Value {
    let mut settings = json!({
        "rcon_password": generated,
    });
    if let Some(seed) = spec.world_seed {
        settings["world_seed"] = Value::from(seed);
    }
    if let Some(size) = spec.world_size {
        settings["world_size"] = Value::from(size);
    }
    settings
}
