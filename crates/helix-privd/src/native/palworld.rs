use helix_privd::{GameKind, GamePortPolicySpec, GamePortRangeSpec, PalworldCreateSpec};

pub(crate) const RUNTIME_IMAGE: &str = "helix-palworld-runtime:1";
pub(crate) const STEAM_APP_ID: &str = "2394010";
pub(crate) const DOCKERFILE: &str = include_str!("../../palworld/Dockerfile");
pub(crate) const ENTRYPOINT: &str = include_str!("../../palworld/entrypoint.sh");
pub(crate) const ARTIFACT_URL: &str = "steam://2394010";
const EMPTY_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub(crate) fn default_port_policy() -> GamePortPolicySpec {
    GamePortPolicySpec {
        game: GameKind::Palworld,
        ranges: vec![GamePortRangeSpec {
            start: 8_211,
            end: 8_245,
        }],
        ports: Vec::new(),
        auto_forward_on_create: false,
    }
}

pub(crate) fn empty_artifact_sha256() -> &'static str {
    EMPTY_SHA256
}

pub(crate) fn validate_create_spec(spec: &PalworldCreateSpec) -> Result<(), String> {
    spec.validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palworld_pool_covers_the_publisher_port() {
        let policy = default_port_policy();
        assert_eq!(policy.game, GameKind::Palworld);
        assert_eq!(policy.ranges[0].start, 8_211);
        assert!(policy.ranges[0].end > 8_211);
    }
}
