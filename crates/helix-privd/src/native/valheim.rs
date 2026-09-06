use helix_privd::{GameKind, GamePortPolicySpec, GamePortRangeSpec, ValheimCreateSpec};

pub(crate) const RUNTIME_IMAGE: &str = "helix-valheim-runtime:1";
pub(crate) const STEAM_APP_ID: &str = "896660";
pub(crate) const DOCKERFILE: &str = include_str!("../../valheim/Dockerfile");
pub(crate) const ENTRYPOINT: &str = include_str!("../../valheim/entrypoint.sh");
pub(crate) const ARTIFACT_URL: &str = "steam://896660";
const EMPTY_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub(crate) fn default_port_policy() -> GamePortPolicySpec {
    GamePortPolicySpec {
        game: GameKind::Valheim,
        ranges: vec![GamePortRangeSpec {
            start: 2_456,
            end: 2_490,
        }],
        ports: Vec::new(),
        auto_forward_on_create: false,
    }
}

pub(crate) fn empty_artifact_sha256() -> &'static str {
    EMPTY_SHA256
}

pub(crate) fn validate_create_spec(spec: &ValheimCreateSpec) -> Result<(), String> {
    spec.validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_valheim_pool_covers_publisher_ports() {
        let policy = default_port_policy();
        assert_eq!(policy.game, GameKind::Valheim);
        assert_eq!(policy.ranges[0].start, 2_456);
        assert_eq!(policy.ranges[0].end, 2_490);
        assert!(!policy.auto_forward_on_create);
    }
}
