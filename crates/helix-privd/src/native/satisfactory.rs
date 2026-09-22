use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../satisfactory/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../satisfactory/entrypoint.sh");

const SLOTS: &[PortSlot] = &[
    PortSlot::game(0, true, true),
    PortSlot::game(1, false, true),
];

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::Satisfactory,
    slug: "satisfactory",
    display: "Satisfactory",
    runtime_image: "helix-satisfactory-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "steam://1690800",
    memory: (6_144, 32_768),
    players: (1, 16),
    defaults: (12_288, 4),
    slots: SLOTS,
    caves_slot: None,
    settings_file: "satisfactory.json",
    install_markers: &["server/FactoryServer.sh"],
    data_dirs: &["server", "steamcmd"],
    pool: (7_777, 7_811),
    create_settings,
};

fn create_settings(_spec: &GameCreateSpec, _generated: &str) -> Value {
    json!({})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn def_shape() {
        assert_eq!(DEF.slug, "satisfactory");
        assert_eq!(DEF.slots.len(), 2);
        assert!(DEF.slots[0].tcp && DEF.slots[0].udp);
        assert!(!DEF.slots[1].tcp && DEF.slots[1].udp);
        assert_eq!(DEF.pool, (7_777, 7_811));
    }
}
