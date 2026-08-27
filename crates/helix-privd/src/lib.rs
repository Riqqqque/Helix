//! Narrow protocol shared by the unprivileged dashboard and `helix-privd`.

pub mod mrpack;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io, path::PathBuf};
use thiserror::Error;

pub const MAX_REQUEST_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAnalysisMode {
    Quick,
    Thorough,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerRequest {
    HostInventory {},
    NetworkInventory {},
    CreateFirewallRule {
        rule: FirewallRuleSpec,
    },
    DeleteFirewallRule {
        rule_id: String,
    },
    RestoreFirewallRule {
        rule_id: String,
    },
    EnableFirewall {
        ssh_port: u16,
        confirmation: String,
    },
    SystemPackageInventory {},
    RefreshSystemPackageLists {},
    ApplySystemPackageUpdates {
        packages: Vec<PackageUpdateCandidate>,
        confirmation: String,
        disruption_acknowledged: bool,
    },
    HookInventory {},
    ManageHookService {
        hook_id: String,
        action: HookServiceAction,
    },
    HostIntegrationStatus {},
    SetHelixStartOnBoot {
        enabled: bool,
    },
    HostRebootPreflight {},
    ScheduleHostReboot {
        confirmation_hostname: String,
        delay_seconds: u16,
        disruption_acknowledged: bool,
    },
    CancelHostReboot {
        operation_id: String,
    },
    SetRecurringHostReboot {
        schedule: RecurringRebootSpec,
    },
    DeleteRecurringHostReboot {
        confirmation_hostname: String,
    },
    ExecuteRecurringHostReboot {
        schedule_id: String,
    },
    ListDirectory {
        path: String,
        cursor: Option<String>,
        limit: u16,
    },
    CreateDirectory {
        parent: String,
        name: String,
    },
    CreateFile {
        parent: String,
        name: String,
    },
    ReadText {
        path: String,
    },
    WriteText {
        path: String,
        content: String,
        expected_modified_unix_ms: Option<u64>,
    },
    Rename {
        path: String,
        new_name: String,
    },
    Trash {
        path: String,
    },
    StartStorageAnalysis {
        path: String,
        mode: StorageAnalysisMode,
    },
    StorageAnalysisStatus {
        job_id: String,
    },
    CancelStorageAnalysis {
        job_id: String,
    },
    ListServers {},
    ListTrashedServers {},
    TrashNativeServer {
        instance_id: String,
        confirmation_name: String,
    },
    RestoreTrashedServer {
        trash_id: String,
    },
    ServerInventoryHealth {},
    ServerManagerReadiness {},
    ServerDetail {
        instance_id: String,
    },
    ServerLogs {
        instance_id: String,
        lines: u16,
    },
    ServerLogHistory {
        instance_id: String,
        cursor: Option<String>,
        lines: u16,
    },
    ServerConsole {
        instance_id: String,
        command: String,
    },
    ServerSettings {
        instance_id: String,
    },
    ServerMarketplaceSearch {
        instance_id: String,
        query: String,
        offset: u32,
        limit: u8,
    },
    ServerMarketplaceProject {
        instance_id: String,
        project_id: String,
    },
    MinecraftModpackSearch {
        query: String,
        offset: u32,
        limit: u8,
    },
    MinecraftModpackProject {
        project_id: String,
    },
    InstallServerMarketplaceContent {
        instance_id: String,
        project_id: String,
        version_id: Option<String>,
    },
    UpdateServerSettings {
        instance_id: String,
        settings: MinecraftSettingsPatch,
    },
    ListBackups {
        instance_id: String,
    },
    RestoreBackup {
        instance_id: String,
        backup_id: String,
    },
    TrashBackup {
        instance_id: String,
        backup_id: String,
    },
    RestoreTrashedBackup {
        instance_id: String,
        trash_id: String,
    },
    ServerAction {
        instance_id: String,
        action: ServerAction,
    },
    CreateMinecraft {
        spec: MinecraftCreateSpec,
    },
    CreateMinecraftModpack {
        spec: MinecraftModpackCreateSpec,
    },
    JobStatus {
        job_id: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirewallProtocol {
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookServiceAction {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FirewallRuleSpec {
    pub name: String,
    pub description: String,
    pub protocol: FirewallProtocol,
    pub port_start: u16,
    pub port_end: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdateCandidate {
    pub name: String,
    pub installed_version: String,
    pub candidate_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecurringRebootSpec {
    pub weekdays: Vec<RebootWeekday>,
    pub hour: u8,
    pub minute: u8,
    pub timezone: String,
    pub confirmation_hostname: String,
    pub disruption_acknowledged: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebootWeekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerAction {
    Start,
    Stop,
    Restart,
    Update,
    Backup,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinecraftSettingsPatch {
    pub expected_revision: String,
    pub motd: String,
    pub game_mode: MinecraftGameMode,
    pub difficulty: MinecraftDifficulty,
    pub max_players: u16,
    pub view_distance: u8,
    pub simulation_distance: u8,
    pub player_idle_timeout: u16,
    pub online_mode: bool,
    pub pvp: bool,
    pub allow_flight: bool,
    pub white_list: bool,
    pub enforce_white_list: bool,
    pub spawn_protection: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MinecraftGameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MinecraftDifficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinecraftCreateSpec {
    pub name: String,
    pub software: MinecraftSoftware,
    pub version: String,
    pub memory_mb: u32,
    pub max_players: u16,
    pub game_port: u16,
    pub start_on_boot: bool,
    pub eula_accepted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinecraftModpackCreateSpec {
    pub name: String,
    pub memory_mb: u32,
    pub max_players: u16,
    pub game_port: u16,
    pub start_on_boot: bool,
    pub eula_accepted: bool,
    pub project_id: String,
    pub version_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MinecraftSoftware {
    Vanilla,
    Paper,
    Purpur,
    Folia,
    Fabric,
    NeoForge,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BrokerResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BrokerProblem>,
}

impl BrokerResponse {
    #[must_use]
    pub fn success<T: Serialize>(value: T) -> Self {
        match serde_json::to_value(value) {
            Ok(data) => Self {
                ok: true,
                data: Some(data),
                error: None,
            },
            Err(_) => Self::failure(
                "serialization_failed",
                "The broker could not encode its response.",
            ),
        }
    }

    #[must_use]
    pub fn failure(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(BrokerProblem {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BrokerProblem {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct BrokerClient {
    socket_path: PathBuf,
}

impl BrokerClient {
    #[must_use]
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    #[must_use]
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    pub fn request(&self, request: &BrokerRequest) -> Result<Value, BrokerClientError> {
        request_over_socket(&self.socket_path, request)
    }
}

#[derive(Debug, Error)]
pub enum BrokerClientError {
    #[error("privileged broker is unavailable")]
    Unavailable,
    #[error("privileged broker protocol failed")]
    Protocol,
    #[error("privileged broker rejected the request: {0}")]
    Rejected(String),
}

#[cfg(unix)]
fn request_over_socket(
    socket_path: &std::path::Path,
    request: &BrokerRequest,
) -> Result<Value, BrokerClientError> {
    use std::{
        io::{Read as _, Write as _},
        os::unix::net::UnixStream,
        time::Duration,
    };

    let body = serde_json::to_vec(request).map_err(|_| BrokerClientError::Protocol)?;
    if body.len() > MAX_REQUEST_BYTES {
        return Err(BrokerClientError::Protocol);
    }
    let length = u32::try_from(body.len()).map_err(|_| BrokerClientError::Protocol)?;
    let mut stream =
        UnixStream::connect(socket_path).map_err(|_| BrokerClientError::Unavailable)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|_| BrokerClientError::Unavailable)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| BrokerClientError::Unavailable)?;
    stream
        .write_all(&length.to_be_bytes())
        .and_then(|()| stream.write_all(&body))
        .map_err(|_| BrokerClientError::Unavailable)?;

    let mut header = [0_u8; 4];
    stream
        .read_exact(&mut header)
        .map_err(|_| BrokerClientError::Unavailable)?;
    let response_length =
        usize::try_from(u32::from_be_bytes(header)).map_err(|_| BrokerClientError::Protocol)?;
    if response_length == 0 || response_length > MAX_RESPONSE_BYTES {
        return Err(BrokerClientError::Protocol);
    }
    let mut response_body = vec![0_u8; response_length];
    stream
        .read_exact(&mut response_body)
        .map_err(|_| BrokerClientError::Unavailable)?;
    let response: BrokerResponse =
        serde_json::from_slice(&response_body).map_err(|_| BrokerClientError::Protocol)?;
    if response.ok {
        response.data.ok_or(BrokerClientError::Protocol)
    } else {
        Err(BrokerClientError::Rejected(response.error.map_or_else(
            || "request rejected".to_owned(),
            |problem| problem.message,
        )))
    }
}

#[cfg(not(unix))]
fn request_over_socket(
    _socket_path: &std::path::Path,
    _request: &BrokerRequest,
) -> Result<Value, BrokerClientError> {
    Err(BrokerClientError::Unavailable)
}

#[doc(hidden)]
pub fn read_frame(reader: &mut impl io::Read) -> io::Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let length = usize::try_from(u32::from_be_bytes(header))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame length"))?;
    if length == 0 || length > MAX_REQUEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request frame is out of bounds",
        ));
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    Ok(body)
}

#[doc(hidden)]
pub fn write_frame(writer: &mut impl io::Write, body: &[u8]) -> io::Result<()> {
    if body.is_empty() || body.len() > MAX_RESPONSE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "response frame is out of bounds",
        ));
    }
    let length = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "response is too large"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_rejects_unknown_fields() {
        let input = br#"{"operation":"host_inventory","unexpected":true}"#;
        assert!(serde_json::from_slice::<BrokerRequest>(input).is_err());
    }

    #[test]
    fn create_spec_has_stable_wire_shape() {
        let request = BrokerRequest::CreateMinecraft {
            spec: MinecraftCreateSpec {
                name: "Survival".to_owned(),
                software: MinecraftSoftware::Paper,
                version: "1.21.8".to_owned(),
                memory_mb: 4096,
                max_players: 20,
                game_port: 25565,
                start_on_boot: true,
                eula_accepted: true,
            },
        };
        let encoded = serde_json::to_value(request).expect("serialize request");
        assert_eq!(encoded["operation"], "create_minecraft");
        assert_eq!(encoded["spec"]["software"], "paper");
    }

    #[test]
    fn backup_trash_requests_have_path_opaque_wire_shapes() {
        let trash = BrokerRequest::TrashBackup {
            instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
            backup_id: "1787799939239".to_owned(),
        };
        let encoded = serde_json::to_value(trash).expect("serialize trash request");
        assert_eq!(encoded["operation"], "trash_backup");
        assert_eq!(encoded["backup_id"], "1787799939239");
        assert!(encoded.get("path").is_none());

        let restore = BrokerRequest::RestoreTrashedBackup {
            instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
            trash_id: "8953dc16-3891-42bf-802f-711b3ba2965a".to_owned(),
        };
        let encoded = serde_json::to_value(restore).expect("serialize undo request");
        assert_eq!(encoded["operation"], "restore_trashed_backup");
        assert_eq!(encoded["trash_id"], "8953dc16-3891-42bf-802f-711b3ba2965a");
        assert!(encoded.get("path").is_none());
    }

    #[test]
    fn host_control_requests_have_narrow_typed_wire_shapes() {
        assert_eq!(
            serde_json::to_value(BrokerRequest::HostIntegrationStatus {}).unwrap()["operation"],
            "host_integration_status"
        );
        assert_eq!(
            serde_json::to_value(BrokerRequest::HostRebootPreflight {}).unwrap()["operation"],
            "host_reboot_preflight"
        );
        let setting = serde_json::to_value(BrokerRequest::SetHelixStartOnBoot { enabled: true })
            .expect("serialize start-on-boot request");
        assert_eq!(setting["operation"], "set_helix_start_on_boot");
        assert_eq!(setting["enabled"], true);
        assert_eq!(setting.as_object().expect("request object").len(), 2);

        let reboot = serde_json::to_value(BrokerRequest::ScheduleHostReboot {
            confirmation_hostname: "helix-host".to_owned(),
            delay_seconds: 30,
            disruption_acknowledged: true,
        })
        .expect("serialize reboot request");
        assert_eq!(reboot["operation"], "schedule_host_reboot");
        assert_eq!(reboot["confirmation_hostname"], "helix-host");
        assert_eq!(reboot["delay_seconds"], 30);
        assert_eq!(reboot["disruption_acknowledged"], true);
        assert!(reboot.get("command").is_none());
        assert!(reboot.get("path").is_none());

        let cancel = serde_json::to_value(BrokerRequest::CancelHostReboot {
            operation_id: "12345678-1234-4234-8234-123456789abc".to_owned(),
        })
        .expect("serialize reboot cancellation");
        assert_eq!(cancel["operation"], "cancel_host_reboot");
        assert_eq!(
            cancel["operation_id"],
            "12345678-1234-4234-8234-123456789abc"
        );
        assert!(cancel.get("unit").is_none());

        let recurring = serde_json::to_value(BrokerRequest::SetRecurringHostReboot {
            schedule: RecurringRebootSpec {
                weekdays: vec![RebootWeekday::Monday, RebootWeekday::Friday],
                hour: 17,
                minute: 30,
                timezone: "America/Denver".to_owned(),
                confirmation_hostname: "helix-host".to_owned(),
                disruption_acknowledged: true,
            },
        })
        .expect("serialize recurring reboot");
        assert_eq!(recurring["operation"], "set_recurring_host_reboot");
        assert_eq!(recurring["schedule"]["weekdays"][0], "monday");
        assert_eq!(recurring["schedule"]["timezone"], "America/Denver");
        assert!(recurring.get("calendar").is_none());
        assert!(recurring.get("unit").is_none());

        let trigger = serde_json::to_value(BrokerRequest::ExecuteRecurringHostReboot {
            schedule_id: "12345678-1234-4234-8234-123456789abc".to_owned(),
        })
        .expect("serialize recurring trigger");
        assert_eq!(trigger["operation"], "execute_recurring_host_reboot");
        assert!(trigger.get("command").is_none());
    }

    #[test]
    fn server_inventory_health_has_a_stable_read_only_wire_shape() {
        let encoded = serde_json::to_value(BrokerRequest::ServerInventoryHealth {})
            .expect("serialize server inventory health request");
        assert_eq!(encoded["operation"], "server_inventory_health");
        assert_eq!(encoded.as_object().expect("request object").len(), 1);
    }

    #[test]
    fn paginated_log_history_request_preserves_legacy_logs_shape() {
        let history = serde_json::to_value(BrokerRequest::ServerLogHistory {
            instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
            cursor: Some("h1.0000000000000001.0000000000000040".to_owned()),
            lines: 500,
        })
        .expect("serialize log history request");
        assert_eq!(history["operation"], "server_log_history");
        assert_eq!(history["lines"], 500);
        assert!(history.get("path").is_none());

        let legacy = serde_json::to_value(BrokerRequest::ServerLogs {
            instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
            lines: 300,
        })
        .expect("serialize legacy log request");
        assert_eq!(legacy["operation"], "server_logs");
        assert_eq!(legacy.as_object().expect("legacy request object").len(), 3);
        assert!(legacy.get("cursor").is_none());
    }

    #[test]
    fn storage_analysis_requests_have_opaque_bounded_wire_shapes() {
        let start = serde_json::to_value(BrokerRequest::StartStorageAnalysis {
            path: "/srv/media".to_owned(),
            mode: StorageAnalysisMode::Thorough,
        })
        .expect("serialize analysis start");
        assert_eq!(start["operation"], "start_storage_analysis");
        assert_eq!(start["path"], "/srv/media");
        assert_eq!(start["mode"], "thorough");
        assert_eq!(start.as_object().expect("start object").len(), 3);

        let job_id = "12345678-1234-4234-8234-123456789abc";
        let status = serde_json::to_value(BrokerRequest::StorageAnalysisStatus {
            job_id: job_id.to_owned(),
        })
        .expect("serialize analysis status");
        assert_eq!(status["operation"], "storage_analysis_status");
        assert_eq!(status["job_id"], job_id);
        assert!(status.get("path").is_none());

        let cancel = serde_json::to_value(BrokerRequest::CancelStorageAnalysis {
            job_id: job_id.to_owned(),
        })
        .expect("serialize analysis cancellation");
        assert_eq!(cancel["operation"], "cancel_storage_analysis");
        assert_eq!(cancel["job_id"], job_id);
        assert!(cancel.get("path").is_none());
    }

    #[test]
    fn directory_listing_request_has_only_cursor_pagination_inputs() {
        let list = serde_json::to_value(BrokerRequest::ListDirectory {
            path: "/HDD10tb1/TV & Movies".to_owned(),
            cursor: Some("movie 050.mkv".to_owned()),
            limit: 50,
        })
        .expect("serialize directory page request");
        assert_eq!(list["operation"], "list_directory");
        assert_eq!(list["path"], "/HDD10tb1/TV & Movies");
        assert_eq!(list["cursor"], "movie 050.mkv");
        assert_eq!(list["limit"], 50);
        assert_eq!(list.as_object().expect("directory request").len(), 4);
    }

    #[test]
    fn marketplace_requests_never_accept_download_paths_or_urls() {
        let search = serde_json::to_value(BrokerRequest::ServerMarketplaceSearch {
            instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
            query: "world edit".to_owned(),
            offset: 0,
            limit: 20,
        })
        .expect("serialize marketplace search");
        assert_eq!(search["operation"], "server_marketplace_search");
        assert_eq!(search["query"], "world edit");
        assert!(search.get("url").is_none());
        assert!(search.get("path").is_none());

        let install = serde_json::to_value(BrokerRequest::InstallServerMarketplaceContent {
            instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
            project_id: "1bokaNcj".to_owned(),
            version_id: Some("abcdef12".to_owned()),
        })
        .expect("serialize marketplace install");
        assert_eq!(install["operation"], "install_server_marketplace_content");
        assert_eq!(install["project_id"], "1bokaNcj");
        assert_eq!(install["version_id"], "abcdef12");
        assert!(install.get("url").is_none());
        assert!(install.get("filename").is_none());
        assert!(install.get("path").is_none());
    }

    #[test]
    fn modpack_requests_only_carry_opaque_modrinth_ids_and_server_settings() {
        let search = serde_json::to_value(BrokerRequest::MinecraftModpackSearch {
            query: "adventure".to_owned(),
            offset: 20,
            limit: 20,
        })
        .expect("serialize modpack search");
        assert_eq!(search["operation"], "minecraft_modpack_search");
        assert!(search.get("url").is_none());
        assert!(search.get("loader").is_none());

        let create = serde_json::to_value(BrokerRequest::CreateMinecraftModpack {
            spec: MinecraftModpackCreateSpec {
                name: "Fabric Adventure".to_owned(),
                memory_mb: 6144,
                max_players: 20,
                game_port: 25_565,
                start_on_boot: true,
                eula_accepted: true,
                project_id: "AABBcc11".to_owned(),
                version_id: "version22".to_owned(),
            },
        })
        .expect("serialize modpack create");
        assert_eq!(create["operation"], "create_minecraft_modpack");
        assert_eq!(create["spec"]["project_id"], "AABBcc11");
        assert_eq!(create["spec"]["version_id"], "version22");
        for forbidden in [
            "url",
            "path",
            "filename",
            "minecraft_version",
            "loader_version",
        ] {
            assert!(create["spec"].get(forbidden).is_none(), "{forbidden}");
        }
    }

    #[test]
    fn network_and_package_requests_have_narrow_typed_wire_shapes() {
        let create = serde_json::to_value(BrokerRequest::CreateFirewallRule {
            rule: FirewallRuleSpec {
                name: "Minecraft".to_owned(),
                description: "Survival server".to_owned(),
                protocol: FirewallProtocol::Tcp,
                port_start: 25_565,
                port_end: 25_570,
            },
        })
        .expect("serialize firewall create");
        assert_eq!(create["operation"], "create_firewall_rule");
        assert_eq!(create["rule"]["protocol"], "tcp");
        assert_eq!(create["rule"]["port_start"], 25_565);
        assert!(create.get("command").is_none());

        let delete = serde_json::to_value(BrokerRequest::DeleteFirewallRule {
            rule_id: "8953dc16-3891-42bf-802f-711b3ba2965a".to_owned(),
        })
        .expect("serialize firewall delete");
        assert_eq!(delete["operation"], "delete_firewall_rule");
        assert!(delete.get("number").is_none());
        assert!(delete.get("comment").is_none());

        assert_eq!(
            serde_json::to_value(BrokerRequest::NetworkInventory {}).unwrap()["operation"],
            "network_inventory"
        );
        assert_eq!(
            serde_json::to_value(BrokerRequest::SystemPackageInventory {}).unwrap()["operation"],
            "system_package_inventory"
        );

        let refresh = serde_json::to_value(BrokerRequest::RefreshSystemPackageLists {})
            .expect("serialize package-list refresh");
        assert_eq!(refresh["operation"], "refresh_system_package_lists");
        assert_eq!(refresh.as_object().expect("refresh object").len(), 1);

        let apply = serde_json::to_value(BrokerRequest::ApplySystemPackageUpdates {
            packages: vec![PackageUpdateCandidate {
                name: "openssl".to_owned(),
                installed_version: "3.0.2-0ubuntu1.18".to_owned(),
                candidate_version: "3.0.2-0ubuntu1.19".to_owned(),
            }],
            confirmation: "APPLY 1 UPDATE".to_owned(),
            disruption_acknowledged: true,
        })
        .expect("serialize package update");
        assert_eq!(apply["operation"], "apply_system_package_updates");
        assert_eq!(apply["packages"][0]["name"], "openssl");
        assert_eq!(apply["confirmation"], "APPLY 1 UPDATE");
        assert_eq!(apply["disruption_acknowledged"], true);
        assert!(apply.get("command").is_none());
        assert!(apply.get("arguments").is_none());
    }

    #[test]
    fn hook_requests_carry_only_an_opaque_id_and_typed_action() {
        let inventory = serde_json::to_value(BrokerRequest::HookInventory {})
            .expect("serialize hook inventory");
        assert_eq!(inventory["operation"], "hook_inventory");
        assert_eq!(inventory.as_object().expect("inventory object").len(), 1);

        let action = serde_json::to_value(BrokerRequest::ManageHookService {
            hook_id: "plex".to_owned(),
            action: HookServiceAction::Restart,
        })
        .expect("serialize hook action");
        assert_eq!(action["operation"], "manage_hook_service");
        assert_eq!(action["hook_id"], "plex");
        assert_eq!(action["action"], "restart");
        assert!(action.get("unit").is_none());
        assert!(action.get("command").is_none());
    }
}
