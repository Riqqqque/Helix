use helix_privd::{GameKind, GamePortPolicySpec, GamePortRangeSpec, VRisingCreateSpec};
use serde_json::{Value, json};

pub(crate) const RUNTIME_IMAGE: &str = "helix-vrising-runtime:1";
pub(crate) const STEAM_APP_ID: &str = "1829350";
pub(crate) const DOCKERFILE: &str = include_str!("../../vrising/Dockerfile");
pub(crate) const ENTRYPOINT: &str = include_str!("../../vrising/entrypoint.sh");
pub(crate) const ARTIFACT_URL: &str = "steam://1829350";
pub(crate) const READY_MARKER: &str = ".helix-ready";
const EMPTY_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub(crate) fn default_port_policy() -> GamePortPolicySpec {
    GamePortPolicySpec {
        game: GameKind::VRising,
        ranges: vec![GamePortRangeSpec {
            start: 9_876,
            end: 9_910,
        }],
        ports: Vec::new(),
        auto_forward_on_create: false,
    }
}

pub(crate) fn empty_artifact_sha256() -> &'static str {
    EMPTY_SHA256
}

pub(crate) fn host_settings_json(
    name: &str,
    game_port: u16,
    query_port: u16,
    max_players: u16,
) -> Value {
    json!({
        "Name": name,
        "Description": "Hosted by Helix",
        "Port": game_port,
        "QueryPort": query_port,
        "MaxConnectedUsers": max_players,
        "MaxConnectedAdmins": 4,
        "ServerFps": 30,
        "SaveName": "world1",
        "Password": "",
        "Secure": true,
        "ListOnSteam": false,
        "ListOnEOS": false,
        "AutoSaveCount": 20,
        "AutoSaveInterval": 120,
        "GameSettingsPreset": "",
        "AdminOnlyDebugEvents": true,
        "DisableDebugEvents": false,
        "API": { "Enabled": false }
    })
}

pub(crate) fn validate_create_spec(spec: &VRisingCreateSpec) -> Result<(), String> {
    spec.validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_settings_carry_the_allocated_udp_ports() {
        let settings = host_settings_json("Castle", 9_876, 9_877, 40);
        assert_eq!(settings["Name"], "Castle");
        assert_eq!(settings["Port"], 9_876);
        assert_eq!(settings["QueryPort"], 9_877);
        assert_eq!(settings["MaxConnectedUsers"], 40);
        assert_eq!(settings["ListOnSteam"], false);
    }

    #[test]
    fn default_vrising_pool_covers_the_publisher_ports() {
        let policy = default_port_policy();
        assert_eq!(policy.game, GameKind::VRising);
        assert_eq!(policy.ranges[0].start, 9_876);
        assert_eq!(policy.ranges[0].end, 9_910);
    }
}
