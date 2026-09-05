//! Narrow protocol shared by the unprivileged dashboard and `helix-privd`.

pub mod migrate_plan;
pub mod mrpack;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io, path::PathBuf};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

pub const MAX_REQUEST_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_FILE_UPLOAD_CHUNK_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_STORAGE_UPLOAD_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_CUSTOM_JAR_UPLOAD_BYTES: u64 = 768 * 1024 * 1024;
pub const MAX_CONCURRENT_FILE_UPLOADS: usize = 2;
pub const MAX_MINECRAFT_VERSION_CATALOG: usize = 128;

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
    GlobeSnapshot {},
    GamePortPolicy {
        game: GameKind,
    },
    SetGamePortPolicy {
        policy: GamePortPolicySpec,
    },
    SetServerNetworkExposure {
        instance_id: String,
        enabled: bool,
    },
    ReleaseAmpRouterForward {
        port: u16,
        confirmation: String,
    },
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
    CheckHelixUpdate {},
    ApplyHelixUpdate {
        target_tag: String,
        confirmation: String,
        disruption_acknowledged: bool,
    },
    HookInventory {},
    HookInstallPreflight {
        hook_id: String,
    },
    InstallHook {
        hook_id: String,
        confirmation: String,
        repository_change_acknowledged: bool,
    },
    ManageHookService {
        hook_id: String,
        action: HookServiceAction,
    },
    DockerInventory {},
    DockerContainerAction {
        name: String,
        action: DockerContainerActionKind,
        confirmation: String,
    },
    HomarrWidgetCatalog {},
    SecurityInventory {},
    SetSecurityControl {
        id: String,
        enabled: bool,
        confirmation: String,
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
    BeginFileUpload {
        target: FileUploadTarget,
        name: String,
        expected_size: u64,
    },
    WriteFileUploadChunk {
        upload_id: String,
        purpose: FileUploadPurpose,
        offset: u64,
        data_base64: String,
    },
    FinishFileUpload {
        upload_id: String,
        purpose: FileUploadPurpose,
    },
    AbortFileUpload {
        upload_id: String,
        purpose: FileUploadPurpose,
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
    PurgeTrashedServer {
        trash_id: String,
        confirmation_name: String,
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
        #[serde(default, skip_serializing_if = "is_modrinth_provider")]
        provider: ModpackProvider,
        #[serde(default, skip_serializing_if = "is_content_catalog")]
        catalog: MarketplaceCatalog,
    },
    ServerMarketplaceProject {
        instance_id: String,
        project_id: String,
        #[serde(default, skip_serializing_if = "is_modrinth_provider")]
        provider: ModpackProvider,
    },
    MinecraftModpackSearch {
        query: String,
        offset: u32,
        limit: u8,
        #[serde(default)]
        provider: ModpackProvider,
    },
    MinecraftModpackProject {
        project_id: String,
        #[serde(default, skip_serializing_if = "is_modrinth_provider")]
        provider: ModpackProvider,
    },
    InstallServerMarketplaceContent {
        instance_id: String,
        project_id: String,
        version_id: Option<String>,
        #[serde(default, skip_serializing_if = "is_modrinth_provider")]
        provider: ModpackProvider,
        #[serde(default, skip_serializing_if = "is_false")]
        restart_server: bool,
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
    SetBackupPolicy {
        instance_id: String,
        keep_count: u16,
        keep_days: u16,
    },
    PruneBackups {
        instance_id: String,
    },
    PurgeBackupTrash {
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
    #[serde(rename = "create_vrising")]
    CreateVRising {
        spec: VRisingCreateSpec,
    },
    CreateValheim {
        spec: ValheimCreateSpec,
    },
    CreateTerraria {
        spec: TerrariaCreateSpec,
    },
    SetNativeStartOnBoot {
        instance_id: String,
        enabled: bool,
    },
    SetNativeMemory {
        instance_id: String,
        memory_mb: u32,
    },
    SetNativeCpu {
        instance_id: String,
        cpu_millis: u32,
    },
    SetNativeBrowserListing {
        instance_id: String,
        list_on_browser: bool,
    },
    ListMinecraftVersions {
        software: MinecraftSoftware,
    },
    MigrateServerPreflight {
        source: ServerMigrateSource,
    },
    MigrateServer {
        spec: ServerMigrateSpec,
    },
    JobStatus {
        job_id: String,
    },
    CurseforgeKeyStatus {},
    SetCurseforgeApiKey {
        key: String,
    },
    ClearCurseforgeApiKey {},
}

pub const CURSEFORGE_API_KEY_REQUIRED: &str = "CurseForge needs an API key. Open Settings → Catalogs, paste a key from console.curseforge.com, then search again.";
pub const CURSEFORGE_KEY_REJECTED: &str =
    "CurseForge rejected the saved API key. Replace it in Settings → Catalogs.";
pub const CURSEFORGE_CDN_BLOCKED: &str = "CurseForge's CDN blocked this host's public IP. That is not a bad key. Search needs this host to reach the internet without a VPS or VPN exit they block.";
pub const CURSEFORGE_RATE_LIMITED: &str =
    "CurseForge rate-limited this host. Wait a bit and try again.";

pub fn validate_curseforge_api_key(value: &str) -> Result<String, String> {
    let mut key = strip_curseforge_key_noise(value);
    for _ in 0..3 {
        let unquoted = strip_wrapping_quotes(key.trim());
        if unquoted.len() == key.trim().len() {
            break;
        }
        key = unquoted.to_owned();
    }
    key = key.trim().to_owned();
    if key.contains("$$2") {
        key = key.replace("$$", "$");
    }
    if let Some(token) = extract_curseforge_console_key(&key) {
        key = token;
    } else {
        key.retain(|character| character.is_ascii_graphic());
    }
    if key.len() < 24 || key.len() > 256 {
        return Err("CurseForge API keys are 24–256 characters".to_owned());
    }
    if !key.chars().all(|character| character.is_ascii_graphic()) {
        return Err("that CurseForge API key contains characters Helix will not store".to_owned());
    }
    Ok(key)
}

fn strip_curseforge_key_noise(value: &str) -> String {
    value
        .nfkc()
        .filter(|character| !curseforge_key_ignorable(*character))
        .map(map_curseforge_key_punctuation)
        .collect()
}

fn curseforge_key_ignorable(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00ad}'
                | '\u{034f}'
                | '\u{061c}'
                | '\u{180e}'
                | '\u{200b}'
                | '\u{200c}'
                | '\u{200d}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'
                | '\u{202b}'
                | '\u{202c}'
                | '\u{202d}'
                | '\u{202e}'
                | '\u{2060}'
                | '\u{2061}'
                | '\u{2062}'
                | '\u{2063}'
                | '\u{2064}'
                | '\u{2066}'
                | '\u{2067}'
                | '\u{2068}'
                | '\u{2069}'
                | '\u{206a}'
                | '\u{206b}'
                | '\u{206c}'
                | '\u{206d}'
                | '\u{206e}'
                | '\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'
                | '\u{fffa}'
                | '\u{fffb}'
        )
}

fn map_curseforge_key_punctuation(character: char) -> char {
    match character {
        '\u{00a0}' | '\u{202f}' | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200a}' => ' ',
        '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' => '\'',
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' => '"',
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' => '-',
        other => other,
    }
}

fn strip_wrapping_quotes(value: &str) -> &str {
    let value = value.trim();
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return value;
    };
    let Some(last) = characters.next_back() else {
        return value;
    };
    if matches!(
        (first, last),
        ('"', '"') | ('\'', '\'') | ('\u{201c}', '\u{201d}') | ('\u{2018}', '\u{2019}')
    ) {
        let start = first.len_utf8();
        &value[start..value.len() - last.len_utf8()]
    } else {
        value
    }
}

fn extract_curseforge_console_key(value: &str) -> Option<String> {
    let start = value.find("$2")?;
    let mut token = String::new();
    for character in value[start..].chars() {
        if character == '$'
            || character == '/'
            || character == '.'
            || character.is_ascii_alphanumeric()
        {
            token.push(character);
            if token.len() >= 256 {
                break;
            }
        } else {
            break;
        }
    }
    (token.len() >= 24).then_some(token)
}

pub fn curl_extra_header_file(headers: &[(&str, &str)]) -> Result<String, String> {
    let mut body = String::new();
    for (name, value) in headers {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.contains(['\r', '\n'])
        {
            return Err("the catalog request used an invalid header".to_owned());
        }
        body.push_str(name);
        body.push_str(": ");
        body.push_str(value);
        body.push('\n');
    }
    Ok(body)
}

pub fn http_dump_status(dump: &str) -> Option<u16> {
    dump.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("HTTP/1.0 ")
                .or_else(|| line.strip_prefix("HTTP/1.1 "))
                .or_else(|| line.strip_prefix("HTTP/2 "))
                .or_else(|| line.strip_prefix("HTTP/3 "))?;
            rest.split_whitespace().next()?.parse().ok()
        })
        .next_back()
}

pub fn http_status_from_curl_stderr(stderr: &str) -> Option<u16> {
    let marker = "returned error: ";
    let idx = stderr.rfind(marker)?;
    stderr[idx + marker.len()..]
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

pub fn classify_curseforge_curl_error(stderr: &str, header_dump: &str, body: &[u8]) -> String {
    let status = http_dump_status(header_dump).or_else(|| http_status_from_curl_stderr(stderr));
    let body_text = String::from_utf8_lossy(body).to_ascii_lowercase();
    let dump = header_dump.to_ascii_lowercase();
    let cloudfront = dump.contains("cloudfront");
    let key_text = body_text.contains("api key")
        || body_text.contains("unauthorized")
        || body_text.contains("missing or invalid");
    match status {
        Some(429) => CURSEFORGE_RATE_LIMITED.to_owned(),
        Some(401 | 403) if key_text => CURSEFORGE_KEY_REJECTED.to_owned(),
        Some(401 | 403) if body.is_empty() || cloudfront => CURSEFORGE_CDN_BLOCKED.to_owned(),
        Some(401 | 403) => CURSEFORGE_KEY_REJECTED.to_owned(),
        Some(code) if (500..600).contains(&code) => {
            format!("CurseForge catalog was unreachable. HTTP {code}")
        }
        _ => format!("CurseForge catalog was unreachable. {stderr}"),
    }
}

pub fn catalog_fetch_is_non_retryable(error: &str) -> bool {
    error == CURSEFORGE_CDN_BLOCKED
        || error == CURSEFORGE_KEY_REJECTED
        || error == CURSEFORGE_RATE_LIMITED
        || error.contains("returned error: 4")
        || error.contains("HTTP 4")
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirewallProtocol {
    Tcp,
    Udp,
}

impl FirewallProtocol {
    pub fn soap_name(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerContainerActionKind {
    Start,
    Stop,
    Restart,
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
    Kill,
    Update,
    Backup,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GameKind {
    #[default]
    Minecraft,
    #[serde(rename = "vrising")]
    VRising,
    Valheim,
    Terraria,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerNetworkExposure {
    #[default]
    Private,
    Public,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GamePortRangeSpec {
    pub start: u16,
    pub end: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GamePortPolicySpec {
    pub game: GameKind,
    pub ranges: Vec<GamePortRangeSpec>,
    pub ports: Vec<u16>,
    pub auto_forward_on_create: bool,
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
    pub game_port: u16,
    pub memory_mb: u32,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pumpkin_bedrock_port: Option<u16>,
    pub name: String,
    pub software: MinecraftSoftware,
    pub version: String,
    pub memory_mb: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub cpu_millis: u32,
    pub max_players: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_port: Option<u16>,
    #[serde(default)]
    pub network_exposure: ServerNetworkExposure,
    pub start_on_boot: bool,
    pub eula_accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_jar: Option<CustomMinecraftJarSpec>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustomMinecraftJarSpec {
    pub source_path: String,
    pub java_version: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerMigrateSource {
    Amp { instance_id: String },
    Folder { path: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerMigrateSpec {
    pub source: ServerMigrateSource,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<GameKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub software: Option<MinecraftSoftware>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub memory_mb: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub cpu_millis: u32,
    pub max_players: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_port: Option<u16>,
    #[serde(default)]
    pub network_exposure: ServerNetworkExposure,
    pub start_on_boot: bool,
    pub eula_accepted: bool,
    pub source_stopped: bool,
    pub copy_acknowledged: bool,
    #[serde(default = "default_true")]
    pub list_on_browser: bool,
    #[serde(default)]
    pub wine_runtime_acknowledged: bool,
}

impl ServerMigrateSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_dedicated_name(&self.name)?;
        if !self.copy_acknowledged {
            return Err(
                "confirm that Helix will copy into a new native server and leave the source manager alone"
                    .to_owned(),
            );
        }
        if !self.source_stopped {
            return Err("confirm the source server is stopped before copying".to_owned());
        }
        validate_cpu_millis(self.cpu_millis)?;
        if self.game_port.is_some_and(|port| port < 1_024) {
            return Err("game port must be at least 1024".to_owned());
        }
        if self.query_port.is_some_and(|port| port < 1_024) {
            return Err("query port must be at least 1024".to_owned());
        }
        Ok(())
    }

    pub fn validate_for_game(&self, game: GameKind) -> Result<(), String> {
        self.validate()?;
        match game {
            GameKind::Minecraft => {
                if !self.eula_accepted {
                    return Err("the Minecraft EULA must be explicitly accepted".to_owned());
                }
                if !(1_024..=24_576).contains(&self.memory_mb) {
                    return Err("memory must be between 1 and 24 GiB".to_owned());
                }
                if !(1..=10_000).contains(&self.max_players) {
                    return Err("player limit must be between 1 and 10,000".to_owned());
                }
                Ok(())
            }
            GameKind::VRising => VRisingCreateSpec {
                name: self.name.clone(),
                memory_mb: self.memory_mb,
                cpu_millis: self.cpu_millis,
                max_players: self.max_players,
                game_port: self.game_port,
                query_port: self.query_port,
                network_exposure: self.network_exposure,
                list_on_browser: self.list_on_browser,
                start_on_boot: self.start_on_boot,
                wine_runtime_acknowledged: self.wine_runtime_acknowledged,
            }
            .validate(),
            GameKind::Valheim => ValheimCreateSpec {
                name: self.name.clone(),
                memory_mb: self.memory_mb,
                cpu_millis: self.cpu_millis,
                max_players: self.max_players,
                game_port: self.game_port,
                network_exposure: self.network_exposure,
                start_on_boot: self.start_on_boot,
            }
            .validate(),
            GameKind::Terraria => TerrariaCreateSpec {
                name: self.name.clone(),
                software: TerrariaSoftware::Vanilla,
                memory_mb: self.memory_mb,
                cpu_millis: self.cpu_millis,
                max_players: self.max_players,
                game_port: self.game_port,
                network_exposure: self.network_exposure,
                start_on_boot: self.start_on_boot,
            }
            .validate(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VRisingCreateSpec {
    pub name: String,
    pub memory_mb: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub cpu_millis: u32,
    pub max_players: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_port: Option<u16>,
    #[serde(default)]
    pub network_exposure: ServerNetworkExposure,
    #[serde(default = "default_true")]
    pub list_on_browser: bool,
    pub start_on_boot: bool,
    pub wine_runtime_acknowledged: bool,
}

impl VRisingCreateSpec {
    pub fn validate(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty()
            || name.len() > 80
            || name.chars().any(char::is_control)
            || name.contains(['/', '\\'])
        {
            return Err("server name must be 1–80 ordinary characters".to_owned());
        }
        if !(2_048..=24_576).contains(&self.memory_mb) {
            return Err("V Rising memory must be between 2 and 24 GiB".to_owned());
        }
        validate_cpu_millis(self.cpu_millis)?;
        if !(1..=128).contains(&self.max_players) {
            return Err("V Rising player limit must be between 1 and 128".to_owned());
        }
        if self.game_port.is_some_and(|port| port < 1_024) {
            return Err("game port must be at least 1024".to_owned());
        }
        if self.query_port.is_some_and(|port| port < 1_024) {
            return Err("query port must be at least 1024".to_owned());
        }
        if let (Some(game_port), Some(query_port)) = (self.game_port, self.query_port)
            && game_port == query_port
        {
            return Err("V Rising game and query ports must be different".to_owned());
        }
        if self.query_port.is_some() && self.game_port.is_none() {
            return Err("a query port also needs a game port".to_owned());
        }
        if !self.wine_runtime_acknowledged {
            return Err("Helix could not confirm the isolated V Rising runtime install".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValheimCreateSpec {
    pub name: String,
    pub memory_mb: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub cpu_millis: u32,
    pub max_players: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_port: Option<u16>,
    #[serde(default)]
    pub network_exposure: ServerNetworkExposure,
    pub start_on_boot: bool,
}

impl ValheimCreateSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_dedicated_name(&self.name)?;
        if !(1_024..=16_384).contains(&self.memory_mb) {
            return Err("Valheim memory must be between 1 and 16 GiB".to_owned());
        }
        validate_cpu_millis(self.cpu_millis)?;
        if !(1..=64).contains(&self.max_players) {
            return Err("Valheim player limit must be between 1 and 64".to_owned());
        }
        if self.game_port.is_some_and(|port| port < 1_024) {
            return Err("game port must be at least 1024".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerrariaCreateSpec {
    pub name: String,
    pub software: TerrariaSoftware,
    pub memory_mb: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub cpu_millis: u32,
    pub max_players: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_port: Option<u16>,
    #[serde(default)]
    pub network_exposure: ServerNetworkExposure,
    pub start_on_boot: bool,
}

impl TerrariaCreateSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_dedicated_name(&self.name)?;
        if !(512..=8_192).contains(&self.memory_mb) {
            return Err("Terraria memory must be between 512 MiB and 8 GiB".to_owned());
        }
        validate_cpu_millis(self.cpu_millis)?;
        if !(1..=255).contains(&self.max_players) {
            return Err("Terraria player limit must be between 1 and 255".to_owned());
        }
        if self.game_port.is_some_and(|port| port < 1_024) {
            return Err("game port must be at least 1024".to_owned());
        }
        Ok(())
    }
}

fn validate_dedicated_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 80
        || name.chars().any(char::is_control)
        || name.contains(['/', '\\'])
    {
        return Err("server name must be 1–80 ordinary characters".to_owned());
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinecraftModpackCreateSpec {
    pub name: String,
    pub memory_mb: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub cpu_millis: u32,
    pub max_players: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_port: Option<u16>,
    #[serde(default)]
    pub network_exposure: ServerNetworkExposure,
    pub start_on_boot: bool,
    pub eula_accepted: bool,
    pub project_id: String,
    pub version_id: String,
    #[serde(default, skip_serializing_if = "is_modrinth_provider")]
    pub provider: ModpackProvider,
}

fn is_modrinth_provider(provider: &ModpackProvider) -> bool {
    matches!(provider, ModpackProvider::Modrinth)
}

fn is_content_catalog(catalog: &MarketplaceCatalog) -> bool {
    matches!(catalog, MarketplaceCatalog::Content)
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn default_true() -> bool {
    true
}

pub fn validate_cpu_millis(cpu_millis: u32) -> Result<(), String> {
    if cpu_millis == 0 || (250..=128_000).contains(&cpu_millis) {
        Ok(())
    } else {
        Err("CPU limit must be off, or between 0.25 and 128 cores".to_owned())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MinecraftSoftware {
    Pumpkin,
    Custom,
    Vanilla,
    Paper,
    Purpur,
    Folia,
    Leaves,
    Fabric,
    NeoForge,
    Forge,
    Quilt,
    Pufferfish,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceCatalog {
    #[default]
    Content,
    Modpacks,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModpackProvider {
    #[default]
    Modrinth,
    Curseforge,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerrariaSoftware {
    #[default]
    Vanilla,
    Tmodloader,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileUploadPurpose {
    Storage,
    CustomJar,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileUploadTarget {
    Directory { parent: String },
    CustomJar,
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
        .set_read_timeout(Some(broker_read_timeout(request)))
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

#[cfg_attr(not(unix), allow(dead_code))]
fn broker_read_timeout(request: &BrokerRequest) -> std::time::Duration {
    match request {
        BrokerRequest::TrashNativeServer { .. }
        | BrokerRequest::RestoreTrashedServer { .. }
        | BrokerRequest::PurgeTrashedServer { .. } => std::time::Duration::from_secs(300),
        BrokerRequest::ServerMarketplaceSearch { .. }
        | BrokerRequest::ServerMarketplaceProject { .. }
        | BrokerRequest::MinecraftModpackSearch { .. }
        | BrokerRequest::MinecraftModpackProject { .. }
        | BrokerRequest::SetCurseforgeApiKey { .. } => std::time::Duration::from_secs(45),
        _ => std::time::Duration::from_secs(30),
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
                pumpkin_bedrock_port: None,
                name: "Survival".to_owned(),
                software: MinecraftSoftware::Paper,
                version: "1.21.8".to_owned(),
                memory_mb: 4096,
                cpu_millis: 0,
                max_players: 20,
                game_port: Some(25565),
                network_exposure: ServerNetworkExposure::Private,
                start_on_boot: true,
                eula_accepted: true,
                custom_jar: None,
            },
        };
        let encoded = serde_json::to_value(request).expect("serialize request");
        assert_eq!(encoded["operation"], "create_minecraft");
        assert_eq!(encoded["spec"]["software"], "paper");
        assert!(encoded["spec"].get("custom_jar").is_none());
        assert!(encoded["spec"].get("cpu_millis").is_none());

        let migrate = serde_json::to_value(BrokerRequest::MigrateServer {
            spec: ServerMigrateSpec {
                source: ServerMigrateSource::Amp {
                    instance_id: "amp:12345678-1234-4234-8234-123456789abc".to_owned(),
                },
                name: "Survival".to_owned(),
                game: Some(GameKind::Minecraft),
                software: Some(MinecraftSoftware::Paper),
                version: Some("1.21.8".to_owned()),
                memory_mb: 4_096,
                cpu_millis: 0,
                max_players: 20,
                game_port: None,
                query_port: None,
                network_exposure: ServerNetworkExposure::Private,
                start_on_boot: true,
                eula_accepted: true,
                source_stopped: true,
                copy_acknowledged: true,
                list_on_browser: true,
                wine_runtime_acknowledged: false,
            },
        })
        .expect("serialize migrate request");
        assert_eq!(migrate["operation"], "migrate_server");
        assert_eq!(migrate["spec"]["source"]["kind"], "amp");
        assert!(migrate["spec"].get("cpu_millis").is_none());
        assert!(migrate["spec"].get("start_after").is_none());

        let vrising = serde_json::to_value(BrokerRequest::CreateVRising {
            spec: VRisingCreateSpec {
                name: "Castle".to_owned(),
                memory_mb: 4_096,
                cpu_millis: 0,
                max_players: 40,
                game_port: None,
                query_port: None,
                network_exposure: ServerNetworkExposure::Private,
                list_on_browser: true,
                start_on_boot: true,
                wine_runtime_acknowledged: true,
            },
        })
        .expect("serialize V Rising request");
        assert_eq!(vrising["operation"], "create_vrising");
        assert_eq!(vrising["spec"]["wine_runtime_acknowledged"], true);
        assert_eq!(vrising["spec"]["list_on_browser"], true);
        assert!(vrising["spec"].get("game_port").is_none());
        assert!(vrising["spec"].get("cpu_millis").is_none());

        let cpu = serde_json::to_value(BrokerRequest::SetNativeCpu {
            instance_id: "helix:test".to_owned(),
            cpu_millis: 2_000,
        })
        .expect("serialize CPU request");
        assert_eq!(cpu["operation"], "set_native_cpu");
        assert_eq!(cpu["cpu_millis"], 2_000);

        let listing = serde_json::to_value(BrokerRequest::SetNativeBrowserListing {
            instance_id: "helix:test".to_owned(),
            list_on_browser: false,
        })
        .expect("serialize listing request");
        assert_eq!(listing["operation"], "set_native_browser_listing");
        assert_eq!(listing["list_on_browser"], false);

        let boot = serde_json::to_value(BrokerRequest::SetNativeStartOnBoot {
            instance_id: "helix:test".to_owned(),
            enabled: false,
        })
        .expect("serialize start-on-boot request");
        assert_eq!(boot["operation"], "set_native_start_on_boot");
        assert_eq!(boot["enabled"], false);

        let memory = serde_json::to_value(BrokerRequest::SetNativeMemory {
            instance_id: "helix:test".to_owned(),
            memory_mb: 8_192,
        })
        .expect("serialize memory request");
        assert_eq!(memory["operation"], "set_native_memory");
        assert_eq!(memory["memory_mb"], 8_192);

        let custom = serde_json::to_value(BrokerRequest::CreateMinecraft {
            spec: MinecraftCreateSpec {
                pumpkin_bedrock_port: None,
                name: "Private build".to_owned(),
                software: MinecraftSoftware::Custom,
                version: "1.21.8".to_owned(),
                memory_mb: 4096,
                cpu_millis: 0,
                max_players: 20,
                game_port: Some(25566),
                network_exposure: ServerNetworkExposure::Private,
                start_on_boot: false,
                eula_accepted: true,
                custom_jar: Some(CustomMinecraftJarSpec {
                    source_path: "/srv/storage/uploads/server.jar".to_owned(),
                    java_version: 21,
                }),
            },
        })
        .expect("serialize custom server request");
        assert_eq!(custom["spec"]["software"], "custom");
        assert_eq!(
            custom["spec"]["custom_jar"]["source_path"],
            "/srv/storage/uploads/server.jar"
        );

        let versions = serde_json::to_value(BrokerRequest::ListMinecraftVersions {
            software: MinecraftSoftware::Paper,
        })
        .expect("serialize version list");
        assert_eq!(versions["operation"], "list_minecraft_versions");
        assert_eq!(versions["software"], "paper");

        let upload = serde_json::to_value(BrokerRequest::BeginFileUpload {
            target: FileUploadTarget::CustomJar,
            name: "server.jar".to_owned(),
            expected_size: 32_768,
        })
        .expect("serialize custom jar upload");
        assert_eq!(upload["operation"], "begin_file_upload");
        assert_eq!(upload["target"]["kind"], "custom_jar");
        assert_eq!(upload["name"], "server.jar");
        assert!(upload.get("content").is_none());
        assert_eq!(custom["spec"]["custom_jar"]["java_version"], 21);

        let automatic: BrokerRequest = serde_json::from_value(serde_json::json!({
            "operation": "create_minecraft",
            "spec": {
                "name": "Automatic",
                "software": "paper",
                "version": "latest",
                "memory_mb": 4096,
                "max_players": 20,
                "network_exposure": "public",
                "start_on_boot": true,
                "eula_accepted": true
            }
        }))
        .expect("automatic ports remain optional on the wire");
        let BrokerRequest::CreateMinecraft { spec } = automatic else {
            panic!("wrong request variant");
        };
        assert_eq!(spec.game_port, None);
        assert_eq!(spec.network_exposure, ServerNetworkExposure::Public);
    }

    #[test]
    fn migrate_spec_requires_stop_and_copy_acknowledgement() {
        let mut spec = ServerMigrateSpec {
            source: ServerMigrateSource::Folder {
                path: "/srv/storage/pterodactyl/world".to_owned(),
            },
            name: "Copied".to_owned(),
            game: Some(GameKind::Minecraft),
            software: Some(MinecraftSoftware::Paper),
            version: Some("latest".to_owned()),
            memory_mb: 4_096,
            cpu_millis: 0,
            max_players: 20,
            game_port: None,
            query_port: None,
            network_exposure: ServerNetworkExposure::Private,
            start_on_boot: true,
            eula_accepted: true,
            source_stopped: true,
            copy_acknowledged: true,
            list_on_browser: true,
            wine_runtime_acknowledged: false,
        };
        spec.validate_for_game(GameKind::Minecraft)
            .expect("valid copy");
        spec.copy_acknowledged = false;
        assert!(spec.validate().is_err());
        spec.copy_acknowledged = true;
        spec.source_stopped = false;
        assert!(spec.validate().is_err());
        spec.source_stopped = true;
        spec.eula_accepted = false;
        assert!(spec.validate_for_game(GameKind::Minecraft).is_err());
        spec.eula_accepted = true;
        spec.memory_mb = 1_024;
        spec.wine_runtime_acknowledged = false;
        assert!(spec.validate_for_game(GameKind::VRising).is_err());
        spec.memory_mb = 4_096;
        spec.wine_runtime_acknowledged = true;
        spec.validate_for_game(GameKind::VRising)
            .expect("V Rising copy");
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

        let purge = BrokerRequest::PurgeTrashedServer {
            trash_id: "8953dc16-3891-42bf-802f-711b3ba2965a".to_owned(),
            confirmation_name: "Survival".to_owned(),
        };
        let encoded = serde_json::to_value(purge).expect("serialize purge request");
        assert_eq!(encoded["operation"], "purge_trashed_server");
        assert_eq!(encoded["trash_id"], "8953dc16-3891-42bf-802f-711b3ba2965a");
        assert_eq!(encoded["confirmation_name"], "Survival");
        assert!(encoded.get("path").is_none());
        assert_eq!(
            broker_read_timeout(&BrokerRequest::PurgeTrashedServer {
                trash_id: "8953dc16-3891-42bf-802f-711b3ba2965a".to_owned(),
                confirmation_name: "Survival".to_owned(),
            }),
            std::time::Duration::from_secs(300)
        );
        assert_eq!(
            broker_read_timeout(&BrokerRequest::ListTrashedServers {}),
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn server_kill_is_a_distinct_typed_action() {
        let encoded = serde_json::to_value(BrokerRequest::ServerAction {
            instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
            action: ServerAction::Kill,
        })
        .expect("serialize kill");
        assert_eq!(encoded["operation"], "server_action");
        assert_eq!(encoded["action"], "kill");
        let parsed: BrokerRequest = serde_json::from_value(encoded).expect("parse kill");
        let BrokerRequest::ServerAction {
            action: ServerAction::Kill,
            ..
        } = parsed
        else {
            panic!("kill must round-trip as its own action");
        };
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
            provider: ModpackProvider::Modrinth,
            catalog: MarketplaceCatalog::Content,
        })
        .expect("serialize marketplace search");
        assert_eq!(search["operation"], "server_marketplace_search");
        assert_eq!(search["query"], "world edit");
        assert!(search.get("url").is_none());
        assert!(search.get("path").is_none());
        assert_eq!(
            broker_read_timeout(&BrokerRequest::ServerMarketplaceSearch {
                instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
                query: "world edit".to_owned(),
                offset: 0,
                limit: 20,
                provider: ModpackProvider::Curseforge,
                catalog: MarketplaceCatalog::Content,
            }),
            std::time::Duration::from_secs(45)
        );

        let install = serde_json::to_value(BrokerRequest::InstallServerMarketplaceContent {
            instance_id: "helix:6f55caa9-1264-4baf-8335-d3f31a704614".to_owned(),
            project_id: "1bokaNcj".to_owned(),
            version_id: Some("abcdef12".to_owned()),
            provider: ModpackProvider::Modrinth,
            restart_server: false,
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
            provider: ModpackProvider::Modrinth,
        })
        .expect("serialize modpack search");
        assert_eq!(search["operation"], "minecraft_modpack_search");
        assert!(search.get("url").is_none());
        assert!(search.get("loader").is_none());

        let project = serde_json::to_value(BrokerRequest::MinecraftModpackProject {
            project_id: "123456".to_owned(),
            provider: ModpackProvider::Curseforge,
        })
        .expect("serialize CurseForge modpack project");
        assert_eq!(project["operation"], "minecraft_modpack_project");
        assert_eq!(project["project_id"], "123456");
        assert_eq!(project["provider"], "curseforge");
        assert!(project.get("url").is_none());

        let create = serde_json::to_value(BrokerRequest::CreateMinecraftModpack {
            spec: MinecraftModpackCreateSpec {
                name: "Fabric Adventure".to_owned(),
                memory_mb: 6144,
                cpu_millis: 0,
                max_players: 20,
                game_port: Some(25_565),
                network_exposure: ServerNetworkExposure::Private,
                start_on_boot: true,
                eula_accepted: true,
                project_id: "AABBcc11".to_owned(),
                version_id: "version22".to_owned(),
                provider: ModpackProvider::Modrinth,
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
        let policy = serde_json::to_value(BrokerRequest::SetGamePortPolicy {
            policy: GamePortPolicySpec {
                game: GameKind::Minecraft,
                ranges: vec![GamePortRangeSpec {
                    start: 25_565,
                    end: 25_574,
                }],
                ports: vec![25_600],
                auto_forward_on_create: true,
            },
        })
        .expect("serialize port policy");
        assert_eq!(policy["operation"], "set_game_port_policy");
        assert_eq!(policy["policy"]["game"], "minecraft");
        assert!(policy.get("command").is_none());

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

        let leftover = serde_json::to_value(BrokerRequest::ReleaseAmpRouterForward {
            port: 25_566,
            confirmation: "REMOVE AMP FORWARD 25566".to_owned(),
        })
        .expect("serialize leftover AMP forward release");
        assert_eq!(leftover["operation"], "release_amp_router_forward");
        assert_eq!(leftover["port"], 25_566);
        assert_eq!(leftover["confirmation"], "REMOVE AMP FORWARD 25566");
        assert!(leftover.get("command").is_none());

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
            serde_json::to_value(BrokerRequest::GlobeSnapshot {}).unwrap()["operation"],
            "globe_snapshot"
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

        assert_eq!(
            serde_json::to_value(BrokerRequest::CheckHelixUpdate {}).unwrap()["operation"],
            "check_helix_update"
        );
        let helix_apply = serde_json::to_value(BrokerRequest::ApplyHelixUpdate {
            target_tag: "v1.0.1".to_owned(),
            confirmation: "UPDATE HELIX".to_owned(),
            disruption_acknowledged: true,
        })
        .expect("serialize Helix update");
        assert_eq!(helix_apply["operation"], "apply_helix_update");
        assert_eq!(helix_apply["target_tag"], "v1.0.1");
        assert_eq!(helix_apply["confirmation"], "UPDATE HELIX");
        assert_eq!(helix_apply["disruption_acknowledged"], true);
        assert!(helix_apply.get("command").is_none());
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

        let preflight = serde_json::to_value(BrokerRequest::HookInstallPreflight {
            hook_id: "tailscale".to_owned(),
        })
        .expect("serialize hook preflight");
        assert_eq!(preflight["operation"], "hook_install_preflight");
        assert_eq!(preflight.as_object().expect("preflight object").len(), 2);

        let install = serde_json::to_value(BrokerRequest::InstallHook {
            hook_id: "jellyfin".to_owned(),
            confirmation: "jellyfin".to_owned(),
            repository_change_acknowledged: true,
        })
        .expect("serialize hook install");
        assert_eq!(install["operation"], "install_hook");
        assert_eq!(install["hook_id"], "jellyfin");
        assert!(install.get("package").is_none());
        assert!(install.get("repository").is_none());
        assert!(install.get("command").is_none());
    }

    #[test]
    fn docker_and_security_requests_are_typed_without_shell_fields() {
        let inventory = serde_json::to_value(BrokerRequest::DockerInventory {})
            .expect("serialize docker inventory");
        assert_eq!(inventory["operation"], "docker_inventory");
        assert_eq!(inventory.as_object().expect("inventory object").len(), 1);

        let action = serde_json::to_value(BrokerRequest::DockerContainerAction {
            name: "plex".to_owned(),
            action: DockerContainerActionKind::Restart,
            confirmation: "plex".to_owned(),
        })
        .expect("serialize docker action");
        assert_eq!(action["operation"], "docker_container_action");
        assert_eq!(action["name"], "plex");
        assert_eq!(action["action"], "restart");
        assert!(action.get("command").is_none());
        assert!(action.get("arguments").is_none());

        let security = serde_json::to_value(BrokerRequest::SecurityInventory {})
            .expect("serialize security inventory");
        assert_eq!(security["operation"], "security_inventory");

        let toggle = serde_json::to_value(BrokerRequest::SetSecurityControl {
            id: "helix_start_on_boot".to_owned(),
            enabled: true,
            confirmation: "start helix after boot".to_owned(),
        })
        .expect("serialize security toggle");
        assert_eq!(toggle["operation"], "set_security_control");
        assert_eq!(toggle["id"], "helix_start_on_boot");
        assert!(toggle.get("command").is_none());
    }

    #[test]
    fn curseforge_api_key_is_bounded_printable_ascii() {
        assert!(validate_curseforge_api_key("short").is_err());
        assert!(
            validate_curseforge_api_key("not-ascii-\u{043a}\u{043b}\u{044e}\u{0447}-xxxxxxxx")
                .is_err()
        );
        let key = "$2a$10$abcdefghijklmnopqrstuvwx";
        assert_eq!(
            validate_curseforge_api_key(&format!("  '{key}'  ")).unwrap(),
            key
        );
        assert_eq!(
            validate_curseforge_api_key(&format!("\"{key}\"")).unwrap(),
            key
        );
        assert_eq!(
            validate_curseforge_api_key("$$2a$$10$$abcdefghijklmnopqrstuvwx").unwrap(),
            key
        );
        assert_eq!(
            validate_curseforge_api_key("$2a$10$abc/defGHIJK.lmnopqrstuv").unwrap(),
            "$2a$10$abc/defGHIJK.lmnopqrstuv"
        );
        assert_eq!(
            validate_curseforge_api_key(&format!("{}\n{}", &key[..16], &key[16..])).unwrap(),
            key
        );
        assert_eq!(
            validate_curseforge_api_key(&format!(
                "CF_API_KEY=$$2a$$10$${}\u{200b}",
                &key["$2a$10$".len()..]
            ))
            .unwrap(),
            key
        );
        assert_eq!(
            validate_curseforge_api_key(&format!("Copy this: {key} please\u{00a0}")).unwrap(),
            key
        );
        assert_eq!(
            validate_curseforge_api_key("\u{ff04}2a\u{ff04}10\u{ff04}abcdefghijklmnopqrstuvwx")
                .unwrap(),
            key
        );
        let header_file = curl_extra_header_file(&[("x-api-key", key)]).expect("header file");
        assert_eq!(header_file, format!("x-api-key: {key}\n"));
        assert!(!header_file.contains("$$"));
        assert!(curl_extra_header_file(&[("x-api-key", "line\nbreak")]).is_err());
        let cloudfront_dump = "HTTP/2 403 \r\ncontent-length: 0\r\nx-cache: Error from cloudfront\r\nvia: 1.1 edge.cloudfront.net (CloudFront)\r\n";
        assert_eq!(http_dump_status(cloudfront_dump), Some(403));
        assert_eq!(
            classify_curseforge_curl_error(
                "The requested URL returned error: 403",
                cloudfront_dump,
                b""
            ),
            CURSEFORGE_CDN_BLOCKED
        );
        assert_eq!(
            classify_curseforge_curl_error(
                "The requested URL returned error: 403",
                "HTTP/1.1 403 Forbidden\r\nserver: Kestrel\r\n",
                b"Forbidden: API Key missing or invalid"
            ),
            CURSEFORGE_KEY_REJECTED
        );
        assert_eq!(
            classify_curseforge_curl_error(
                "The requested URL returned error: 429",
                "HTTP/2 429 \r\n",
                b""
            ),
            CURSEFORGE_RATE_LIMITED
        );
        assert!(catalog_fetch_is_non_retryable(CURSEFORGE_CDN_BLOCKED));
        let status = serde_json::to_value(BrokerRequest::CurseforgeKeyStatus {})
            .expect("serialize curseforge status");
        assert_eq!(status["operation"], "curseforge_key_status");
        assert!(status.get("key").is_none());
        let set = serde_json::to_value(BrokerRequest::SetCurseforgeApiKey {
            key: key.to_owned(),
        })
        .expect("serialize curseforge key");
        assert_eq!(set["operation"], "set_curseforge_api_key");
        assert_eq!(set["key"], key);
        assert!(set.get("url").is_none());
        let clear = serde_json::to_value(BrokerRequest::ClearCurseforgeApiKey {})
            .expect("serialize curseforge clear");
        assert_eq!(clear["operation"], "clear_curseforge_api_key");
        assert!(clear.get("key").is_none());
    }

    #[test]
    fn vrising_create_spec_rejects_missing_wine_ack_and_accepts_public_udp() {
        let mut spec = VRisingCreateSpec {
            name: "Castle".to_owned(),
            memory_mb: 4_096,
            cpu_millis: 0,
            max_players: 40,
            game_port: None,
            query_port: None,
            network_exposure: ServerNetworkExposure::Private,
            list_on_browser: true,
            start_on_boot: true,
            wine_runtime_acknowledged: true,
        };
        spec.validate().expect("valid V Rising spec");
        spec.wine_runtime_acknowledged = false;
        assert!(spec.validate().unwrap_err().contains("runtime"));
        spec.wine_runtime_acknowledged = true;
        spec.network_exposure = ServerNetworkExposure::Public;
        spec.validate().expect("public UDP is allowed for V Rising");
        spec.cpu_millis = 100;
        assert!(spec.validate().unwrap_err().contains("CPU"));
        spec.cpu_millis = 2_000;
        spec.validate().expect("two cores is a valid cap");
        spec.game_port = Some(9_876);
        spec.query_port = Some(9_876);
        assert!(spec.validate().unwrap_err().contains("different"));
    }
}
