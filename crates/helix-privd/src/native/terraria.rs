use helix_privd::{GameKind, GamePortPolicySpec, GamePortRangeSpec, TerrariaCreateSpec};

pub(crate) const RUNTIME_IMAGE: &str = "helix-terraria-runtime:1";
pub(crate) const DOCKERFILE: &str = include_str!("../../terraria/Dockerfile");
pub(crate) const ENTRYPOINT: &str = include_str!("../../terraria/entrypoint.sh");
pub(crate) const VANILLA_ARTIFACT_URL: &str =
    "https://terraria.org/api/download/pc-dedicated-server/terraria-server-1449.zip";
pub(crate) const TMOD_ARTIFACT_URL: &str = "steam://1281930";
pub(crate) const READY_MARKER: &str = ".helix-ready";
pub(crate) const VANILLA_VERSION: &str = "1449";
const EMPTY_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub(crate) fn default_port_policy() -> GamePortPolicySpec {
    GamePortPolicySpec {
        game: GameKind::Terraria,
        ranges: vec![GamePortRangeSpec {
            start: 7_777,
            end: 7_796,
        }],
        ports: Vec::new(),
        auto_forward_on_create: false,
    }
}

pub(crate) fn empty_artifact_sha256() -> &'static str {
    EMPTY_SHA256
}

pub(crate) fn validate_create_spec(spec: &TerrariaCreateSpec) -> Result<(), String> {
    spec.validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_terraria_pool_covers_publisher_port() {
        let policy = default_port_policy();
        assert_eq!(policy.game, GameKind::Terraria);
        assert_eq!(policy.ranges[0].start, 7_777);
    }
}
