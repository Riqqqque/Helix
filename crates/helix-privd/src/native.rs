use crate::amp::{AmpClient, AmpServer, amp_port_claimed_message};
use crate::files::{
    FileUploadProgress, FileUploadStart, StreamingUpload, commit_upload, decode_upload_chunk,
    prune_uploads, upload_lock_error, validate_name,
};
use helix_privd::{
    CURSEFORGE_API_KEY_REQUIRED, CURSEFORGE_CDN_BLOCKED, CURSEFORGE_KEY_REJECTED,
    CURSEFORGE_RATE_LIMITED, CustomMinecraftJarSpec, FileUploadPurpose, GameKind,
    GamePortPolicySpec, GamePortRangeSpec, MAX_CONCURRENT_FILE_UPLOADS,
    MAX_CUSTOM_JAR_UPLOAD_BYTES, MAX_FILE_UPLOAD_CHUNK_BYTES, MAX_MINECRAFT_VERSION_CATALOG,
    MinecraftCreateSpec, MinecraftDifficulty, MinecraftGameMode, MinecraftModpackCreateSpec,
    MinecraftSettingsPatch, MinecraftSoftware, ModpackProvider, ServerAction, TerrariaCreateSpec,
    TerrariaSoftware, VRisingCreateSpec, ValheimCreateSpec, catalog_fetch_is_non_retryable,
    classify_curseforge_curl_error, curl_extra_header_file, migrate_plan,
    validate_curseforge_api_key,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read as _, Seek as _, SeekFrom, Write as _},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

mod marketplace;
mod modpacks;
mod pumpkin;
mod terraria;
mod valheim;
mod vrising;

const MANIFEST_VERSION: u32 = 1;
const READY_MARKER: &str = ".helix-ready";
const MAX_MANIFEST_BYTES: u64 = 128 * 1024;
const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SERVER_JAR_BYTES: u64 = 768 * 1024 * 1024;
const MAX_PROPERTIES_BYTES: u64 = 512 * 1024;
const MAX_CONSOLE_COMMAND_BYTES: usize = 512;
const MAX_RCON_PACKET_BYTES: usize = 1024 * 1024;
const MAX_CONSOLE_LINE_BYTES: usize = 1024 * 1024;
const MAX_CONSOLE_HISTORY_PAGE_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_NATIVE_COMMAND_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MODPACK_LOCK_FILE: &str = ".helix-modpack.lock.json";
const MAX_MODPACK_LOCK_BYTES: u64 = 4 * 1024 * 1024;
const MIN_CONSOLE_HISTORY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CONSOLE_HISTORY_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const MAX_BACKUP_CATALOG_ENTRIES: usize = 2_048;
const MAX_SERVER_TRASH_ENTRIES: usize = 512;
const MAX_PORT_POLICY_RANGES: usize = 32;
const MAX_PORT_POLICY_PORTS: usize = 256;
const MAX_PORT_POLICY_CANDIDATES: usize = 4_096;
const MAX_MINECRAFT_STATUS_WORKERS: usize = 8;
const MINECRAFT_TPS_CACHE_HIT: Duration = Duration::from_secs(60);
const MINECRAFT_TPS_CACHE_MISS: Duration = Duration::from_secs(120);
const MINECRAFT_TPS_CACHE_ERROR: Duration = Duration::from_secs(30);
const MINECRAFT_TPS_CONNECT_TIMEOUT: Duration = Duration::from_millis(350);
const MINECRAFT_TPS_IO_TIMEOUT: Duration = Duration::from_millis(500);
const DOCKER_TIMEOUT_SECONDS: u64 = 300;
const USER_AGENT: &str = concat!(
    "Helix/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Riqqqque/Helix)"
);
const FORGECDN_DOWNLOAD_HOSTS: &[&str] = &[
    "edge.forgecdn.net",
    "mediafilez.forgecdn.net",
    "media.forgecdn.net",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeConfig {
    pub state_root: PathBuf,
    pub instance_root: PathBuf,
    pub backup_root: PathBuf,
    #[serde(default = "default_docker_binary")]
    pub docker_binary: PathBuf,
    #[serde(default = "default_console_history_max_bytes")]
    pub console_history_max_bytes: u64,
    #[serde(default = "default_console_history_files")]
    pub console_history_files: u16,
    #[serde(default = "default_backup_trash_retention_days")]
    pub backup_trash_retention_days: u16,
    #[serde(default)]
    pub custom_artifact_roots: Vec<PathBuf>,
}

pub struct NativeManager {
    state_root: PathBuf,
    instance_root: PathBuf,
    backup_root: PathBuf,
    docker_binary: PathBuf,
    console_retention: ConsoleRetention,
    backup_trash_retention_days: u16,
    custom_artifact_roots: Vec<PathBuf>,
    custom_import_root: PathBuf,
    uploads: Mutex<HashMap<String, crate::files::StreamingUpload>>,
    operations: Mutex<HashSet<String>>,
    port_policies: Mutex<()>,
    console_archives: Mutex<HashMap<String, Arc<Mutex<ConsoleArchiveWriter>>>>,
    console_stops: Mutex<HashMap<String, Arc<AtomicBool>>>,
    tps_cache: Mutex<HashMap<String, CachedTps>>,
    amp: Option<Arc<AmpClient>>,
}

#[derive(Clone, Copy)]
struct ConsoleRetention {
    maximum_bytes: u64,
    files: u16,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InstanceManifest {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "is_minecraft_kind")]
    kind: GameKind,
    id: String,
    name: String,
    instance_name: String,
    container_name: String,
    software: MinecraftSoftware,
    minecraft_version: String,
    build: String,
    java_version: u16,
    runtime_image: String,
    artifact_url: String,
    artifact_sha256: String,
    memory_mb: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    cpu_millis: u32,
    max_players: u16,
    game_port: u16,
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    query_port: u16,
    rcon_port: u16,
    rcon_password: String,
    start_on_boot: bool,
    run_uid: u32,
    created_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unix_args: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    backup_keep_count: u16,
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    backup_keep_days: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modpack: Option<InstalledModpack>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InstalledModpack {
    schema_version: u32,
    provider: ModpackProvider,
    project_id: String,
    project_title: String,
    version_id: String,
    version_name: String,
    version_number: String,
    minecraft_version: String,
    loader: String,
    loader_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModpackLock {
    schema_version: u32,
    provider: ModpackProvider,
    project_id: String,
    version_id: String,
    managed_files: Vec<ManagedModpackFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManagedModpackFile {
    path: String,
    sha256: String,
}

impl InstanceManifest {
    fn is_pumpkin(&self) -> bool {
        matches!(self.software, MinecraftSoftware::Pumpkin) && self.is_minecraft()
    }

    fn artifact_name(&self) -> &'static str {
        if self.is_pumpkin() {
            "pumpkin"
        } else {
            "server.jar"
        }
    }

    fn settings_name(&self) -> &'static str {
        if self.is_pumpkin() {
            "pumpkin.toml"
        } else {
            "server.properties"
        }
    }

    fn is_minecraft(&self) -> bool {
        matches!(self.kind, GameKind::Minecraft)
    }

    fn is_vrising(&self) -> bool {
        matches!(self.kind, GameKind::VRising)
    }

    fn is_valheim(&self) -> bool {
        matches!(self.kind, GameKind::Valheim)
    }

    fn is_terraria(&self) -> bool {
        matches!(self.kind, GameKind::Terraria)
    }

    fn uses_ready_marker(&self) -> bool {
        !self.is_minecraft()
    }

    fn kind_slug(&self) -> &'static str {
        match self.kind {
            GameKind::Minecraft => "minecraft",
            GameKind::VRising => "vrising",
            GameKind::Valheim => "valheim",
            GameKind::Terraria => "terraria",
        }
    }

    fn occupied_ports(&self) -> Vec<u16> {
        let mut ports = vec![self.game_port];
        if self.is_valheim() {
            ports.push(self.game_port.saturating_add(1));
            ports.push(self.game_port.saturating_add(2));
        } else if self.query_port != 0 && self.query_port != self.game_port {
            ports.push(self.query_port);
        }
        ports
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredGamePortPolicy {
    schema_version: u32,
    policy: GamePortPolicySpec,
}

struct Artifact {
    software: MinecraftSoftware,
    version: String,
    build: String,
    java_version: u16,
    url: String,
    local_source: Option<PathBuf>,
    expected_hash: Option<ExpectedHash>,
    install_server: bool,
}

struct ExpectedHash {
    algorithm: HashAlgorithm,
    value: String,
}

#[derive(Clone, Copy)]
enum HashAlgorithm {
    Sha1,
    Sha256,
}

#[derive(Default)]
struct RuntimeState {
    running: bool,
    cpu_percent: f64,
    memory_used_mb: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ContainerStartupState {
    running: bool,
    restarting: bool,
    oom_killed: bool,
    exit_code: i32,
    restart_count: u64,
}

struct MinecraftStatus {
    players_online: u64,
    max_players: u64,
    version: Option<String>,
}

struct CachedTps {
    expires_at: Instant,
    tps: Option<f64>,
}

#[derive(Clone, Copy)]
struct TpsProbeResult {
    tps: Option<f64>,
    ttl: Duration,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BackupTrashRecord {
    schema_version: u32,
    trash_id: String,
    instance_id: String,
    backup_id: String,
    trashed_at_unix_ms: u64,
    purge_eligible_at_unix_ms: u64,
    definition_present: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ServerTrashRecord {
    schema_version: u32,
    trash_id: String,
    instance_id: String,
    name: String,
    trashed_at_unix_ms: u64,
    was_running: bool,
}

struct InstanceOperationGuard<'a> {
    operations: &'a Mutex<HashSet<String>>,
    key: String,
}

impl Drop for InstanceOperationGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.operations.lock() {
            operations.remove(&self.key);
        }
    }
}

#[derive(Clone)]
struct ConsoleArchiverConfig {
    instance_id: String,
    container_name: String,
    docker_binary: PathBuf,
    docker_cli_home: PathBuf,
    archive_root: PathBuf,
    stop: Arc<AtomicBool>,
}

struct ConsoleArchiveWriter {
    root: PathBuf,
    retention: ConsoleRetention,
    sequence: u64,
    current_bytes: u64,
}

impl ConsoleArchiveWriter {
    fn open(root: PathBuf, retention: ConsoleRetention) -> Result<Self, String> {
        fs::create_dir_all(&root)
            .map_err(|_| "could not create the server console archive".to_owned())?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the server console archive".to_owned())?;
        let paths = console_segment_paths(&root)?;
        let sequence = paths
            .last()
            .and_then(|path| console_segment_sequence(path))
            .unwrap_or(1);
        let current_bytes = paths
            .last()
            .and_then(|path| fs::metadata(path).ok())
            .map_or(0, |metadata| metadata.len());
        let mut writer = Self {
            root,
            retention,
            sequence,
            current_bytes,
        };
        writer.prune()?;
        Ok(writer)
    }

    fn append(&mut self, value: &str) -> Result<(), String> {
        let segment_limit = self.segment_limit();
        let maximum_line = usize::try_from(segment_limit.saturating_sub(1))
            .unwrap_or(usize::MAX)
            .min(MAX_CONSOLE_LINE_BYTES);
        let mut line = value
            .trim_end_matches(['\r', '\n'])
            .chars()
            .filter(|character| *character != '\0')
            .collect::<String>();
        if line.len() > maximum_line {
            const SUFFIX: &str = " [line truncated by Helix]";
            let mut boundary = maximum_line.saturating_sub(SUFFIX.len());
            while boundary > 0 && !line.is_char_boundary(boundary) {
                boundary -= 1;
            }
            line.truncate(boundary);
            if maximum_line >= SUFFIX.len() {
                line.push_str(SUFFIX);
            }
        }
        let entry_bytes = u64::try_from(line.len().saturating_add(1)).unwrap_or(u64::MAX);
        let mut rotated = false;
        if self.current_bytes > 0 && self.current_bytes.saturating_add(entry_bytes) > segment_limit
        {
            self.sequence = self.sequence.saturating_add(1);
            self.current_bytes = 0;
            rotated = true;
        }
        let path = console_segment_path(&self.root, self.sequence);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| "could not open the server console archive".to_owned())?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.flush())
            .map_err(|_| "could not persist server console history".to_owned())?;
        self.current_bytes = self.current_bytes.saturating_add(entry_bytes);
        if rotated {
            self.prune()?;
        }
        Ok(())
    }

    fn segment_limit(&self) -> u64 {
        self.retention
            .maximum_bytes
            .div_ceil(u64::from(self.retention.files))
            .max(4 * 1024)
    }

    fn prune(&mut self) -> Result<(), String> {
        let paths = console_segment_paths(&self.root)?;
        let excess = paths
            .len()
            .saturating_sub(usize::from(self.retention.files));
        for path in paths.into_iter().take(excess) {
            fs::remove_file(path)
                .map_err(|_| "could not apply console history retention".to_owned())?;
        }
        Ok(())
    }
}

impl NativeManager {
    pub fn new(config: NativeConfig) -> Result<Self, String> {
        validate_root(&config.state_root, "native state")?;
        validate_root(&config.instance_root, "native instance")?;
        validate_root(&config.backup_root, "native backup")?;
        validate_binary(&config.docker_binary, "Docker")?;
        for root in &config.custom_artifact_roots {
            validate_custom_artifact_root(root)?;
        }
        if !(MIN_CONSOLE_HISTORY_BYTES..=MAX_CONSOLE_HISTORY_BYTES)
            .contains(&config.console_history_max_bytes)
            || !(2..=256).contains(&config.console_history_files)
        {
            return Err(
                "console history retention must use 16 MiB to 32 GiB across 2 to 256 files"
                    .to_owned(),
            );
        }
        if !(1..=365).contains(&config.backup_trash_retention_days) {
            return Err("backup trash retention must be between 1 and 365 days".to_owned());
        }
        for root in [
            &config.state_root,
            &config.instance_root,
            &config.backup_root,
        ] {
            fs::create_dir_all(root).map_err(|_| format!("could not create {}", root.display()))?;
        }
        fs::set_permissions(&config.state_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the native state directory".to_owned())?;
        fs::create_dir_all(config.state_root.join("console"))
            .map_err(|_| "could not create the protected console archive".to_owned())?;
        fs::set_permissions(
            config.state_root.join("console"),
            fs::Permissions::from_mode(0o700),
        )
        .map_err(|_| "could not protect the console archive".to_owned())?;
        fs::create_dir_all(config.state_root.join(".staging"))
            .map_err(|_| "could not create the protected update staging directory".to_owned())?;
        fs::set_permissions(
            config.state_root.join(".staging"),
            fs::Permissions::from_mode(0o700),
        )
        .map_err(|_| "could not protect the update staging directory".to_owned())?;
        prepare_docker_cli_home(&config.state_root)?;
        fs::create_dir_all(config.state_root.join("server-trash"))
            .map_err(|_| "could not create the protected server trash registry".to_owned())?;
        fs::set_permissions(
            config.state_root.join("server-trash"),
            fs::Permissions::from_mode(0o700),
        )
        .map_err(|_| "could not protect the server trash registry".to_owned())?;
        fs::set_permissions(&config.instance_root, fs::Permissions::from_mode(0o750))
            .map_err(|_| "could not protect the native instance directory".to_owned())?;
        fs::create_dir_all(config.instance_root.join(".trash"))
            .map_err(|_| "could not create the protected server data trash".to_owned())?;
        fs::set_permissions(
            config.instance_root.join(".trash"),
            fs::Permissions::from_mode(0o700),
        )
        .map_err(|_| "could not protect the server data trash".to_owned())?;
        fs::set_permissions(&config.backup_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the native backup directory".to_owned())?;
        fs::create_dir_all(config.backup_root.join(".trash"))
            .map_err(|_| "could not create the protected backup trash".to_owned())?;
        fs::set_permissions(
            config.backup_root.join(".trash"),
            fs::Permissions::from_mode(0o700),
        )
        .map_err(|_| "could not protect the backup trash".to_owned())?;
        fs::create_dir_all(config.instance_root.join(".failed"))
            .map_err(|_| "could not create the failed-install recovery directory".to_owned())?;
        fs::set_permissions(
            config.instance_root.join(".failed"),
            fs::Permissions::from_mode(0o700),
        )
        .map_err(|_| "could not protect the failed-install recovery directory".to_owned())?;
        let custom_import_root = config.state_root.join("imports");
        fs::create_dir_all(&custom_import_root)
            .map_err(|_| "could not create the protected custom JAR import directory".to_owned())?;
        fs::set_permissions(&custom_import_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the custom JAR import directory".to_owned())?;
        validate_custom_artifact_root(&custom_import_root)?;
        let custom_import_root = canonical_directory(&custom_import_root)?;
        let mut custom_artifact_roots = config
            .custom_artifact_roots
            .iter()
            .map(|root| canonical_directory(root))
            .collect::<Result<Vec<_>, _>>()?;
        if !custom_artifact_roots
            .iter()
            .any(|root| root == &custom_import_root)
        {
            custom_artifact_roots.insert(0, custom_import_root.clone());
        }
        let manager = Self {
            state_root: canonical_directory(&config.state_root)?,
            instance_root: canonical_directory(&config.instance_root)?,
            backup_root: canonical_directory(&config.backup_root)?,
            docker_binary: config.docker_binary,
            console_retention: ConsoleRetention {
                maximum_bytes: config.console_history_max_bytes,
                files: config.console_history_files,
            },
            backup_trash_retention_days: config.backup_trash_retention_days,
            custom_artifact_roots,
            custom_import_root,
            uploads: Mutex::new(HashMap::new()),
            operations: Mutex::new(HashSet::new()),
            port_policies: Mutex::new(()),
            console_archives: Mutex::new(HashMap::new()),
            console_stops: Mutex::new(HashMap::new()),
            tps_cache: Mutex::new(HashMap::new()),
            amp: None,
        };
        for manifest in manager.load_manifests()? {
            let path = manager.instance_path(&manifest.id)?;
            if path.is_dir() {
                manager.protect_instance_artifacts(&path, manifest.run_uid)?;
            }
            manager.ensure_console_archiver(&manifest)?;
        }
        Ok(manager)
    }

    pub fn with_amp(mut self, amp: Arc<AmpClient>) -> Self {
        self.amp = Some(amp);
        self
    }

    fn load_tps_map(&self, targets: &[(String, u16, String)]) -> HashMap<String, Option<f64>> {
        let now = Instant::now();
        let mut ready = HashMap::with_capacity(targets.len());
        let mut pending = Vec::new();
        {
            let cache = self
                .tps_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for (id, port, password) in targets {
                if *port == 0 || password.is_empty() {
                    ready.insert(id.clone(), None);
                    continue;
                }
                if let Some(sample) = cache.get(id)
                    && sample.expires_at > now
                {
                    ready.insert(id.clone(), sample.tps);
                    continue;
                }
                pending.push((id.clone(), *port, password.clone()));
            }
        }
        if pending.is_empty() {
            return ready;
        }
        let probed = collect_minecraft_tps(&pending, probe_minecraft_tps);
        let now = Instant::now();
        {
            let mut cache = self
                .tps_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for (id, sample) in &probed {
                cache.insert(
                    id.clone(),
                    CachedTps {
                        expires_at: now + sample.ttl,
                        tps: sample.tps,
                    },
                );
            }
        }
        for (id, sample) in probed {
            ready.insert(id, sample.tps);
        }
        ready
    }

    fn prune_tps_cache(&self, live_ids: &HashSet<String>) {
        let now = Instant::now();
        let mut cache = self
            .tps_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.retain(|id, sample| live_ids.contains(id) && sample.expires_at > now);
    }

    fn amp_occupied_ports(&self) -> HashSet<u16> {
        self.amp
            .as_ref()
            .map(|amp| amp.occupied_ports())
            .unwrap_or_default()
    }

    fn port_conflict_error(&self, port: u16, helix: &HashSet<u16>) -> Option<String> {
        if self.amp_occupied_ports().contains(&port) {
            Some(
                self.amp
                    .as_ref()
                    .map(|amp| amp.explain_claimed_port(port))
                    .unwrap_or_else(|| amp_port_claimed_message(port)),
            )
        } else if helix.contains(&port) {
            Some(format!(
                "game port {port} is already assigned to another Helix server"
            ))
        } else {
            None
        }
    }

    fn used_game_ports(&self, manifests: &[InstanceManifest]) -> HashSet<u16> {
        let mut used = assigned_game_ports(manifests);
        used.extend(self.amp_occupied_ports());
        used
    }

    pub fn list_servers(&self) -> Result<Vec<AmpServer>, String> {
        let manifests = self.load_manifests()?;
        let states = self.runtime_states(&manifests);
        let status_targets = manifests
            .iter()
            .filter(|manifest| {
                manifest.is_minecraft()
                    && states
                        .get(&manifest.container_name)
                        .is_some_and(|state| state.running)
            })
            .map(|manifest| (manifest.id.clone(), manifest.game_port))
            .collect::<Vec<_>>();
        let mut statuses = collect_minecraft_statuses(&status_targets, |port| {
            minecraft_status(port, Duration::from_millis(450)).ok()
        });
        let live_ids = manifests
            .iter()
            .map(|manifest| manifest.id.clone())
            .collect();
        let tps_by_id = self.load_tps_map(&minecraft_rcon_targets(&manifests, &states));
        self.prune_tps_cache(&live_ids);
        let mut servers = Vec::with_capacity(manifests.len());
        for manifest in manifests {
            let state = states
                .get(&manifest.container_name)
                .cloned()
                .unwrap_or_default();
            let status = statuses.remove(&manifest.id);
            let data_path = self.instance_path(&manifest.id)?;
            let mut warnings = Vec::new();
            if manifest.is_minecraft() && !data_path.join(manifest.artifact_name()).is_file() {
                warnings.push("Server executable is missing".to_owned());
            }
            if manifest.is_vrising()
                && !data_path.join("server").join("VRisingServer.exe").is_file()
            {
                warnings.push("V Rising dedicated server files are not installed yet".to_owned());
            }
            if manifest.is_valheim()
                && !data_path
                    .join("server")
                    .join("valheim_server.x86_64")
                    .is_file()
            {
                warnings.push("Valheim dedicated server files are not installed yet".to_owned());
            }
            if manifest.is_terraria()
                && !data_path.join("serverconfig.txt").is_file()
                && !data_path.join(READY_MARKER).is_file()
            {
                warnings.push("Terraria dedicated server files are not installed yet".to_owned());
            }
            if !states.contains_key(&manifest.container_name) {
                warnings.push("Execution container is missing".to_owned());
            }
            let (status, players_online, player_count_verified, max_players, version) =
                if manifest.uses_ready_marker() {
                    (
                        if state.running {
                            "online"
                        } else {
                            "manager_stopped"
                        },
                        0,
                        !state.running,
                        u64::from(manifest.max_players),
                        manifest.minecraft_version.clone(),
                    )
                } else {
                    (
                        if status.is_some() {
                            "online"
                        } else if state.running {
                            "offline"
                        } else {
                            "manager_stopped"
                        },
                        status.as_ref().map_or(0, |status| status.players_online),
                        !state.running || status.is_some(),
                        status
                            .as_ref()
                            .map_or(u64::from(manifest.max_players), |status| status.max_players),
                        status
                            .as_ref()
                            .and_then(|status| status.version.clone())
                            .unwrap_or_else(|| manifest.minecraft_version.clone()),
                    )
                };
            let kind = manifest.kind_slug();
            let software = display_software(&manifest).to_owned();
            servers.push(AmpServer {
                id: format!("helix:{}", manifest.id),
                name: manifest.name,
                instance_name: manifest.instance_name,
                kind,
                software,
                version,
                status: status.to_owned(),
                panel_running: state.running,
                start_on_boot: manifest.start_on_boot,
                players_online,
                player_count_verified,
                max_players,
                cpu_percent: state.cpu_percent,
                memory_used_mb: state.memory_used_mb,
                memory_limit_mb: u64::from(manifest.memory_mb),
                tps: tps_by_id.get(&manifest.id).copied().flatten(),
                manager_panel_port: 0,
                panel_port: 0,
                game_port: Some(manifest.game_port),
                query_port: if manifest.query_port == 0 {
                    None
                } else {
                    Some(manifest.query_port)
                },
                path: data_path.to_string_lossy().into_owned(),
                warnings,
                manager: "helix",
                execution_backend: "docker",
            });
        }
        servers.sort_by_key(|server| server.name.to_lowercase());
        Ok(servers)
    }

    pub fn game_port_policy(&self, game: GameKind) -> Result<Value, String> {
        let manifests = self.load_manifests()?;
        let used = self.used_game_ports(&manifests);
        let _policy = self
            .port_policies
            .lock()
            .map_err(|_| "the game port policy lock is unavailable".to_owned())?;
        let policy = self.read_game_port_policy(game)?;
        let candidates = policy_candidates(&policy)?;
        let next_available = candidates
            .iter()
            .copied()
            .find(|port| !used.contains(port) && ensure_port_available(*port, true).is_ok());
        Ok(game_port_policy_response(
            policy,
            candidates,
            used,
            next_available,
            self.amp_occupied_ports(),
        ))
    }

    pub fn minecraft_auto_forward_on_create(&self) -> Option<bool> {
        let _policy = self.port_policies.lock().ok()?;
        self.read_game_port_policy(GameKind::Minecraft)
            .ok()
            .map(|policy| policy.auto_forward_on_create)
    }

    pub fn set_game_port_policy(&self, policy: GamePortPolicySpec) -> Result<Value, String> {
        let policy = normalize_game_port_policy(policy)?;
        let _policy_guard = self
            .port_policies
            .lock()
            .map_err(|_| "the game port policy lock is unavailable".to_owned())?;
        let body = serde_json::to_string_pretty(&StoredGamePortPolicy {
            schema_version: 1,
            policy: policy.clone(),
        })
        .map_err(|_| "could not encode the game port policy".to_owned())?;
        write_private_text(
            &self.game_port_policy_path(policy.game),
            &format!("{body}\n"),
        )?;
        sync_directory(&self.state_root)?;
        drop(_policy_guard);
        self.game_port_policy(policy.game)
    }

    fn resolve_game_port(
        &self,
        game: GameKind,
        requested: Option<u16>,
        manifests: &[InstanceManifest],
    ) -> Result<(u16, bool), String> {
        if let Some(port) = requested {
            if port < 1_024 {
                return Err("game port must be at least 1024".to_owned());
            }
            let helix = assigned_game_ports(manifests);
            if let Some(error) = self.port_conflict_error(port, &helix) {
                return Err(error);
            }
            ensure_port_available(port, true)?;
            return Ok((port, false));
        }
        let used = self.used_game_ports(manifests);
        let _policy = self
            .port_policies
            .lock()
            .map_err(|_| "the game port policy lock is unavailable".to_owned())?;
        let policy = self.read_game_port_policy(game)?;
        for port in policy_candidates(&policy)? {
            if !used.contains(&port) && ensure_port_available(port, true).is_ok() {
                return Ok((port, true));
            }
        }
        let label = match game {
            GameKind::Minecraft => "Minecraft",
            GameKind::VRising => "V Rising",
            GameKind::Valheim => "Valheim",
            GameKind::Terraria => "Terraria",
        };
        Err(format!(
            "the {label} port pool has no available ports; add ports or expand its ranges in Servers > Port pools"
        ))
    }

    fn resolve_vrising_ports(
        &self,
        requested_game: Option<u16>,
        requested_query: Option<u16>,
        manifests: &[InstanceManifest],
    ) -> Result<(u16, u16, bool), String> {
        let helix = assigned_game_ports(manifests);
        if let Some(game_port) = requested_game {
            let query_port = requested_query.unwrap_or(game_port.saturating_add(1));
            if query_port < 1_024 || query_port == game_port {
                return Err(
                    "V Rising query port must be at least 1024 and different from the game port"
                        .to_owned(),
                );
            }
            if let Some(error) = self.port_conflict_error(game_port, &helix) {
                return Err(error);
            }
            if let Some(error) = self.port_conflict_error(query_port, &helix) {
                return Err(error);
            }
            ensure_port_available(game_port, true)?;
            ensure_port_available(query_port, true)?;
            return Ok((game_port, query_port, false));
        }
        let _policy = self
            .port_policies
            .lock()
            .map_err(|_| "the game port policy lock is unavailable".to_owned())?;
        let policy = self.read_game_port_policy(GameKind::VRising)?;
        let candidates = policy_candidates(&policy)?;
        let mut used = helix;
        used.extend(self.amp_occupied_ports());
        let available = candidates
            .iter()
            .copied()
            .filter(|port| !used.contains(port) && ensure_port_available(*port, true).is_ok())
            .collect::<Vec<_>>();
        for window in available.windows(2) {
            if window[1] == window[0].saturating_add(1) {
                return Ok((window[0], window[1], true));
            }
        }
        if available.len() >= 2 {
            return Ok((available[0], available[1], true));
        }
        Err("the V Rising port pool does not have two free UDP ports; add ports or expand its ranges in Servers > Port pools".to_owned())
    }

    fn resolve_valheim_ports(
        &self,
        requested_game_port: Option<u16>,
        manifests: &[InstanceManifest],
    ) -> Result<(u16, u16, bool), String> {
        let helix = assigned_game_ports(manifests);
        if let Some(game_port) = requested_game_port {
            let query_port = game_port.saturating_add(1);
            let steam_port = game_port.saturating_add(2);
            if steam_port < game_port {
                return Err(
                    "that Valheim game port is too high to reserve the next two UDP ports"
                        .to_owned(),
                );
            }
            for port in [game_port, query_port, steam_port] {
                if let Some(error) = self.port_conflict_error(port, &helix) {
                    return Err(error);
                }
                ensure_port_available(port, true)?;
            }
            return Ok((game_port, query_port, false));
        }
        let _policy = self
            .port_policies
            .lock()
            .map_err(|_| "the game port policy lock is unavailable".to_owned())?;
        let policy = self.read_game_port_policy(GameKind::Valheim)?;
        let candidates = policy_candidates(&policy)?;
        let mut used = helix;
        used.extend(self.amp_occupied_ports());
        let available = candidates
            .iter()
            .copied()
            .filter(|port| !used.contains(port) && ensure_port_available(*port, true).is_ok())
            .collect::<Vec<_>>();
        for window in available.windows(3) {
            if window[1] == window[0].saturating_add(1) && window[2] == window[0].saturating_add(2)
            {
                return Ok((window[0], window[1], true));
            }
        }
        Err("the Valheim port pool does not have three consecutive free UDP ports; add ports or expand its ranges in Servers > Port pools".to_owned())
    }

    fn read_game_port_policy(&self, game: GameKind) -> Result<GamePortPolicySpec, String> {
        let path = self.game_port_policy_path(game);
        if !path.exists() {
            return Ok(default_game_port_policy(game));
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "the game port policy is unavailable".to_owned())?;
        if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > 64 * 1024 {
            return Err("the game port policy is invalid".to_owned());
        }
        let stored: StoredGamePortPolicy = serde_json::from_slice(
            &fs::read(&path).map_err(|_| "could not read the game port policy".to_owned())?,
        )
        .map_err(|_| "the game port policy is invalid".to_owned())?;
        if stored.schema_version != 1 || stored.policy.game != game {
            return Err("the game port policy version or game does not match".to_owned());
        }
        normalize_game_port_policy(stored.policy)
    }

    fn game_port_policy_path(&self, game: GameKind) -> PathBuf {
        self.state_root.join(match game {
            GameKind::Minecraft => "port-policy-minecraft.json",
            GameKind::VRising => "port-policy-vrising.json",
            GameKind::Valheim => "port-policy-valheim.json",
            GameKind::Terraria => "port-policy-terraria.json",
        })
    }

    pub fn list_trashed_servers(&self) -> Result<Value, String> {
        let root = self.server_trash_root();
        let mut entries = Vec::new();
        for entry in fs::read_dir(&root)
            .map_err(|_| "could not read the removed server catalog".to_owned())?
        {
            let entry = entry.map_err(|_| "could not read a removed server entry".to_owned())?;
            let path = entry.path();
            let Some(trash_id) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if validate_trash_id(trash_id).is_err()
                || !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir())
            {
                continue;
            }
            let record = read_server_trash_record(&path.join("record.json"))?;
            let manifest = read_manifest(&path.join("manifest.json"))?;
            if record.schema_version != 1
                || record.trash_id != trash_id
                || record.instance_id != manifest.id
                || record.name != manifest.name
            {
                return Err("a removed server recovery record is inconsistent".to_owned());
            }
            let data_present = self.server_trash_data_path(trash_id)?.is_dir();
            entries.push(json!({
                "trash_id": record.trash_id,
                "instance_id": format!("helix:{}", record.instance_id),
                "name": record.name,
                "software": software_name(manifest.software),
                "minecraft_version": manifest.minecraft_version,
                "game_port": manifest.game_port,
                "trashed_at_unix_ms": record.trashed_at_unix_ms,
                "data_present": data_present,
                "backups_preserved": self.backup_path(&manifest.id)?.is_dir()
            }));
            if entries.len() >= MAX_SERVER_TRASH_ENTRIES {
                break;
            }
        }
        entries.sort_by(|left, right| {
            right["trashed_at_unix_ms"]
                .as_u64()
                .cmp(&left["trashed_at_unix_ms"].as_u64())
        });
        Ok(json!({
            "schema_version": 1,
            "servers": entries,
            "policy": {
                "recoverable": true,
                "automatic_purge": false,
                "note": "Removed native servers stay in protected recovery storage until you delete them forever from Removed and hidden or Settings → Helix data. That wipe includes world files, backups, and console history. Helix never auto-purges."
            }
        }))
    }

    pub fn trash_server(&self, id: &str, confirmation_name: &str) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(id))?;
        if confirmation_name != manifest.name {
            return Err("type the exact server name to confirm removal".to_owned());
        }
        let _operation = self.begin_instance_operation(&manifest.id, "server removal")?;
        if self.list_trashed_servers()?["servers"]
            .as_array()
            .is_some_and(|servers| servers.len() >= MAX_SERVER_TRASH_ENTRIES)
        {
            return Err("the removed server recovery catalog is full".to_owned());
        }
        if let Some((managed, instance_id)) =
            self.exact_container_identity(&manifest.container_name)?
            && (managed != "true" || instance_id != manifest.id)
        {
            return Err(
                "the workload name belongs to a container Helix cannot prove it owns".to_owned(),
            );
        }

        let was_running = self.container_running(&manifest.container_name);
        if was_running {
            self.docker(
                ["stop", "--time", "45", manifest.container_name.as_str()],
                75,
            )?;
        }

        let trash_id = Uuid::new_v4().to_string();
        let record_root = self.server_trash_record_path(&trash_id)?;
        let data_root = self.server_trash_data_path(&trash_id)?;
        let source_data = self.instance_path(&manifest.id)?;
        let source_manifest = self.manifest_path(&manifest.id)?;
        fs::create_dir(&record_root)
            .map_err(|_| "could not create the removed server recovery record".to_owned())?;
        fs::set_permissions(&record_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the removed server recovery record".to_owned())?;

        let result = (|| {
            if !source_data.is_dir() {
                return Err(
                    "the server data directory is unavailable; nothing was removed".to_owned(),
                );
            }
            fs::rename(&source_data, &data_root).map_err(|_| {
                "could not move the server data into protected recovery storage".to_owned()
            })?;
            fs::rename(&source_manifest, record_root.join("manifest.json")).map_err(|_| {
                "could not move the server definition into recovery storage".to_owned()
            })?;
            write_server_trash_record(
                &record_root.join("record.json"),
                &ServerTrashRecord {
                    schema_version: 1,
                    trash_id: trash_id.clone(),
                    instance_id: manifest.id.clone(),
                    name: manifest.name.clone(),
                    trashed_at_unix_ms: now_unix_ms(),
                    was_running,
                },
            )?;
            if self
                .exact_container_identity(&manifest.container_name)?
                .is_some()
            {
                self.docker(["rm", manifest.container_name.as_str()], 60)?;
            }
            Ok::<(), String>(())
        })();

        if let Err(error) = result {
            if record_root.join("manifest.json").is_file() && !source_manifest.exists() {
                let _ = fs::rename(record_root.join("manifest.json"), &source_manifest);
            }
            if data_root.is_dir() && !source_data.exists() {
                let _ = fs::rename(&data_root, &source_data);
            }
            let _ = fs::remove_file(record_root.join("record.json"));
            let _ = fs::remove_dir(&record_root);
            if was_running {
                let _ = self.docker(["start", manifest.container_name.as_str()], 90);
            }
            return Err(format!(
                "the server was not removed; Helix rolled its files back: {error}"
            ));
        }

        self.stop_console_archiver(&manifest.id);
        self.reclaim_unused_runtime(manifest.kind);
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "trash_id": trash_id,
            "recoverable": true,
            "backups_preserved": self.backup_path(&manifest.id)?.is_dir(),
            "console_history_preserved": self.console_archive_path(&manifest.id)?.is_dir()
        }))
    }

    pub fn restore_trashed_server(&self, trash_id: &str) -> Result<Value, String> {
        validate_trash_id(trash_id)?;
        let record_root = self.server_trash_record_path(trash_id)?;
        let record = read_server_trash_record(&record_root.join("record.json"))?;
        let manifest = read_manifest(&record_root.join("manifest.json"))?;
        if record.schema_version != 1
            || record.trash_id != trash_id
            || record.instance_id != manifest.id
            || record.name != manifest.name
        {
            return Err("the removed server recovery record is inconsistent".to_owned());
        }
        let _operation = self.begin_instance_operation(&manifest.id, "server restore")?;
        let data_source = self.server_trash_data_path(trash_id)?;
        let data_destination = self.instance_path(&manifest.id)?;
        let manifest_destination = self.manifest_path(&manifest.id)?;
        if !data_source.is_dir() {
            return Err("the removed server data is missing; restore was not attempted".to_owned());
        }
        if data_destination.exists()
            || manifest_destination.exists()
            || self
                .exact_container_identity(&manifest.container_name)?
                .is_some()
        {
            return Err(
                "an active server already uses this identity; restore was not attempted".to_owned(),
            );
        }
        for port in manifest.occupied_ports() {
            ensure_port_available(port, true)?;
        }
        if manifest.is_vrising() {
            self.ensure_vrising_runtime_image(&mut |_, _| {})?;
        } else if manifest.is_valheim() {
            self.ensure_valheim_runtime_image(&mut |_, _| {})?;
        } else if manifest.is_terraria() {
            self.ensure_terraria_runtime_image(&mut |_, _| {})?;
        } else {
            ensure_port_available(manifest.rcon_port, false)?;
        }

        fs::rename(&data_source, &data_destination)
            .map_err(|_| "could not restore the server data directory".to_owned())?;
        let result = (|| {
            self.create_container(&manifest, &data_destination)?;
            fs::rename(record_root.join("manifest.json"), &manifest_destination)
                .map_err(|_| "could not restore the server definition".to_owned())?;
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            if self
                .exact_container_identity(&manifest.container_name)
                .ok()
                .flatten()
                .is_some()
            {
                let _ = self.docker(["rm", "--force", manifest.container_name.as_str()], 60);
            }
            if manifest_destination.is_file() {
                let _ = fs::rename(&manifest_destination, record_root.join("manifest.json"));
            }
            let _ = fs::rename(&data_destination, &data_source);
            return Err(format!(
                "the removed server could not be restored and remains recoverable: {error}"
            ));
        }
        let _ = fs::remove_file(record_root.join("record.json"));
        let _ = fs::remove_dir(&record_root);
        self.ensure_console_archiver(&manifest)?;
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "restored": true,
            "online": false,
            "note": "The server was restored in the stopped state. Start it when ready."
        }))
    }

    pub fn purge_trashed_server(
        &self,
        trash_id: &str,
        confirmation_name: &str,
    ) -> Result<Value, String> {
        validate_trash_id(trash_id)?;
        let record_root = self.server_trash_record_path(trash_id)?;
        require_real_directory(
            &record_root,
            "the removed server recovery record is unavailable",
        )?;
        let record = read_server_trash_record(&record_root.join("record.json"))?;
        let manifest = read_manifest(&record_root.join("manifest.json"))?;
        if record.schema_version != 1
            || record.trash_id != trash_id
            || record.instance_id != manifest.id
            || record.name != manifest.name
        {
            return Err("the removed server recovery record is inconsistent".to_owned());
        }
        if confirmation_name != record.name {
            return Err("type the exact server name to confirm permanent deletion".to_owned());
        }
        let _operation =
            self.begin_instance_operation(&manifest.id, "permanent server deletion")?;
        let instance_path = self.instance_path(&manifest.id)?;
        let manifest_path = self.manifest_path(&manifest.id)?;
        if instance_path.exists() || manifest_path.exists() {
            return Err(
                "an active server still uses this identity; restore or remove it from Servers first"
                    .to_owned(),
            );
        }
        if let Some((managed, instance_id)) =
            self.exact_container_identity(&manifest.container_name)?
        {
            if managed != "true" || instance_id != manifest.id {
                return Err(
                    "the workload name belongs to a container Helix cannot prove it owns"
                        .to_owned(),
                );
            }
            self.docker(["rm", "--force", manifest.container_name.as_str()], 60)?;
            if self
                .exact_container_identity(&manifest.container_name)?
                .is_some()
            {
                return Err(
                    "the leftover Helix container still exists; nothing was permanently deleted"
                        .to_owned(),
                );
            }
        }

        self.stop_console_archiver(&manifest.id);
        remove_managed_directory(
            &self.server_trash_data_path(trash_id)?,
            "the removed server data",
        )?;
        remove_managed_directory(
            &self.backup_path(&manifest.id)?,
            "the removed server backups",
        )?;
        remove_managed_directory(
            &self.backup_trash_path(&manifest.id)?,
            "the removed server backup trash",
        )?;
        remove_managed_directory(
            &self.console_archive_path(&manifest.id)?,
            "the removed server console history",
        )?;
        remove_managed_directory(&record_root, "the removed server recovery record")?;
        self.reclaim_unused_runtime(manifest.kind);

        let mut result = json!({
            "instance_id": format!("helix:{}", manifest.id),
            "trash_id": trash_id,
            "name": manifest.name,
            "kind": manifest.kind_slug(),
            "game_port": manifest.game_port,
            "purged": true,
            "purged_at_unix_ms": now_unix_ms()
        });
        if manifest.query_port != 0 {
            result["query_port"] = json!(manifest.query_port);
        }
        Ok(result)
    }

    pub fn readiness(&self) -> Result<Value, String> {
        let docker_version = self
            .docker(["version", "--format", "{{.Server.Version}}"], 20)?
            .trim()
            .to_owned();
        if docker_version.is_empty() {
            return Err("the Helix execution backend did not report a version".to_owned());
        }
        let supported_software = [
            "pumpkin",
            "paper",
            "purpur",
            "folia",
            "leaves",
            "vanilla",
            "fabric",
            "neoforge",
            "forge",
            "quilt",
            "pufferfish",
            "custom",
        ];
        let features = [
            "publisher_checksum_verification_where_available",
            "pinned_artifact_digest",
            "isolated_execution",
            "lifecycle",
            "console",
            "persistent_console_history",
            "settings",
            "restart_metadata",
            "files",
            "backups",
            "recoverable_backup_trash",
            "recoverable_server_trash",
            "server_trash_purge",
            "restore",
            "logs",
            "performance",
            "operation_serialization",
            "vrising_wine_runtime",
            "native_start_on_boot",
            "local_custom_jar_import",
            "minecraft_version_catalog",
            "bounded_file_upload",
            "server_migrate",
        ];
        Ok(json!({
            "schema_version": 1,
            "availability": "ready",
            "manager": "helix",
            "execution_backend": "docker",
            "backend_version": docker_version,
            "supported_games": ["minecraft", "vrising", "valheim", "terraria"],
            "supported_minecraft_software": supported_software,
            "minecraft_software_catalog": minecraft_software_catalog(),
            "features": features,
            "custom_artifact_roots_configured": self.custom_artifact_roots.len(),
            "console_history_retention_bytes": self.console_retention.maximum_bytes,
            "console_history_retention_files": self.console_retention.files,
            "backup_trash_retention_days": self.backup_trash_retention_days,
            "instance_root": self.instance_root,
            "backup_root": self.backup_root,
            "collected_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn list_minecraft_versions(&self, software: MinecraftSoftware) -> Result<Value, String> {
        let (allows_latest, versions) = match software {
            MinecraftSoftware::Pumpkin => (true, self.pumpkin_versions()?),
            MinecraftSoftware::Paper => (
                true,
                self.fill_installable_versions(MinecraftSoftware::Paper, "paper")?,
            ),
            MinecraftSoftware::Folia => (
                true,
                self.fill_installable_versions(MinecraftSoftware::Folia, "folia")?,
            ),
            MinecraftSoftware::Purpur => (true, self.purpur_published_versions()?),
            MinecraftSoftware::Leaves => (true, self.leaves_installable_versions()?),
            MinecraftSoftware::Fabric => (true, self.fabric_published_versions()?),
            MinecraftSoftware::Quilt => (true, self.quilt_published_versions()?),
            MinecraftSoftware::Forge => (true, self.forge_published_versions()?),
            MinecraftSoftware::NeoForge => (true, self.neoforge_published_versions()?),
            MinecraftSoftware::Pufferfish => (true, self.pufferfish_published_versions()?),
            MinecraftSoftware::Vanilla => (true, self.vanilla_release_versions()?),
            MinecraftSoftware::Custom => (false, self.vanilla_release_versions()?),
        };
        Ok(json!({
            "schema_version": 1,
            "software": software,
            "allows_latest": allows_latest,
            "latest_version": versions.first().cloned(),
            "versions": versions,
        }))
    }

    pub fn begin_custom_jar_upload(
        &self,
        name: &str,
        expected_size: u64,
    ) -> Result<FileUploadStart, String> {
        if !(16 * 1024..=MAX_CUSTOM_JAR_UPLOAD_BYTES).contains(&expected_size) {
            return Err("custom server JARs must be between 16 KiB and 768 MiB".to_owned());
        }
        let mut uploads = self.uploads.lock().map_err(|_| upload_lock_error())?;
        prune_uploads(&mut uploads);
        if uploads.len() >= MAX_CONCURRENT_FILE_UPLOADS {
            return Err("Helix is already receiving the maximum number of uploads".to_owned());
        }
        let reserved = uploads
            .values()
            .map(|upload| upload.destination.clone())
            .collect::<HashSet<_>>();
        let name = unique_jar_name(&self.custom_import_root, name, &reserved)?;
        let destination = self.custom_import_root.join(&name);
        let upload = StreamingUpload::start(
            self.custom_import_root.clone(),
            destination,
            expected_size,
            FileUploadPurpose::CustomJar,
        )?;
        let upload_id = Uuid::new_v4().to_string();
        let start = FileUploadStart {
            upload_id: upload_id.clone(),
            expected_size,
            max_chunk_bytes: MAX_FILE_UPLOAD_CHUNK_BYTES as u64,
            purpose: FileUploadPurpose::CustomJar,
        };
        uploads.insert(upload_id, upload);
        Ok(start)
    }

    pub fn write_custom_jar_chunk(
        &self,
        upload_id: &str,
        purpose: FileUploadPurpose,
        offset: u64,
        data_base64: &str,
    ) -> Result<FileUploadProgress, String> {
        if purpose != FileUploadPurpose::CustomJar {
            return Err("that upload is not a custom server JAR".to_owned());
        }
        let data = decode_upload_chunk(data_base64)?;
        let mut uploads = self.uploads.lock().map_err(|_| upload_lock_error())?;
        prune_uploads(&mut uploads);
        let upload = uploads
            .get_mut(upload_id)
            .ok_or_else(|| "that upload is no longer active".to_owned())?;
        if upload.purpose != FileUploadPurpose::CustomJar {
            return Err("that upload is not a custom server JAR".to_owned());
        }
        let written = upload.write_chunk(offset, &data)?;
        Ok(FileUploadProgress {
            upload_id: upload_id.to_owned(),
            bytes_written: written,
            expected_size: upload.expected_size,
        })
    }

    pub fn finish_custom_jar_upload(
        &self,
        upload_id: &str,
        purpose: FileUploadPurpose,
    ) -> Result<crate::files::PathResult, String> {
        if purpose != FileUploadPurpose::CustomJar {
            return Err("that upload is not a custom server JAR".to_owned());
        }
        let mut uploads = self.uploads.lock().map_err(|_| upload_lock_error())?;
        prune_uploads(&mut uploads);
        let upload = uploads
            .remove(upload_id)
            .ok_or_else(|| "that upload is no longer active".to_owned())?;
        if upload.purpose != FileUploadPurpose::CustomJar {
            uploads.insert(upload_id.to_owned(), upload);
            return Err("that upload is not a custom server JAR".to_owned());
        }
        let destination = commit_upload(upload)?;
        Ok(crate::files::PathResult {
            path: destination.to_string_lossy().into_owned(),
        })
    }

    pub fn abort_custom_jar_upload(
        &self,
        upload_id: &str,
        purpose: FileUploadPurpose,
    ) -> Result<Value, String> {
        if purpose != FileUploadPurpose::CustomJar {
            return Err("that upload is not a custom server JAR".to_owned());
        }
        let mut uploads = self.uploads.lock().map_err(|_| upload_lock_error())?;
        prune_uploads(&mut uploads);
        if let Some(upload) = uploads.remove(upload_id) {
            if upload.purpose != FileUploadPurpose::CustomJar {
                uploads.insert(upload_id.to_owned(), upload);
                return Err("that upload is not a custom server JAR".to_owned());
            }
            upload.abort();
        }
        Ok(json!({ "aborted": true }))
    }

    pub fn server_detail(&self, id: &str) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(id))?;
        let states = self.runtime_states(std::slice::from_ref(&manifest));
        let state = states
            .get(&manifest.container_name)
            .cloned()
            .unwrap_or_default();
        let status = if !state.running {
            None
        } else if manifest.is_minecraft() {
            minecraft_status(manifest.game_port, Duration::from_millis(650)).ok()
        } else {
            None
        };
        let data_path = self.instance_path(&manifest.id)?;
        let disk_bytes = run_program(
            Path::new("/usr/bin/du"),
            &[
                "--summarize".to_owned(),
                "--bytes".to_owned(),
                "--one-file-system".to_owned(),
                data_path.to_string_lossy().into_owned(),
            ],
            30,
        )
        .ok()
        .and_then(|output| output.split_whitespace().next()?.parse::<u64>().ok())
        .unwrap_or(0);
        let inspect = self
            .docker(
                [
                    "inspect",
                    "--format",
                    "{{json .State}}",
                    manifest.container_name.as_str(),
                ],
                20,
            )
            .ok()
            .and_then(|output| serde_json::from_str::<Value>(output.trim()).ok())
            .unwrap_or_else(|| json!({}));
        let settings = if manifest.is_minecraft() {
            Some(self.server_settings_for(&manifest)?)
        } else {
            None
        };
        let detail_status = if manifest.uses_ready_marker() {
            if state.running { "online" } else { "stopped" }
        } else if status.is_some() {
            "online"
        } else if state.running {
            "starting"
        } else {
            "stopped"
        };
        let mut capabilities = vec![
            "console",
            "files",
            "backups",
            "restore",
            "recoverable_backup_trash",
            "logs",
            "performance",
            "advanced",
        ];
        if manifest.is_minecraft() {
            capabilities.insert(1, "settings");
        }
        let tps = if state.running && manifest.is_minecraft() {
            self.load_tps_map(&[(
                manifest.id.clone(),
                manifest.rcon_port,
                manifest.rcon_password.clone(),
            )])
            .get(&manifest.id)
            .copied()
            .flatten()
        } else {
            None
        };
        Ok(json!({
            "id": format!("helix:{}", manifest.id),
            "name": manifest.name,
            "instance_name": manifest.instance_name,
            "manager": "helix",
            "execution_backend": "docker",
            "kind": manifest.kind_slug(),
            "software": display_software(&manifest),
            "minecraft_version": manifest.minecraft_version,
            "build": manifest.build,
            "java_version": manifest.java_version,
            "runtime_image": manifest.runtime_image,
            "artifact_sha256": manifest.artifact_sha256,
            "modpack": manifest.modpack,
            "memory_limit_mb": manifest.memory_mb,
            "cpu_limit_millis": manifest.cpu_millis,
            "game_port": manifest.game_port,
            "query_port": if manifest.query_port == 0 { Value::Null } else { json!(manifest.query_port) },
            "console_endpoint": "local_only",
            "start_on_boot": manifest.start_on_boot,
            "created_at_unix_ms": manifest.created_at_unix_ms,
            "data_path": data_path,
            "disk_bytes": disk_bytes,
            "status": detail_status,
            "players_online": status.as_ref().map_or(0, |value| value.players_online),
            "max_players": status.as_ref().map_or(u64::from(manifest.max_players), |value| value.max_players),
            "cpu_percent": state.cpu_percent,
            "memory_used_mb": state.memory_used_mb,
            "tps": tps,
            "container_state": inspect,
            "settings": settings,
            "console_history": {
                "persistent": true,
                "retention_bytes": self.console_retention.maximum_bytes,
                "retention_files": self.console_retention.files,
                "scope": "per_server"
            },
            "capabilities": capabilities,
            "browser_listing": if manifest.is_vrising() {
                read_vrising_browser_listing(&data_path)
            } else {
                Value::Null
            }
        }))
    }

    pub fn server_logs(&self, id: &str, lines: u16) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(id))?;
        let lines = lines.clamp(1, 1_000);
        self.ensure_console_archiver(&manifest)?;
        let archive_root = self.console_archive_path(&manifest.id)?;
        let mut history = read_console_tail(&archive_root, usize::from(lines))?;
        if history.is_empty() {
            let output = run_program_combined(
                &self.docker_binary,
                &[
                    "logs".to_owned(),
                    "--timestamps".to_owned(),
                    "--tail".to_owned(),
                    lines.to_string(),
                    manifest.container_name.clone(),
                ],
                30,
            )?;
            history = output.lines().map(str::to_owned).collect();
        }
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "lines": history,
            "collected_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn server_log_history(
        &self,
        id: &str,
        cursor: Option<&str>,
        lines: u16,
    ) -> Result<Value, String> {
        if !(1..=500).contains(&lines) {
            return Err("console history pages must contain between 1 and 500 lines".to_owned());
        }
        let manifest = self.load_manifest(native_id(id))?;
        let archive = self.ensure_console_archiver(&manifest)?;
        let archive_root = self.console_archive_path(&manifest.id)?;
        let _archive_guard = archive
            .lock()
            .map_err(|_| "the console archive is unavailable".to_owned())?;
        let page = read_console_history_page(&archive_root, cursor, usize::from(lines))?;
        let has_more = page.next_cursor.is_some();
        let entries = page
            .lines
            .into_iter()
            .map(|line| console_history_entry(&line))
            .collect::<Vec<_>>();
        Ok(json!({
            "schema_version": 1,
            "instance_id": format!("helix:{}", manifest.id),
            "entries": entries,
            "next_cursor": page.next_cursor,
            "has_more": has_more,
            "order": "chronological_within_page",
            "pagination_direction": "newest_to_older",
            "page_text_byte_limit": MAX_CONSOLE_HISTORY_PAGE_TEXT_BYTES,
            "retention": {
                "maximum_bytes": self.console_retention.maximum_bytes,
                "files": self.console_retention.files,
                "scope": "per_server"
            },
            "collected_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn server_console(&self, id: &str, command: &str) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(id))?;
        if !manifest.is_minecraft() {
            return Err(format!(
                "{} does not expose a command console; use the log view and Files instead",
                display_software(&manifest)
            ));
        }
        let command = command.trim().trim_start_matches('/');
        if command.is_empty()
            || command.len() > MAX_CONSOLE_COMMAND_BYTES
            || command
                .chars()
                .any(|value| value == '\0' || value == '\r' || value == '\n')
        {
            return Err("console command must be one non-empty line under 512 bytes".to_owned());
        }
        if !self.container_running(&manifest.container_name) {
            return Err("start the server before using its console".to_owned());
        }
        let archive = self.ensure_console_archiver(&manifest)?;
        let response = rcon_command(manifest.rcon_port, &manifest.rcon_password, command)?;
        let mut history_recorded = false;
        if let Ok(mut archive) = archive.lock()
            && archive
                .append(&format!("[helix {}] > /{command}", now_unix_ms()))
                .is_ok()
        {
            history_recorded = true;
            for line in response.lines() {
                if archive
                    .append(&format!("[helix {}] < {line}", now_unix_ms()))
                    .is_err()
                {
                    history_recorded = false;
                    break;
                }
            }
        }
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "command": command,
            "response": response,
            "history_recorded": history_recorded,
            "completed_at_unix_ms": now_unix_ms()
        }))
    }

    fn begin_instance_operation(
        &self,
        instance_id: &str,
        operation: &str,
    ) -> Result<InstanceOperationGuard<'_>, String> {
        validate_id(instance_id)?;
        let key = instance_id.to_owned();
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "the server operation registry failed".to_owned())?;
        if !operations.insert(key.clone()) {
            return Err(format!(
                "another operation is already in progress for this server; {operation} was not started"
            ));
        }
        drop(operations);
        Ok(InstanceOperationGuard {
            operations: &self.operations,
            key,
        })
    }

    fn begin_creation_operation(&self) -> Result<InstanceOperationGuard<'_>, String> {
        let key = "__native_creation__".to_owned();
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "the server operation registry failed".to_owned())?;
        if !operations.insert(key.clone()) {
            return Err("another Helix server is already being created".to_owned());
        }
        drop(operations);
        Ok(InstanceOperationGuard {
            operations: &self.operations,
            key,
        })
    }

    fn ensure_console_archiver(
        &self,
        manifest: &InstanceManifest,
    ) -> Result<Arc<Mutex<ConsoleArchiveWriter>>, String> {
        let mut archives = self
            .console_archives
            .lock()
            .map_err(|_| "the console archive registry failed".to_owned())?;
        if let Some(archive) = archives.get(&manifest.id) {
            return Ok(Arc::clone(archive));
        }
        let archive_root = self.console_archive_path(&manifest.id)?;
        let archive = Arc::new(Mutex::new(ConsoleArchiveWriter::open(
            archive_root.clone(),
            self.console_retention,
        )?));
        archives.insert(manifest.id.clone(), Arc::clone(&archive));
        drop(archives);

        let stop = Arc::new(AtomicBool::new(false));
        self.console_stops
            .lock()
            .map_err(|_| "the console archive stop registry failed".to_owned())?
            .insert(manifest.id.clone(), Arc::clone(&stop));

        let config = ConsoleArchiverConfig {
            instance_id: manifest.id.clone(),
            container_name: manifest.container_name.clone(),
            docker_binary: self.docker_binary.clone(),
            docker_cli_home: prepare_docker_cli_home(&self.state_root)?,
            archive_root,
            stop,
        };
        let worker_archive = Arc::clone(&archive);
        if thread::Builder::new()
            .name(format!("console-{}", &manifest.id[..8]))
            .stack_size(256 * 1024)
            .spawn(move || console_archiver_loop(config, worker_archive))
            .is_err()
        {
            if let Ok(mut archives) = self.console_archives.lock() {
                archives.remove(&manifest.id);
            }
            if let Ok(mut stops) = self.console_stops.lock() {
                stops.remove(&manifest.id);
            }
            return Err("could not start persistent console capture".to_owned());
        }
        Ok(archive)
    }

    fn stop_console_archiver(&self, instance_id: &str) {
        if let Ok(mut stops) = self.console_stops.lock()
            && let Some(stop) = stops.remove(instance_id)
        {
            stop.store(true, Ordering::Release);
        }
        if let Ok(mut archives) = self.console_archives.lock() {
            archives.remove(instance_id);
        }
    }

    pub fn server_settings(&self, id: &str) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(id))?;
        self.server_settings_for(&manifest)
    }

    pub fn update_server_settings(
        &self,
        id: &str,
        settings: &MinecraftSettingsPatch,
    ) -> Result<Value, String> {
        validate_settings(settings)?;
        let mut manifest = self.load_manifest(native_id(id))?;
        if !manifest.is_minecraft() {
            return Err(format!(
                "{} settings live in the save data; use Files to edit them",
                display_software(&manifest)
            ));
        }
        let _operation = self.begin_instance_operation(&manifest.id, "settings update")?;
        let path = self
            .instance_path(&manifest.id)?
            .join(manifest.settings_name());
        let original = read_small_regular_file(&path, MAX_PROPERTIES_BYTES, "server settings")?;
        let revision = file_sha256(&path)?;
        if revision != settings.expected_revision {
            return Err(
                "server settings changed since this page was opened; reload before saving"
                    .to_owned(),
            );
        }
        let changed_fields = {
            let properties = if manifest.is_pumpkin() {
                pumpkin::properties(&original)?
            } else {
                parse_properties(&original)
            };
            let mut fields = changed_setting_fields(&properties, settings);
            if settings.memory_mb != manifest.memory_mb {
                fields.push("memory_mb");
            }
            fields
        };
        let updated = if manifest.is_pumpkin() {
            pumpkin::update_config(&original, settings)?
        } else {
            update_properties(&original, settings)
        };
        let port_changed = settings.game_port != manifest.game_port;
        if manifest.is_pumpkin() && settings.game_port == manifest.query_port {
            return Err("Java and Bedrock require separate TCP ports".to_owned());
        }
        let memory_changed = settings.memory_mb != manifest.memory_mb;
        if port_changed {
            self.ensure_replacement_game_port(&manifest, settings.game_port)?;
        }
        if memory_changed {
            validate_allocated_memory(manifest.kind, settings.memory_mb)?;
        }
        if updated == original && !port_changed && !memory_changed {
            return Ok(json!({
                "instance_id": format!("helix:{}", manifest.id),
                "changed": false,
                "restart_required": false,
                "container_republished": false,
                "changed_fields": changed_fields,
                "settings": self.server_settings_for(&manifest)?
            }));
        }
        let backup = path.with_extension(format!(
            "{}.{}.bak",
            if manifest.is_pumpkin() {
                "toml"
            } else {
                "properties"
            },
            now_unix_ms()
        ));
        let previous = manifest.clone();
        let data_path = self.instance_path(&manifest.id)?;
        if updated != original {
            write_managed_file(&backup, original.as_bytes(), 0o600, 0, 0)?;
            write_managed_file(&path, updated.as_bytes(), 0o660, 0, manifest.run_uid)?;
        }
        manifest.max_players = settings.max_players;
        manifest.game_port = settings.game_port;
        manifest.memory_mb = settings.memory_mb;
        if let Err(error) = write_manifest(&self.manifest_path(&manifest.id)?, &manifest) {
            if updated != original {
                let _ = write_managed_file(&path, original.as_bytes(), 0o660, 0, manifest.run_uid);
            }
            return Err(error);
        }
        let running = self.container_running(&manifest.container_name);
        let needs_republish = port_changed || memory_changed;
        if needs_republish
            && let Err(error) = self.republish_minecraft_container(&manifest, &data_path, running)
        {
            if updated != original {
                let _ = write_managed_file(&path, original.as_bytes(), 0o660, 0, previous.run_uid);
            }
            let _ = write_manifest(&self.manifest_path(&previous.id)?, &previous);
            let _ = self.republish_minecraft_container(&previous, &data_path, running);
            return Err(error);
        }
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "changed": true,
            "restart_required": running && !needs_republish,
            "container_republished": needs_republish,
            "previous_game_port": if port_changed { json!(previous.game_port) } else { Value::Null },
            "changed_fields": changed_fields,
            "settings": self.server_settings_for(&manifest)?,
            "rollback_file": if updated != original { json!(backup) } else { Value::Null }
        }))
    }

    pub fn list_backups(&self, id: &str) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(id))?;
        let root = self.backup_path(&manifest.id)?;
        let mut backups = Vec::new();
        let mut backups_truncated = false;
        if root.is_dir() {
            for entry in fs::read_dir(&root)
                .map_err(|_| "could not read the server backup catalog".to_owned())?
            {
                let entry = entry.map_err(|_| "could not read a server backup entry".to_owned())?;
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                let Some(id) = name.strip_suffix(".tar.gz") else {
                    continue;
                };
                if !valid_backup_id(id) {
                    continue;
                }
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|_| "could not inspect a server backup".to_owned())?;
                if !metadata.file_type().is_file() {
                    continue;
                }
                if backups.len() >= MAX_BACKUP_CATALOG_ENTRIES {
                    backups_truncated = true;
                    break;
                }
                backups.push(json!({
                    "id": id,
                    "created_at_unix_ms": id.parse::<u64>().unwrap_or(0),
                    "size_bytes": metadata.len(),
                    "definition_present": fs::symlink_metadata(root.join(format!("{id}.json")))
                        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() > 0)
                }));
            }
        }
        backups.sort_by_key(|backup| {
            std::cmp::Reverse(backup["created_at_unix_ms"].as_u64().unwrap_or(0))
        });
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "backups": backups,
            "backups_truncated": backups_truncated,
            "backup_catalog_limit": MAX_BACKUP_CATALOG_ENTRIES,
            "trash": self.list_backup_trash(&manifest)?,
            "trash_policy": {
                "purge_after_days": self.backup_trash_retention_days,
                "automatic_purge_enabled": false,
                "note": "Deleted backups stay in protected trash until you restore them or delete them forever."
            },
            "policy": {
                "keep_count": manifest.backup_keep_count,
                "keep_days": manifest.backup_keep_days,
                "keep_count_max": 50,
                "keep_days_max": 365,
                "note": backup_retention_note(manifest.backup_keep_count, manifest.backup_keep_days)
            }
        }))
    }

    pub fn set_backup_policy(
        &self,
        id: &str,
        keep_count: u16,
        keep_days: u16,
    ) -> Result<Value, String> {
        validate_backup_policy(keep_count, keep_days)?;
        let mut manifest = self.load_manifest(native_id(id))?;
        let _operation = self.begin_instance_operation(&manifest.id, "backup policy")?;
        manifest.backup_keep_count = keep_count;
        manifest.backup_keep_days = keep_days;
        write_manifest(&self.manifest_path(&manifest.id)?, &manifest)?;
        let trashed = self.enforce_backup_retention(&manifest, "")?;
        let mut catalog = self.list_backups_unlocked(&manifest)?;
        catalog["retention_trashed"] = json!(trashed);
        Ok(catalog)
    }

    pub fn prune_backups(&self, id: &str) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(id))?;
        let _operation = self.begin_instance_operation(&manifest.id, "backup prune")?;
        let trashed = self.enforce_backup_retention(&manifest, "")?;
        let mut catalog = self.list_backups_unlocked(&manifest)?;
        catalog["retention_trashed"] = json!(trashed);
        Ok(catalog)
    }

    pub fn purge_backup_trash(&self, id: &str, trash_id: &str) -> Result<Value, String> {
        validate_trash_id(trash_id)?;
        let manifest = self.load_manifest(native_id(id))?;
        let _operation = self.begin_instance_operation(&manifest.id, "backup purge")?;
        let trash_directory = self.backup_trash_path(&manifest.id)?.join(trash_id);
        require_real_directory(
            &trash_directory,
            "the selected deleted backup is unavailable",
        )?;
        let record = read_backup_trash_record(&trash_directory.join("trash.json"))?;
        if record.schema_version != 1
            || record.trash_id != trash_id
            || record.instance_id != manifest.id
            || !valid_backup_id(&record.backup_id)
        {
            return Err("the selected deleted backup record is invalid".to_owned());
        }
        remove_known_backup_trash_directory(
            &trash_directory,
            &record.backup_id,
            record.definition_present,
        )?;
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "trash_id": trash_id,
            "backup_id": record.backup_id,
            "purged": true,
            "purged_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn trash_backup(&self, id: &str, backup_id: &str) -> Result<Value, String> {
        if !valid_backup_id(backup_id) {
            return Err("backup ID is invalid".to_owned());
        }
        let manifest = self.load_manifest(native_id(id))?;
        let _operation = self.begin_instance_operation(&manifest.id, "backup deletion")?;
        self.trash_named_backup(&manifest, backup_id)
    }

    fn list_backups_unlocked(&self, manifest: &InstanceManifest) -> Result<Value, String> {
        self.list_backups(&format!("helix:{}", manifest.id))
    }

    fn enforce_backup_retention(
        &self,
        manifest: &InstanceManifest,
        protect_id: &str,
    ) -> Result<Vec<String>, String> {
        if manifest.backup_keep_count == 0 && manifest.backup_keep_days == 0 {
            return Ok(Vec::new());
        }
        let catalog = self.list_backups_unlocked(manifest)?;
        let backups = catalog
            .get("backups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let cutoff = if manifest.backup_keep_days == 0 {
            0
        } else {
            now_unix_ms().saturating_sub(
                u64::from(manifest.backup_keep_days).saturating_mul(24 * 60 * 60 * 1_000),
            )
        };
        let mut keepers = Vec::new();
        let mut victims = Vec::new();
        for backup in backups {
            let id = backup
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if id.is_empty() {
                continue;
            }
            let created = backup
                .get("created_at_unix_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let over_age = id != protect_id
                && manifest.backup_keep_days > 0
                && created > 0
                && created < cutoff;
            if over_age {
                victims.push(id);
            } else {
                keepers.push(id);
            }
        }
        if manifest.backup_keep_count > 0 {
            let limit = usize::from(manifest.backup_keep_count);
            if keepers.len() > limit {
                for extra in keepers.split_off(limit) {
                    if extra != protect_id {
                        victims.push(extra);
                    }
                }
            }
        }
        let mut trashed = Vec::new();
        for backup_id in victims {
            self.trash_named_backup(manifest, &backup_id)?;
            trashed.push(backup_id);
        }
        Ok(trashed)
    }

    fn trash_named_backup(
        &self,
        manifest: &InstanceManifest,
        backup_id: &str,
    ) -> Result<Value, String> {
        if !valid_backup_id(backup_id) {
            return Err("backup ID is invalid".to_owned());
        }
        let backup_root = self.backup_path(&manifest.id)?;
        let archive = backup_root.join(format!("{backup_id}.tar.gz"));
        let definition = backup_root.join(format!("{backup_id}.json"));
        require_regular_backup_file(&archive, "the selected backup archive is unavailable")?;
        let definition_present = match fs::symlink_metadata(&definition) {
            Ok(metadata) if metadata.file_type().is_file() && metadata.len() > 0 => true,
            Ok(_) => return Err("the selected backup definition is invalid".to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err("could not inspect the selected backup definition".to_owned()),
        };

        let trash_root = self.backup_trash_path(&manifest.id)?;
        fs::create_dir_all(&trash_root)
            .map_err(|_| "could not create the server backup trash".to_owned())?;
        fs::set_permissions(&trash_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the server backup trash".to_owned())?;
        let trash_id = Uuid::new_v4().to_string();
        let staging = trash_root.join(format!(".partial-{trash_id}"));
        let destination = trash_root.join(&trash_id);
        fs::create_dir(&staging).map_err(|_| "could not stage backup deletion".to_owned())?;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect staged backup deletion".to_owned())?;
        let trashed_at_unix_ms = now_unix_ms();
        let purge_eligible_at_unix_ms = trashed_at_unix_ms.saturating_add(
            u64::from(self.backup_trash_retention_days).saturating_mul(24 * 60 * 60 * 1_000),
        );
        let record = BackupTrashRecord {
            schema_version: 1,
            trash_id: trash_id.clone(),
            instance_id: manifest.id.clone(),
            backup_id: backup_id.to_owned(),
            trashed_at_unix_ms,
            purge_eligible_at_unix_ms,
            definition_present,
        };
        let staged_archive = staging.join(format!("{backup_id}.tar.gz"));
        let staged_definition = staging.join(format!("{backup_id}.json"));
        let transaction = (|| {
            fs::hard_link(&archive, &staged_archive)
                .map_err(|_| "could not stage the backup archive for deletion".to_owned())?;
            if definition_present {
                fs::hard_link(&definition, &staged_definition)
                    .map_err(|_| "could not stage the backup definition for deletion".to_owned())?;
            }
            write_backup_trash_record(&staging.join("trash.json"), &record)?;
            sync_directory(&staging)?;
            fs::rename(&staging, &destination)
                .map_err(|_| "could not commit recoverable backup deletion".to_owned())?;
            sync_directory(&trash_root)?;
            fs::remove_file(&archive)
                .map_err(|_| "could not remove the backup from the active catalog".to_owned())?;
            if definition_present {
                fs::remove_file(&definition).map_err(|_| {
                    "could not remove the backup definition from the active catalog".to_owned()
                })?;
            }
            sync_directory(&backup_root)?;
            Ok::<(), String>(())
        })();
        if let Err(error) = transaction {
            let mut rollback_errors = Vec::new();
            if !archive.exists()
                && destination.join(format!("{backup_id}.tar.gz")).is_file()
                && fs::hard_link(destination.join(format!("{backup_id}.tar.gz")), &archive).is_err()
            {
                rollback_errors.push("could not restore the active backup archive");
            }
            if definition_present
                && !definition.exists()
                && destination.join(format!("{backup_id}.json")).is_file()
                && fs::hard_link(destination.join(format!("{backup_id}.json")), &definition)
                    .is_err()
            {
                rollback_errors.push("could not restore the active backup definition");
            }
            if require_regular_backup_file(
                &archive,
                "the active backup archive could not be verified after rollback",
            )
            .is_err()
            {
                rollback_errors.push("the active backup archive could not be verified");
            }
            if definition_present
                && require_regular_backup_file(
                    &definition,
                    "the active backup definition could not be verified after rollback",
                )
                .is_err()
            {
                rollback_errors.push("the active backup definition could not be verified");
            }
            if rollback_errors.is_empty() && sync_directory(&backup_root).is_err() {
                rollback_errors.push("could not durably sync the restored active backup catalog");
            }
            if rollback_errors.is_empty() {
                let cleanup = remove_known_backup_trash_directory(
                    if destination.is_dir() {
                        &destination
                    } else {
                        &staging
                    },
                    backup_id,
                    definition_present,
                );
                return Err(if cleanup.is_ok() {
                    format!(
                        "backup deletion was rolled back without discarding the backup: {error}"
                    )
                } else {
                    format!(
                        "backup deletion was rolled back, but duplicate recovery cleanup remains pending: {error}"
                    )
                });
            }
            return Err(format!(
                "backup deletion failed and Helix retained its recovery copy because rollback could not be proven: {error}; {}",
                rollback_errors.join("; ")
            ));
        }
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "backup_id": backup_id,
            "trash_id": trash_id,
            "trashed_at_unix_ms": trashed_at_unix_ms,
            "undo_available": true,
            "undo_expires_at_unix_ms": Value::Null,
            "purge_eligible_at_unix_ms": purge_eligible_at_unix_ms,
            "automatic_purge_enabled": false
        }))
    }

    pub fn restore_trashed_backup(&self, id: &str, trash_id: &str) -> Result<Value, String> {
        validate_trash_id(trash_id)?;
        let manifest = self.load_manifest(native_id(id))?;
        let _operation = self.begin_instance_operation(&manifest.id, "backup undo")?;
        let trash_directory = self.backup_trash_path(&manifest.id)?.join(trash_id);
        require_real_directory(
            &trash_directory,
            "the selected deleted backup is unavailable",
        )?;
        let record = read_backup_trash_record(&trash_directory.join("trash.json"))?;
        if record.schema_version != 1
            || record.trash_id != trash_id
            || record.instance_id != manifest.id
            || !valid_backup_id(&record.backup_id)
        {
            return Err("the selected deleted backup record is invalid".to_owned());
        }
        let trashed_archive = trash_directory.join(format!("{}.tar.gz", record.backup_id));
        let trashed_definition = trash_directory.join(format!("{}.json", record.backup_id));
        require_regular_backup_file(
            &trashed_archive,
            "the deleted backup archive is unavailable",
        )?;
        if record.definition_present {
            require_regular_backup_file(
                &trashed_definition,
                "the deleted backup definition is unavailable",
            )?;
        }
        let backup_root = self.backup_path(&manifest.id)?;
        fs::create_dir_all(&backup_root)
            .map_err(|_| "could not create the server backup directory".to_owned())?;
        let archive = backup_root.join(format!("{}.tar.gz", record.backup_id));
        let definition = backup_root.join(format!("{}.json", record.backup_id));
        let archive_already_linked = archive.exists();
        if archive_already_linked && !same_regular_file(&archive, &trashed_archive)? {
            return Err("a different backup with the original ID already exists".to_owned());
        }
        let definition_already_linked = definition.exists();
        if definition_already_linked
            && (!record.definition_present || !same_regular_file(&definition, &trashed_definition)?)
        {
            return Err(
                "a different backup definition with the original ID already exists".to_owned(),
            );
        }
        if !archive_already_linked {
            fs::hard_link(&trashed_archive, &archive)
                .map_err(|_| "could not restore the backup archive".to_owned())?;
        }
        if record.definition_present
            && !definition_already_linked
            && let Err(error) = fs::hard_link(&trashed_definition, &definition)
        {
            if !archive_already_linked {
                let _ = fs::remove_file(&archive);
            }
            return Err(format!("could not restore the backup definition: {error}"));
        }
        sync_directory(&backup_root)?;
        let cleanup_pending = remove_known_backup_trash_directory(
            &trash_directory,
            &record.backup_id,
            record.definition_present,
        )
        .is_err();
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "backup_id": record.backup_id,
            "trash_id": trash_id,
            "restored_at_unix_ms": now_unix_ms(),
            "cleanup_pending": cleanup_pending
        }))
    }

    fn list_backup_trash(&self, manifest: &InstanceManifest) -> Result<Vec<Value>, String> {
        let root = self.backup_trash_path(&manifest.id)?;
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in
            fs::read_dir(&root).map_err(|_| "could not read the server backup trash".to_owned())?
        {
            let entry = entry.map_err(|_| "could not read a deleted backup entry".to_owned())?;
            let path = entry.path();
            let Some(trash_id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if validate_trash_id(trash_id).is_err() || !metadata.file_type().is_dir() {
                continue;
            }
            let Ok(record) = read_backup_trash_record(&path.join("trash.json")) else {
                continue;
            };
            if record.schema_version != 1
                || record.trash_id != trash_id
                || record.instance_id != manifest.id
                || !valid_backup_id(&record.backup_id)
            {
                continue;
            }
            let archive = path.join(format!("{}.tar.gz", record.backup_id));
            let Ok(metadata) = fs::symlink_metadata(&archive) else {
                continue;
            };
            if !metadata.file_type().is_file() {
                continue;
            }
            records.push(json!({
                "trash_id": record.trash_id,
                "backup_id": record.backup_id,
                "trashed_at_unix_ms": record.trashed_at_unix_ms,
                "undo_available": true,
                "undo_expires_at_unix_ms": Value::Null,
                "purge_eligible_at_unix_ms": record.purge_eligible_at_unix_ms,
                "automatic_purge_enabled": false,
                "size_bytes": metadata.len(),
                "definition_present": record.definition_present
            }));
            if records.len() >= 2_048 {
                break;
            }
        }
        records.sort_by_key(|record| {
            std::cmp::Reverse(record["trashed_at_unix_ms"].as_u64().unwrap_or(0))
        });
        Ok(records)
    }

    pub fn restore_backup(&self, id: &str, backup_id: &str) -> Result<Value, String> {
        if !valid_backup_id(backup_id) {
            return Err("backup ID is invalid".to_owned());
        }
        let manifest = self.load_manifest(native_id(id))?;
        let _operation = self.begin_instance_operation(&manifest.id, "backup restore")?;
        let backup_root = self.backup_path(&manifest.id)?;
        let archive = backup_root.join(format!("{backup_id}.tar.gz"));
        let definition = backup_root.join(format!("{backup_id}.json"));
        let restored_manifest = read_manifest(&definition)
            .map_err(|_| "this backup predates restorable server definitions".to_owned())?;
        if restored_manifest.id != manifest.id
            || restored_manifest.container_name != manifest.container_name
        {
            return Err("the selected backup does not belong to this server".to_owned());
        }
        let metadata = fs::symlink_metadata(&archive)
            .map_err(|_| "the selected backup archive is unavailable".to_owned())?;
        if !metadata.file_type().is_file() || metadata.len() == 0 {
            return Err("the selected backup archive is invalid".to_owned());
        }
        let running = self.container_running(&manifest.container_name);
        if running {
            self.docker(
                ["stop", "--time", "45", manifest.container_name.as_str()],
                75,
            )?;
        }
        let safety_backup = match self.archive_data(&manifest) {
            Ok(path) => path,
            Err(error) => {
                let restart = self.restart_if_previously_running(&manifest, running);
                return Err(match restart {
                    Ok(()) => {
                        format!("restore was not started because the safety backup failed: {error}")
                    }
                    Err(restart) => format!(
                        "restore was not started because the safety backup failed: {error}; the original server also failed to restart: {restart}"
                    ),
                });
            }
        };
        let data_path = self.instance_path(&manifest.id)?;
        let previous =
            self.instance_root
                .join(format!(".restore-{}-{}", manifest.id, now_unix_ms()));
        if fs::rename(&data_path, &previous).is_err() {
            let restart = self.restart_if_previously_running(&manifest, running);
            return Err(match restart {
                Ok(()) => "could not stage the current server for restore".to_owned(),
                Err(restart) => format!(
                    "could not stage the current server for restore; the original server also failed to restart: {restart}"
                ),
            });
        }
        let restore_result = (|| {
            fs::create_dir(&data_path)
                .map_err(|_| "could not create the restore directory".to_owned())?;
            fs::set_permissions(&data_path, fs::Permissions::from_mode(0o750))
                .map_err(|_| "could not protect the restore directory".to_owned())?;
            run_program(
                Path::new("/usr/bin/tar"),
                &[
                    "--extract".to_owned(),
                    "--gzip".to_owned(),
                    "--file".to_owned(),
                    archive.to_string_lossy().into_owned(),
                    "--directory".to_owned(),
                    data_path.to_string_lossy().into_owned(),
                    "--no-same-owner".to_owned(),
                    "--no-same-permissions".to_owned(),
                ],
                30 * 60,
            )?;
            self.chown_instance(&data_path, restored_manifest.run_uid)?;
            self.protect_instance_artifacts(&data_path, restored_manifest.run_uid)?;
            write_manifest(&self.manifest_path(&manifest.id)?, &restored_manifest)?;
            if running {
                self.docker(["start", restored_manifest.container_name.as_str()], 90)?;
                self.wait_until_ready(
                    &restored_manifest,
                    self.ready_timeout(&restored_manifest),
                    |_| {},
                )?;
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = restore_result {
            let _ = self.docker(
                ["stop", "--time", "15", manifest.container_name.as_str()],
                30,
            );
            if data_path.exists() {
                let failed = self.instance_root.join(".failed").join(format!(
                    "restore-{}-{}",
                    manifest.id,
                    now_unix_ms()
                ));
                let _ = fs::rename(&data_path, failed);
            }
            let _ = fs::rename(&previous, &data_path);
            let _ = write_manifest(&self.manifest_path(&manifest.id)?, &manifest);
            let restart = self.restart_if_previously_running(&manifest, running);
            return Err(format!(
                "restore validation failed and the previous server was put back: {error}{}",
                restart
                    .err()
                    .map(|restart| format!("; the original server failed to restart: {restart}"))
                    .unwrap_or_default()
            ));
        }
        let recovery = self.instance_root.join(".failed").join(format!(
            "pre-restore-{}-{}",
            manifest.id,
            now_unix_ms()
        ));
        fs::rename(previous, &recovery).map_err(|_| {
            "restore completed, but the previous files could not be moved into recovery".to_owned()
        })?;
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "backup_id": backup_id,
            "safety_backup": backup_id_from_path(&safety_backup),
            "previous_files_recovery": recovery,
            "online": running
        }))
    }

    fn server_settings_for(&self, manifest: &InstanceManifest) -> Result<Value, String> {
        let path = self
            .instance_path(&manifest.id)?
            .join(manifest.settings_name());
        let content = read_small_regular_file(&path, MAX_PROPERTIES_BYTES, "server settings")?;
        let properties = if manifest.is_pumpkin() {
            pumpkin::properties(&content)?
        } else {
            parse_properties(&content)
        };
        Ok(json!({
            "expected_revision": file_sha256(&path)?,
            "motd": property_text(&properties, "motd", &manifest.name),
            "game_mode": property_choice(&properties, "gamemode", &["survival", "creative", "adventure", "spectator"], "survival"),
            "difficulty": property_choice(&properties, "difficulty", &["peaceful", "easy", "normal", "hard"], "easy"),
            "max_players": property_u64(&properties, "max-players", u64::from(manifest.max_players), 1, 10_000),
            "view_distance": property_u64(&properties, "view-distance", 10, 2, 32),
            "simulation_distance": property_u64(&properties, "simulation-distance", 10, 2, 32),
            "player_idle_timeout": property_u64(&properties, "player-idle-timeout", 0, 0, 65_535),
            "online_mode": property_bool(&properties, "online-mode", true),
            "pvp": property_bool(&properties, "pvp", true),
            "allow_flight": property_bool(&properties, "allow-flight", false),
            "white_list": property_bool(&properties, "white-list", false),
            "enforce_white_list": property_bool(&properties, "enforce-whitelist", false),
            "spawn_protection": property_u64(&properties, "spawn-protection", 16, 0, 65_535),
            "game_port": property_u64(&properties, "server-port", u64::from(manifest.game_port), 1_024, 65_535),
            "memory_mb": manifest.memory_mb,
            "restart_behavior": {
                "activation": "server_restart",
                "restart_required_fields": [
                    "motd", "game_mode", "difficulty", "max_players", "view_distance",
                    "simulation_distance", "player_idle_timeout", "online_mode", "pvp",
                    "allow_flight", "white_list", "enforce_white_list", "spawn_protection",
                    "game_port"
                ],
                "message": "Most changes take effect the next time Minecraft starts. Changing the game port or memory rebinds the published container immediately."
            }
        }))
    }

    pub fn server_action(&self, id: &str, action: ServerAction) -> Result<Value, String> {
        let manifest = self.load_manifest(id.strip_prefix("helix:").unwrap_or(id))?;
        match action {
            ServerAction::Kill => self.kill_instance(&manifest),
            exclusive => self.exclusive_server_action(&manifest, exclusive),
        }
    }

    fn exclusive_server_action(
        &self,
        manifest: &InstanceManifest,
        action: ServerAction,
    ) -> Result<Value, String> {
        let _operation = self.begin_instance_operation(&manifest.id, "server action")?;
        self.ensure_console_archiver(manifest)?;
        let was_running = self.container_running(&manifest.container_name);
        let detail = match action {
            ServerAction::Start => {
                if !was_running {
                    self.clear_ready_marker(manifest)?;
                    self.docker(["start", manifest.container_name.as_str()], 90)?;
                }
                self.wait_until_ready(manifest, self.ready_timeout(manifest), |_| {})?;
                json!({"online": true, "already_running": was_running})
            }
            ServerAction::Stop => {
                if was_running {
                    self.terminate_container(&manifest.container_name, false)?;
                }
                json!({"online": false, "already_stopped": !was_running})
            }
            ServerAction::Restart => {
                self.clear_ready_marker(manifest)?;
                if was_running {
                    self.docker(
                        ["restart", "--time", "45", manifest.container_name.as_str()],
                        90,
                    )?;
                } else {
                    self.docker(["start", manifest.container_name.as_str()], 90)?;
                }
                self.wait_until_ready(manifest, self.ready_timeout(manifest), |_| {})?;
                json!({"online": true, "previously_running": was_running})
            }
            ServerAction::Backup => {
                let path = self.backup(manifest)?;
                let backup_id = backup_id_from_path(&path);
                let retention = self.enforce_backup_retention(manifest, &backup_id);
                json!({
                    "backup_id": backup_id,
                    "retention_trashed": retention.as_ref().ok().cloned().unwrap_or_default(),
                    "retention_error": retention.err()
                })
            }
            ServerAction::Update => {
                if manifest.modpack.is_some() {
                    return self.update_minecraft_modpack(manifest).map(|detail| {
                        json!({
                            "instance_id": format!("helix:{}", manifest.id),
                            "action": action,
                            "accepted": true,
                            "detail": detail
                        })
                    });
                }
                let changed = self.update(manifest)?;
                json!({
                    "updated": changed,
                    "already_current": !changed,
                    "server_was_running": was_running,
                    "restart_required": changed && !was_running,
                    "runtime_validation_performed": changed && was_running,
                    "rollback_on_failed_startup": changed && was_running
                })
            }
            ServerAction::Kill => {
                return Err("kill cannot run under the exclusive server lock".to_owned());
            }
        };
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "action": action,
            "accepted": true,
            "detail": detail
        }))
    }

    fn kill_instance(&self, manifest: &InstanceManifest) -> Result<Value, String> {
        self.ensure_console_archiver(manifest)?;
        let was_running = self.container_running(&manifest.container_name);
        if was_running {
            self.terminate_container(&manifest.container_name, true)?;
        }
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "action": ServerAction::Kill,
            "accepted": true,
            "detail": {
                "online": false,
                "already_stopped": !was_running,
                "forced": true
            }
        }))
    }

    fn terminate_container(&self, name: &str, force: bool) -> Result<(), String> {
        let result = if force {
            self.docker(["kill", name], 20)
        } else {
            self.docker(["stop", "--time", "45", name], 75)
        };
        match result {
            Ok(_) => Ok(()),
            Err(_) if !self.container_running(name) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn ensure_replacement_game_port(
        &self,
        manifest: &InstanceManifest,
        port: u16,
    ) -> Result<(), String> {
        if port < 1_024 {
            return Err("game port must be at least 1024".to_owned());
        }
        if port == manifest.rcon_port {
            return Err("game port cannot reuse this server's RCON port".to_owned());
        }
        let manifests = self.load_manifests()?;
        let mut helix = assigned_game_ports(&manifests);
        for occupied in manifest.occupied_ports() {
            helix.remove(&occupied);
        }
        if let Some(error) = self.port_conflict_error(port, &helix) {
            return Err(error);
        }
        ensure_port_available(port, true)
    }

    fn republish_minecraft_container(
        &self,
        manifest: &InstanceManifest,
        data_path: &Path,
        was_running: bool,
    ) -> Result<(), String> {
        if self
            .exact_container_identity(&manifest.container_name)?
            .is_some()
        {
            if was_running || self.container_running(&manifest.container_name) {
                self.terminate_container(&manifest.container_name, false)?;
            }
            self.docker(["rm", manifest.container_name.as_str()], 60)?;
        }
        self.create_container(manifest, data_path)?;
        if was_running {
            self.clear_ready_marker(manifest)?;
            self.docker(["start", manifest.container_name.as_str()], 90)?;
        }
        Ok(())
    }

    pub fn create_minecraft<F>(
        &self,
        spec: &MinecraftCreateSpec,
        overlay: Option<&Path>,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        validate_create_spec(spec)?;
        if matches!(spec.software, MinecraftSoftware::Pumpkin) && overlay.is_some() {
            return Err("Pumpkin world and plugin conversion is not a safe automatic migration; create a separate server and import only verified compatible data from a backup".to_owned());
        }
        let _operation = self.begin_creation_operation()?;
        progress("Checking ports, names, and storage", 6);
        let manifests = self.load_manifests()?;
        if manifests
            .iter()
            .any(|manifest| manifest.name.eq_ignore_ascii_case(spec.name.trim()))
        {
            return Err("a Helix server with that name already exists".to_owned());
        }
        let (game_port, allocated_automatically) =
            self.resolve_game_port(GameKind::Minecraft, spec.game_port, &manifests)?;
        let mut resolved_spec = spec.clone();
        resolved_spec.game_port = Some(game_port);
        let bedrock_port = if matches!(spec.software, MinecraftSoftware::Pumpkin) {
            self.resolve_pumpkin_bedrock_port(game_port, spec.pumpkin_bedrock_port, &manifests)?
        } else {
            0
        };
        let mut reserved_ports = self.amp_occupied_ports();
        reserved_ports.extend([game_port, bedrock_port]);
        let rcon_port = allocate_rcon_port(&manifests, &reserved_ports)?;
        let id = Uuid::new_v4().to_string();
        let instance_name = instance_name(spec.name.trim(), &id);
        let container_name = format!("helix-game-{id}");
        let run_uid = allocate_run_uid(&id, &manifests)?;
        let data_path = self.instance_path(&id)?;
        let manifest_path = self.manifest_path(&id)?;
        let mut container_create_attempted = false;

        let result = (|| -> Result<Value, String> {
            progress(
                if matches!(spec.software, MinecraftSoftware::Custom) {
                    "Validating the selected local server JAR"
                } else {
                    "Resolving a supported Minecraft build"
                },
                12,
            );
            let artifact = if matches!(spec.software, MinecraftSoftware::Custom) {
                self.resolve_custom_artifact(&resolved_spec)?
            } else {
                self.resolve_artifact(resolved_spec.software, resolved_spec.version.trim())?
            };
            if !matches!(artifact.software, MinecraftSoftware::Pumpkin)
                && (artifact.java_version < 17 || artifact.java_version > 25)
            {
                return Err(format!(
                    "Minecraft {} requires Java {}, which this Helix release does not manage yet",
                    artifact.version, artifact.java_version
                ));
            }

            progress("Preparing the isolated server directory", 20);
            fs::create_dir(&data_path)
                .map_err(|_| "could not create the server directory".to_owned())?;
            fs::set_permissions(&data_path, fs::Permissions::from_mode(0o750))
                .map_err(|_| "could not protect the server directory".to_owned())?;

            progress(
                if artifact.local_source.is_some() {
                    "Importing and hashing the local server JAR"
                } else {
                    "Downloading and verifying the server"
                },
                30,
            );
            let pumpkin = matches!(artifact.software, MinecraftSoftware::Pumpkin);
            let jar_path = data_path.join(if pumpkin { "pumpkin" } else { "server.jar" });
            let artifact_sha256 = self.download_artifact(&artifact, &jar_path)?;
            write_new_file(&data_path.join("eula.txt"), b"eula=true\n", 0o640)?;
            let rcon_password = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            let properties = if pumpkin {
                pumpkin::initial_config(
                    &resolved_spec,
                    game_port,
                    bedrock_port,
                    rcon_port,
                    &rcon_password,
                )?
            } else {
                server_properties(&resolved_spec, game_port, rcon_port, &rcon_password)
            };
            write_new_file(
                &data_path.join(if pumpkin {
                    "pumpkin.toml"
                } else {
                    "server.properties"
                }),
                properties.as_bytes(),
                0o640,
            )?;

            progress(
                if pumpkin {
                    "Pinning the native Pumpkin runtime (no Java)"
                } else {
                    "Pinning the Java runtime"
                },
                48,
            );
            let runtime_image = if pumpkin {
                self.pumpkin_runtime()?
            } else {
                self.resolve_runtime_image(artifact.java_version)?
            };
            let unix_args = if artifact.install_server {
                progress("Running the official loader installer", 52);
                Some(self.run_loader_installer(&artifact, &data_path, run_uid, &runtime_image)?)
            } else {
                None
            };
            let manifest = InstanceManifest {
                schema_version: MANIFEST_VERSION,
                id: id.clone(),
                name: spec.name.trim().to_owned(),
                instance_name: instance_name.clone(),
                container_name: container_name.clone(),
                software: artifact.software,
                minecraft_version: artifact.version,
                build: artifact.build,
                java_version: artifact.java_version,
                runtime_image,
                artifact_url: artifact.url,
                artifact_sha256,
                memory_mb: spec.memory_mb,
                cpu_millis: spec.cpu_millis,
                max_players: spec.max_players,
                game_port,
                rcon_port,
                rcon_password: rcon_password.clone(),
                start_on_boot: spec.start_on_boot,
                run_uid,
                created_at_unix_ms: now_unix_ms(),
                kind: GameKind::Minecraft,
                query_port: bedrock_port,
                unix_args,
                backup_keep_count: 0,
                backup_keep_days: 0,
                modpack: None,
            };
            write_manifest(&manifest_path, &manifest)?;
            self.chown_instance(&data_path, run_uid)?;
            self.protect_instance_artifacts(&data_path, run_uid)?;
            if let Some(source) = overlay {
                progress("Copying world, plugins, and configs", 56);
                self.overlay_migrated_game(
                    GameKind::Minecraft,
                    source,
                    &data_path,
                    matches!(spec.software, MinecraftSoftware::Custom),
                    run_uid,
                )?;
                if let Some(source_properties) = migrate_plan::read_source_properties(source) {
                    let properties_path = data_path.join("server.properties");
                    let current = fs::read_to_string(&properties_path).unwrap_or_default();
                    let merged = migrate_plan::merge_server_properties(
                        &current,
                        &source_properties,
                        game_port,
                        rcon_port,
                        &rcon_password,
                        spec.max_players,
                    );
                    fs::write(&properties_path, merged)
                        .map_err(|_| "could not merge server.properties".to_owned())?;
                    self.chown_instance(&data_path, run_uid)?;
                    self.protect_instance_artifacts(&data_path, run_uid)?;
                }
            }

            progress("Creating the Helix workload", 62);
            container_create_attempted = true;
            self.create_validation_container(&manifest, &data_path)?;

            progress("Starting Minecraft", 76);
            self.docker(["start", manifest.container_name.as_str()], 90)?;
            self.wait_until_ready(&manifest, self.ready_timeout(&manifest), |elapsed| {
                let percent = 76_u64.saturating_add((elapsed / 9).min(21));
                progress(
                    "Generating the world and waiting for Minecraft",
                    u8::try_from(percent).unwrap_or(97),
                );
            })?;
            self.finalize_container_restart_policy(&manifest)?;
            self.ensure_console_archiver(&manifest)?;
            progress("Online", 100);
            Ok(json!({
                "instance_id": format!("helix:{id}"),
                "instance_name": instance_name,
                "game_port": game_port,
                "port_allocated_automatically": allocated_automatically,
                "software": spec.software,
                "query_port": if pumpkin { json!(bedrock_port) } else { Value::Null },
                "manager": "helix",
                "execution_backend": "docker"
            }))
        })();

        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                progress("Preserving the failed install for recovery", 98);
                let cleanup_error = self.rollback_creation(
                    &id,
                    &container_name,
                    &data_path,
                    &manifest_path,
                    container_create_attempted,
                );
                Err(match cleanup_error {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; cleanup also failed: {cleanup}"),
                })
            }
        }
    }

    pub fn create_vrising<F>(
        &self,
        spec: &VRisingCreateSpec,
        overlay: Option<&Path>,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        vrising::validate_create_spec(spec)?;
        let _operation = self.begin_creation_operation()?;
        progress("Checking ports, names, and storage", 6);
        let manifests = self.load_manifests()?;
        if manifests
            .iter()
            .any(|manifest| manifest.name.eq_ignore_ascii_case(spec.name.trim()))
        {
            return Err("a Helix server with that name already exists".to_owned());
        }
        let (game_port, query_port, allocated_automatically) =
            self.resolve_vrising_ports(spec.game_port, spec.query_port, &manifests)?;
        let id = Uuid::new_v4().to_string();
        let instance_name = instance_name(spec.name.trim(), &id);
        let container_name = format!("helix-game-{id}");
        let run_uid = allocate_run_uid(&id, &manifests)?;
        let data_path = self.instance_path(&id)?;
        let manifest_path = self.manifest_path(&id)?;
        let mut container_create_attempted = false;

        let result = (|| -> Result<Value, String> {
            progress("Building or reusing the isolated V Rising runtime", 14);
            let runtime_image = self.ensure_vrising_runtime_image(&mut progress)?;

            progress("Preparing the isolated V Rising directory", 28);
            fs::create_dir(&data_path)
                .map_err(|_| "could not create the server directory".to_owned())?;
            fs::set_permissions(&data_path, fs::Permissions::from_mode(0o750))
                .map_err(|_| "could not protect the server directory".to_owned())?;
            for folder in ["server", "save/Settings", "logs", "steamcmd", "wine"] {
                fs::create_dir_all(data_path.join(folder))
                    .map_err(|_| "could not create V Rising data folders".to_owned())?;
            }
            let settings_path = data_path
                .join("save")
                .join("Settings")
                .join("ServerHostSettings.json");
            let settings = serde_json::to_vec_pretty(&vrising::host_settings_json(
                spec.name.trim(),
                game_port,
                query_port,
                spec.max_players,
                spec.list_on_browser,
            ))
            .map_err(|_| "could not encode V Rising host settings".to_owned())?;
            write_new_file(&settings_path, &settings, 0o660)?;

            let manifest = InstanceManifest {
                schema_version: MANIFEST_VERSION,
                kind: GameKind::VRising,
                id: id.clone(),
                name: spec.name.trim().to_owned(),
                instance_name: instance_name.clone(),
                container_name: container_name.clone(),
                software: MinecraftSoftware::Vanilla,
                minecraft_version: "dedicated".to_owned(),
                build: vrising::STEAM_APP_ID.to_owned(),
                java_version: 0,
                runtime_image,
                artifact_url: vrising::ARTIFACT_URL.to_owned(),
                artifact_sha256: vrising::empty_artifact_sha256().to_owned(),
                memory_mb: spec.memory_mb,
                cpu_millis: spec.cpu_millis,
                max_players: spec.max_players,
                game_port,
                query_port,
                rcon_port: 0,
                rcon_password: String::new(),
                start_on_boot: spec.start_on_boot,
                run_uid,
                created_at_unix_ms: now_unix_ms(),
                unix_args: None,
                backup_keep_count: 0,
                backup_keep_days: 0,
                modpack: None,
            };
            write_manifest(&manifest_path, &manifest)?;
            self.chown_instance(&data_path, run_uid)?;
            if let Some(source) = overlay {
                progress("Copying V Rising saves", 48);
                self.overlay_migrated_game(GameKind::VRising, source, &data_path, false, run_uid)?;
                let settings_path = data_path
                    .join("save")
                    .join("Settings")
                    .join("ServerHostSettings.json");
                let source_settings = fs::read_to_string(&settings_path).unwrap_or_default();
                let settings = migrate_plan::merge_vrising_host_settings(
                    vrising::host_settings_json(
                        spec.name.trim(),
                        game_port,
                        query_port,
                        spec.max_players,
                        spec.list_on_browser,
                    ),
                    &source_settings,
                );
                fs::write(
                    &settings_path,
                    serde_json::to_vec_pretty(&settings)
                        .map_err(|_| "could not write V Rising host settings".to_owned())?,
                )
                .map_err(|_| "could not write V Rising host settings".to_owned())?;
                self.chown_instance(&data_path, run_uid)?;
            }

            progress("Creating the isolated V Rising container", 52);
            container_create_attempted = true;
            self.create_validation_container(&manifest, &data_path)?;

            progress(
                "Downloading V Rising through SteamCMD and starting the runtime",
                62,
            );
            self.clear_ready_marker(&manifest)?;
            self.docker(["start", manifest.container_name.as_str()], 90)?;
            self.wait_until_ready(&manifest, Duration::from_secs(45 * 60), |elapsed| {
                let percent = 62_u64.saturating_add((elapsed / 40).min(35));
                progress(
                    "Downloading V Rising and waiting for first boot",
                    u8::try_from(percent).unwrap_or(97),
                );
            })?;
            self.finalize_container_restart_policy(&manifest)?;
            self.ensure_console_archiver(&manifest)?;
            progress("Online", 100);
            Ok(json!({
                "instance_id": format!("helix:{id}"),
                "instance_name": instance_name,
                "game_port": game_port,
                "query_port": query_port,
                "port_allocated_automatically": allocated_automatically,
                "manager": "helix",
                "execution_backend": "docker",
                "kind": "vrising",
                "runtime_image": vrising::RUNTIME_IMAGE
            }))
        })();

        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                progress("Preserving the failed install for recovery", 98);
                let cleanup_error = self.rollback_creation(
                    &id,
                    &container_name,
                    &data_path,
                    &manifest_path,
                    container_create_attempted,
                );
                Err(match cleanup_error {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; cleanup also failed: {cleanup}"),
                })
            }
        }
    }

    pub fn create_valheim<F>(
        &self,
        spec: &ValheimCreateSpec,
        overlay: Option<&Path>,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        valheim::validate_create_spec(spec)?;
        let _operation = self.begin_creation_operation()?;
        progress("Checking ports, names, and storage", 6);
        let manifests = self.load_manifests()?;
        if manifests
            .iter()
            .any(|manifest| manifest.name.eq_ignore_ascii_case(spec.name.trim()))
        {
            return Err("a Helix server with that name already exists".to_owned());
        }
        let (game_port, query_port, allocated_automatically) =
            self.resolve_valheim_ports(spec.game_port, &manifests)?;
        let id = Uuid::new_v4().to_string();
        let instance_name = instance_name(spec.name.trim(), &id);
        let container_name = format!("helix-game-{id}");
        let run_uid = allocate_run_uid(&id, &manifests)?;
        let data_path = self.instance_path(&id)?;
        let manifest_path = self.manifest_path(&id)?;
        let mut container_create_attempted = false;

        let result = (|| -> Result<Value, String> {
            progress("Building or reusing the isolated Valheim runtime", 14);
            let runtime_image = self.ensure_valheim_runtime_image(&mut progress)?;

            progress("Preparing the isolated Valheim directory", 28);
            fs::create_dir(&data_path)
                .map_err(|_| "could not create the server directory".to_owned())?;
            fs::set_permissions(&data_path, fs::Permissions::from_mode(0o750))
                .map_err(|_| "could not protect the server directory".to_owned())?;
            for folder in ["server", "logs", "steamcmd", "plugins", "worlds"] {
                fs::create_dir_all(data_path.join(folder))
                    .map_err(|_| "could not create Valheim data folders".to_owned())?;
            }

            let manifest = InstanceManifest {
                schema_version: MANIFEST_VERSION,
                kind: GameKind::Valheim,
                id: id.clone(),
                name: spec.name.trim().to_owned(),
                instance_name: instance_name.clone(),
                container_name: container_name.clone(),
                software: MinecraftSoftware::Vanilla,
                minecraft_version: "dedicated".to_owned(),
                build: valheim::STEAM_APP_ID.to_owned(),
                java_version: 0,
                runtime_image,
                artifact_url: valheim::ARTIFACT_URL.to_owned(),
                artifact_sha256: valheim::empty_artifact_sha256().to_owned(),
                memory_mb: spec.memory_mb,
                cpu_millis: spec.cpu_millis,
                max_players: spec.max_players,
                game_port,
                query_port,
                rcon_port: 0,
                rcon_password: String::new(),
                start_on_boot: spec.start_on_boot,
                run_uid,
                created_at_unix_ms: now_unix_ms(),
                unix_args: None,
                backup_keep_count: 0,
                backup_keep_days: 0,
                modpack: None,
            };
            write_manifest(&manifest_path, &manifest)?;
            self.chown_instance(&data_path, run_uid)?;
            if let Some(source) = overlay {
                progress("Copying Valheim worlds", 48);
                self.overlay_migrated_game(GameKind::Valheim, source, &data_path, false, run_uid)?;
            }

            progress("Creating the isolated Valheim container", 52);
            container_create_attempted = true;
            self.create_validation_container(&manifest, &data_path)?;

            progress(
                "Downloading Valheim through SteamCMD and starting the runtime",
                62,
            );
            self.clear_ready_marker(&manifest)?;
            self.docker(["start", manifest.container_name.as_str()], 90)?;
            self.wait_until_ready(&manifest, Duration::from_secs(45 * 60), |elapsed| {
                let percent = 62_u64.saturating_add((elapsed / 40).min(35));
                progress(
                    "Downloading Valheim and waiting for first boot",
                    u8::try_from(percent).unwrap_or(97),
                );
            })?;
            self.finalize_container_restart_policy(&manifest)?;
            self.ensure_console_archiver(&manifest)?;
            progress("Online", 100);
            Ok(json!({
                "instance_id": format!("helix:{id}"),
                "instance_name": instance_name,
                "game_port": game_port,
                "query_port": query_port,
                "steam_port": game_port.saturating_add(2),
                "port_allocated_automatically": allocated_automatically,
                "manager": "helix",
                "execution_backend": "docker",
                "kind": "valheim",
                "runtime_image": valheim::RUNTIME_IMAGE
            }))
        })();

        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                progress("Preserving the failed install for recovery", 98);
                let cleanup_error = self.rollback_creation(
                    &id,
                    &container_name,
                    &data_path,
                    &manifest_path,
                    container_create_attempted,
                );
                Err(match cleanup_error {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; cleanup also failed: {cleanup}"),
                })
            }
        }
    }

    pub fn create_terraria<F>(
        &self,
        spec: &TerrariaCreateSpec,
        overlay: Option<&Path>,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        terraria::validate_create_spec(spec)?;
        let _operation = self.begin_creation_operation()?;
        progress("Checking ports, names, and storage", 6);
        let manifests = self.load_manifests()?;
        if manifests
            .iter()
            .any(|manifest| manifest.name.eq_ignore_ascii_case(spec.name.trim()))
        {
            return Err("a Helix server with that name already exists".to_owned());
        }
        let (game_port, allocated_automatically) =
            self.resolve_game_port(GameKind::Terraria, spec.game_port, &manifests)?;
        let id = Uuid::new_v4().to_string();
        let instance_name = instance_name(spec.name.trim(), &id);
        let container_name = format!("helix-game-{id}");
        let run_uid = allocate_run_uid(&id, &manifests)?;
        let data_path = self.instance_path(&id)?;
        let manifest_path = self.manifest_path(&id)?;
        let mut container_create_attempted = false;
        let (software_label, artifact_url, version, build) = match spec.software {
            TerrariaSoftware::Vanilla => (
                "vanilla",
                terraria::VANILLA_ARTIFACT_URL,
                terraria::VANILLA_VERSION,
                terraria::VANILLA_VERSION,
            ),
            TerrariaSoftware::Tmodloader => (
                "tmodloader",
                terraria::TMOD_ARTIFACT_URL,
                "dedicated",
                "1281930",
            ),
        };

        let result = (|| -> Result<Value, String> {
            progress("Building or reusing the isolated Terraria runtime", 14);
            let runtime_image = self.ensure_terraria_runtime_image(&mut progress)?;

            progress("Preparing the isolated Terraria directory", 28);
            fs::create_dir(&data_path)
                .map_err(|_| "could not create the server directory".to_owned())?;
            fs::set_permissions(&data_path, fs::Permissions::from_mode(0o750))
                .map_err(|_| "could not protect the server directory".to_owned())?;
            for folder in ["server", "logs", "steamcmd", "worlds", "mods"] {
                fs::create_dir_all(data_path.join(folder))
                    .map_err(|_| "could not create Terraria data folders".to_owned())?;
            }

            let manifest = InstanceManifest {
                schema_version: MANIFEST_VERSION,
                kind: GameKind::Terraria,
                id: id.clone(),
                name: spec.name.trim().to_owned(),
                instance_name: instance_name.clone(),
                container_name: container_name.clone(),
                software: MinecraftSoftware::Vanilla,
                minecraft_version: version.to_owned(),
                build: format!("{software_label}-{build}"),
                java_version: 0,
                runtime_image,
                artifact_url: artifact_url.to_owned(),
                artifact_sha256: terraria::empty_artifact_sha256().to_owned(),
                memory_mb: spec.memory_mb,
                cpu_millis: spec.cpu_millis,
                max_players: spec.max_players,
                game_port,
                query_port: 0,
                rcon_port: 0,
                rcon_password: String::new(),
                start_on_boot: spec.start_on_boot,
                run_uid,
                created_at_unix_ms: now_unix_ms(),
                unix_args: None,
                backup_keep_count: 0,
                backup_keep_days: 0,
                modpack: None,
            };
            write_manifest(&manifest_path, &manifest)?;
            self.chown_instance(&data_path, run_uid)?;
            if let Some(source) = overlay {
                progress("Copying Terraria worlds and mods", 48);
                self.overlay_migrated_game(GameKind::Terraria, source, &data_path, false, run_uid)?;
            }

            progress("Creating the isolated Terraria container", 52);
            container_create_attempted = true;
            self.create_validation_container(&manifest, &data_path)?;

            progress("Downloading Terraria and starting the runtime", 62);
            self.clear_ready_marker(&manifest)?;
            self.docker(["start", manifest.container_name.as_str()], 90)?;
            self.wait_until_ready(&manifest, Duration::from_secs(30 * 60), |elapsed| {
                let percent = 62_u64.saturating_add((elapsed / 40).min(35));
                progress(
                    "Downloading Terraria and waiting for first boot",
                    u8::try_from(percent).unwrap_or(97),
                );
            })?;
            self.finalize_container_restart_policy(&manifest)?;
            self.ensure_console_archiver(&manifest)?;
            progress("Online", 100);
            Ok(json!({
                "instance_id": format!("helix:{id}"),
                "instance_name": instance_name,
                "game_port": game_port,
                "port_allocated_automatically": allocated_automatically,
                "manager": "helix",
                "execution_backend": "docker",
                "kind": "terraria",
                "software": software_label,
                "runtime_image": terraria::RUNTIME_IMAGE
            }))
        })();

        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                progress("Preserving the failed install for recovery", 98);
                let cleanup_error = self.rollback_creation(
                    &id,
                    &container_name,
                    &data_path,
                    &manifest_path,
                    container_create_attempted,
                );
                Err(match cleanup_error {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; cleanup also failed: {cleanup}"),
                })
            }
        }
    }

    pub fn set_start_on_boot(&self, id: &str, enabled: bool) -> Result<Value, String> {
        let mut manifest = self.load_manifest(native_id(id))?;
        let _operation = self.begin_instance_operation(&manifest.id, "start-on-boot")?;
        let desired = if enabled { "unless-stopped" } else { "no" };
        let container_present = self
            .exact_container_identity(&manifest.container_name)?
            .is_some();
        let previous = manifest.start_on_boot;
        if container_present {
            let original = self.container_restart_policy(&manifest.container_name)?;
            if original != desired {
                if let Err(error) = self.docker(
                    [
                        "update",
                        "--restart",
                        desired,
                        manifest.container_name.as_str(),
                    ],
                    20,
                ) {
                    return Err(format!("start-on-boot was not changed: {error}"));
                }
                let verified = self.container_restart_policy(&manifest.container_name)?;
                if verified != desired {
                    let _ = self.docker(
                        [
                            "update",
                            "--restart",
                            original.as_str(),
                            manifest.container_name.as_str(),
                        ],
                        20,
                    );
                    return Err("Docker did not persist the requested restart policy".to_owned());
                }
            }
        }
        manifest.start_on_boot = enabled;
        if let Err(error) = write_manifest(&self.manifest_path(&manifest.id)?, &manifest) {
            if container_present && previous != enabled {
                let rollback = if previous { "unless-stopped" } else { "no" };
                let _ = self.docker(
                    [
                        "update",
                        "--restart",
                        rollback,
                        manifest.container_name.as_str(),
                    ],
                    20,
                );
            }
            return Err(error);
        }
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "enabled": enabled,
            "policy": desired,
            "container_updated": container_present,
            "current_runtime_changed": false,
            "persisted": true
        }))
    }

    pub fn set_memory_mb(&self, id: &str, memory_mb: u32) -> Result<Value, String> {
        let mut manifest = self.load_manifest(native_id(id))?;
        validate_allocated_memory(manifest.kind, memory_mb)?;
        if manifest.memory_mb == memory_mb {
            return Ok(json!({
                "instance_id": format!("helix:{}", manifest.id),
                "changed": false,
                "memory_mb": memory_mb,
                "container_republished": false
            }));
        }
        let _operation = self.begin_instance_operation(&manifest.id, "memory update")?;
        let previous = manifest.clone();
        let data_path = self.instance_path(&manifest.id)?;
        let running = self.container_running(&manifest.container_name);
        manifest.memory_mb = memory_mb;
        write_manifest(&self.manifest_path(&manifest.id)?, &manifest)?;
        if let Err(error) = self.republish_minecraft_container(&manifest, &data_path, running) {
            let _ = write_manifest(&self.manifest_path(&previous.id)?, &previous);
            let _ = self.republish_minecraft_container(&previous, &data_path, running);
            return Err(error);
        }
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "changed": true,
            "memory_mb": memory_mb,
            "container_republished": true,
            "was_running": running
        }))
    }

    pub fn set_cpu_millis(&self, id: &str, cpu_millis: u32) -> Result<Value, String> {
        helix_privd::validate_cpu_millis(cpu_millis)?;
        let mut manifest = self.load_manifest(native_id(id))?;
        if manifest.cpu_millis == cpu_millis {
            return Ok(json!({
                "instance_id": format!("helix:{}", manifest.id),
                "changed": false,
                "cpu_millis": cpu_millis,
                "container_republished": false
            }));
        }
        let _operation = self.begin_instance_operation(&manifest.id, "cpu update")?;
        let previous = manifest.clone();
        let data_path = self.instance_path(&manifest.id)?;
        let running = self.container_running(&manifest.container_name);
        manifest.cpu_millis = cpu_millis;
        write_manifest(&self.manifest_path(&manifest.id)?, &manifest)?;
        if let Err(error) = self.republish_minecraft_container(&manifest, &data_path, running) {
            let _ = write_manifest(&self.manifest_path(&previous.id)?, &previous);
            let _ = self.republish_minecraft_container(&previous, &data_path, running);
            return Err(error);
        }
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "changed": true,
            "cpu_millis": cpu_millis,
            "container_republished": true,
            "was_running": running
        }))
    }

    pub fn set_vrising_browser_listing(
        &self,
        id: &str,
        list_on_browser: bool,
    ) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(id))?;
        if !manifest.is_vrising() {
            return Err("only V Rising servers have an in-game server list".to_owned());
        }
        let _operation = self.begin_instance_operation(&manifest.id, "listing update")?;
        let data_path = self.instance_path(&manifest.id)?;
        let path = vrising_host_settings_path(&data_path);
        let mut settings = if path.is_file() {
            serde_json::from_slice::<Value>(
                &fs::read(&path).map_err(|_| "could not read V Rising host settings".to_owned())?,
            )
            .map_err(|_| "V Rising host settings are invalid".to_owned())?
        } else {
            vrising::host_settings_json(
                &manifest.name,
                manifest.game_port,
                manifest.query_port,
                manifest.max_players,
                list_on_browser,
            )
        };
        let Some(object) = settings.as_object_mut() else {
            return Err("V Rising host settings are invalid".to_owned());
        };
        object.insert("ListOnSteam".to_owned(), json!(list_on_browser));
        object.insert("ListOnEOS".to_owned(), json!(list_on_browser));
        object.insert("HideIPAddress".to_owned(), json!(list_on_browser));
        let encoded = serde_json::to_string_pretty(&settings)
            .map_err(|_| "could not encode V Rising host settings".to_owned())?;
        write_private_text(&path, &format!("{encoded}\n"))?;
        let running = self.container_running(&manifest.container_name);
        Ok(json!({
            "instance_id": format!("helix:{}", manifest.id),
            "list_on_browser": list_on_browser,
            "list_on_eos": list_on_browser,
            "list_on_steam": list_on_browser,
            "hide_ip_address": list_on_browser,
            "restart_required": running
        }))
    }

    fn require_minecraft_content(&self, manifest: &InstanceManifest) -> Result<(), String> {
        if !manifest.is_minecraft() {
            return Err(format!(
                "{} does not have a Modrinth marketplace in this Helix release",
                display_software(manifest)
            ));
        }
        Ok(())
    }

    fn resolve_artifact(
        &self,
        software: MinecraftSoftware,
        requested_version: &str,
    ) -> Result<Artifact, String> {
        match software {
            MinecraftSoftware::Pumpkin => self.resolve_pumpkin(requested_version),
            MinecraftSoftware::Custom => {
                Err("custom JAR servers use the local artifact import flow".to_owned())
            }
            MinecraftSoftware::Paper => self.resolve_paper(requested_version),
            MinecraftSoftware::Purpur => self.resolve_purpur(requested_version),
            MinecraftSoftware::Folia => self.resolve_folia(requested_version),
            MinecraftSoftware::Leaves => self.resolve_leaves(requested_version),
            MinecraftSoftware::Vanilla => self.resolve_vanilla(requested_version),
            MinecraftSoftware::Fabric => self.resolve_fabric(requested_version),
            MinecraftSoftware::Quilt => self.resolve_quilt(requested_version),
            MinecraftSoftware::Forge => self.resolve_forge(requested_version),
            MinecraftSoftware::NeoForge => self.resolve_neoforge(requested_version),
            MinecraftSoftware::Pufferfish => self.resolve_pufferfish(requested_version),
        }
    }

    fn resolve_custom_artifact(&self, spec: &MinecraftCreateSpec) -> Result<Artifact, String> {
        let custom = spec
            .custom_jar
            .as_ref()
            .ok_or_else(|| "choose a local server JAR and Java runtime".to_owned())?;
        if !matches!(custom.java_version, 17 | 21 | 25) {
            return Err("custom JAR servers support Java 17, 21, or 25".to_owned());
        }
        validate_version(spec.version.trim())?;
        let source = PathBuf::from(custom.source_path.trim());
        if !source.is_absolute()
            || !source
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("jar"))
        {
            return Err("the custom server source must be an absolute .jar path".to_owned());
        }
        let canonical = fs::canonicalize(&source)
            .map_err(|_| "the custom server JAR is unavailable".to_owned())?;
        if !self
            .custom_artifact_roots
            .iter()
            .any(|root| canonical.starts_with(root))
        {
            return Err(
                "the custom server JAR must be inside a Storage root managed by Helix".to_owned(),
            );
        }
        let metadata = fs::symlink_metadata(&source)
            .map_err(|_| "the custom server JAR is unavailable".to_owned())?;
        if !metadata.file_type().is_file()
            || metadata.len() < 16 * 1024
            || metadata.len() > MAX_SERVER_JAR_BYTES
        {
            return Err(
                "the custom server JAR is not a regular file within the supported size range"
                    .to_owned(),
            );
        }
        Ok(Artifact {
            software: MinecraftSoftware::Custom,
            version: spec.version.trim().to_owned(),
            build: "local-import".to_owned(),
            java_version: custom.java_version,
            url: "local-import".to_owned(),
            local_source: Some(canonical),
            expected_hash: None,
            install_server: false,
        })
    }

    fn resolve_paper(&self, requested_version: &str) -> Result<Artifact, String> {
        self.resolve_paper_project(MinecraftSoftware::Paper, "paper", requested_version)
    }

    fn resolve_folia(&self, requested_version: &str) -> Result<Artifact, String> {
        self.resolve_paper_project(MinecraftSoftware::Folia, "folia", requested_version)
    }

    fn resolve_paper_project(
        &self,
        software: MinecraftSoftware,
        project: &str,
        requested_version: &str,
    ) -> Result<Artifact, String> {
        let (version, build) = if requested_version.eq_ignore_ascii_case("latest") {
            self.latest_stable_fill_build(software, project)?
        } else {
            validate_version(requested_version)?;
            let version = requested_version.to_owned();
            let builds = self.fetch_json(
                &format!("https://fill.papermc.io/v3/projects/{project}/versions/{version}/builds"),
                &["fill.papermc.io"],
            )?;
            let build = stable_fill_build(&builds).ok_or_else(|| {
                format!(
                    "{} has no stable build for Minecraft {version}",
                    software_name(software)
                )
            })?;
            (version, build)
        };
        let version_url =
            format!("https://fill.papermc.io/v3/projects/{project}/versions/{version}");
        let version_data = self.fetch_json(&version_url, &["fill.papermc.io"])?;
        let java_version = version_data
            .pointer("/version/java/version/minimum")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| {
                format!(
                    "{} did not report its required Java version",
                    software_name(software)
                )
            })?;
        let download = build.pointer("/downloads/server:default").ok_or_else(|| {
            format!(
                "{} returned no runnable server download",
                software_name(software)
            )
        })?;
        let url = required_json_text(download, "url", 4096)?;
        require_https_host(&url, &["fill-data.papermc.io"])?;
        let sha256 = download
            .pointer("/checksums/sha256")
            .and_then(Value::as_str)
            .filter(|hash| valid_hex(hash, 64))
            .ok_or_else(|| {
                format!(
                    "{} returned an invalid server checksum",
                    software_name(software)
                )
            })?
            .to_owned();
        let build = build
            .get("id")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
            .ok_or_else(|| "Paper returned an invalid build number".to_owned())?;
        Ok(Artifact {
            software,
            version,
            build,
            java_version,
            url,
            local_source: None,
            expected_hash: Some(ExpectedHash {
                algorithm: HashAlgorithm::Sha256,
                value: sha256,
            }),
            install_server: false,
        })
    }

    fn latest_stable_fill_build(
        &self,
        software: MinecraftSoftware,
        project: &str,
    ) -> Result<(String, Value), String> {
        let project_data = self.fetch_json(
            &format!("https://fill.papermc.io/v3/projects/{project}"),
            &["fill.papermc.io"],
        )?;
        let versions = fill_project_versions(&project_data);
        if versions.is_empty() {
            return Err(format!(
                "{} did not report any supported Minecraft versions",
                software_name(software)
            ));
        }
        for version in versions.into_iter().take(16) {
            let builds = self.fetch_json(
                &format!("https://fill.papermc.io/v3/projects/{project}/versions/{version}/builds"),
                &["fill.papermc.io"],
            )?;
            if let Some(build) = stable_fill_build(&builds) {
                return Ok((version, build));
            }
        }
        Err(format!(
            "{} did not report a stable build in its 16 newest Minecraft versions",
            software_name(software)
        ))
    }

    fn resolve_fabric(&self, requested_version: &str) -> Result<Artifact, String> {
        let (version, version_data) = self.minecraft_version_metadata(requested_version)?;
        let loaders = self.fetch_json(
            &format!("https://meta.fabricmc.net/v2/versions/loader/{version}"),
            &["meta.fabricmc.net"],
        )?;
        let loader = select_stable_fabric_loader(&loaders)?;
        let installers = self.fetch_json(
            "https://meta.fabricmc.net/v2/versions/installer",
            &["meta.fabricmc.net"],
        )?;
        let installer = select_stable_fabric_installer(&installers)?;
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(&version));
        let url = format!(
            "https://meta.fabricmc.net/v2/versions/loader/{version}/{loader}/{installer}/server/jar"
        );
        require_https_host(&url, &["meta.fabricmc.net"])?;
        Ok(Artifact {
            software: MinecraftSoftware::Fabric,
            version,
            build: format!("loader-{loader}_installer-{installer}"),
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: false,
        })
    }

    fn resolve_pinned_fabric(
        &self,
        minecraft_version: &str,
        loader_version: &str,
    ) -> Result<Artifact, String> {
        validate_version(minecraft_version)?;
        validate_version(loader_version)?;
        let (_, version_data) = self.minecraft_version_metadata(minecraft_version)?;
        let loaders = self.fetch_json(
            &format!(
                "https://meta.fabricmc.net/v2/versions/loader/{minecraft_version}/{loader_version}"
            ),
            &["meta.fabricmc.net"],
        )?;
        let loader_is_stable = loaders.as_array().is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry.pointer("/loader/version").and_then(Value::as_str) == Some(loader_version)
                    && entry.pointer("/loader/stable").and_then(Value::as_bool) == Some(true)
            })
        });
        if !loader_is_stable {
            return Err(format!(
                "Fabric Loader {loader_version} is not a stable server loader for Minecraft {minecraft_version}"
            ));
        }
        let installers = self.fetch_json(
            "https://meta.fabricmc.net/v2/versions/installer",
            &["meta.fabricmc.net"],
        )?;
        let installer = select_stable_fabric_installer(&installers)?;
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(minecraft_version));
        let url = format!(
            "https://meta.fabricmc.net/v2/versions/loader/{minecraft_version}/{loader_version}/{installer}/server/jar"
        );
        require_https_host(&url, &["meta.fabricmc.net"])?;
        Ok(Artifact {
            software: MinecraftSoftware::Fabric,
            version: minecraft_version.to_owned(),
            build: format!("loader-{loader_version}-installer-{installer}"),
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: false,
        })
    }

    fn quilt_published_versions(&self) -> Result<Vec<String>, String> {
        let games = self.fetch_json(
            "https://meta.quiltmc.org/v3/versions/game",
            &["meta.quiltmc.org"],
        )?;
        Ok(bounded_version_list(fabric_stable_game_versions(&games)))
    }

    fn resolve_quilt(&self, requested_version: &str) -> Result<Artifact, String> {
        let versions = self.quilt_published_versions()?;
        let version = resolve_requested_version(requested_version, &versions)?;
        let (_, version_data) = self.minecraft_version_metadata(&version)?;
        let loaders = self.fetch_json(
            &format!("https://meta.quiltmc.org/v3/versions/loader/{version}"),
            &["meta.quiltmc.org"],
        )?;
        let loader = select_stable_quilt_loader(&loaders)?;
        let installers = self.fetch_json(
            "https://meta.quiltmc.org/v3/versions/installer",
            &["meta.quiltmc.org"],
        )?;
        let installer = select_stable_quilt_installer(&installers)?;
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(&version));
        let url = format!(
            "https://meta.quiltmc.org/v3/versions/loader/{version}/{loader}/{installer}/server/jar"
        );
        require_https_host(&url, &["meta.quiltmc.org"])?;
        Ok(Artifact {
            software: MinecraftSoftware::Quilt,
            version,
            build: format!("loader-{loader}_installer-{installer}"),
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: false,
        })
    }

    fn forge_published_versions(&self) -> Result<Vec<String>, String> {
        let promotions = self.fetch_json(
            "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json",
            &["files.minecraftforge.net"],
        )?;
        let mut versions = forge_promo_versions(&promotions);
        versions.retain(|version| minecraft_at_least(version, 17));
        Ok(bounded_version_list(versions))
    }

    fn resolve_forge(&self, requested_version: &str) -> Result<Artifact, String> {
        let versions = self.forge_published_versions()?;
        let version = resolve_requested_version(requested_version, &versions)?;
        if !minecraft_at_least(&version, 17) {
            return Err(
                "Helix installs Forge through the official installer for Minecraft 1.17 and newer"
                    .to_owned(),
            );
        }
        let promotions = self.fetch_json(
            "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json",
            &["files.minecraftforge.net"],
        )?;
        let forge_build = forge_promo_build(&promotions, &version)?;
        let (_, version_data) = self.minecraft_version_metadata(&version)?;
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(&version));
        let url = format!(
            "https://maven.minecraftforge.net/net/minecraftforge/forge/{version}-{forge_build}/forge-{version}-{forge_build}-installer.jar"
        );
        require_https_host(&url, &["maven.minecraftforge.net"])?;
        Ok(Artifact {
            software: MinecraftSoftware::Forge,
            version,
            build: forge_build,
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: true,
        })
    }

    fn neoforge_published_versions(&self) -> Result<Vec<String>, String> {
        let catalog = self.fetch_json(
            "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge",
            &["maven.neoforged.net"],
        )?;
        Ok(bounded_version_list(neoforge_game_versions(&catalog)))
    }

    fn resolve_neoforge(&self, requested_version: &str) -> Result<Artifact, String> {
        let versions = self.neoforge_published_versions()?;
        let version = resolve_requested_version(requested_version, &versions)?;
        let catalog = self.fetch_json(
            "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge",
            &["maven.neoforged.net"],
        )?;
        let loader = neoforge_loader_for_game(&catalog, &version)?;
        let (_, version_data) = self.minecraft_version_metadata(&version)?;
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(&version));
        let url = format!(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/{loader}/neoforge-{loader}-installer.jar"
        );
        require_https_host(&url, &["maven.neoforged.net"])?;
        Ok(Artifact {
            software: MinecraftSoftware::NeoForge,
            version,
            build: loader,
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: true,
        })
    }

    fn pufferfish_published_versions(&self) -> Result<Vec<String>, String> {
        let mut versions = Vec::new();
        for family in ["1.21", "1.20", "1.19", "1.18"] {
            if let Ok(job_versions) = self.pufferfish_family_versions(family) {
                versions.extend(job_versions);
            }
        }
        versions.sort_by(|left, right| compare_minecraft_versions(right, left));
        versions.dedup();
        Ok(bounded_version_list(versions))
    }

    fn pufferfish_family_versions(&self, family: &str) -> Result<Vec<String>, String> {
        let job = self.fetch_json(
            &format!(
                "https://ci.pufferfish.host/job/Pufferfish-{family}/lastSuccessfulBuild/api/json?tree=number,artifacts[fileName,relativePath]"
            ),
            &["ci.pufferfish.host"],
        )?;
        Ok(pufferfish_versions_from_job(&job))
    }

    fn resolve_pufferfish(&self, requested_version: &str) -> Result<Artifact, String> {
        let versions = self.pufferfish_published_versions()?;
        let version = resolve_requested_version(requested_version, &versions)?;
        let family = pufferfish_family(&version)?;
        let job = self.fetch_json(
            &format!(
                "https://ci.pufferfish.host/job/Pufferfish-{family}/lastSuccessfulBuild/api/json?tree=number,artifacts[fileName,relativePath]"
            ),
            &["ci.pufferfish.host"],
        )?;
        let (url, build) = pufferfish_artifact_url(&job, &version)?;
        require_https_host(&url, &["ci.pufferfish.host"])?;
        Ok(Artifact {
            software: MinecraftSoftware::Pufferfish,
            version: version.clone(),
            build,
            java_version: fallback_java_version(&version),
            url,
            local_source: None,
            expected_hash: None,
            install_server: false,
        })
    }

    fn resolve_pinned_quilt(
        &self,
        minecraft_version: &str,
        loader_version: &str,
    ) -> Result<Artifact, String> {
        validate_version(minecraft_version)?;
        validate_version(loader_version)?;
        let (_, version_data) = self.minecraft_version_metadata(minecraft_version)?;
        let installers = self.fetch_json(
            "https://meta.quiltmc.org/v3/versions/installer",
            &["meta.quiltmc.org"],
        )?;
        let installer = select_stable_quilt_installer(&installers)?;
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(minecraft_version));
        let url = format!(
            "https://meta.quiltmc.org/v3/versions/loader/{minecraft_version}/{loader_version}/{installer}/server/jar"
        );
        require_https_host(&url, &["meta.quiltmc.org"])?;
        Ok(Artifact {
            software: MinecraftSoftware::Quilt,
            version: minecraft_version.to_owned(),
            build: format!("loader-{loader_version}-installer-{installer}"),
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: false,
        })
    }

    fn resolve_pinned_forge(
        &self,
        minecraft_version: &str,
        forge_build: &str,
    ) -> Result<Artifact, String> {
        validate_version(minecraft_version)?;
        validate_version(forge_build)?;
        if !minecraft_at_least(minecraft_version, 17) {
            return Err(
                "Helix installs Forge through the official installer for Minecraft 1.17 and newer"
                    .to_owned(),
            );
        }
        let (_, version_data) = self.minecraft_version_metadata(minecraft_version)?;
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(minecraft_version));
        let url = format!(
            "https://maven.minecraftforge.net/net/minecraftforge/forge/{minecraft_version}-{forge_build}/forge-{minecraft_version}-{forge_build}-installer.jar"
        );
        require_https_host(&url, &["maven.minecraftforge.net"])?;
        Ok(Artifact {
            software: MinecraftSoftware::Forge,
            version: minecraft_version.to_owned(),
            build: forge_build.to_owned(),
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: true,
        })
    }

    fn resolve_pinned_neoforge(
        &self,
        minecraft_version: &str,
        loader: &str,
    ) -> Result<Artifact, String> {
        validate_version(minecraft_version)?;
        validate_version(loader)?;
        let (_, version_data) = self.minecraft_version_metadata(minecraft_version)?;
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(minecraft_version));
        let url = format!(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/{loader}/neoforge-{loader}-installer.jar"
        );
        require_https_host(&url, &["maven.neoforged.net"])?;
        Ok(Artifact {
            software: MinecraftSoftware::NeoForge,
            version: minecraft_version.to_owned(),
            build: loader.to_owned(),
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: true,
        })
    }

    fn resolve_purpur(&self, requested_version: &str) -> Result<Artifact, String> {
        let project =
            self.fetch_json("https://api.purpurmc.org/v2/purpur", &["api.purpurmc.org"])?;
        let version = if requested_version.eq_ignore_ascii_case("latest") {
            project
                .pointer("/metadata/current")
                .and_then(Value::as_str)
                .filter(|version| validate_version(version).is_ok())
                .ok_or_else(|| "Purpur did not report a current release".to_owned())?
                .to_owned()
        } else {
            validate_version(requested_version)?;
            requested_version.to_owned()
        };
        if !project
            .get("versions")
            .and_then(Value::as_array)
            .is_some_and(|versions| versions.iter().any(|item| item.as_str() == Some(&version)))
        {
            return Err(format!("Purpur does not publish Minecraft {version}"));
        }
        let version_data = self.fetch_json(
            &format!("https://api.purpurmc.org/v2/purpur/{version}"),
            &["api.purpurmc.org"],
        )?;
        let build = version_data
            .pointer("/builds/latest")
            .and_then(Value::as_str)
            .filter(|build| build.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or_else(|| "Purpur did not report a current build".to_owned())?
            .to_owned();
        let java_version = self
            .paper_java_version(&version)
            .unwrap_or_else(|_| fallback_java_version(&version));
        let url = format!("https://api.purpurmc.org/v2/purpur/{version}/{build}/download");
        Ok(Artifact {
            software: MinecraftSoftware::Purpur,
            version,
            build,
            java_version,
            url,
            local_source: None,
            expected_hash: None,
            install_server: false,
        })
    }

    fn resolve_leaves(&self, requested_version: &str) -> Result<Artifact, String> {
        let versions = self.leaves_published_versions()?;
        let (version, build) = if requested_version.eq_ignore_ascii_case("latest") {
            self.latest_leaves_release(&versions)?
        } else {
            validate_version(requested_version)?;
            if !versions.iter().any(|item| item == requested_version) {
                return Err(format!(
                    "Leaves does not publish Minecraft {requested_version}"
                ));
            }
            let build = self.leaves_default_build(requested_version)?;
            (requested_version.to_owned(), build)
        };
        let download = build
            .pointer("/downloads/application")
            .ok_or_else(|| "Leaves returned no runnable server download".to_owned())?;
        let sha256 = download
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|hash| valid_hex(hash, 64))
            .ok_or_else(|| "Leaves returned an invalid server checksum".to_owned())?
            .to_owned();
        let filename = required_json_text(download, "name", 180)?;
        if filename.contains(['/', '\\'])
            || !filename.to_ascii_lowercase().ends_with(".jar")
            || filename.starts_with('.')
        {
            return Err("Leaves returned an unsafe download name".to_owned());
        }
        let build_id = build
            .get("build")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Leaves returned an invalid build number".to_owned())?;
        let java_version = self
            .paper_java_version(&version)
            .unwrap_or_else(|_| fallback_java_version(&version));
        let url = format!(
            "https://api.leavesmc.org/v2/projects/leaves/versions/{version}/builds/{build_id}/downloads/{filename}"
        );
        require_https_host(&url, &["api.leavesmc.org"])?;
        Ok(Artifact {
            software: MinecraftSoftware::Leaves,
            version,
            build: build_id.to_string(),
            java_version,
            url,
            local_source: None,
            expected_hash: Some(ExpectedHash {
                algorithm: HashAlgorithm::Sha256,
                value: sha256,
            }),
            install_server: false,
        })
    }

    fn resolve_vanilla(&self, requested_version: &str) -> Result<Artifact, String> {
        let (version, version_data) = self.minecraft_version_metadata(requested_version)?;
        let download = version_data
            .pointer("/downloads/server")
            .ok_or_else(|| format!("Minecraft {version} has no dedicated server download"))?;
        let url = required_json_text(download, "url", 4096)?;
        require_https_host(&url, &["piston-data.mojang.com", "launcher.mojang.com"])?;
        let sha1 = required_json_text(download, "sha1", 64)?;
        if !valid_hex(&sha1, 40) {
            return Err("Mojang returned an invalid server checksum".to_owned());
        }
        let java_version = version_data
            .pointer("/javaVersion/majorVersion")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or_else(|| fallback_java_version(&version));
        Ok(Artifact {
            software: MinecraftSoftware::Vanilla,
            version: version.clone(),
            build: version,
            java_version,
            url,
            local_source: None,
            expected_hash: Some(ExpectedHash {
                algorithm: HashAlgorithm::Sha1,
                value: sha1,
            }),
            install_server: false,
        })
    }

    fn minecraft_version_metadata(
        &self,
        requested_version: &str,
    ) -> Result<(String, Value), String> {
        let manifest = self.fetch_json(
            "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
            &["piston-meta.mojang.com"],
        )?;
        let version = if requested_version.eq_ignore_ascii_case("latest") {
            manifest
                .pointer("/latest/release")
                .and_then(Value::as_str)
                .filter(|version| validate_version(version).is_ok())
                .ok_or_else(|| "Mojang did not report a current release".to_owned())?
                .to_owned()
        } else {
            validate_version(requested_version)?;
            requested_version.to_owned()
        };
        let entry = manifest
            .get("versions")
            .and_then(Value::as_array)
            .and_then(|versions| {
                versions
                    .iter()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == Some(&version))
            })
            .ok_or_else(|| format!("Mojang does not publish Minecraft {version}"))?;
        let metadata_url = required_json_text(entry, "url", 4096)?;
        require_https_host(&metadata_url, &["piston-meta.mojang.com"])?;
        let version_data = self.fetch_json(&metadata_url, &["piston-meta.mojang.com"])?;
        Ok((version, version_data))
    }

    fn paper_java_version(&self, version: &str) -> Result<u16, String> {
        let data = self.fetch_json(
            &format!("https://fill.papermc.io/v3/projects/paper/versions/{version}"),
            &["fill.papermc.io"],
        )?;
        data.pointer("/version/java/version/minimum")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| "Paper did not report its required Java version".to_owned())
    }

    fn fill_published_versions(&self, project: &str) -> Result<Vec<String>, String> {
        let project_data = self.fetch_json(
            &format!("https://fill.papermc.io/v3/projects/{project}"),
            &["fill.papermc.io"],
        )?;
        Ok(bounded_version_list(fill_project_versions(&project_data)))
    }

    fn fill_installable_versions(
        &self,
        software: MinecraftSoftware,
        project: &str,
    ) -> Result<Vec<String>, String> {
        let versions = self.fill_published_versions(project)?;
        let Ok((latest, _)) = self.latest_stable_fill_build(software, project) else {
            return Ok(versions);
        };
        Ok(versions_not_newer_than(versions, &latest))
    }

    fn purpur_published_versions(&self) -> Result<Vec<String>, String> {
        let project =
            self.fetch_json("https://api.purpurmc.org/v2/purpur", &["api.purpurmc.org"])?;
        let mut versions = string_version_list(project.get("versions"), true);
        versions.reverse();
        Ok(bounded_version_list(versions))
    }

    fn leaves_published_versions(&self) -> Result<Vec<String>, String> {
        let project = self.fetch_json(
            "https://api.leavesmc.org/v2/projects/leaves",
            &["api.leavesmc.org"],
        )?;
        require_leaves_ok(&project)?;
        let mut versions = string_version_list(project.get("versions"), true);
        versions.reverse();
        Ok(bounded_version_list(versions))
    }

    fn leaves_installable_versions(&self) -> Result<Vec<String>, String> {
        let versions = self.leaves_published_versions()?;
        let Ok((latest, _)) = self.latest_leaves_release(&versions) else {
            return Ok(versions);
        };
        Ok(versions_not_newer_than(versions, &latest))
    }

    fn fabric_published_versions(&self) -> Result<Vec<String>, String> {
        let games = self.fetch_json(
            "https://meta.fabricmc.net/v2/versions/game",
            &["meta.fabricmc.net"],
        )?;
        Ok(bounded_version_list(fabric_stable_game_versions(&games)))
    }

    fn vanilla_release_versions(&self) -> Result<Vec<String>, String> {
        let manifest = self.fetch_json(
            "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
            &["piston-meta.mojang.com"],
        )?;
        Ok(bounded_version_list(mojang_release_versions(&manifest)))
    }

    fn latest_leaves_release(&self, versions: &[String]) -> Result<(String, Value), String> {
        for version in versions.iter().take(16) {
            if let Ok(build) = self.leaves_default_build(version) {
                return Ok((version.clone(), build));
            }
        }
        Err(
            "Leaves did not report a default-channel build in its 16 newest Minecraft versions"
                .to_owned(),
        )
    }

    fn leaves_default_build(&self, version: &str) -> Result<Value, String> {
        let version_data = self.fetch_json(
            &format!("https://api.leavesmc.org/v2/projects/leaves/versions/{version}"),
            &["api.leavesmc.org"],
        )?;
        require_leaves_ok(&version_data)?;
        let builds = version_data
            .get("builds")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("Leaves did not report builds for Minecraft {version}"))?;
        for build_id in builds.iter().rev().take(8) {
            let Some(build_id) = build_id.as_u64() else {
                continue;
            };
            let build = self.fetch_json(
                &format!(
                    "https://api.leavesmc.org/v2/projects/leaves/versions/{version}/builds/{build_id}"
                ),
                &["api.leavesmc.org"],
            )?;
            require_leaves_ok(&build)?;
            if leaves_build_is_release(&build) {
                return Ok(build);
            }
        }
        Err(format!(
            "Leaves has no default-channel build for Minecraft {version} yet"
        ))
    }

    fn fetch_json(&self, url: &str, hosts: &[&str]) -> Result<Value, String> {
        self.fetch_json_headers(url, hosts, &[])
    }

    fn fetch_json_headers(
        &self,
        url: &str,
        hosts: &[&str],
        extra_headers: &[(&str, &str)],
    ) -> Result<Value, String> {
        self.fetch_json_headers_timed(url, hosts, extra_headers, 20)
    }

    fn fetch_json_headers_timed(
        &self,
        url: &str,
        hosts: &[&str],
        extra_headers: &[(&str, &str)],
        timeout_seconds: u64,
    ) -> Result<Value, String> {
        require_https_host(url, hosts)?;
        let mut last_error = "the catalog request failed".to_owned();
        let attempt_budget = Duration::from_secs(timeout_seconds.saturating_sub(1).max(1));
        for attempt in 0..3_u8 {
            let started = Instant::now();
            match self.fetch_json_headers_once(url, extra_headers, timeout_seconds) {
                Ok(value) => return Ok(value),
                Err(error) => {
                    last_error = error;
                    if attempt == 2
                        || started.elapsed() >= attempt_budget
                        || catalog_fetch_is_non_retryable(&last_error)
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(250 * u64::from(attempt + 1)));
                }
            }
        }
        Err(last_error)
    }

    fn fetch_json_headers_once(
        &self,
        url: &str,
        extra_headers: &[(&str, &str)],
        timeout_seconds: u64,
    ) -> Result<Value, String> {
        let cache = self.state_root.join("metadata");
        fs::create_dir_all(&cache).map_err(|_| "could not create the metadata cache".to_owned())?;
        let path = cache.join(format!("{}.json", Uuid::new_v4()));
        let result = (|| {
            self.curl_no_redirect_headers(
                url,
                &path,
                MAX_METADATA_BYTES,
                timeout_seconds,
                extra_headers,
            )?;
            let metadata =
                fs::metadata(&path).map_err(|_| "downloaded metadata is unavailable".to_owned())?;
            if metadata.len() == 0 || metadata.len() > MAX_METADATA_BYTES {
                return Err("downloaded metadata is outside the size limit".to_owned());
            }
            serde_json::from_slice(
                &fs::read(&path).map_err(|_| "could not read downloaded metadata".to_owned())?,
            )
            .map_err(|_| "the software catalog returned invalid metadata".to_owned())
        })();
        let _ = fs::remove_file(&path);
        result
    }

    fn curseforge_key_path(&self) -> PathBuf {
        self.state_root.join("curseforge-api-key")
    }

    fn read_curseforge_api_key(&self) -> Result<Option<String>, String> {
        let path = self.curseforge_key_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .map_err(|_| "could not read the saved CurseForge API key".to_owned())?;
        if bytes.len() > 256 {
            return Err("the saved CurseForge API key is invalid".to_owned());
        }
        let raw = String::from_utf8(bytes)
            .map_err(|_| "the saved CurseForge API key is invalid".to_owned())?;
        Ok(Some(validate_curseforge_api_key(&raw)?))
    }

    fn require_curseforge_api_key(&self) -> Result<String, String> {
        self.read_curseforge_api_key()?
            .ok_or_else(|| CURSEFORGE_API_KEY_REQUIRED.to_owned())
    }

    pub fn curseforge_key_status(&self) -> Result<Value, String> {
        self.curseforge_key_status_with_probe(None)
    }

    fn curseforge_key_status_with_probe(&self, probe: Option<&str>) -> Result<Value, String> {
        let mut status = json!({
            "schema_version": 1,
            "configured": self.read_curseforge_api_key()?.is_some(),
            "catalog": "api.curseforge.com",
        });
        if let Some(probe) = probe {
            if let Some(object) = status.as_object_mut() {
                object.insert("probe".to_owned(), json!(probe));
            }
        }
        Ok(status)
    }

    pub fn set_curseforge_api_key(&self, key: &str) -> Result<Value, String> {
        let key = validate_curseforge_api_key(key)?;
        let path = self.curseforge_key_path();
        let previous = fs::read(&path).ok();
        write_replaced_secret_file(&path, key.as_bytes())?;
        match self.probe_curseforge_api_key() {
            Ok(()) => self.curseforge_key_status_with_probe(Some("ok")),
            Err(error) if error == CURSEFORGE_KEY_REJECTED => {
                match previous {
                    Some(bytes) => {
                        let _ = write_replaced_secret_file(&path, &bytes);
                    }
                    None => {
                        let _ = fs::remove_file(&path);
                    }
                }
                Err(error)
            }
            Err(error) if error == CURSEFORGE_CDN_BLOCKED => {
                self.curseforge_key_status_with_probe(Some("cdn_blocked"))
            }
            Err(_) => self.curseforge_key_status_with_probe(Some("unreachable")),
        }
    }

    pub fn clear_curseforge_api_key(&self) -> Result<Value, String> {
        let path = self.curseforge_key_path();
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|_| "could not remove the saved CurseForge API key".to_owned())?;
        }
        self.curseforge_key_status()
    }

    fn probe_curseforge_api_key(&self) -> Result<(), String> {
        self.curseforge_v1("games/432").map(|_| ())
    }

    fn curseforge_v1(&self, path: &str) -> Result<Value, String> {
        if path.is_empty()
            || path.contains("://")
            || path.contains(['\\', '\0', ' ', '\n', '\r'])
            || path
                .split(['/', '?', '&', '='])
                .any(|segment| segment == "." || segment == "..")
        {
            return Err("the CurseForge catalog path is invalid".to_owned());
        }
        let key = self.require_curseforge_api_key()?;
        let url = format!("https://api.curseforge.com/v1/{path}");
        self.fetch_json_headers_timed(
            &url,
            &["api.curseforge.com"],
            &[("x-api-key", key.as_str())],
            12,
        )
        .map_err(|error| {
            if error == CURSEFORGE_CDN_BLOCKED
                || error == CURSEFORGE_KEY_REJECTED
                || error == CURSEFORGE_RATE_LIMITED
                || error.starts_with("CurseForge catalog was unreachable.")
            {
                error
            } else {
                let lower = error.to_ascii_lowercase();
                if lower.contains("401") || lower.contains("403") {
                    CURSEFORGE_CDN_BLOCKED.to_owned()
                } else {
                    format!("CurseForge catalog was unreachable. {error}")
                }
            }
        })
    }

    fn resolve_curseforge_download_url(
        &self,
        project_id: &str,
        file: &Value,
    ) -> Result<String, String> {
        if let Some(candidate) = file.get("downloadUrl").and_then(Value::as_str) {
            if candidate.len() <= 4_096 {
                if let Ok(direct) = marketplace::direct_forgecdn_download_url(candidate) {
                    return Ok(direct);
                }
            }
        }
        let file_id = file
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "CurseForge returned a file without an id".to_owned())?;
        if let Ok(response) =
            self.curseforge_v1(&format!("mods/{project_id}/files/{file_id}/download-url"))
        {
            if let Some(url) = response.get("data").and_then(Value::as_str) {
                if let Ok(direct) = marketplace::direct_forgecdn_download_url(url) {
                    return Ok(direct);
                }
            }
        }
        let filename = file
            .get("fileName")
            .and_then(Value::as_str)
            .ok_or_else(|| "CurseForge returned a file without a name".to_owned())?;
        let fallback = marketplace::forgecdn_file_url(file_id, filename)?;
        require_https_host(&fallback, FORGECDN_DOWNLOAD_HOSTS)?;
        Ok(fallback)
    }

    fn download_artifact(&self, artifact: &Artifact, destination: &Path) -> Result<String, String> {
        if matches!(artifact.software, MinecraftSoftware::Pumpkin) {
            return self.download_pumpkin(artifact, destination);
        }
        if let Some(source) = &artifact.local_source {
            return self.import_local_artifact(source, destination);
        }
        require_https_host(
            &artifact.url,
            &[
                "fill-data.papermc.io",
                "api.purpurmc.org",
                "api.leavesmc.org",
                "piston-data.mojang.com",
                "launcher.mojang.com",
                "meta.fabricmc.net",
                "meta.quiltmc.org",
                "maven.minecraftforge.net",
                "maven.neoforged.net",
                "ci.pufferfish.host",
            ],
        )?;
        let partial = destination.with_extension("jar.partial");
        self.curl_no_redirect(&artifact.url, &partial, MAX_SERVER_JAR_BYTES, 10 * 60)?;
        let metadata =
            fs::metadata(&partial).map_err(|_| "the server download is unavailable".to_owned())?;
        let minimum_bytes = if artifact.install_server
            || matches!(
                artifact.software,
                MinecraftSoftware::Fabric | MinecraftSoftware::Quilt
            ) {
            16 * 1024
        } else {
            1024 * 1024
        };
        if metadata.len() < minimum_bytes || metadata.len() > MAX_SERVER_JAR_BYTES {
            let _ = fs::remove_file(&partial);
            return Err("the downloaded server file is outside the expected size range".to_owned());
        }
        if let Some(expected) = &artifact.expected_hash {
            let actual = match expected.algorithm {
                HashAlgorithm::Sha1 => self.file_sha1(&partial)?,
                HashAlgorithm::Sha256 => file_sha256(&partial)?,
            };
            if !actual.eq_ignore_ascii_case(&expected.value) {
                let _ = fs::remove_file(&partial);
                return Err("the server download failed its publisher checksum".to_owned());
            }
        }
        let sha256 = file_sha256(&partial)?;
        fs::rename(&partial, destination)
            .map_err(|_| "could not commit the verified server download".to_owned())?;
        Ok(sha256)
    }

    fn run_loader_installer(
        &self,
        artifact: &Artifact,
        data_path: &Path,
        run_uid: u32,
        runtime_image: &str,
    ) -> Result<String, String> {
        let installer = data_path.join("server.jar");
        if !installer.is_file() {
            return Err("the loader installer was not downloaded".to_owned());
        }
        self.chown_instance(data_path, run_uid)?;
        let mount = format!("type=bind,src={},dst=/data", data_path.display());
        let user = format!("{run_uid}:{run_uid}");
        let args = vec![
            "run".to_owned(),
            "--rm".to_owned(),
            "--user".to_owned(),
            user,
            "--network".to_owned(),
            "bridge".to_owned(),
            "--mount".to_owned(),
            mount,
            "--workdir".to_owned(),
            "/data".to_owned(),
            "--entrypoint".to_owned(),
            "java".to_owned(),
            runtime_image.to_owned(),
            "-jar".to_owned(),
            "/data/server.jar".to_owned(),
            "--installServer".to_owned(),
        ];
        self.docker_owned(&args, 15 * 60).map_err(|error| {
            let detail = error
                .strip_prefix("the Helix execution backend failed: ")
                .unwrap_or(error.as_str());
            if detail.contains("Unable to access jarfile") {
                "the loader installer could not read server.jar in the instance directory"
                    .to_owned()
            } else {
                format!("the loader installer failed: {detail}")
            }
        })?;
        find_unix_args(data_path)
            .ok_or_else(|| {
                format!(
                    "{} installer finished without writing unix_args.txt; Helix cannot launch that loader version",
                    software_name(artifact.software)
                )
            })
    }

    fn import_local_artifact(&self, source: &Path, destination: &Path) -> Result<String, String> {
        let canonical = fs::canonicalize(source)
            .map_err(|_| "the custom server JAR is unavailable".to_owned())?;
        if !self
            .custom_artifact_roots
            .iter()
            .any(|root| canonical.starts_with(root))
        {
            return Err("the custom server JAR moved outside a managed Storage root".to_owned());
        }
        let mut input = File::open(&canonical)
            .map_err(|_| "could not open the custom server JAR".to_owned())?;
        let metadata = input
            .metadata()
            .map_err(|_| "could not inspect the custom server JAR".to_owned())?;
        if !metadata.is_file()
            || metadata.len() < 16 * 1024
            || metadata.len() > MAX_SERVER_JAR_BYTES
        {
            return Err("the custom server JAR changed before import".to_owned());
        }
        let partial = destination.with_extension("jar.partial");
        let result = (|| {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&partial)
                .map_err(|_| "could not stage the custom server JAR".to_owned())?;
            let copied = std::io::copy(&mut input, &mut output)
                .map_err(|_| "could not copy the custom server JAR".to_owned())?;
            if copied != metadata.len() {
                return Err("the custom server JAR changed during import".to_owned());
            }
            output
                .sync_all()
                .map_err(|_| "could not persist the custom server JAR".to_owned())?;
            let sha256 = file_sha256(&partial)?;
            fs::rename(&partial, destination)
                .map_err(|_| "could not commit the custom server JAR".to_owned())?;
            Ok(sha256)
        })();
        if result.is_err() {
            let _ = fs::remove_file(partial);
        }
        result
    }

    fn resolve_runtime_image(&self, java_version: u16) -> Result<String, String> {
        let tag = format!("eclipse-temurin:{java_version}-jre-noble");
        self.docker(["pull", tag.as_str()], 10 * 60)?;
        let digest = self.docker(
            [
                "image",
                "inspect",
                "--format",
                "{{index .RepoDigests 0}}",
                tag.as_str(),
            ],
            30,
        )?;
        let digest = digest.trim();
        if !digest.starts_with("eclipse-temurin@sha256:")
            || !valid_hex(digest.trim_start_matches("eclipse-temurin@sha256:"), 64)
        {
            return Err("Docker did not return a pinned Temurin image digest".to_owned());
        }
        Ok(digest.to_owned())
    }

    fn create_container(
        &self,
        manifest: &InstanceManifest,
        data_path: &Path,
    ) -> Result<(), String> {
        if manifest.is_vrising() {
            return self.create_vrising_container(manifest, data_path);
        }
        if manifest.is_valheim() {
            return self.create_valheim_container(manifest, data_path);
        }
        if manifest.is_terraria() {
            return self.create_terraria_container(manifest, data_path);
        }
        let restart = if manifest.start_on_boot {
            "unless-stopped"
        } else {
            "no"
        };
        let memory_limit = u64::from(manifest.memory_mb).saturating_add(if manifest.is_pumpkin() {
            0
        } else {
            1024
        });
        let minimum_heap = manifest.memory_mb.min(1024);
        let game_tcp = format!("0.0.0.0:{0}:{0}/tcp", manifest.game_port);
        let game_udp = format!("0.0.0.0:{0}:{0}/udp", manifest.game_port);
        let rcon = format!("127.0.0.1:{0}:{0}/tcp", manifest.rcon_port);
        let mount = format!("type=bind,src={},dst=/data", data_path.display());
        let user = format!("{}:{}", manifest.run_uid, manifest.run_uid);
        let memory = format!("{memory_limit}m");
        let xms = format!("-Xms{minimum_heap}M");
        let xmx = format!("-Xmx{}M", manifest.memory_mb);
        let instance_label = format!("io.helix.instance={}", manifest.id);
        let version_label = format!("io.helix.minecraft.version={}", manifest.minecraft_version);
        let mut args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            manifest.container_name.clone(),
            "--label".to_owned(),
            "io.helix.managed=true".to_owned(),
            "--label".to_owned(),
            instance_label,
            "--label".to_owned(),
            version_label,
            "--restart".to_owned(),
            restart.to_owned(),
            "--memory".to_owned(),
            memory.clone(),
            "--memory-swap".to_owned(),
            memory,
            "--pids-limit".to_owned(),
            "1024".to_owned(),
            "--cap-drop".to_owned(),
            "ALL".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges:true".to_owned(),
            "--read-only".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp:rw,exec,nosuid,nodev,size=256m,mode=1777".to_owned(),
            "--mount".to_owned(),
            mount,
            "--workdir".to_owned(),
            "/data".to_owned(),
            "--user".to_owned(),
            user,
            "--env".to_owned(),
            "HOME=/data".to_owned(),
            "--publish".to_owned(),
            game_tcp,
            "--publish".to_owned(),
            game_udp,
            "--publish".to_owned(),
            rcon,
            "--stop-timeout".to_owned(),
            "45".to_owned(),
            "--log-opt".to_owned(),
            "max-size=20m".to_owned(),
            "--log-opt".to_owned(),
            "max-file=5".to_owned(),
            "--entrypoint".to_owned(),
            "java".to_owned(),
            manifest.runtime_image.clone(),
            xms,
            xmx,
            "-XX:+UseG1GC".to_owned(),
            "-XX:+ParallelRefProcEnabled".to_owned(),
            "-XX:+DisableExplicitGC".to_owned(),
        ];
        if manifest.is_pumpkin() {
            let unused_udp = format!("0.0.0.0:{0}:{0}/udp", manifest.game_port);
            if let Some(index) = args.iter().position(|arg| arg == &unused_udp) {
                args.drain(index - 1..=index);
            }
            let entry = args
                .iter()
                .position(|arg| arg == "--entrypoint")
                .ok_or("Missing runtime entrypoint")?;
            args.truncate(entry + 1);
            args.pop();
            args.extend([
                "--publish".to_owned(),
                format!("0.0.0.0:{0}:{0}/tcp", manifest.query_port),
                "--publish".to_owned(),
                format!("0.0.0.0:{0}:{0}/udp", manifest.query_port),
                "--entrypoint".to_owned(),
            ]);
            args.push("/data/pumpkin".to_owned());
            args.push(manifest.runtime_image.clone());
        } else if let Some(unix_args) = &manifest.unix_args {
            args.push(format!("@{unix_args}"));
            args.push("nogui".to_owned());
        } else {
            args.push("-jar".to_owned());
            args.push("server.jar".to_owned());
            args.push("--nogui".to_owned());
        }
        insert_cpu_limit(&mut args, manifest.cpu_millis);
        self.docker_owned(&args, DOCKER_TIMEOUT_SECONDS)?;
        Ok(())
    }

    fn create_validation_container(
        &self,
        manifest: &InstanceManifest,
        data_path: &Path,
    ) -> Result<(), String> {
        let mut validation_manifest = manifest.clone();
        validation_manifest.start_on_boot = false;
        self.create_container(&validation_manifest, data_path)
    }

    fn finalize_container_restart_policy(&self, manifest: &InstanceManifest) -> Result<(), String> {
        let desired = if manifest.start_on_boot {
            "unless-stopped"
        } else {
            "no"
        };
        if self.container_restart_policy(&manifest.container_name)? == desired {
            return Ok(());
        }
        self.docker(
            [
                "update",
                "--restart",
                desired,
                manifest.container_name.as_str(),
            ],
            20,
        )?;
        if self.container_restart_policy(&manifest.container_name)? != desired {
            return Err("Docker did not persist the server's restart policy".to_owned());
        }
        Ok(())
    }

    fn create_vrising_container(
        &self,
        manifest: &InstanceManifest,
        data_path: &Path,
    ) -> Result<(), String> {
        let restart = if manifest.start_on_boot {
            "unless-stopped"
        } else {
            "no"
        };
        let memory_limit = u64::from(manifest.memory_mb).saturating_add(1024);
        let game_udp = format!("0.0.0.0:{0}:{0}/udp", manifest.game_port);
        let query_udp = format!("0.0.0.0:{0}:{0}/udp", manifest.query_port);
        let mount = format!("type=bind,src={},dst=/data", data_path.display());
        let user = format!("{}:{}", manifest.run_uid, manifest.run_uid);
        let memory = format!("{memory_limit}m");
        let instance_label = format!("io.helix.instance={}", manifest.id);
        let mut args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            manifest.container_name.clone(),
            "--label".to_owned(),
            "io.helix.managed=true".to_owned(),
            "--label".to_owned(),
            "io.helix.game=vrising".to_owned(),
            "--label".to_owned(),
            instance_label,
            "--restart".to_owned(),
            restart.to_owned(),
            "--memory".to_owned(),
            memory.clone(),
            "--memory-swap".to_owned(),
            memory,
            "--pids-limit".to_owned(),
            "2048".to_owned(),
            "--cap-drop".to_owned(),
            "ALL".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges:true".to_owned(),
            "--security-opt".to_owned(),
            "seccomp=unconfined".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp:rw,exec,nosuid,nodev,size=512m,mode=1777".to_owned(),
            "--mount".to_owned(),
            mount,
            "--workdir".to_owned(),
            "/data".to_owned(),
            "--user".to_owned(),
            user,
            "--env".to_owned(),
            "HOME=/data".to_owned(),
            "--env".to_owned(),
            format!("HELIX_SERVER_NAME={}", manifest.name),
            "--publish".to_owned(),
            game_udp,
            "--publish".to_owned(),
            query_udp,
            "--stop-timeout".to_owned(),
            "45".to_owned(),
            "--log-opt".to_owned(),
            "max-size=20m".to_owned(),
            "--log-opt".to_owned(),
            "max-file=5".to_owned(),
            manifest.runtime_image.clone(),
        ];
        insert_cpu_limit(&mut args, manifest.cpu_millis);
        self.docker_owned(&args, DOCKER_TIMEOUT_SECONDS)?;
        Ok(())
    }

    fn create_valheim_container(
        &self,
        manifest: &InstanceManifest,
        data_path: &Path,
    ) -> Result<(), String> {
        let restart = if manifest.start_on_boot {
            "unless-stopped"
        } else {
            "no"
        };
        let memory_limit = u64::from(manifest.memory_mb).saturating_add(1024);
        let game_udp = format!("0.0.0.0:{0}:{0}/udp", manifest.game_port);
        let query_udp = format!("0.0.0.0:{0}:{0}/udp", manifest.game_port.saturating_add(1));
        let steam_udp = format!("0.0.0.0:{0}:{0}/udp", manifest.game_port.saturating_add(2));
        let mount = format!("type=bind,src={},dst=/data", data_path.display());
        let user = format!("{}:{}", manifest.run_uid, manifest.run_uid);
        let memory = format!("{memory_limit}m");
        let instance_label = format!("io.helix.instance={}", manifest.id);
        let mut args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            manifest.container_name.clone(),
            "--label".to_owned(),
            "io.helix.managed=true".to_owned(),
            "--label".to_owned(),
            "io.helix.game=valheim".to_owned(),
            "--label".to_owned(),
            instance_label,
            "--restart".to_owned(),
            restart.to_owned(),
            "--memory".to_owned(),
            memory.clone(),
            "--memory-swap".to_owned(),
            memory,
            "--pids-limit".to_owned(),
            "2048".to_owned(),
            "--cap-drop".to_owned(),
            "ALL".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges:true".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp:rw,exec,nosuid,nodev,size=512m,mode=1777".to_owned(),
            "--mount".to_owned(),
            mount,
            "--workdir".to_owned(),
            "/data".to_owned(),
            "--user".to_owned(),
            user,
            "--env".to_owned(),
            "HOME=/data".to_owned(),
            "--env".to_owned(),
            format!("HELIX_SERVER_NAME={}", manifest.name),
            "--env".to_owned(),
            format!("HELIX_GAME_PORT={}", manifest.game_port),
            "--env".to_owned(),
            format!("HELIX_MAX_PLAYERS={}", manifest.max_players),
            "--publish".to_owned(),
            game_udp,
            "--publish".to_owned(),
            query_udp,
            "--publish".to_owned(),
            steam_udp,
            "--stop-timeout".to_owned(),
            "45".to_owned(),
            "--log-opt".to_owned(),
            "max-size=20m".to_owned(),
            "--log-opt".to_owned(),
            "max-file=5".to_owned(),
            manifest.runtime_image.clone(),
        ];
        insert_cpu_limit(&mut args, manifest.cpu_millis);
        self.docker_owned(&args, DOCKER_TIMEOUT_SECONDS)?;
        Ok(())
    }

    fn create_terraria_container(
        &self,
        manifest: &InstanceManifest,
        data_path: &Path,
    ) -> Result<(), String> {
        let restart = if manifest.start_on_boot {
            "unless-stopped"
        } else {
            "no"
        };
        let memory_limit = u64::from(manifest.memory_mb).saturating_add(512);
        let game_tcp = format!("0.0.0.0:{0}:{0}/tcp", manifest.game_port);
        let mount = format!("type=bind,src={},dst=/data", data_path.display());
        let user = format!("{}:{}", manifest.run_uid, manifest.run_uid);
        let memory = format!("{memory_limit}m");
        let instance_label = format!("io.helix.instance={}", manifest.id);
        let flavor = if manifest.build.starts_with("tmodloader") {
            "tmodloader"
        } else {
            "vanilla"
        };
        let mut args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            manifest.container_name.clone(),
            "--label".to_owned(),
            "io.helix.managed=true".to_owned(),
            "--label".to_owned(),
            "io.helix.game=terraria".to_owned(),
            "--label".to_owned(),
            instance_label,
            "--restart".to_owned(),
            restart.to_owned(),
            "--memory".to_owned(),
            memory.clone(),
            "--memory-swap".to_owned(),
            memory,
            "--pids-limit".to_owned(),
            "1024".to_owned(),
            "--cap-drop".to_owned(),
            "ALL".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges:true".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp:rw,exec,nosuid,nodev,size=256m,mode=1777".to_owned(),
            "--mount".to_owned(),
            mount,
            "--workdir".to_owned(),
            "/data".to_owned(),
            "--user".to_owned(),
            user,
            "--env".to_owned(),
            "HOME=/data".to_owned(),
            "--env".to_owned(),
            format!("HELIX_SERVER_NAME={}", manifest.name),
            "--env".to_owned(),
            format!("HELIX_GAME_PORT={}", manifest.game_port),
            "--env".to_owned(),
            format!("HELIX_MAX_PLAYERS={}", manifest.max_players),
            "--env".to_owned(),
            format!("HELIX_TERRARIA_SOFTWARE={flavor}"),
            "--env".to_owned(),
            format!("HELIX_TERRARIA_VERSION={}", manifest.minecraft_version),
            "--publish".to_owned(),
            game_tcp,
            "--stop-timeout".to_owned(),
            "45".to_owned(),
            "--log-opt".to_owned(),
            "max-size=20m".to_owned(),
            "--log-opt".to_owned(),
            "max-file=5".to_owned(),
            manifest.runtime_image.clone(),
        ];
        insert_cpu_limit(&mut args, manifest.cpu_millis);
        self.docker_owned(&args, DOCKER_TIMEOUT_SECONDS)?;
        Ok(())
    }

    fn wait_until_ready<F>(
        &self,
        manifest: &InstanceManifest,
        timeout: Duration,
        progress: F,
    ) -> Result<(), String>
    where
        F: FnMut(u64),
    {
        if manifest.uses_ready_marker() {
            self.wait_for_ready_marker(manifest, timeout, progress)
        } else {
            self.wait_for_minecraft(manifest, timeout, progress)
        }
    }

    fn ready_timeout(&self, manifest: &InstanceManifest) -> Duration {
        if manifest.uses_ready_marker() || manifest.modpack.is_some() {
            Duration::from_secs(20 * 60)
        } else {
            Duration::from_secs(6 * 60)
        }
    }

    fn clear_ready_marker(&self, manifest: &InstanceManifest) -> Result<(), String> {
        if !manifest.uses_ready_marker() {
            return Ok(());
        }
        let marker = self.instance_path(&manifest.id)?.join(READY_MARKER);
        if marker.exists() {
            fs::remove_file(&marker).map_err(|_| "could not clear the ready marker".to_owned())?;
        }
        Ok(())
    }

    fn wait_for_ready_marker<F>(
        &self,
        manifest: &InstanceManifest,
        timeout: Duration,
        mut progress: F,
    ) -> Result<(), String>
    where
        F: FnMut(u64),
    {
        let started = Instant::now();
        let deadline = started + timeout;
        let marker = self.instance_path(&manifest.id)?.join(READY_MARKER);
        let label = display_software(manifest);
        while Instant::now() < deadline {
            if marker.is_file() && self.container_running(&manifest.container_name) {
                return Ok(());
            }
            if !self.container_running(&manifest.container_name) {
                let logs = self
                    .docker(
                        ["logs", "--tail", "40", manifest.container_name.as_str()],
                        20,
                    )
                    .unwrap_or_default();
                return Err(if logs.trim().is_empty() {
                    format!("{label} stopped before it became ready")
                } else {
                    format!(
                        "{label} stopped before it became ready: {}",
                        one_line_tail(&logs, 600)
                    )
                });
            }
            progress(Instant::now().saturating_duration_since(started).as_secs());
            std::thread::sleep(Duration::from_secs(2));
        }
        Err(format!(
            "{label} did not become ready before the install timeout"
        ))
    }

    fn wait_for_minecraft<F>(
        &self,
        manifest: &InstanceManifest,
        timeout: Duration,
        mut progress: F,
    ) -> Result<(), String>
    where
        F: FnMut(u64),
    {
        let started = Instant::now();
        let deadline = started + timeout;
        let initial_state = self.container_startup_state(&manifest.container_name)?;
        while Instant::now() < deadline {
            if minecraft_status(manifest.game_port, Duration::from_secs(2)).is_ok()
                && (!manifest.is_pumpkin() || pumpkin::bedrock_ready(manifest.query_port))
            {
                return Ok(());
            }
            let state = self.container_startup_state(&manifest.container_name)?;
            if state.restarting || state.restart_count > initial_state.restart_count {
                return Err(self.minecraft_startup_failure(
                    manifest,
                    state,
                    "Minecraft restarted unexpectedly during its first boot",
                ));
            }
            if !state.running {
                return Err(self.minecraft_startup_failure(
                    manifest,
                    state,
                    &format!(
                        "Minecraft exited with code {} before it became ready",
                        state.exit_code
                    ),
                ));
            }
            progress(Instant::now().saturating_duration_since(started).as_secs());
            thread::sleep(Duration::from_secs(3));
        }
        let logs = self
            .docker(
                ["logs", "--tail", "80", manifest.container_name.as_str()],
                20,
            )
            .unwrap_or_default();
        let elapsed = Instant::now().saturating_duration_since(started).as_secs();
        Err(if logs.trim().is_empty() {
            format!(
                "Minecraft kept running but did not open port {} within {elapsed} seconds",
                manifest.game_port
            )
        } else {
            format!(
                "Minecraft kept running but did not open port {} within {elapsed} seconds. Latest startup output: {}",
                manifest.game_port,
                startup_failure_summary(&logs, 1_800)
            )
        })
    }

    fn container_startup_state(&self, name: &str) -> Result<ContainerStartupState, String> {
        let output = self.docker(
            [
                "inspect",
                "--format",
                "{{.State.Running}}|{{.State.Restarting}}|{{.State.OOMKilled}}|{{.State.ExitCode}}|{{.RestartCount}}",
                name,
            ],
            20,
        )?;
        parse_container_startup_state(&output)
    }

    fn minecraft_startup_failure(
        &self,
        manifest: &InstanceManifest,
        state: ContainerStartupState,
        reason: &str,
    ) -> String {
        let logs = self
            .docker(
                ["logs", "--tail", "500", manifest.container_name.as_str()],
                20,
            )
            .unwrap_or_default();
        let normalized = logs.to_ascii_lowercase();
        let memory_exhausted = state.oom_killed
            || normalized.contains("outofmemoryerror")
            || normalized.contains("java heap space")
            || normalized.contains("gc overhead limit exceeded");
        let cause = if memory_exhausted {
            format!(
                "The {} MiB memory allocation was exhausted. Increase the server memory and try again",
                manifest.memory_mb
            )
        } else {
            reason.to_owned()
        };
        let summary = startup_failure_summary(&logs, 1_800);
        if summary.is_empty() {
            cause
        } else {
            format!("{cause}. Relevant startup output: {summary}")
        }
    }

    fn backup(&self, manifest: &InstanceManifest) -> Result<PathBuf, String> {
        let running = self.container_running(&manifest.container_name);
        if running {
            self.docker(
                ["stop", "--time", "45", manifest.container_name.as_str()],
                75,
            )?;
        }
        let archive_result = self.archive_data(manifest);
        let restart_result = if running {
            self.clear_ready_marker(manifest).and_then(|()| {
                self.docker(["start", manifest.container_name.as_str()], 90)
                    .and_then(|_| {
                        self.wait_until_ready(manifest, self.ready_timeout(manifest), |_| {})
                    })
            })
        } else {
            Ok(())
        };
        match (archive_result, restart_result) {
            (Ok(path), Ok(())) => {
                let protect = backup_id_from_path(&path);
                if let Err(error) = self.enforce_backup_retention(manifest, &protect) {
                    eprintln!("backup retention failed: {error}");
                }
                Ok(path)
            }
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(format!("backup completed, but restart failed: {error}")),
            (Err(backup), Err(restart)) => Err(format!(
                "backup failed: {backup}; restart also failed: {restart}"
            )),
        }
    }

    fn restart_if_previously_running(
        &self,
        manifest: &InstanceManifest,
        previously_running: bool,
    ) -> Result<(), String> {
        if !previously_running {
            return Ok(());
        }
        self.clear_ready_marker(manifest)?;
        self.docker(["start", manifest.container_name.as_str()], 90)?;
        self.wait_until_ready(manifest, self.ready_timeout(manifest), |_| {})
    }

    fn archive_data(&self, manifest: &InstanceManifest) -> Result<PathBuf, String> {
        let destination_root = self.backup_path(&manifest.id)?;
        fs::create_dir_all(&destination_root)
            .map_err(|_| "could not create the backup directory".to_owned())?;
        fs::set_permissions(&destination_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the backup directory".to_owned())?;
        let timestamp = now_unix_ms();
        let destination = destination_root.join(format!("{timestamp}.tar.gz"));
        let partial = destination.with_extension("tar.gz.partial");
        let data_path = self.instance_path(&manifest.id)?;
        if let Err(error) = run_program(
            Path::new("/usr/bin/tar"),
            &[
                "--numeric-owner".to_owned(),
                "--one-file-system".to_owned(),
                "-czf".to_owned(),
                partial.to_string_lossy().into_owned(),
                "-C".to_owned(),
                data_path.to_string_lossy().into_owned(),
                ".".to_owned(),
            ],
            30 * 60,
        ) {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        if let Err(error) = run_program(
            Path::new("/usr/bin/gzip"),
            &["--test".to_owned(), partial.to_string_lossy().into_owned()],
            30 * 60,
        ) {
            let _ = fs::remove_file(&partial);
            return Err(format!(
                "the completed backup failed its integrity check: {error}"
            ));
        }
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o600))
            .map_err(|_| "could not protect the completed backup".to_owned())?;
        fs::rename(&partial, &destination)
            .map_err(|_| "could not commit the completed backup".to_owned())?;
        write_manifest(
            &destination_root.join(format!("{timestamp}.json")),
            manifest,
        )?;
        Ok(destination)
    }

    fn update(&self, manifest: &InstanceManifest) -> Result<bool, String> {
        if manifest.uses_ready_marker() {
            return self.update_ready_marker_game(manifest);
        }
        let artifact = self.resolve_artifact(
            manifest.software,
            if manifest.is_pumpkin() {
                "latest"
            } else {
                &manifest.minecraft_version
            },
        )?;
        if manifest.is_pumpkin() && artifact.version != manifest.minecraft_version {
            return Err("The latest Pumpkin release changes Minecraft client/world versions. Keep this server pinned and test the new release in a separate server before migrating a backup.".to_owned());
        }
        if artifact.java_version != manifest.java_version {
            return Err(format!(
                "this update changes the Java requirement from {} to {}; use the guided version upgrade flow",
                manifest.java_version, artifact.java_version
            ));
        }
        let data_path = self.instance_path(&manifest.id)?;
        let update_path = self
            .state_root
            .join(".staging")
            .join(format!("{}.jar", manifest.id));
        let _ = fs::remove_file(&update_path);
        let sha256 = self.download_artifact(&artifact, &update_path)?;
        if sha256 == manifest.artifact_sha256 {
            let _ = fs::remove_file(update_path);
            return Ok(false);
        }
        let running = self.container_running(&manifest.container_name);
        if running {
            self.docker(
                ["stop", "--time", "45", manifest.container_name.as_str()],
                75,
            )?;
        }
        if let Err(error) = self.archive_data(manifest) {
            let restart = self.restart_if_previously_running(manifest, running);
            let _ = fs::remove_file(&update_path);
            return Err(match restart {
                Ok(()) => format!("the update safety backup failed: {error}"),
                Err(restart) => format!(
                    "the update safety backup failed: {error}; the original server also failed to restart: {restart}"
                ),
            });
        }
        let jar = data_path.join(manifest.artifact_name());
        let rollback = data_path.join(format!("{}.rollback", manifest.artifact_name()));
        if rollback.exists() {
            let restart = self.restart_if_previously_running(manifest, running);
            let _ = fs::remove_file(&update_path);
            return Err(match restart {
                Ok(()) => "a previous server update rollback file still needs attention".to_owned(),
                Err(restart) => format!(
                    "a previous server update rollback file still needs attention; the original server also failed to restart: {restart}"
                ),
            });
        }
        let mut updated = manifest.clone();
        updated.build = artifact.build;
        updated.artifact_url = artifact.url;
        updated.artifact_sha256 = sha256;
        let activation = (|| {
            fs::rename(&jar, &rollback)
                .map_err(|_| "could not stage the current server for rollback".to_owned())?;
            fs::rename(&update_path, &jar)
                .map_err(|error| format!("could not activate the update: {error}"))?;
            self.protect_instance_artifacts(&data_path, manifest.run_uid)?;
            write_manifest(&self.manifest_path(&manifest.id)?, &updated)?;
            if running {
                self.docker(["start", updated.container_name.as_str()], 90)?;
                self.wait_until_ready(&updated, self.ready_timeout(&updated), |_| {})?;
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = activation {
            let _ = self.docker(
                ["stop", "--time", "15", updated.container_name.as_str()],
                30,
            );
            if rollback.is_file() {
                if jar.is_file() {
                    let _ = fs::rename(&jar, &update_path);
                }
                let _ = fs::rename(&rollback, &jar);
            }
            let _ = write_manifest(&self.manifest_path(&manifest.id)?, manifest);
            let restart = self.restart_if_previously_running(manifest, running);
            let _ = fs::remove_file(&update_path);
            return Err(format!(
                "the update failed validation and was rolled back: {error}{}",
                restart
                    .err()
                    .map(|restart| format!("; the original server failed to restart: {restart}"))
                    .unwrap_or_default()
            ));
        }
        let _ = fs::remove_file(rollback);
        Ok(true)
    }

    fn update_ready_marker_game(&self, manifest: &InstanceManifest) -> Result<bool, String> {
        let running = self.container_running(&manifest.container_name);
        if !running {
            return Ok(false);
        }
        self.clear_ready_marker(manifest)?;
        self.docker(
            ["restart", "--time", "45", manifest.container_name.as_str()],
            90,
        )?;
        self.wait_until_ready(manifest, self.ready_timeout(manifest), |_| {})?;
        Ok(true)
    }

    fn ensure_vrising_runtime_image<F>(&self, progress: &mut F) -> Result<String, String>
    where
        F: FnMut(&str, u8),
    {
        if self
            .docker(["image", "inspect", vrising::RUNTIME_IMAGE], 20)
            .is_ok()
        {
            return Ok(vrising::RUNTIME_IMAGE.to_owned());
        }
        progress(
            "Building the isolated V Rising runtime image (one-time)",
            16,
        );
        let staging = self.state_root.join(".staging").join("vrising-runtime");
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)
            .map_err(|_| "could not stage the V Rising runtime build".to_owned())?;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the V Rising runtime build".to_owned())?;
        write_new_file(
            &staging.join("Dockerfile"),
            vrising::DOCKERFILE.as_bytes(),
            0o600,
        )?;
        write_new_file(
            &staging.join("entrypoint.sh"),
            vrising::ENTRYPOINT.as_bytes(),
            0o755,
        )?;
        let result = self.docker_owned(
            &[
                "build".to_owned(),
                "--tag".to_owned(),
                vrising::RUNTIME_IMAGE.to_owned(),
                "--file".to_owned(),
                staging.join("Dockerfile").to_string_lossy().into_owned(),
                staging.to_string_lossy().into_owned(),
            ],
            20 * 60,
        );
        let _ = fs::remove_dir_all(&staging);
        result?;
        Ok(vrising::RUNTIME_IMAGE.to_owned())
    }

    fn ensure_valheim_runtime_image<F>(&self, progress: &mut F) -> Result<String, String>
    where
        F: FnMut(&str, u8),
    {
        self.ensure_bundled_runtime_image(
            valheim::RUNTIME_IMAGE,
            valheim::DOCKERFILE,
            valheim::ENTRYPOINT,
            "valheim-runtime",
            "Building the isolated Valheim runtime image (one-time)",
            progress,
        )
    }

    fn ensure_terraria_runtime_image<F>(&self, progress: &mut F) -> Result<String, String>
    where
        F: FnMut(&str, u8),
    {
        self.ensure_bundled_runtime_image(
            terraria::RUNTIME_IMAGE,
            terraria::DOCKERFILE,
            terraria::ENTRYPOINT,
            "terraria-runtime",
            "Building the isolated Terraria runtime image (one-time)",
            progress,
        )
    }

    fn ensure_bundled_runtime_image<F>(
        &self,
        image: &str,
        dockerfile: &str,
        entrypoint: &str,
        staging_name: &str,
        progress_label: &str,
        progress: &mut F,
    ) -> Result<String, String>
    where
        F: FnMut(&str, u8),
    {
        if self.docker(["image", "inspect", image], 20).is_ok() {
            return Ok(image.to_owned());
        }
        progress(progress_label, 16);
        let staging = self.state_root.join(".staging").join(staging_name);
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)
            .map_err(|_| format!("could not stage the {staging_name} build"))?;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))
            .map_err(|_| format!("could not protect the {staging_name} build"))?;
        write_new_file(&staging.join("Dockerfile"), dockerfile.as_bytes(), 0o600)?;
        write_new_file(&staging.join("entrypoint.sh"), entrypoint.as_bytes(), 0o755)?;
        let result = self.docker_owned(
            &[
                "build".to_owned(),
                "--tag".to_owned(),
                image.to_owned(),
                "--file".to_owned(),
                staging.join("Dockerfile").to_string_lossy().into_owned(),
                staging.to_string_lossy().into_owned(),
            ],
            20 * 60,
        );
        let _ = fs::remove_dir_all(&staging);
        result?;
        Ok(image.to_owned())
    }

    fn reclaim_unused_runtime(&self, kind: GameKind) {
        let image = match kind {
            GameKind::VRising => vrising::RUNTIME_IMAGE,
            GameKind::Valheim => valheim::RUNTIME_IMAGE,
            GameKind::Terraria => terraria::RUNTIME_IMAGE,
            GameKind::Minecraft => return,
        };
        let remaining = self
            .load_manifests()
            .ok()
            .is_some_and(|manifests| manifests.iter().any(|manifest| manifest.kind == kind));
        if remaining {
            return;
        }
        let _ = self.docker(["rmi", "--force", image], 60);
    }

    fn container_restart_policy(&self, name: &str) -> Result<String, String> {
        let policy = self.docker(
            [
                "inspect",
                "--format",
                "{{.HostConfig.RestartPolicy.Name}}",
                name,
            ],
            20,
        )?;
        Ok(policy.trim().to_owned())
    }

    fn rollback_creation(
        &self,
        id: &str,
        container_name: &str,
        data_path: &Path,
        manifest_path: &Path,
        container_create_attempted: bool,
    ) -> Result<(), String> {
        validate_id(id)?;
        if container_name != format!("helix-game-{id}") {
            return Err("the failed workload identity is invalid".to_owned());
        }
        if container_create_attempted {
            if let Some((managed, instance_id)) = self.exact_container_identity(container_name)?
                && (managed != "true" || instance_id != id)
            {
                return Err(
                    "the failed workload name belongs to a container Helix cannot prove it owns"
                        .to_owned(),
                );
            }
            let removal_error = self.docker(["rm", "--force", container_name], 60).err();
            let remaining = self.exact_container_identity(container_name)?;
            if let Some((managed, instance_id)) = remaining {
                return Err(if managed == "true" && instance_id == id {
                    removal_error.unwrap_or_else(|| {
                        "the exact failed Helix container still exists after removal".to_owned()
                    })
                } else {
                    "the failed workload name belongs to a container Helix cannot prove it owns"
                        .to_owned()
                });
            }
        }
        if manifest_path.exists() {
            fs::remove_file(manifest_path)
                .map_err(|_| "could not remove the incomplete server manifest".to_owned())?;
            sync_directory(&self.state_root)?;
        }
        if data_path.exists() {
            let recovery = self
                .instance_root
                .join(".failed")
                .join(format!("{id}-{}", now_unix_ms()));
            fs::rename(data_path, recovery)
                .map_err(|_| "could not move the incomplete files into recovery".to_owned())?;
            sync_directory(&self.instance_root.join(".failed"))?;
            sync_directory(&self.instance_root)?;
        }
        Ok(())
    }

    fn exact_container_identity(
        &self,
        container_name: &str,
    ) -> Result<Option<(String, String)>, String> {
        let name_filter = format!("name=^/{container_name}$");
        let output = self.docker(
            [
                "container",
                "ls",
                "--all",
                "--filter",
                name_filter.as_str(),
                "--format",
                r#"{{.Names}}|{{.Label "io.helix.managed"}}|{{.Label "io.helix.instance"}}"#,
            ],
            30,
        )?;
        let mut identity = None;
        for line in output.lines() {
            let fields = line.split('|').collect::<Vec<_>>();
            if fields.first().copied() != Some(container_name) {
                continue;
            }
            if fields.len() != 3 || identity.is_some() {
                return Err("the failed workload identity could not be verified".to_owned());
            }
            identity = Some((fields[1].to_owned(), fields[2].to_owned()));
        }
        Ok(identity)
    }

    fn runtime_states(&self, manifests: &[InstanceManifest]) -> HashMap<String, RuntimeState> {
        let mut states = HashMap::new();
        if let Ok(output) = self.docker(
            [
                "ps",
                "--all",
                "--filter",
                "label=io.helix.managed=true",
                "--format",
                "{{.Names}}|{{.State}}",
            ],
            30,
        ) {
            for line in output.lines() {
                if let Some((name, state)) = line.split_once('|') {
                    states.insert(
                        name.to_owned(),
                        RuntimeState {
                            running: state == "running",
                            ..RuntimeState::default()
                        },
                    );
                }
            }
        }
        let running = manifests
            .iter()
            .filter(|manifest| {
                states
                    .get(&manifest.container_name)
                    .is_some_and(|state| state.running)
            })
            .map(|manifest| manifest.container_name.clone())
            .collect::<Vec<_>>();
        if !running.is_empty() {
            let mut args = vec![
                "stats".to_owned(),
                "--no-stream".to_owned(),
                "--format".to_owned(),
                "{{.Name}}|{{.CPUPerc}}|{{.MemUsage}}".to_owned(),
            ];
            args.extend(running);
            if let Ok(output) = self.docker_owned(&args, 45) {
                for line in output.lines() {
                    let mut columns = line.split('|');
                    let Some(name) = columns.next() else {
                        continue;
                    };
                    let cpu = columns
                        .next()
                        .and_then(|value| value.trim_end_matches('%').parse().ok())
                        .unwrap_or(0.0);
                    let memory = columns
                        .next()
                        .and_then(|value| value.split('/').next())
                        .and_then(parse_human_bytes)
                        .map(|bytes| bytes / 1024 / 1024)
                        .unwrap_or(0);
                    if let Some(state) = states.get_mut(name) {
                        state.cpu_percent = cpu;
                        state.memory_used_mb = memory;
                    }
                }
            }
        }
        states
    }

    fn container_running(&self, name: &str) -> bool {
        self.docker(["inspect", "--format", "{{.State.Running}}", name], 20)
            .is_ok_and(|output| output.trim() == "true")
    }

    fn load_manifests(&self) -> Result<Vec<InstanceManifest>, String> {
        let mut manifests = Vec::new();
        let entries = fs::read_dir(&self.state_root)
            .map_err(|_| "could not read the native instance registry".to_owned())?;
        for entry in entries {
            let entry = entry.map_err(|_| "could not read a native instance entry".to_owned())?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            match read_manifest(&path) {
                Ok(manifest) if manifest.schema_version == MANIFEST_VERSION => {
                    manifests.push(manifest)
                }
                Ok(_) | Err(_) => {}
            }
            if manifests.len() >= 512 {
                break;
            }
        }
        Ok(manifests)
    }

    fn load_manifest(&self, id: &str) -> Result<InstanceManifest, String> {
        validate_id(id)?;
        let manifest = read_manifest(&self.manifest_path(id)?)?;
        if manifest.schema_version != MANIFEST_VERSION || manifest.id != id {
            return Err("the selected Helix server definition is invalid".to_owned());
        }
        Ok(manifest)
    }

    fn manifest_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.state_root.join(format!("{id}.json")))
    }

    fn instance_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.instance_root.join(id))
    }

    fn backup_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.backup_root.join(id))
    }

    fn backup_trash_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.backup_root.join(".trash").join(id))
    }

    fn server_trash_root(&self) -> PathBuf {
        self.state_root.join("server-trash")
    }

    fn server_trash_record_path(&self, trash_id: &str) -> Result<PathBuf, String> {
        validate_trash_id(trash_id)?;
        Ok(self.server_trash_root().join(trash_id))
    }

    fn server_trash_data_path(&self, trash_id: &str) -> Result<PathBuf, String> {
        validate_trash_id(trash_id)?;
        Ok(self.instance_root.join(".trash").join(trash_id))
    }

    fn console_archive_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.state_root.join("console").join(id))
    }

    fn overlay_migrated_game(
        &self,
        kind: GameKind,
        source: &Path,
        data_path: &Path,
        copy_server_jar: bool,
        run_uid: u32,
    ) -> Result<migrate_plan::OverlayReport, String> {
        let report = migrate_plan::apply_overlay(kind, source, data_path, copy_server_jar)?;
        match kind {
            GameKind::Terraria => {
                let _ = migrate_plan::ensure_named_save(
                    &data_path.join("worlds"),
                    "world",
                    "wld",
                    &["wld.bak"],
                )?;
            }
            GameKind::Valheim => {
                if migrate_plan::ensure_named_save(
                    &data_path.join("worlds_local"),
                    "Dedicated",
                    "fwl",
                    &["db", "db.old"],
                )?
                .is_none()
                {
                    let _ = migrate_plan::ensure_named_save(
                        &data_path.join("worlds"),
                        "Dedicated",
                        "fwl",
                        &["db", "db.old"],
                    )?;
                }
            }
            GameKind::Minecraft | GameKind::VRising => {}
        }
        self.chown_instance(data_path, run_uid)?;
        Ok(report)
    }

    pub(crate) fn stage_migrate_jar(
        &self,
        source_jar: &Path,
    ) -> Result<CustomMinecraftJarSpec, String> {
        let metadata = fs::symlink_metadata(source_jar)
            .map_err(|_| "the source server JAR is unavailable".to_owned())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("the source server JAR must be a regular file".to_owned());
        }
        if metadata.len() == 0 || metadata.len() > MAX_SERVER_JAR_BYTES {
            return Err("the source server JAR is empty or larger than 768 MiB".to_owned());
        }
        let name = format!("migrate-{}.jar", Uuid::new_v4().simple());
        let destination = self.custom_import_root.join(name);
        fs::copy(source_jar, &destination)
            .map_err(|_| "could not stage the source server JAR".to_owned())?;
        Ok(CustomMinecraftJarSpec {
            source_path: destination.to_string_lossy().into_owned(),
            java_version: 21,
        })
    }

    fn chown_instance(&self, path: &Path, uid: u32) -> Result<(), String> {
        run_program(
            Path::new("/usr/bin/chown"),
            &[
                "--recursive".to_owned(),
                format!("{uid}:{uid}"),
                path.to_string_lossy().into_owned(),
            ],
            60,
        )?;
        Ok(())
    }

    fn protect_instance_artifacts(&self, path: &Path, uid: u32) -> Result<(), String> {
        for (name, mode) in [
            ("pumpkin", 0o550),
            ("pumpkin.toml", 0o660),
            ("server.jar", 0o440),
            ("server.properties", 0o660),
            ("eula.txt", 0o440),
            (MODPACK_LOCK_FILE, 0o440),
        ] {
            let artifact = path.join(name);
            if !artifact.is_file() {
                continue;
            }
            run_program(
                Path::new("/usr/bin/chown"),
                &[format!("0:{uid}"), artifact.to_string_lossy().into_owned()],
                20,
            )?;
            fs::set_permissions(&artifact, fs::Permissions::from_mode(mode))
                .map_err(|_| format!("could not protect {}", artifact.display()))?;
        }
        Ok(())
    }

    fn curl_no_redirect(
        &self,
        url: &str,
        destination: &Path,
        maximum_bytes: u64,
        maximum_seconds: u64,
    ) -> Result<(), String> {
        self.curl_no_redirect_headers(url, destination, maximum_bytes, maximum_seconds, &[])
    }

    fn curl_no_redirect_headers(
        &self,
        url: &str,
        destination: &Path,
        maximum_bytes: u64,
        maximum_seconds: u64,
        extra_headers: &[(&str, &str)],
    ) -> Result<(), String> {
        let cache = self.state_root.join("metadata");
        fs::create_dir_all(&cache).map_err(|_| "could not create the metadata cache".to_owned())?;
        let header_path = if extra_headers.is_empty() {
            None
        } else {
            let body = curl_extra_header_file(extra_headers)?;
            let path = cache.join(format!("curl-{}.hdr", Uuid::new_v4()));
            write_replaced_secret_file(&path, body.as_bytes())?;
            Some(path)
        };
        let dump_path = if extra_headers.is_empty() {
            None
        } else {
            Some(cache.join(format!("curl-{}.dmp", Uuid::new_v4())))
        };
        let mut args = vec![
            "--disable".to_owned(),
            if extra_headers.is_empty() {
                "--fail".to_owned()
            } else {
                "--fail-with-body".to_owned()
            },
            "--silent".to_owned(),
            "--show-error".to_owned(),
            "--proto".to_owned(),
            "=https".to_owned(),
            "--tlsv1.2".to_owned(),
            "--connect-timeout".to_owned(),
            "10".to_owned(),
            "--max-time".to_owned(),
            maximum_seconds.to_string(),
            "--max-filesize".to_owned(),
            maximum_bytes.to_string(),
            "--header".to_owned(),
            format!("User-Agent: {USER_AGENT}"),
            "--header".to_owned(),
            "Accept: application/json, application/octet-stream".to_owned(),
        ];
        if let Some(path) = &header_path {
            args.push("--header".to_owned());
            args.push(format!("@{}", path.display()));
        }
        if let Some(path) = &dump_path {
            args.push("--dump-header".to_owned());
            args.push(path.to_string_lossy().into_owned());
        }
        args.push("--output".to_owned());
        args.push(destination.to_string_lossy().into_owned());
        args.push(url.to_owned());
        let result = run_program(
            Path::new("/usr/bin/curl"),
            &args,
            maximum_seconds.saturating_add(15),
        );
        if let Some(path) = header_path {
            let _ = fs::remove_file(path);
        }
        let classified = if extra_headers.is_empty() {
            result.map(|_| ()).map_err(map_download_curl_error)
        } else {
            let dump = dump_path
                .as_ref()
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            let body = fs::read(destination).unwrap_or_default();
            match result {
                Ok(_) => Ok(()),
                Err(error) => Err(classify_curseforge_curl_error(&error, &dump, &body)),
            }
        };
        if let Some(path) = dump_path {
            let _ = fs::remove_file(path);
        }
        classified
    }

    fn file_sha1(&self, path: &Path) -> Result<String, String> {
        let output = run_program(
            Path::new("/usr/bin/sha1sum"),
            &[path.to_string_lossy().into_owned()],
            60,
        )?;
        output
            .split_whitespace()
            .next()
            .filter(|hash| valid_hex(hash, 40))
            .map(str::to_owned)
            .ok_or_else(|| "could not verify the Mojang server checksum".to_owned())
    }

    fn docker<'a>(
        &self,
        args: impl IntoIterator<Item = &'a str>,
        timeout_seconds: u64,
    ) -> Result<String, String> {
        let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        self.docker_owned(&args, timeout_seconds)
    }

    fn docker_owned(&self, args: &[String], timeout_seconds: u64) -> Result<String, String> {
        let home = prepare_docker_cli_home(&self.state_root)?;
        let cache = home.join("cache");
        let buildx = home.join("buildx");
        let tmp = home.join("tmp");
        let home_s = home.to_string_lossy().into_owned();
        let cache_s = cache.to_string_lossy().into_owned();
        let buildx_s = buildx.to_string_lossy().into_owned();
        let tmp_s = tmp.to_string_lossy().into_owned();
        run_program_with_env(
            &self.docker_binary,
            args,
            timeout_seconds,
            &[
                ("HOME", home_s.as_str()),
                ("DOCKER_CONFIG", home_s.as_str()),
                ("XDG_CACHE_HOME", cache_s.as_str()),
                ("XDG_CONFIG_HOME", home_s.as_str()),
                ("BUILDX_CONFIG", buildx_s.as_str()),
                ("TMPDIR", tmp_s.as_str()),
            ],
        )
        .map_err(|error| format!("the Helix execution backend failed: {error}"))
    }
}

fn collect_minecraft_statuses<Probe>(
    targets: &[(String, u16)],
    probe: Probe,
) -> HashMap<String, MinecraftStatus>
where
    Probe: Fn(u16) -> Option<MinecraftStatus> + Sync,
{
    if targets.is_empty() {
        return HashMap::new();
    }
    let next = AtomicUsize::new(0);
    let results = Mutex::new(HashMap::with_capacity(targets.len()));
    let workers = targets.len().min(MAX_MINECRAFT_STATUS_WORKERS);
    thread::scope(|scope| {
        for _ in 0..workers {
            let probe = &probe;
            let next = &next;
            let results = &results;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((id, port)) = targets.get(index) else {
                        break;
                    };
                    let Some(status) = probe(*port) else {
                        continue;
                    };
                    if let Ok(mut results) = results.lock() {
                        results.insert(id.clone(), status);
                    }
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn minecraft_rcon_targets(
    manifests: &[InstanceManifest],
    states: &HashMap<String, RuntimeState>,
) -> Vec<(String, u16, String)> {
    manifests
        .iter()
        .filter(|manifest| {
            manifest.is_minecraft()
                && manifest.rcon_port != 0
                && !manifest.rcon_password.is_empty()
                && states
                    .get(&manifest.container_name)
                    .is_some_and(|state| state.running)
        })
        .map(|manifest| {
            (
                manifest.id.clone(),
                manifest.rcon_port,
                manifest.rcon_password.clone(),
            )
        })
        .collect()
}

fn collect_minecraft_tps<Probe>(
    targets: &[(String, u16, String)],
    probe: Probe,
) -> HashMap<String, TpsProbeResult>
where
    Probe: Fn(u16, &str) -> TpsProbeResult + Sync,
{
    if targets.is_empty() {
        return HashMap::new();
    }
    let next = AtomicUsize::new(0);
    let results = Mutex::new(HashMap::with_capacity(targets.len()));
    let workers = targets.len().min(MAX_MINECRAFT_STATUS_WORKERS);
    thread::scope(|scope| {
        for _ in 0..workers {
            let probe = &probe;
            let next = &next;
            let results = &results;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((id, port, password)) = targets.get(index) else {
                        break;
                    };
                    let sample = probe(*port, password);
                    if let Ok(mut results) = results.lock() {
                        results.insert(id.clone(), sample);
                    }
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn probe_minecraft_tps(port: u16, password: &str) -> TpsProbeResult {
    match rcon_command_timed(
        port,
        password,
        "tps",
        MINECRAFT_TPS_CONNECT_TIMEOUT,
        MINECRAFT_TPS_IO_TIMEOUT,
    ) {
        Ok(response) => {
            let tps = parse_tps_response(&response);
            TpsProbeResult {
                tps,
                ttl: if tps.is_some() {
                    MINECRAFT_TPS_CACHE_HIT
                } else {
                    MINECRAFT_TPS_CACHE_MISS
                },
            }
        }
        Err(_) => TpsProbeResult {
            tps: None,
            ttl: MINECRAFT_TPS_CACHE_ERROR,
        },
    }
}

fn strip_minecraft_formatting(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '§' && index + 1 < chars.len() {
            if chars[index + 1] == 'x' || chars[index + 1] == 'X' {
                index += 2;
                for _ in 0..6 {
                    if index + 1 < chars.len() && chars[index] == '§' {
                        index += 2;
                    } else {
                        break;
                    }
                }
                continue;
            }
            index += 2;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn first_tps_number(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'*' {
            index += 1;
            continue;
        }
        if bytes[index].is_ascii_digit() {
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            let mut has_fraction = false;
            if index < bytes.len() && bytes[index] == b'.' {
                let fraction = index + 1;
                if fraction < bytes.len() && bytes[fraction].is_ascii_digit() {
                    has_fraction = true;
                    index = fraction;
                    while index < bytes.len() && bytes[index].is_ascii_digit() {
                        index += 1;
                    }
                }
            }
            if has_fraction
                && let Ok(value) = text[start..index].parse::<f64>()
                && (0.0..=40.0).contains(&value)
            {
                return Some(value);
            }
            continue;
        }
        index += 1;
    }
    None
}

fn parse_tps_response(text: &str) -> Option<f64> {
    let stripped = strip_minecraft_formatting(text);
    let lower = stripped.to_ascii_lowercase();
    if lower.contains("unknown command")
        || lower.contains("unknown or incomplete command")
        || lower.contains("incorrect argument")
    {
        return None;
    }
    let offset = lower.find("tps")?;
    first_tps_number(&lower[offset..])
}

impl Clone for RuntimeState {
    fn clone(&self) -> Self {
        Self {
            running: self.running,
            cpu_percent: self.cpu_percent,
            memory_used_mb: self.memory_used_mb,
        }
    }
}

fn default_docker_binary() -> PathBuf {
    PathBuf::from("/usr/bin/docker")
}

pub(crate) fn prepare_docker_cli_home(state_root: &Path) -> Result<PathBuf, String> {
    let home = state_root.join(".docker-cli");
    for dir in [
        home.clone(),
        home.join("cache"),
        home.join("buildx"),
        home.join("buildx").join("activity"),
        home.join("tmp"),
    ] {
        fs::create_dir_all(&dir)
            .map_err(|_| "could not create the Docker CLI state directory".to_owned())?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the Docker CLI state directory".to_owned())?;
    }
    Ok(home)
}

const fn default_console_history_max_bytes() -> u64 {
    512 * 1024 * 1024
}

const fn default_console_history_files() -> u16 {
    32
}

const fn default_backup_trash_retention_days() -> u16 {
    30
}

fn console_segment_path(root: &Path, sequence: u64) -> PathBuf {
    root.join(format!("{sequence:010}.log"))
}

fn console_segment_sequence(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    let sequence = name.strip_suffix(".log")?;
    if sequence.len() != 10 || !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    sequence.parse().ok()
}

fn console_segment_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(root).map_err(|_| "could not read the server console archive".to_owned())?
    {
        let entry = entry.map_err(|_| "could not read a console archive entry".to_owned())?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "could not inspect a console archive entry".to_owned())?;
        if metadata.file_type().is_file() && console_segment_sequence(&path).is_some() {
            paths.push(path);
        }
        if paths.len() > 512 {
            return Err("the server console archive has too many segments".to_owned());
        }
    }
    paths.sort_by_key(|path| console_segment_sequence(path).unwrap_or(0));
    Ok(paths)
}

fn read_console_tail(root: &Path, maximum_lines: usize) -> Result<Vec<String>, String> {
    if maximum_lines == 0 {
        return Ok(Vec::new());
    }
    let paths = console_segment_paths(root)?;
    let mut newest_first = Vec::with_capacity(maximum_lines.min(1_000));
    let mut text_bytes = 0_usize;
    'segments: for path in paths.iter().rev() {
        let remaining = maximum_lines.saturating_sub(newest_first.len());
        if remaining == 0 {
            break;
        }
        for line in read_file_tail_reversed(path, remaining)? {
            let next = text_bytes.saturating_add(line.len());
            if next > MAX_CONSOLE_HISTORY_PAGE_TEXT_BYTES && !newest_first.is_empty() {
                break 'segments;
            }
            text_bytes = next;
            newest_first.push(line);
        }
    }
    newest_first.reverse();
    Ok(newest_first)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConsoleHistoryCursor {
    sequence: u64,
    offset: u64,
}

#[derive(Debug)]
struct ConsoleHistoryPage {
    lines: Vec<String>,
    next_cursor: Option<String>,
}

fn read_console_history_page(
    root: &Path,
    cursor: Option<&str>,
    maximum_lines: usize,
) -> Result<ConsoleHistoryPage, String> {
    if maximum_lines == 0 || maximum_lines > 500 {
        return Err("console history pages must contain between 1 and 500 lines".to_owned());
    }
    let paths = console_segment_paths(root)?;
    if paths.is_empty() {
        if cursor.is_some() {
            return Err(
                "the console history cursor expired because retained data changed".to_owned(),
            );
        }
        return Ok(ConsoleHistoryPage {
            lines: Vec::new(),
            next_cursor: None,
        });
    }

    let (mut path_index, mut position) = if let Some(encoded) = cursor {
        let decoded = decode_console_history_cursor(encoded)?;
        let index = paths
            .iter()
            .position(|path| console_segment_sequence(path) == Some(decoded.sequence))
            .ok_or_else(|| {
                "the console history cursor expired because retained data changed".to_owned()
            })?;
        let length = regular_file_length(&paths[index])?;
        if decoded.offset > length
            || !console_history_cursor_is_boundary(&paths[index], decoded.offset, length)?
        {
            return Err("the console history cursor is invalid".to_owned());
        }
        (index, decoded.offset)
    } else {
        let index = paths.len() - 1;
        (index, regular_file_length(&paths[index])?)
    };

    let mut newest_first = Vec::with_capacity(maximum_lines);
    let mut page_text_bytes = 0_usize;
    let mut page_full = false;
    loop {
        let mut file = File::open(&paths[path_index])
            .map_err(|_| "could not open a console history segment".to_owned())?;
        while newest_first.len() < maximum_lines {
            let previous_position = position;
            let Some(line) = read_previous_console_line(&mut file, &mut position)? else {
                break;
            };
            let next_page_bytes = page_text_bytes.saturating_add(line.len());
            if next_page_bytes > MAX_CONSOLE_HISTORY_PAGE_TEXT_BYTES && !newest_first.is_empty() {
                position = previous_position;
                page_full = true;
                break;
            }
            page_text_bytes = next_page_bytes;
            newest_first.push(line);
        }
        if newest_first.len() == maximum_lines || page_full {
            break;
        }
        if path_index == 0 {
            break;
        }
        path_index -= 1;
        position = regular_file_length(&paths[path_index])?;
    }

    let next_cursor = next_console_history_cursor(&paths, path_index, position)?;
    newest_first.reverse();
    Ok(ConsoleHistoryPage {
        lines: newest_first,
        next_cursor,
    })
}

fn console_history_cursor_is_boundary(
    path: &Path,
    offset: u64,
    file_length: u64,
) -> Result<bool, String> {
    if offset == 0 || offset == file_length {
        return Ok(true);
    }
    let mut file =
        File::open(path).map_err(|_| "could not open a console history segment".to_owned())?;
    file.seek(SeekFrom::Start(offset - 1))
        .map_err(|_| "could not seek in console history".to_owned())?;
    let mut preceding = [0_u8; 1];
    file.read_exact(&mut preceding)
        .map_err(|_| "could not read console history".to_owned())?;
    Ok(preceding[0] == b'\n')
}

fn next_console_history_cursor(
    paths: &[PathBuf],
    mut path_index: usize,
    mut position: u64,
) -> Result<Option<String>, String> {
    loop {
        if position > 0 {
            let sequence = console_segment_sequence(&paths[path_index])
                .ok_or_else(|| "a console history segment is invalid".to_owned())?;
            return Ok(Some(encode_console_history_cursor(ConsoleHistoryCursor {
                sequence,
                offset: position,
            })));
        }
        if path_index == 0 {
            return Ok(None);
        }
        path_index -= 1;
        position = regular_file_length(&paths[path_index])?;
    }
}

fn regular_file_length(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "could not inspect a console history segment".to_owned())?;
    if !metadata.file_type().is_file() {
        return Err("a console history segment is invalid".to_owned());
    }
    Ok(metadata.len())
}

fn read_previous_console_line(
    file: &mut File,
    position: &mut u64,
) -> Result<Option<String>, String> {
    if *position == 0 {
        return Ok(None);
    }
    let mut line_end = *position;
    file.seek(SeekFrom::Start(line_end - 1))
        .map_err(|_| "could not seek in console history".to_owned())?;
    let mut trailing = [0_u8; 1];
    file.read_exact(&mut trailing)
        .map_err(|_| "could not read console history".to_owned())?;
    if trailing[0] == b'\n' {
        line_end -= 1;
    }

    let mut search_end = line_end;
    let mut line_start = 0_u64;
    while search_end > 0 {
        let chunk_size = search_end.min(64 * 1024);
        let chunk_start = search_end - chunk_size;
        file.seek(SeekFrom::Start(chunk_start))
            .map_err(|_| "could not seek in console history".to_owned())?;
        let mut chunk = vec![0_u8; usize::try_from(chunk_size).unwrap_or(0)];
        file.read_exact(&mut chunk)
            .map_err(|_| "could not read console history".to_owned())?;
        if let Some(index) = chunk.iter().rposition(|byte| *byte == b'\n') {
            line_start = chunk_start
                .saturating_add(u64::try_from(index).unwrap_or(u64::MAX))
                .saturating_add(1);
            break;
        }
        if line_end.saturating_sub(chunk_start)
            > u64::try_from(MAX_CONSOLE_LINE_BYTES.saturating_add(128)).unwrap_or(u64::MAX)
        {
            return Err("a console history line is too large".to_owned());
        }
        search_end = chunk_start;
    }

    let line_length = line_end.saturating_sub(line_start);
    if line_length > u64::try_from(MAX_CONSOLE_LINE_BYTES.saturating_add(128)).unwrap_or(u64::MAX) {
        return Err("a console history line is too large".to_owned());
    }
    file.seek(SeekFrom::Start(line_start))
        .map_err(|_| "could not seek in console history".to_owned())?;
    let mut line = vec![0_u8; usize::try_from(line_length).unwrap_or(usize::MAX)];
    file.read_exact(&mut line)
        .map_err(|_| "could not read console history".to_owned())?;
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    *position = line_start;
    Ok(Some(String::from_utf8_lossy(&line).into_owned()))
}

fn encode_console_history_cursor(cursor: ConsoleHistoryCursor) -> String {
    format!("h1.{:016x}.{:016x}", cursor.sequence, cursor.offset)
}

fn decode_console_history_cursor(value: &str) -> Result<ConsoleHistoryCursor, String> {
    let mut fields = value.split('.');
    let version = fields.next();
    let sequence = fields.next();
    let offset = fields.next();
    if version != Some("h1")
        || fields.next().is_some()
        || sequence.is_none_or(|value| value.len() != 16)
        || offset.is_none_or(|value| value.len() != 16)
    {
        return Err("the console history cursor is invalid".to_owned());
    }
    let sequence = u64::from_str_radix(sequence.unwrap_or_default(), 16)
        .map_err(|_| "the console history cursor is invalid".to_owned())?;
    let offset = u64::from_str_radix(offset.unwrap_or_default(), 16)
        .map_err(|_| "the console history cursor is invalid".to_owned())?;
    Ok(ConsoleHistoryCursor { sequence, offset })
}

fn console_history_entry(line: &str) -> Value {
    let docker_timestamp = docker_log_timestamp(line);
    let helix_timestamp = line
        .strip_prefix("[helix ")
        .and_then(|value| value.split_once(']'))
        .and_then(|(timestamp, _)| timestamp.parse::<u64>().ok());
    let boot_started_at = line
        .split_once("---- Minecraft boot ")
        .and_then(|(_, value)| value.split_once(" (").map(|(started, _)| started))
        .filter(|value| !value.is_empty() && value.len() <= 64);
    let kind = if boot_started_at.is_some() {
        "boot"
    } else if line.starts_with("[helix ") && line.contains("] >") {
        "command"
    } else if line.starts_with("[helix ") && line.contains("] <") {
        "command_response"
    } else {
        "output"
    };
    json!({
        "kind": kind,
        "text": line,
        "timestamp": docker_timestamp,
        "timestamp_unix_ms": helix_timestamp,
        "boot_started_at": boot_started_at
    })
}

fn read_file_tail_reversed(path: &Path, maximum_lines: usize) -> Result<Vec<String>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "could not inspect a console history segment".to_owned())?;
    if !metadata.file_type().is_file() {
        return Err("a console history segment is invalid".to_owned());
    }
    let mut file =
        File::open(path).map_err(|_| "could not open a console history segment".to_owned())?;
    let mut position = metadata.len();
    let mut carry = Vec::new();
    let mut lines = Vec::with_capacity(maximum_lines.min(1_000));
    while position > 0 && lines.len() < maximum_lines {
        let chunk_size = position.min(64 * 1024);
        let start = position - chunk_size;
        file.seek(SeekFrom::Start(start))
            .map_err(|_| "could not seek in console history".to_owned())?;
        let mut chunk = vec![0_u8; usize::try_from(chunk_size).unwrap_or(0)];
        file.read_exact(&mut chunk)
            .map_err(|_| "could not read console history".to_owned())?;
        chunk.extend_from_slice(&carry);
        let mut end = chunk.len();
        while lines.len() < maximum_lines {
            let Some(index) = chunk[..end].iter().rposition(|byte| *byte == b'\n') else {
                break;
            };
            if index + 1 < end {
                lines.push(String::from_utf8_lossy(&chunk[index + 1..end]).into_owned());
            }
            end = index;
        }
        carry = chunk[..end].to_vec();
        if carry.len() > MAX_CONSOLE_LINE_BYTES.saturating_add(128) {
            return Err("a console history line is too large".to_owned());
        }
        position = start;
    }
    if position == 0 && !carry.is_empty() && lines.len() < maximum_lines {
        lines.push(String::from_utf8_lossy(&carry).into_owned());
    }
    Ok(lines)
}

fn docker_log_timestamp(line: &str) -> Option<&str> {
    let timestamp = line.split_whitespace().next()?;
    let bytes = timestamp.as_bytes();
    if !(20..=40).contains(&bytes.len())
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || !timestamp.ends_with('Z')
    {
        return None;
    }
    Some(timestamp)
}

fn latest_docker_log(root: &Path) -> Result<Option<(String, String)>, String> {
    Ok(read_console_tail(root, 1_000)?
        .into_iter()
        .rev()
        .find_map(|line| {
            let timestamp = docker_log_timestamp(&line)?.to_owned();
            Some((timestamp, line))
        }))
}

fn console_archiver_loop(config: ConsoleArchiverConfig, archive: Arc<Mutex<ConsoleArchiveWriter>>) {
    while !config.stop.load(Ordering::Acquire) {
        record_boot_marker(&config, &archive);
        let latest = latest_docker_log(&config.archive_root).ok().flatten();
        let since = latest
            .as_ref()
            .map_or_else(|| "0".to_owned(), |(timestamp, _)| timestamp.clone());
        let capture_started = Instant::now();
        let mut child = match Command::new(&config.docker_binary)
            .args([
                "logs",
                "--follow",
                "--timestamps",
                "--since",
                since.as_str(),
                config.container_name.as_str(),
            ])
            .env("HOME", &config.docker_cli_home)
            .env("DOCKER_CONFIG", &config.docker_cli_home)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => {
                thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        let duplicate = Arc::new(Mutex::new(latest.map(|(_, line)| line)));
        let stderr_worker = child.stderr.take().map(|stderr| {
            let archive = Arc::clone(&archive);
            let duplicate = Arc::clone(&duplicate);
            thread::Builder::new()
                .name("console-stderr".to_owned())
                .stack_size(128 * 1024)
                .spawn(move || archive_console_stream(stderr, &archive, &duplicate))
        });
        if let Some(stdout) = child.stdout.take() {
            archive_console_stream(stdout, &archive, &duplicate);
        }
        let _ = child.wait();
        if let Some(Ok(worker)) = stderr_worker {
            let _ = worker.join();
        }
        if config.stop.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(if capture_started.elapsed() < Duration::from_secs(10) {
            Duration::from_secs(10)
        } else {
            Duration::from_secs(2)
        });
    }
}

fn is_rcon_session_noise(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("[rcon: tps from last") {
        return true;
    }
    lower.contains("thread rcon client")
        && (lower.contains(" started") || lower.contains("shutting down"))
}

fn archive_console_stream(
    input: impl std::io::Read,
    archive: &Arc<Mutex<ConsoleArchiveWriter>>,
    duplicate: &Arc<Mutex<Option<String>>>,
) {
    let mut reader = BufReader::new(input);
    let mut buffer = Vec::new();
    loop {
        match read_bounded_line(
            &mut reader,
            &mut buffer,
            MAX_CONSOLE_LINE_BYTES.saturating_add(128),
        ) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&buffer)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        if docker_log_timestamp(&line).is_none() {
            continue;
        }
        let repeated = duplicate
            .lock()
            .ok()
            .and_then(|mut previous| {
                if previous.as_deref() == Some(&line) {
                    previous.take()
                } else {
                    None
                }
            })
            .is_some();
        if repeated {
            continue;
        }
        if is_rcon_session_noise(&line) {
            continue;
        }
        if let Ok(mut archive) = archive.lock()
            && let Err(error) = archive.append(&line)
        {
            eprintln!("persistent console capture failed: {error}");
        }
    }
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    buffer: &mut Vec<u8>,
    maximum_bytes: usize,
) -> std::io::Result<usize> {
    buffer.clear();
    let retained_limit = maximum_bytes;
    let mut consumed_total = 0_usize;
    loop {
        let (consumed, line_complete) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(consumed_total);
            }
            let line_complete = available.iter().position(|byte| *byte == b'\n');
            let consumed = line_complete.map_or(available.len(), |index| index.saturating_add(1));
            let retained = retained_limit.saturating_sub(buffer.len()).min(consumed);
            buffer.extend_from_slice(&available[..retained]);
            (consumed, line_complete.is_some())
        };
        reader.consume(consumed);
        consumed_total = consumed_total.saturating_add(consumed);
        if line_complete {
            return Ok(consumed_total);
        }
    }
}

fn record_boot_marker(config: &ConsoleArchiverConfig, archive: &Arc<Mutex<ConsoleArchiveWriter>>) {
    let started_at = run_program(
        &config.docker_binary,
        &[
            "inspect".to_owned(),
            "--format".to_owned(),
            "{{.State.StartedAt}}".to_owned(),
            config.container_name.clone(),
        ],
        20,
    )
    .ok()
    .map(|value| value.trim().to_owned())
    .filter(|value| docker_log_timestamp(&format!("{value} value")).is_some())
    .filter(|value| !value.starts_with("0001-01-01"));
    let Some(started_at) = started_at else {
        return;
    };
    let marker_path = config.archive_root.join("last-boot");
    if fs::read_to_string(&marker_path)
        .ok()
        .is_some_and(|previous| previous.trim() == started_at)
    {
        return;
    }
    if let Ok(mut archive) = archive.lock() {
        let _ = archive.append(&format!(
            "[helix {}] ---- Minecraft boot {} ({}) ----",
            now_unix_ms(),
            started_at,
            config.instance_id
        ));
    }
    let _ = write_private_text(&marker_path, &format!("{started_at}\n"));
}

fn write_private_text(path: &Path, content: &str) -> Result<(), String> {
    let temporary = path.with_extension(format!("partial.{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "could not stage private state".to_owned())?;
        file.write_all(content.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|_| "could not persist private state".to_owned())?;
        fs::rename(&temporary, path).map_err(|_| "could not commit private state".to_owned())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "could not persist the native server directory".to_owned())
}

fn validate_root(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute() || path == Path::new("/") || path.components().count() < 3 {
        return Err(format!("the {label} root must be a narrow absolute path"));
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(format!("the {label} root cannot be a symlink"));
    }
    Ok(())
}

fn validate_custom_artifact_root(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path == Path::new("/") || path.components().count() < 2 {
        return Err("the custom artifact root must be an absolute path below /".to_owned());
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err("the custom artifact root cannot be a symlink".to_owned());
    }
    Ok(())
}

pub(crate) fn custom_artifact_root_is_safe(path: &Path) -> bool {
    validate_custom_artifact_root(path).is_ok() && path.is_dir()
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical =
        fs::canonicalize(path).map_err(|_| format!("could not resolve {}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    Ok(canonical)
}

fn validate_binary(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute() || !path.is_file() {
        return Err(format!("the {label} executable is unavailable"));
    }
    Ok(())
}

fn validate_create_spec(spec: &MinecraftCreateSpec) -> Result<(), String> {
    if spec.pumpkin_bedrock_port.is_some() && !matches!(spec.software, MinecraftSoftware::Pumpkin) {
        return Err("A Bedrock port is only accepted for Pumpkin".to_owned());
    }
    let name = spec.name.trim();
    if name.is_empty()
        || name.len() > 80
        || name.chars().any(char::is_control)
        || name.contains(['/', '\\'])
    {
        return Err("server name must be 1–80 ordinary characters".to_owned());
    }
    if matches!(spec.software, MinecraftSoftware::Custom) {
        if spec.custom_jar.is_none() || spec.version.eq_ignore_ascii_case("latest") {
            return Err("custom JAR servers require a local .jar path, Java version, and explicit Minecraft version".to_owned());
        }
    } else if spec.custom_jar.is_some() {
        return Err("custom JAR details are only accepted for a custom server".to_owned());
    }
    if !spec.version.eq_ignore_ascii_case("latest") {
        if matches!(spec.software, MinecraftSoftware::Pumpkin) {
            pumpkin::release_versions(spec.version.trim())?;
        } else {
            validate_version(spec.version.trim())?;
        }
    }
    if !(1_024..=24_576).contains(&spec.memory_mb) {
        return Err("memory must be between 1 and 24 GiB".to_owned());
    }
    helix_privd::validate_cpu_millis(spec.cpu_millis)?;
    if !(1..=10_000).contains(&spec.max_players) {
        return Err("player limit must be between 1 and 10,000".to_owned());
    }
    if spec.game_port.is_some_and(|port| port < 1024) {
        return Err("game port must be at least 1024".to_owned());
    }
    if !spec.eula_accepted {
        return Err("the Minecraft EULA must be explicitly accepted".to_owned());
    }
    Ok(())
}

fn allocated_memory_bounds(kind: GameKind) -> (u32, u32) {
    match kind {
        GameKind::Minecraft => (1_024, 24_576),
        GameKind::VRising => (2_048, 24_576),
        GameKind::Valheim => (1_024, 16_384),
        GameKind::Terraria => (512, 8_192),
    }
}

fn validate_allocated_memory(kind: GameKind, memory_mb: u32) -> Result<(), String> {
    let (minimum, maximum) = allocated_memory_bounds(kind);
    if (minimum..=maximum).contains(&memory_mb) {
        return Ok(());
    }
    Err(match kind {
        GameKind::Minecraft => "memory must be between 1 and 24 GiB".to_owned(),
        GameKind::VRising => "V Rising memory must be between 2 and 24 GiB".to_owned(),
        GameKind::Valheim => "Valheim memory must be between 1 and 16 GiB".to_owned(),
        GameKind::Terraria => "Terraria memory must be between 512 MiB and 8 GiB".to_owned(),
    })
}

fn default_game_port_policy(game: GameKind) -> GamePortPolicySpec {
    match game {
        GameKind::Minecraft => GamePortPolicySpec {
            game,
            ranges: vec![GamePortRangeSpec {
                start: 25_565,
                end: 25_599,
            }],
            ports: Vec::new(),
            auto_forward_on_create: false,
        },
        GameKind::VRising => vrising::default_port_policy(),
        GameKind::Valheim => valheim::default_port_policy(),
        GameKind::Terraria => terraria::default_port_policy(),
    }
}

fn normalize_game_port_policy(
    mut policy: GamePortPolicySpec,
) -> Result<GamePortPolicySpec, String> {
    if policy.ranges.len() > MAX_PORT_POLICY_RANGES {
        return Err(format!(
            "a game port policy can contain at most {MAX_PORT_POLICY_RANGES} ranges"
        ));
    }
    if policy.ports.len() > MAX_PORT_POLICY_PORTS {
        return Err(format!(
            "a game port policy can contain at most {MAX_PORT_POLICY_PORTS} individual ports"
        ));
    }
    for range in &policy.ranges {
        if range.start < 1_024 || range.end < range.start {
            return Err("port ranges must be ordered and use ports 1024–65535".to_owned());
        }
    }
    if policy.ports.iter().any(|port| *port < 1_024) {
        return Err("individual game ports must use ports 1024–65535".to_owned());
    }
    policy.ranges.sort_by_key(|range| (range.start, range.end));
    policy.ranges.dedup();
    policy.ports.sort_unstable();
    policy.ports.dedup();
    if policy.ranges.is_empty() && policy.ports.is_empty() {
        return Err("add at least one port or port range".to_owned());
    }
    let _ = policy_candidates(&policy)?;
    Ok(policy)
}

fn policy_candidates(policy: &GamePortPolicySpec) -> Result<Vec<u16>, String> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for port in &policy.ports {
        if seen.insert(*port) {
            candidates.push(*port);
        }
    }
    for range in &policy.ranges {
        for port in range.start..=range.end {
            if seen.insert(port) {
                candidates.push(port);
                if candidates.len() > MAX_PORT_POLICY_CANDIDATES {
                    return Err(format!(
                        "a game port policy can cover at most {MAX_PORT_POLICY_CANDIDATES} unique ports"
                    ));
                }
            }
        }
    }
    Ok(candidates)
}

fn game_port_policy_response(
    policy: GamePortPolicySpec,
    candidates: Vec<u16>,
    used: HashSet<u16>,
    next_available: Option<u16>,
    amp_claimed: HashSet<u16>,
) -> Value {
    let used_ports = candidates
        .iter()
        .copied()
        .filter(|port| used.contains(port))
        .collect::<Vec<_>>();
    let available_count = candidates.len().saturating_sub(used_ports.len());
    let mut amp_claimed_ports = candidates
        .iter()
        .copied()
        .filter(|port| amp_claimed.contains(port))
        .collect::<Vec<_>>();
    amp_claimed_ports.sort_unstable();
    json!({
        "schema_version": 1,
        "policy": policy,
        "capacity": candidates.len(),
        "assigned_ports": used_ports,
        "amp_claimed_ports": amp_claimed_ports,
        "available_count": available_count,
        "next_available_port": next_available,
        "allocation_order": "individual_ports_then_ranges",
        "note": "Automatic allocation skips ports already assigned to Helix servers, ports AMP already has claimed, and ports currently bound on the host. AMP-claimed numbers in this pool are listed in amp_claimed_ports. Helix will not steal those; change the port in AMP or pick a different Helix port."
    })
}

fn validate_version(version: &str) -> Result<(), String> {
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
    {
        return Err("Minecraft version is invalid".to_owned());
    }
    Ok(())
}

fn unique_jar_name(
    parent: &Path,
    name: &str,
    reserved: &HashSet<PathBuf>,
) -> Result<String, String> {
    let file_name = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "drop a .jar file with an ordinary file name".to_owned())?;
    if file_name != name {
        return Err("drop a .jar file with an ordinary file name".to_owned());
    }
    validate_name(file_name)?;
    if !file_name.to_ascii_lowercase().ends_with(".jar") || file_name.starts_with('.') {
        return Err("drop a .jar file with an ordinary file name".to_owned());
    }
    let taken = |candidate: &str| {
        let path = parent.join(candidate);
        path.exists() || reserved.contains(&path)
    };
    if !taken(file_name) {
        return Ok(file_name.to_owned());
    }
    let stem = file_name
        .get(..file_name.len().saturating_sub(4))
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| "drop a .jar file with an ordinary file name".to_owned())?;
    for _ in 0..8 {
        let generated = format!("{stem}-{}.jar", &Uuid::new_v4().simple().to_string()[..8]);
        validate_name(&generated)?;
        if !taken(&generated) {
            return Ok(generated);
        }
    }
    Err("could not reserve a unique JAR name".to_owned())
}

fn validate_id(id: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(id).map_err(|_| "server ID is invalid".to_owned())?;
    if parsed.to_string() != id {
        return Err("server ID is invalid".to_owned());
    }
    Ok(())
}

fn required_json_text(value: &Value, key: &str, maximum: usize) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= maximum)
        .map(str::to_owned)
        .ok_or_else(|| format!("catalog field {key} is missing or invalid"))
}

fn select_stable_fabric_loader(value: &Value) -> Result<String, String> {
    value
        .as_array()
        .and_then(|entries| {
            entries.iter().find_map(|entry| {
                (entry.pointer("/loader/stable").and_then(Value::as_bool) == Some(true))
                    .then(|| entry.pointer("/loader/version").and_then(Value::as_str))
                    .flatten()
            })
        })
        .filter(|version| validate_version(version).is_ok())
        .map(str::to_owned)
        .ok_or_else(|| "Fabric has no stable loader for that Minecraft version".to_owned())
}

fn select_stable_fabric_installer(value: &Value) -> Result<String, String> {
    value
        .as_array()
        .and_then(|entries| {
            entries.iter().find_map(|entry| {
                (entry.get("stable").and_then(Value::as_bool) == Some(true))
                    .then(|| entry.get("version").and_then(Value::as_str))
                    .flatten()
            })
        })
        .filter(|version| validate_version(version).is_ok())
        .map(str::to_owned)
        .ok_or_else(|| "Fabric did not report a stable server installer".to_owned())
}

fn select_stable_quilt_loader(value: &Value) -> Result<String, String> {
    select_stable_fabric_loader(value).or_else(|_| {
        value
            .as_array()
            .and_then(|entries| {
                entries.iter().find_map(|entry| {
                    entry
                        .pointer("/loader/version")
                        .or_else(|| entry.get("version"))
                        .and_then(Value::as_str)
                })
            })
            .filter(|version| validate_version(version).is_ok())
            .map(str::to_owned)
            .ok_or_else(|| "Quilt has no loader for that Minecraft version".to_owned())
    })
}

fn select_stable_quilt_installer(value: &Value) -> Result<String, String> {
    select_stable_fabric_installer(value).or_else(|_| {
        value
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .find_map(|entry| entry.get("version").and_then(Value::as_str))
            })
            .filter(|version| validate_version(version).is_ok())
            .map(str::to_owned)
            .ok_or_else(|| "Quilt did not report a server installer".to_owned())
    })
}

fn resolve_requested_version(requested: &str, versions: &[String]) -> Result<String, String> {
    if requested.eq_ignore_ascii_case("latest") {
        return versions.first().cloned().ok_or_else(|| {
            "that software catalog did not report any Minecraft versions".to_owned()
        });
    }
    validate_version(requested)?;
    if versions.iter().any(|version| version == requested) {
        return Ok(requested.to_owned());
    }
    Err(format!(
        "Minecraft {requested} is not available for that server software"
    ))
}

fn minecraft_at_least(version: &str, minor: u32) -> bool {
    matches!(numeric_version_parts(version).as_slice(), [1, found, ..] if *found >= minor)
}

fn compare_minecraft_versions(left: &str, right: &str) -> std::cmp::Ordering {
    numeric_version_parts(left).cmp(&numeric_version_parts(right))
}

fn forge_promo_versions(value: &Value) -> Vec<String> {
    let mut versions = value
        .get("promos")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|promos| promos.keys())
        .filter_map(|key| {
            key.strip_suffix("-latest")
                .or_else(|| key.strip_suffix("-recommended"))
        })
        .filter(|version| validate_version(version).is_ok())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| compare_minecraft_versions(right, left));
    versions.dedup();
    versions
}

fn forge_promo_build(value: &Value, minecraft: &str) -> Result<String, String> {
    let promos = value
        .get("promos")
        .and_then(Value::as_object)
        .ok_or_else(|| "Forge did not report installer promotions".to_owned())?;
    let recommended = format!("{minecraft}-recommended");
    let latest = format!("{minecraft}-latest");
    promos
        .get(&recommended)
        .or_else(|| promos.get(&latest))
        .and_then(Value::as_str)
        .filter(|build| validate_version(build).is_ok())
        .map(str::to_owned)
        .ok_or_else(|| format!("Forge has no installer promotion for Minecraft {minecraft}"))
}

fn neoforge_game_versions(value: &Value) -> Vec<String> {
    let mut versions = value
        .get("versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(neoforge_loader_to_game)
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| compare_minecraft_versions(right, left));
    versions.dedup();
    versions
}

fn neoforge_loader_to_game(loader: &str) -> Option<String> {
    let parts = numeric_version_parts(loader);
    match parts.as_slice() {
        [major, 0, ..] => Some(format!("1.{major}")),
        [major, minor, ..] => Some(format!("1.{major}.{minor}")),
        _ => None,
    }
}

fn neoforge_loader_for_game(value: &Value, game: &str) -> Result<String, String> {
    let prefix = game
        .strip_prefix("1.")
        .map(|rest| {
            if rest.contains('.') {
                rest.to_owned()
            } else {
                format!("{rest}.0")
            }
        })
        .ok_or_else(|| format!("NeoForge does not publish Minecraft {game}"))?;
    value
        .get("versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|loader| loader.starts_with(&format!("{prefix}.")))
        .max_by_key(|loader| numeric_version_parts(loader))
        .filter(|loader| validate_version(loader).is_ok())
        .map(str::to_owned)
        .ok_or_else(|| format!("NeoForge has no installer for Minecraft {game}"))
}

fn pufferfish_family(version: &str) -> Result<String, String> {
    match numeric_version_parts(version).as_slice() {
        [1, minor, ..] => Ok(format!("1.{minor}")),
        _ => Err(format!(
            "Pufferfish has no Jenkins family for Minecraft {version}"
        )),
    }
}

fn pufferfish_versions_from_job(value: &Value) -> Vec<String> {
    value
        .get("artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artifact| artifact.get("fileName").and_then(Value::as_str))
        .filter_map(minecraft_version_from_filename)
        .collect()
}

fn minecraft_version_from_filename(name: &str) -> Option<String> {
    name.split(['-', '_'])
        .find(|part| part.starts_with("1.") && validate_version(part).is_ok())
        .map(str::to_owned)
}

fn pufferfish_artifact_url(value: &Value, version: &str) -> Result<(String, String), String> {
    let artifacts = value
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| "Pufferfish did not report build artifacts".to_owned())?;
    let artifact = artifacts.iter().find(|entry| {
        entry
            .get("fileName")
            .and_then(Value::as_str)
            .is_some_and(|name| {
                name.contains(version) && name.ends_with(".jar") && !name.contains("sources")
            })
    });
    let artifact = artifact.or_else(|| {
        artifacts.iter().find(|entry| {
            entry
                .get("fileName")
                .and_then(Value::as_str)
                .is_some_and(|name| name.ends_with(".jar") && !name.contains("sources"))
        })
    });
    let relative = artifact
        .and_then(|entry| entry.get("relativePath").and_then(Value::as_str))
        .filter(|path| !path.contains("..") && !path.starts_with('/'))
        .ok_or_else(|| format!("Pufferfish has no jar for Minecraft {version}"))?;
    let build = value
        .get("number")
        .and_then(Value::as_u64)
        .map(|number| number.to_string())
        .unwrap_or_else(|| "lastSuccessfulBuild".to_owned());
    let family = pufferfish_family(version)?;
    Ok((
        format!(
            "https://ci.pufferfish.host/job/Pufferfish-{family}/lastSuccessfulBuild/artifact/{relative}"
        ),
        build,
    ))
}

fn find_unix_args(root: &Path) -> Option<String> {
    let libraries = root.join("libraries");
    if !libraries.is_dir() {
        return None;
    }
    find_named_file(&libraries, "unix_args.txt", 0).and_then(|path| {
        path.strip_prefix(root)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
    })
}

fn find_named_file(root: &Path, name: &str, depth: u8) -> Option<PathBuf> {
    if depth > 12 {
        return None;
    }
    let entries = fs::read_dir(root).ok()?;
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = entry.file_type().ok()?;
        if file_type.is_file() && entry.file_name() == name {
            return Some(path);
        }
        if file_type.is_dir() {
            dirs.push(path);
        }
    }
    for dir in dirs {
        if let Some(found) = find_named_file(&dir, name, depth.saturating_add(1)) {
            return Some(found);
        }
    }
    None
}

fn stable_fill_build(value: &Value) -> Option<Value> {
    value.as_array().and_then(|builds| {
        builds
            .iter()
            .find(|build| build.get("channel").and_then(Value::as_str) == Some("STABLE"))
            .cloned()
    })
}

fn fill_project_versions(value: &Value) -> Vec<String> {
    let mut versions = value
        .get("versions")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|groups| groups.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .filter(|version| validate_version(version).is_ok())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    versions.sort_by_key(|version| std::cmp::Reverse(numeric_version_parts(version)));
    versions.dedup();
    versions
}

fn bounded_version_list(mut versions: Vec<String>) -> Vec<String> {
    versions.truncate(MAX_MINECRAFT_VERSION_CATALOG);
    versions
}

fn versions_not_newer_than(versions: Vec<String>, latest: &str) -> Vec<String> {
    let ceiling = numeric_version_parts(latest);
    versions
        .into_iter()
        .filter(|version| numeric_version_parts(version) <= ceiling)
        .collect()
}

fn string_version_list(value: Option<&Value>, require_valid: bool) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|version| !require_valid || validate_version(version).is_ok())
        .map(str::to_owned)
        .collect()
}

fn fabric_stable_game_versions(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("stable").and_then(Value::as_bool) == Some(true))
        .filter_map(|entry| entry.get("version").and_then(Value::as_str))
        .filter(|version| validate_version(version).is_ok())
        .map(str::to_owned)
        .collect()
}

fn mojang_release_versions(value: &Value) -> Vec<String> {
    value
        .get("versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("release"))
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .filter(|version| validate_version(version).is_ok())
        .map(str::to_owned)
        .collect()
}

fn require_leaves_ok(value: &Value) -> Result<(), String> {
    match value.get("code").and_then(Value::as_u64) {
        Some(200) | None => Ok(()),
        Some(_) => Err("Leaves rejected the catalog request".to_owned()),
    }
}

fn leaves_build_is_release(value: &Value) -> bool {
    value.get("promoted").and_then(Value::as_bool) == Some(true)
        || value
            .get("channel")
            .and_then(Value::as_str)
            .is_some_and(|channel| channel.eq_ignore_ascii_case("default"))
}

fn numeric_version_parts(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0))
        .collect()
}

fn minecraft_software_catalog() -> Vec<Value> {
    vec![
        software_catalog_entry(
            "pumpkin",
            "Pumpkin",
            "native_server",
            "ready",
            false,
            "Native Rust Minecraft server: no Java. Java uses TCP; Bedrock NetherNet uses a separate TCP/UDP port.",
            "Early-development server with its own plugin API. Versioned Linux x86-64/ARM64 binaries are checksum verified. Not a Paper/Fabric/Forge replacement: no automatic JAR/modpack or world conversion. Native plugins must match the build; PatchBukkit is separate and compatibility is not guaranteed.",
        ),
        software_catalog_entry(
            "custom",
            "Custom server JAR",
            "custom_server",
            "ready",
            false,
            "Bring an existing runnable Minecraft server JAR. Drop it here or choose one already visible in Storage.",
            "You choose the Minecraft and Java versions. Helix hashes the imported file and isolates it, but it cannot verify the publisher or safely auto-update an unknown JAR.",
        ),
        software_catalog_entry(
            "paper",
            "Paper",
            "plugin_server",
            "ready",
            true,
            "The best default for most plugin servers: strong performance, a broad ecosystem, and close Bukkit/Spigot compatibility.",
            "Official stable builds and publisher SHA-256 checksums are available through Paper's downloads service.",
        ),
        software_catalog_entry(
            "purpur",
            "Purpur",
            "plugin_server",
            "ready",
            false,
            "Paper compatibility with many extra gameplay and behavior controls.",
            "Helix pins the exact downloaded build over HTTPS; Purpur's current API does not publish a checksum for this endpoint.",
        ),
        software_catalog_entry(
            "leaves",
            "Leaves",
            "plugin_server",
            "ready",
            false,
            "A Paper-compatible fork with extra world, lag, and gameplay controls.",
            "Helix installs the newest default-channel Leaves build for the selected Minecraft version and verifies the publisher SHA-256. Experimental-only versions stay unavailable.",
        ),
        software_catalog_entry(
            "folia",
            "Folia",
            "plugin_server",
            "ready",
            false,
            "Regionized multithreading for spread-out, high-concurrency worlds on suitable hardware.",
            "Only plugins that explicitly support Folia are safe to install; it is not a drop-in Paper replacement.",
        ),
        software_catalog_entry(
            "vanilla",
            "Vanilla",
            "vanilla_server",
            "ready",
            false,
            "The official, unmodified Minecraft server and the cleanest baseline behavior.",
            "Mojang publishes the server artifact and SHA-1 used for verification.",
        ),
        software_catalog_entry(
            "fabric",
            "Fabric",
            "mod_server",
            "ready",
            false,
            "A lightweight mod loader with a large modern mod ecosystem and fast version adoption.",
            "Helix uses Fabric's official stable loader and installer metadata; installed mods must match both Fabric and the server's Minecraft version.",
        ),
        software_catalog_entry(
            "neoforge",
            "NeoForge",
            "mod_server",
            "ready",
            false,
            "A modern Forge-family loader for large content mods and modpacks.",
            "Helix downloads the official installer, runs --installServer, and launches with the generated unix_args. Minecraft 1.20.2+ only.",
        ),
        software_catalog_entry(
            "forge",
            "Forge",
            "mod_server",
            "ready",
            false,
            "The long-running mod-loader ecosystem used by many older and current modpacks.",
            "Helix installs Minecraft 1.17+ through the official installer and unix_args launch path. Older universal-jar versions stay unsupported.",
        ),
        software_catalog_entry(
            "quilt",
            "Quilt",
            "mod_server",
            "ready",
            false,
            "A Fabric-derived mod-loader ecosystem with its own loader and compatibility rules.",
            "Helix uses Quilt's official server jar metadata. Fabric mods are not assumed compatible.",
        ),
        software_catalog_entry(
            "spigot",
            "Spigot",
            "plugin_server",
            "manual_build_required",
            false,
            "The original Bukkit-compatible server base used by a large plugin ecosystem.",
            "Spigot is built with BuildTools rather than redistributed as a direct Helix download. Paper is the supported one-click default.",
        ),
        software_catalog_entry(
            "pufferfish",
            "Pufferfish",
            "plugin_server",
            "ready",
            false,
            "A Paper-derived server focused on additional performance tuning.",
            "Helix installs the last successful Jenkins artifact for the matching Minecraft family from ci.pufferfish.host over HTTPS.",
        ),
        software_catalog_entry(
            "velocity",
            "Velocity",
            "proxy",
            "topology_planned",
            false,
            "A modern proxy for routing players across multiple backend servers.",
            "A proxy needs a guided network topology and backend authentication flow, not the normal single-server wizard.",
        ),
        software_catalog_entry(
            "bungeecord",
            "BungeeCord",
            "proxy",
            "topology_planned",
            false,
            "A widely used Minecraft proxy with an established plugin ecosystem.",
            "It requires the same explicit proxy and backend security workflow as Velocity.",
        ),
        software_catalog_entry(
            "waterfall",
            "Waterfall",
            "proxy",
            "retired",
            false,
            "A retired PaperMC BungeeCord fork.",
            "PaperMC ended support for Waterfall; Helix will not offer it for new deployments.",
        ),
        software_catalog_entry(
            "sponge",
            "Sponge",
            "plugin_server",
            "validation_pending",
            false,
            "A separate plugin API available in Vanilla and Forge-oriented forms.",
            "Its platform-specific packaging and plugin compatibility need a dedicated install flow.",
        ),
        software_catalog_entry(
            "hybrid",
            "Hybrid mod + plugin servers",
            "hybrid_server",
            "not_recommended",
            false,
            "Projects such as Arclight, Mohist, and Magma try to combine mod loaders with Bukkit plugins.",
            "Cross-platform behavior and security vary significantly, so Helix does not present them as safe one-click choices.",
        ),
        software_catalog_entry(
            "bedrock-dedicated",
            "Bedrock Dedicated Server",
            "bedrock_server",
            "platform_planned",
            false,
            "Mojang's official server for Bedrock Edition players.",
            "It is a different native runtime and update channel from Java Edition and needs its own manager.",
        ),
        software_catalog_entry(
            "pocketmine",
            "PocketMine-MP",
            "bedrock_server",
            "platform_planned",
            false,
            "A plugin-oriented Bedrock-compatible server written in PHP.",
            "It needs a separate PHP runtime, plugin compatibility model, and Bedrock-specific testing.",
        ),
        software_catalog_entry(
            "powernukkitx",
            "PowerNukkitX",
            "bedrock_server",
            "platform_planned",
            false,
            "A Java-based Bedrock-compatible server with its own plugin ecosystem.",
            "It needs a separate Bedrock protocol, artifact, and plugin validation path.",
        ),
    ]
}

fn software_catalog_entry(
    id: &str,
    name: &str,
    kind: &str,
    status: &str,
    recommended: bool,
    appeal: &str,
    note: &str,
) -> Value {
    json!({
        "id": id,
        "name": name,
        "kind": kind,
        "status": status,
        "installable": status == "ready",
        "recommended": recommended,
        "appeal": appeal,
        "note": note,
    })
}

fn require_https_host(url: &str, allowed_hosts: &[&str]) -> Result<(), String> {
    let remainder = url
        .strip_prefix("https://")
        .ok_or_else(|| "the catalog returned a non-HTTPS download".to_owned())?;
    let authority = remainder.split('/').next().unwrap_or_default();
    if authority.contains(['@', ':'])
        || !allowed_hosts
            .iter()
            .any(|allowed| authority.eq_ignore_ascii_case(allowed))
    {
        return Err("the catalog returned an untrusted download host".to_owned());
    }
    Ok(())
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|_| "could not open the downloaded server".to_owned())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "could not hash the downloaded server".to_owned())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let mut output = String::with_capacity(64);
    for byte in digest.finalize() {
        let _ = write!(output, "{byte:02x}");
    }
    Ok(output)
}

fn write_replaced_secret_file(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "secret path has no parent".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|_| "could not create the Helix secret directory".to_owned())?;
    let temporary = parent.join(format!(".helix-secret-{}.partial", Uuid::new_v4()));
    let result = (|| {
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|_| "could not stage a Helix secret".to_owned())?;
            file.write_all(content)
                .and_then(|()| file.sync_all())
                .map_err(|_| "could not persist a Helix secret".to_owned())?;
        }
        fs::rename(&temporary, path).map_err(|_| "could not install a Helix secret".to_owned())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_new_file(path: &Path, content: &[u8], mode: u32) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .map_err(|_| format!("could not create {}", path.display()))?;
    file.write_all(content)
        .and_then(|()| file.sync_all())
        .map_err(|_| format!("could not write {}", path.display()))
}

fn write_managed_file(
    path: &Path,
    content: &[u8],
    mode: u32,
    owner_uid: u32,
    owner_gid: u32,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "managed file path has no parent".to_owned())?;
    let temporary = parent.join(format!(".helix-write-{}.partial", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|_| "could not stage the managed file".to_owned())?;
        file.write_all(content)
            .and_then(|()| file.sync_all())
            .map_err(|_| "could not persist the managed file".to_owned())?;
        run_program(
            Path::new("/usr/bin/chown"),
            &[
                format!("{owner_uid}:{owner_gid}"),
                temporary.to_string_lossy().into_owned(),
            ],
            20,
        )?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
            .map_err(|_| "could not protect the managed file".to_owned())?;
        fs::rename(&temporary, path).map_err(|_| "could not commit the managed file".to_owned())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn write_manifest(path: &Path, manifest: &InstanceManifest) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(manifest)
        .map_err(|_| "could not encode the server definition".to_owned())?;
    let temporary = path.with_extension(format!("json.{}.partial", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "could not stage the server definition".to_owned())?;
        file.write_all(&body)
            .and_then(|()| file.sync_all())
            .map_err(|_| "could not persist the server definition".to_owned())?;
        fs::rename(&temporary, path)
            .map_err(|_| "could not commit the server definition".to_owned())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn read_manifest(path: &Path) -> Result<InstanceManifest, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "the server definition is unavailable".to_owned())?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > MAX_MANIFEST_BYTES
    {
        return Err("the server definition is invalid".to_owned());
    }
    serde_json::from_slice(
        &fs::read(path).map_err(|_| "could not read the server definition".to_owned())?,
    )
    .map_err(|_| "the server definition is invalid".to_owned())
}

fn write_server_trash_record(path: &Path, record: &ServerTrashRecord) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(record)
        .map_err(|_| "could not encode the removed server recovery record".to_owned())?;
    write_new_file(path, &body, 0o600)
        .map_err(|_| "could not persist the removed server recovery record".to_owned())
}

fn read_server_trash_record(path: &Path) -> Result<ServerTrashRecord, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "the removed server recovery record is unavailable".to_owned())?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > MAX_MANIFEST_BYTES
    {
        return Err("the removed server recovery record is invalid".to_owned());
    }
    serde_json::from_slice(
        &fs::read(path)
            .map_err(|_| "could not read the removed server recovery record".to_owned())?,
    )
    .map_err(|_| "the removed server recovery record is invalid".to_owned())
}

fn server_properties(
    spec: &MinecraftCreateSpec,
    game_port: u16,
    rcon_port: u16,
    rcon_password: &str,
) -> String {
    format!(
        "# Managed by Helix\n\
         allow-flight=false\n\
         broadcast-rcon-to-ops=false\n\
         difficulty=easy\n\
         enable-command-block=false\n\
         enable-query=true\n\
         enable-rcon=true\n\
         enable-status=true\n\
         enforce-secure-profile=true\n\
         gamemode=survival\n\
         hardcore=false\n\
         max-players={}\n\
         motd={}\n\
         online-mode=true\n\
         player-idle-timeout=0\n\
         pvp=true\n\
         query.port={}\n\
         rcon.password={}\n\
         rcon.port={}\n\
         server-port={}\n\
         simulation-distance=10\n\
         spawn-protection=16\n\
         white-list=false\n\
         enforce-whitelist=false\n\
         view-distance=10\n",
        spec.max_players,
        spec.name.trim(),
        game_port,
        rcon_password,
        rcon_port,
        game_port,
    )
}

fn native_id(id: &str) -> &str {
    id.strip_prefix("helix:").unwrap_or(id)
}

fn valid_backup_id(id: &str) -> bool {
    (10..=20).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn validate_trash_id(id: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(id).map_err(|_| "backup trash ID is invalid".to_owned())?;
    if parsed.to_string() != id {
        return Err("backup trash ID is invalid".to_owned());
    }
    Ok(())
}

fn require_regular_backup_file(path: &Path, message: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| message.to_owned())?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(message.to_owned());
    }
    Ok(())
}

fn same_regular_file(left: &Path, right: &Path) -> Result<bool, String> {
    let left = fs::symlink_metadata(left)
        .map_err(|_| "could not inspect the active backup entry".to_owned())?;
    let right = fs::symlink_metadata(right)
        .map_err(|_| "could not inspect the deleted backup entry".to_owned())?;
    Ok(left.file_type().is_file()
        && right.file_type().is_file()
        && left.len() > 0
        && right.len() > 0
        && left.dev() == right.dev()
        && left.ino() == right.ino())
}

fn require_real_directory(path: &Path, message: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| message.to_owned())?;
    if !metadata.file_type().is_dir() {
        return Err(message.to_owned());
    }
    Ok(())
}

fn remove_managed_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(format!("could not inspect {label}")),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(format!(
            "{label} is not a real directory; Helix stopped before deleting more"
        ));
    }
    fs::remove_dir_all(path).map_err(|_| format!("could not delete {label}"))?;
    if let Some(parent) = path.parent() {
        let _ = sync_directory(parent);
    }
    Ok(())
}

fn write_backup_trash_record(path: &Path, record: &BackupTrashRecord) -> Result<(), String> {
    let content = serde_json::to_string_pretty(record)
        .map_err(|_| "could not encode the deleted backup record".to_owned())?;
    write_private_text(path, &content)
}

fn read_backup_trash_record(path: &Path) -> Result<BackupTrashRecord, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "the deleted backup record is unavailable".to_owned())?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > 64 * 1024 {
        return Err("the deleted backup record is invalid".to_owned());
    }
    serde_json::from_slice(
        &fs::read(path).map_err(|_| "could not read the deleted backup record".to_owned())?,
    )
    .map_err(|_| "the deleted backup record is invalid".to_owned())
}

fn remove_known_backup_trash_directory(
    directory: &Path,
    backup_id: &str,
    definition_present: bool,
) -> Result<(), String> {
    require_real_directory(directory, "the deleted backup directory is unavailable")?;
    let parent = directory
        .parent()
        .ok_or_else(|| "the deleted backup directory is invalid".to_owned())?;
    let definition_name = format!("{backup_id}.json");
    for path in [
        directory.join(format!("{backup_id}.tar.gz")),
        directory.join(&definition_name),
        directory.join("trash.json"),
    ] {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_file() {
            return Err("the deleted backup directory contains an invalid entry".to_owned());
        }
        if path.file_name().and_then(|value| value.to_str()) == Some(definition_name.as_str())
            && !definition_present
        {
            return Err("the deleted backup directory contains unexpected metadata".to_owned());
        }
        fs::remove_file(&path)
            .map_err(|_| "could not clean the deleted backup directory".to_owned())?;
    }
    fs::remove_dir(directory)
        .map_err(|_| "could not remove the empty deleted backup directory".to_owned())?;
    sync_directory(parent)
}

fn backup_id_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_suffix(".tar.gz"))
        .unwrap_or("")
        .to_owned()
}

fn read_small_regular_file(path: &Path, maximum: u64, label: &str) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("{label} are unavailable"))?;
    if !metadata.file_type().is_file() || metadata.len() > maximum {
        return Err(format!("{label} are invalid"));
    }
    String::from_utf8(fs::read(path).map_err(|_| format!("could not read {label}"))?)
        .map_err(|_| format!("{label} are not UTF-8 text"))
}

fn parse_properties(content: &str) -> HashMap<String, String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn property_text(properties: &HashMap<String, String>, key: &str, fallback: &str) -> String {
    properties
        .get(key)
        .filter(|value| value.len() <= 256 && !value.chars().any(char::is_control))
        .cloned()
        .unwrap_or_else(|| fallback.to_owned())
}

fn property_choice(
    properties: &HashMap<String, String>,
    key: &str,
    choices: &[&str],
    fallback: &str,
) -> String {
    properties
        .get(key)
        .map(String::as_str)
        .filter(|value| choices.contains(value))
        .unwrap_or(fallback)
        .to_owned()
}

fn property_u64(
    properties: &HashMap<String, String>,
    key: &str,
    fallback: u64,
    minimum: u64,
    maximum: u64,
) -> u64 {
    properties
        .get(key)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (minimum..=maximum).contains(value))
        .unwrap_or(fallback)
}

fn property_bool(properties: &HashMap<String, String>, key: &str, fallback: bool) -> bool {
    properties
        .get(key)
        .and_then(|value| match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
        .unwrap_or(fallback)
}

fn validate_settings(settings: &MinecraftSettingsPatch) -> Result<(), String> {
    if !valid_hex(&settings.expected_revision, 64) {
        return Err("server settings revision is invalid".to_owned());
    }
    if settings.motd.trim().is_empty()
        || settings.motd.len() > 128
        || settings.motd.chars().any(char::is_control)
    {
        return Err("message of the day must be 1–128 ordinary characters".to_owned());
    }
    if !(1..=10_000).contains(&settings.max_players) {
        return Err("player limit must be between 1 and 10,000".to_owned());
    }
    if settings.game_port < 1_024 {
        return Err("game port must be at least 1024".to_owned());
    }
    validate_allocated_memory(GameKind::Minecraft, settings.memory_mb)?;
    if !(2..=32).contains(&settings.view_distance)
        || !(2..=32).contains(&settings.simulation_distance)
    {
        return Err("view and simulation distances must be between 2 and 32".to_owned());
    }
    Ok(())
}

fn changed_setting_fields(
    properties: &HashMap<String, String>,
    settings: &MinecraftSettingsPatch,
) -> Vec<&'static str> {
    let expected = [
        ("motd", "motd", settings.motd.trim().to_owned()),
        (
            "game_mode",
            "gamemode",
            game_mode_name(settings.game_mode).to_owned(),
        ),
        (
            "difficulty",
            "difficulty",
            difficulty_name(settings.difficulty).to_owned(),
        ),
        (
            "max_players",
            "max-players",
            settings.max_players.to_string(),
        ),
        (
            "view_distance",
            "view-distance",
            settings.view_distance.to_string(),
        ),
        (
            "simulation_distance",
            "simulation-distance",
            settings.simulation_distance.to_string(),
        ),
        (
            "player_idle_timeout",
            "player-idle-timeout",
            settings.player_idle_timeout.to_string(),
        ),
        (
            "online_mode",
            "online-mode",
            settings.online_mode.to_string(),
        ),
        ("pvp", "pvp", settings.pvp.to_string()),
        (
            "allow_flight",
            "allow-flight",
            settings.allow_flight.to_string(),
        ),
        ("white_list", "white-list", settings.white_list.to_string()),
        (
            "enforce_white_list",
            "enforce-whitelist",
            settings.enforce_white_list.to_string(),
        ),
        (
            "spawn_protection",
            "spawn-protection",
            settings.spawn_protection.to_string(),
        ),
        ("game_port", "server-port", settings.game_port.to_string()),
    ];
    expected
        .iter()
        .filter(|(_, property, expected)| properties.get(*property) != Some(expected))
        .map(|(field, _, _)| *field)
        .collect()
}

fn update_properties(original: &str, settings: &MinecraftSettingsPatch) -> String {
    let properties = parse_properties(original);
    let new_port = settings.game_port.to_string();
    let current_server_port = properties.get("server-port");
    let mut replacements = vec![
        ("motd", settings.motd.trim().to_owned()),
        ("gamemode", game_mode_name(settings.game_mode).to_owned()),
        (
            "difficulty",
            difficulty_name(settings.difficulty).to_owned(),
        ),
        ("max-players", settings.max_players.to_string()),
        ("view-distance", settings.view_distance.to_string()),
        (
            "simulation-distance",
            settings.simulation_distance.to_string(),
        ),
        (
            "player-idle-timeout",
            settings.player_idle_timeout.to_string(),
        ),
        ("online-mode", settings.online_mode.to_string()),
        ("pvp", settings.pvp.to_string()),
        ("allow-flight", settings.allow_flight.to_string()),
        ("white-list", settings.white_list.to_string()),
        ("enforce-whitelist", settings.enforce_white_list.to_string()),
        ("spawn-protection", settings.spawn_protection.to_string()),
        ("server-port", new_port.clone()),
    ];
    if current_server_port.map(String::as_str) != Some(new_port.as_str()) {
        let current_query = properties.get("query.port");
        if current_query.is_none() || current_query == current_server_port {
            replacements.push(("query.port", new_port));
        }
    }
    let mut seen = HashSet::new();
    let mut output = String::with_capacity(original.len().saturating_add(256));
    for line in original.lines() {
        let key = line
            .trim()
            .split_once('=')
            .map(|(key, _)| key.trim())
            .filter(|_| !line.trim_start().starts_with(['#', '!']));
        if let Some((replacement_key, replacement_value)) = key.and_then(|key| {
            replacements
                .iter()
                .find(|(replacement_key, _)| *replacement_key == key)
        }) {
            if seen.insert(*replacement_key) {
                let _ = writeln!(output, "{replacement_key}={replacement_value}");
            }
        } else {
            let _ = writeln!(output, "{line}");
        }
    }
    for (key, value) in replacements {
        if seen.insert(key) {
            let _ = writeln!(output, "{key}={value}");
        }
    }
    output
}

fn game_mode_name(value: MinecraftGameMode) -> &'static str {
    match value {
        MinecraftGameMode::Survival => "survival",
        MinecraftGameMode::Creative => "creative",
        MinecraftGameMode::Adventure => "adventure",
        MinecraftGameMode::Spectator => "spectator",
    }
}

fn difficulty_name(value: MinecraftDifficulty) -> &'static str {
    match value {
        MinecraftDifficulty::Peaceful => "peaceful",
        MinecraftDifficulty::Easy => "easy",
        MinecraftDifficulty::Normal => "normal",
        MinecraftDifficulty::Hard => "hard",
    }
}

fn instance_name(name: &str, id: &str) -> String {
    let mut slug = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(40)
        .collect::<String>()
        .to_ascii_lowercase();
    if slug.is_empty() {
        slug = "minecraft".to_owned();
    }
    format!("{slug}-{}", &id[..8])
}

fn allocate_rcon_port(manifests: &[InstanceManifest], extra: &HashSet<u16>) -> Result<u16, String> {
    let mut used = manifests
        .iter()
        .map(|manifest| manifest.rcon_port)
        .collect::<HashSet<_>>();
    used.extend(extra.iter().copied());
    (30_000..=31_999)
        .find(|port| !used.contains(port) && ensure_port_available(*port, false).is_ok())
        .ok_or_else(|| "no local console port is available".to_owned())
}

fn allocate_run_uid(id: &str, manifests: &[InstanceManifest]) -> Result<u32, String> {
    let used = manifests
        .iter()
        .map(|manifest| manifest.run_uid)
        .collect::<HashSet<_>>();
    let seed = u32::from_str_radix(&id.replace('-', "")[..8], 16).unwrap_or(0);
    for offset in 0..40_000_u32 {
        let candidate = 20_000 + (seed + offset) % 40_000;
        if !used.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err("no isolated runtime identity is available".to_owned())
}

fn ensure_port_available(port: u16, udp: bool) -> Result<(), String> {
    let tcp = TcpListener::bind((IpAddr::from([0, 0, 0, 0]), port));
    let udp_result = if udp {
        UdpSocket::bind((IpAddr::from([0, 0, 0, 0]), port)).map(|_| ())
    } else {
        Ok(())
    };
    if tcp.is_err() || udp_result.is_err() {
        return Err(format!("port {port} is already in use"));
    }
    Ok(())
}

fn fallback_java_version(version: &str) -> u16 {
    if version
        .split('.')
        .next()
        .and_then(|part| part.parse::<u16>().ok())
        .is_some_and(|major| major >= 26)
    {
        return 25;
    }
    let parts = version
        .split('.')
        .filter_map(|part| part.parse::<u16>().ok())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [1, minor, patch, ..] if *minor > 20 || (*minor == 20 && *patch >= 5) => 21,
        [1, minor, ..] if *minor >= 17 => 17,
        [1, 16, 5, ..] => 16,
        [1, minor, ..] if *minor >= 12 => 11,
        _ => 8,
    }
}

#[cfg(test)]
fn port_claimed_error(port: u16, helix: &HashSet<u16>, amp: &HashSet<u16>) -> Option<String> {
    if amp.contains(&port) {
        Some(amp_port_claimed_message(port))
    } else if helix.contains(&port) {
        Some(format!(
            "game port {port} is already assigned to another Helix server"
        ))
    } else {
        None
    }
}

fn assigned_game_ports(manifests: &[InstanceManifest]) -> HashSet<u16> {
    manifests
        .iter()
        .flat_map(InstanceManifest::occupied_ports)
        .collect()
}

fn display_software(manifest: &InstanceManifest) -> &'static str {
    match manifest.kind {
        GameKind::VRising => "V Rising",
        GameKind::Valheim => "Valheim",
        GameKind::Terraria => {
            if manifest.build.starts_with("tmodloader") {
                "tModLoader"
            } else {
                "Terraria"
            }
        }
        GameKind::Minecraft => software_name(manifest.software),
    }
}

fn insert_cpu_limit(args: &mut Vec<String>, cpu_millis: u32) {
    if cpu_millis == 0 {
        return;
    }
    let Some(index) = args.iter().position(|argument| argument == "--memory-swap") else {
        return;
    };
    let insert_at = index.saturating_add(2);
    args.splice(
        insert_at..insert_at,
        ["--cpus".to_owned(), format_docker_cpus(cpu_millis)],
    );
}

fn format_docker_cpus(cpu_millis: u32) -> String {
    let formatted = format!("{:.3}", f64::from(cpu_millis) / 1000.0);
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn vrising_host_settings_path(data_path: &Path) -> PathBuf {
    data_path
        .join("save")
        .join("Settings")
        .join("ServerHostSettings.json")
}

fn read_vrising_browser_listing(data_path: &Path) -> Value {
    let path = vrising_host_settings_path(data_path);
    let parsed = fs::read(&path)
        .ok()
        .and_then(|body| serde_json::from_slice::<Value>(&body).ok());
    let list_on_eos = parsed
        .as_ref()
        .and_then(|value| value.get("ListOnEOS"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let list_on_steam = parsed
        .as_ref()
        .and_then(|value| value.get("ListOnSteam"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "list_on_browser": list_on_eos || list_on_steam,
        "list_on_eos": list_on_eos,
        "list_on_steam": list_on_steam,
        "hide_ip_address": parsed
            .as_ref()
            .and_then(|value| value.get("HideIPAddress"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

fn is_minecraft_kind(kind: &GameKind) -> bool {
    matches!(kind, GameKind::Minecraft)
}

fn is_zero_u16(port: &u16) -> bool {
    *port == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn validate_backup_policy(keep_count: u16, keep_days: u16) -> Result<(), String> {
    if keep_count > 50 {
        return Err("keep at most 50 backups, or 0 for no count limit".to_owned());
    }
    if keep_days > 365 {
        return Err("keep backups at most 365 days, or 0 for no age limit".to_owned());
    }
    Ok(())
}

fn backup_retention_note(keep_count: u16, keep_days: u16) -> String {
    match (keep_count, keep_days) {
        (0, 0) => "Helix keeps every backup until you delete one.".to_owned(),
        (count, 0) => format!(
            "Helix keeps the newest {count} backups. Older ones move to trash after the next backup."
        ),
        (0, days) => format!(
            "Helix keeps backups for {days} days. Older ones move to trash after the next backup."
        ),
        (count, days) => format!(
            "Helix keeps the newest {count} backups and anything newer than {days} days. Extra oldest copies move to trash."
        ),
    }
}

fn software_name(software: MinecraftSoftware) -> &'static str {
    match software {
        MinecraftSoftware::Pumpkin => "Pumpkin",
        MinecraftSoftware::Custom => "Custom JAR",
        MinecraftSoftware::Vanilla => "Vanilla",
        MinecraftSoftware::Paper => "Paper",
        MinecraftSoftware::Purpur => "Purpur",
        MinecraftSoftware::Folia => "Folia",
        MinecraftSoftware::Leaves => "Leaves",
        MinecraftSoftware::Fabric => "Fabric",
        MinecraftSoftware::NeoForge => "NeoForge",
        MinecraftSoftware::Forge => "Forge",
        MinecraftSoftware::Quilt => "Quilt",
        MinecraftSoftware::Pufferfish => "Pufferfish",
    }
}

fn rcon_command(port: u16, password: &str, command: &str) -> Result<String, String> {
    rcon_command_timed(
        port,
        password,
        command,
        Duration::from_secs(3),
        Duration::from_secs(4),
    )
}

fn rcon_command_timed(
    port: u16,
    password: &str,
    command: &str,
    connect_timeout: Duration,
    io_timeout: Duration,
) -> Result<String, String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, connect_timeout)
        .map_err(|_| "could not connect to the local Minecraft console".to_owned())?;
    stream
        .set_read_timeout(Some(io_timeout))
        .and_then(|()| stream.set_write_timeout(Some(io_timeout)))
        .map_err(|_| "could not configure the Minecraft console connection".to_owned())?;
    write_rcon_packet(&mut stream, 1, 3, password)?;
    let (auth_id, _, _) = read_rcon_packet(&mut stream)?;
    if auth_id == -1 {
        return Err("Minecraft rejected the local console credential".to_owned());
    }
    write_rcon_packet(&mut stream, 2, 2, command)?;
    let (response_id, _, response) = read_rcon_packet(&mut stream)?;
    if response_id != 2 {
        return Err("Minecraft returned an invalid console response".to_owned());
    }
    Ok(response
        .chars()
        .filter(|value| !value.is_control() || matches!(value, '\n' | '\r' | '\t'))
        .take(256 * 1024)
        .collect())
}

fn write_rcon_packet(
    stream: &mut TcpStream,
    request_id: i32,
    packet_type: i32,
    body: &str,
) -> Result<(), String> {
    let length = 10_usize
        .checked_add(body.len())
        .ok_or_else(|| "console packet is too large".to_owned())?;
    if length > MAX_RCON_PACKET_BYTES {
        return Err("console packet is too large".to_owned());
    }
    let length = i32::try_from(length).map_err(|_| "console packet is too large".to_owned())?;
    let mut packet = Vec::with_capacity(body.len().saturating_add(14));
    packet.extend_from_slice(&length.to_le_bytes());
    packet.extend_from_slice(&request_id.to_le_bytes());
    packet.extend_from_slice(&packet_type.to_le_bytes());
    packet.extend_from_slice(body.as_bytes());
    packet.extend_from_slice(&[0, 0]);
    stream
        .write_all(&packet)
        .and_then(|()| stream.flush())
        .map_err(|_| "could not write to the Minecraft console".to_owned())
}

fn read_rcon_packet(stream: &mut TcpStream) -> Result<(i32, i32, String), String> {
    let mut length_bytes = [0_u8; 4];
    stream
        .read_exact(&mut length_bytes)
        .map_err(|_| "Minecraft did not answer the console request".to_owned())?;
    let length = i32::from_le_bytes(length_bytes);
    let length = usize::try_from(length)
        .map_err(|_| "Minecraft returned an invalid console packet".to_owned())?;
    if !(10..=MAX_RCON_PACKET_BYTES).contains(&length) {
        return Err("Minecraft returned an invalid console packet".to_owned());
    }
    let mut packet = vec![0_u8; length];
    stream
        .read_exact(&mut packet)
        .map_err(|_| "Minecraft returned an incomplete console packet".to_owned())?;
    if packet[length - 2..] != [0, 0] {
        return Err("Minecraft returned an invalid console packet".to_owned());
    }
    let request_id = i32::from_le_bytes(packet[0..4].try_into().unwrap_or_default());
    let packet_type = i32::from_le_bytes(packet[4..8].try_into().unwrap_or_default());
    let body = String::from_utf8(packet[8..length - 2].to_vec())
        .map_err(|_| "Minecraft returned invalid console text".to_owned())?;
    Ok((request_id, packet_type, body))
}

fn run_program_combined(
    program: &Path,
    args: &[String],
    timeout_seconds: u64,
) -> Result<String, String> {
    let output = run_program_output_bounded(
        program,
        args,
        timeout_seconds,
        MAX_CONSOLE_HISTORY_PAGE_TEXT_BYTES,
        &[],
    )?;
    if output.status.success() {
        let mut combined = String::from_utf8(output.stdout)
            .map_err(|_| format!("{} returned invalid output", program.display()))?;
        let stderr = String::from_utf8(output.stderr)
            .map_err(|_| format!("{} returned invalid output", program.display()))?;
        if !combined.is_empty() && !stderr.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&stderr);
        return Ok(combined);
    }
    let detail = if output.status.code() == Some(124) {
        "operation timed out".to_owned()
    } else {
        one_line_tail(&String::from_utf8_lossy(&output.stderr), 700)
    };
    Err(if detail.is_empty() {
        format!("{} failed", program.display())
    } else {
        detail
    })
}

fn map_download_curl_error(error: String) -> String {
    if error.contains("Maximum file size exceeded") {
        "the download is larger than Helix allows for this file".to_owned()
    } else {
        error
    }
}

fn run_program(program: &Path, args: &[String], timeout_seconds: u64) -> Result<String, String> {
    run_program_with_env(program, args, timeout_seconds, &[])
}

fn run_program_with_env(
    program: &Path,
    args: &[String],
    timeout_seconds: u64,
    extra_env: &[(&str, &str)],
) -> Result<String, String> {
    let output = run_program_output_bounded(
        program,
        args,
        timeout_seconds,
        MAX_NATIVE_COMMAND_OUTPUT_BYTES,
        extra_env,
    )?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|_| format!("{} returned invalid output", program.display()));
    }
    let detail = if output.status.code() == Some(124) {
        "operation timed out".to_owned()
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        one_line_tail(&stderr, 700)
    };
    Err(if detail.is_empty() {
        format!("{} failed", program.display())
    } else {
        detail
    })
}

struct BoundedProgramOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_program_output_bounded(
    program: &Path,
    args: &[String],
    timeout_seconds: u64,
    maximum_bytes: usize,
    extra_env: &[(&str, &str)],
) -> Result<BoundedProgramOutput, String> {
    let mut command = Command::new("/usr/bin/timeout");
    command
        .arg("--signal=TERM")
        .arg("--kill-after=5s")
        .arg(format!("{timeout_seconds}s"))
        .arg(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in extra_env {
        command.env(name, value);
    }
    let mut child = command
        .spawn()
        .map_err(|_| format!("could not run {}", program.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("could not read {} output", program.display()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("could not read {} output", program.display()))?;
    let stream_limit = u64::try_from(maximum_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let (stdout, stderr) = thread::scope(|scope| {
        let stderr_worker = scope.spawn(move || {
            let mut bytes = Vec::with_capacity(maximum_bytes.min(64 * 1024));
            stderr
                .take(stream_limit)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let mut stdout_bytes = Vec::with_capacity(maximum_bytes.min(64 * 1024));
        let stdout_result = stdout
            .take(stream_limit)
            .read_to_end(&mut stdout_bytes)
            .map(|_| stdout_bytes);
        let stderr_result = stderr_worker
            .join()
            .map_err(|_| std::io::Error::other("command output reader failed"))?;
        Ok::<_, std::io::Error>((stdout_result?, stderr_result?))
    })
    .map_err(|_| format!("could not read {} output", program.display()))?;
    let status = child
        .wait()
        .map_err(|_| format!("could not wait for {}", program.display()))?;
    if stdout.len() > maximum_bytes
        || stderr.len() > maximum_bytes
        || stdout.len().saturating_add(stderr.len()) > maximum_bytes
    {
        return Err(format!("{} returned too much output", program.display()));
    }
    Ok(BoundedProgramOutput {
        status,
        stdout,
        stderr,
    })
}

fn one_line_tail(value: &str, maximum: usize) -> String {
    let flattened = value
        .lines()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" · ");
    flattened.chars().take(maximum).collect()
}

fn parse_container_startup_state(value: &str) -> Result<ContainerStartupState, String> {
    let fields = value.trim().split('|').collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err("Docker returned an invalid Minecraft workload state".to_owned());
    }
    let parse_bool = |value: &str| match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("Docker returned an invalid Minecraft workload state".to_owned()),
    };
    Ok(ContainerStartupState {
        running: parse_bool(fields[0])?,
        restarting: parse_bool(fields[1])?,
        oom_killed: parse_bool(fields[2])?,
        exit_code: fields[3]
            .parse()
            .map_err(|_| "Docker returned an invalid Minecraft exit code".to_owned())?,
        restart_count: fields[4]
            .parse()
            .map_err(|_| "Docker returned an invalid Minecraft restart count".to_owned())?,
    })
}

fn startup_failure_summary(value: &str, maximum: usize) -> String {
    let lines = value
        .lines()
        .map(strip_terminal_sequences)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let important = lines
        .iter()
        .filter(|line| {
            let normalized = line.to_ascii_lowercase();
            [
                "error",
                "exception",
                "failed",
                "failure",
                "caused by",
                "outofmemory",
                "java heap space",
                "requires",
                "missing",
                "incompatible",
                "fatal",
                "could not",
            ]
            .iter()
            .any(|needle| normalized.contains(needle))
        })
        .rev()
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    let mut selected = if important.is_empty() {
        lines.iter().rev().take(12).cloned().collect::<Vec<_>>()
    } else {
        important
    };
    selected.reverse();
    let summary = selected.join(" · ");
    if summary.chars().count() <= maximum {
        summary
    } else {
        let mut truncated = summary
            .chars()
            .take(maximum.saturating_sub(1))
            .collect::<String>();
        truncated.push('…');
        truncated
    }
}

fn strip_terminal_sequences(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            match characters.next() {
                Some('[') => {
                    for sequence in characters.by_ref() {
                        if ('@'..='~').contains(&sequence) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(sequence) = characters.next() {
                        if sequence == '\u{7}' {
                            break;
                        }
                        if sequence == '\u{1b}' && characters.next_if_eq(&'\\').is_some() {
                            break;
                        }
                    }
                }
                Some(_) | None => {}
            }
        } else if !character.is_control() || character == '\t' {
            result.push(character);
        }
    }
    result
}

fn parse_human_bytes(value: &str) -> Option<u64> {
    let value = value.trim();
    let split = value
        .bytes()
        .position(|byte| !byte.is_ascii_digit() && byte != b'.')?;
    let number = value[..split].parse::<f64>().ok()?;
    let unit = value[split..].trim();
    let multiplier = match unit {
        "B" => 1.0,
        "kB" | "KB" => 1_000.0,
        "KiB" => 1024.0,
        "MB" => 1_000_000.0,
        "MiB" => 1024.0 * 1024.0,
        "GB" => 1_000_000_000.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * multiplier).max(0.0) as u64)
}

fn minecraft_status(port: u16, timeout: Duration) -> Result<MinecraftStatus, String> {
    let address = SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port);
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .map_err(|_| "Minecraft is not reachable".to_owned())?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|_| "Minecraft status timeout failed".to_owned())?;
    let mut handshake = Vec::new();
    write_varint(&mut handshake, 0);
    write_varint(&mut handshake, -1);
    write_protocol_string(&mut handshake, "127.0.0.1")?;
    handshake.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut handshake, 1);
    let mut packet = Vec::new();
    write_varint(
        &mut packet,
        i32::try_from(handshake.len()).map_err(|_| "status packet is too large".to_owned())?,
    );
    packet.extend_from_slice(&handshake);
    packet.extend_from_slice(&[1, 0]);
    stream
        .write_all(&packet)
        .map_err(|_| "could not request Minecraft status".to_owned())?;
    let length = read_varint(&mut stream)?;
    if !(1..=2_097_152).contains(&length) {
        return Err("Minecraft returned an invalid status packet".to_owned());
    }
    let packet_id = read_varint(&mut stream)?;
    if packet_id != 0 {
        return Err("Minecraft returned an unexpected status packet".to_owned());
    }
    let json_length = read_varint(&mut stream)?;
    if json_length <= 0 || json_length > length || json_length > 2_097_152 {
        return Err("Minecraft returned an invalid status document".to_owned());
    }
    let mut body = vec![0_u8; usize::try_from(json_length).unwrap_or(0)];
    stream
        .read_exact(&mut body)
        .map_err(|_| "Minecraft status was incomplete".to_owned())?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| "Minecraft returned invalid status JSON".to_owned())?;
    Ok(MinecraftStatus {
        players_online: value
            .pointer("/players/online")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        max_players: value
            .pointer("/players/max")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        version: value
            .pointer("/version/name")
            .and_then(Value::as_str)
            .filter(|value| value.len() <= 128)
            .map(str::to_owned),
    })
}

fn write_varint(output: &mut Vec<u8>, mut value: i32) {
    loop {
        if value & !0x7f == 0 {
            output.push(value as u8);
            return;
        }
        output.push(((value & 0x7f) | 0x80) as u8);
        value = ((value as u32) >> 7) as i32;
    }
}

fn read_varint(input: &mut impl std::io::Read) -> Result<i32, String> {
    let mut result = 0_i32;
    for position in 0..5 {
        let mut byte = [0_u8; 1];
        input
            .read_exact(&mut byte)
            .map_err(|_| "Minecraft status was incomplete".to_owned())?;
        result |= i32::from(byte[0] & 0x7f) << (7 * position);
        if byte[0] & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err("Minecraft returned an oversized integer".to_owned())
}

fn write_protocol_string(output: &mut Vec<u8>, value: &str) -> Result<(), String> {
    write_varint(
        output,
        i32::try_from(value.len()).map_err(|_| "status hostname is too long".to_owned())?,
    );
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_startup_state_without_guessing_at_restart_health() {
        assert_eq!(
            parse_container_startup_state("false|false|true|137|3\n").unwrap(),
            ContainerStartupState {
                running: false,
                restarting: false,
                oom_killed: true,
                exit_code: 137,
                restart_count: 3,
            }
        );
        assert!(parse_container_startup_state("true|maybe|false|0|0").is_err());
        assert!(parse_container_startup_state("true|false|false|0").is_err());
    }

    #[test]
    fn startup_summary_prioritizes_the_cause_and_removes_terminal_sequences() {
        let logs = "\u{1b}[32mLoading mod one\u{1b}[0m\nLoading mod two\nCaused by: java.lang.OutOfMemoryError: Java heap space\nStopping server\n";
        let summary = startup_failure_summary(logs, 500);
        assert_eq!(
            summary,
            "Caused by: java.lang.OutOfMemoryError: Java heap space"
        );
        assert!(!summary.contains('\u{1b}'));
    }

    #[test]
    fn startup_summary_falls_back_to_recent_output_and_stays_bounded() {
        let summary = startup_failure_summary("loading alpha\nloading beta\n", 18);
        assert_eq!(summary, "loading alpha · l…");
        assert_eq!(summary.chars().count(), 18);
    }

    #[test]
    fn port_policy_is_deterministic_deduplicated_and_bounded() {
        let normalized = normalize_game_port_policy(GamePortPolicySpec {
            game: GameKind::Minecraft,
            ranges: vec![
                GamePortRangeSpec {
                    start: 25_565,
                    end: 25_568,
                },
                GamePortRangeSpec {
                    start: 25_565,
                    end: 25_568,
                },
            ],
            ports: vec![25_570, 25_565, 25_570],
            auto_forward_on_create: true,
        })
        .unwrap();
        assert_eq!(normalized.ports, vec![25_565, 25_570]);
        assert_eq!(normalized.ranges.len(), 1);
        assert_eq!(
            policy_candidates(&normalized).unwrap(),
            vec![25_565, 25_570, 25_566, 25_567, 25_568]
        );
        assert!(
            normalize_game_port_policy(GamePortPolicySpec {
                game: GameKind::Minecraft,
                ranges: vec![GamePortRangeSpec {
                    start: 1_024,
                    end: 6_000,
                }],
                ports: Vec::new(),
                auto_forward_on_create: false,
            })
            .is_err()
        );
    }

    #[test]
    fn vrising_port_policy_can_enable_public_auto_forward() {
        let normalized = normalize_game_port_policy(GamePortPolicySpec {
            game: GameKind::VRising,
            ranges: vec![GamePortRangeSpec {
                start: 9_876,
                end: 9_879,
            }],
            ports: Vec::new(),
            auto_forward_on_create: true,
        })
        .unwrap();
        assert!(normalized.auto_forward_on_create);
    }

    #[test]
    fn docker_cpus_formats_fractional_cores() {
        assert_eq!(format_docker_cpus(250), "0.25");
        assert_eq!(format_docker_cpus(1_000), "1");
        assert_eq!(format_docker_cpus(2_000), "2");
    }

    #[test]
    fn requested_ports_name_amp_claims_ahead_of_helix_servers() {
        let helix = HashSet::from([25_565_u16]);
        let amp = HashSet::from([25_566_u16, 8_081_u16]);
        assert_eq!(
            port_claimed_error(25_566, &helix, &amp).as_deref(),
            Some("AMP already has port 25566 claimed")
        );
        assert_eq!(
            port_claimed_error(8_081, &helix, &amp).as_deref(),
            Some("AMP already has port 8081 claimed")
        );
        assert_eq!(
            port_claimed_error(25_565, &helix, &amp).as_deref(),
            Some("game port 25565 is already assigned to another Helix server")
        );
        assert_eq!(port_claimed_error(25_570, &helix, &amp), None);
    }

    #[test]
    fn automatic_create_specs_are_valid_before_a_port_is_resolved() {
        let spec = MinecraftCreateSpec {
            pumpkin_bedrock_port: None,
            name: "Automatic".to_owned(),
            software: MinecraftSoftware::Paper,
            version: "latest".to_owned(),
            memory_mb: 4_096,
            cpu_millis: 0,
            max_players: 20,
            game_port: None,
            network_exposure: helix_privd::ServerNetworkExposure::Private,
            start_on_boot: true,
            eula_accepted: true,
            custom_jar: None,
        };
        validate_create_spec(&spec).unwrap();
    }

    #[test]
    fn minecraft_manifests_without_kind_still_load_and_vrising_roundtrips() {
        let minecraft = serde_json::json!({
            "schema_version": 1,
            "id": "f2210f81-6b2d-4f4f-badc-c9cf2f80b7ca",
            "name": "Survival",
            "instance_name": "survival-f2210f81",
            "container_name": "helix-game-f2210f81-6b2d-4f4f-badc-c9cf2f80b7ca",
            "software": "paper",
            "minecraft_version": "1.21.8",
            "build": "1",
            "java_version": 21,
            "runtime_image": "eclipse-temurin@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifact_url": "https://example.invalid/server.jar",
            "artifact_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "memory_mb": 4096,
            "max_players": 20,
            "game_port": 25565,
            "rcon_port": 30000,
            "rcon_password": "secret",
            "start_on_boot": true,
            "run_uid": 20000,
            "created_at_unix_ms": 1
        });
        let manifest: InstanceManifest = serde_json::from_value(minecraft).unwrap();
        assert!(manifest.is_minecraft());
        assert_eq!(manifest.query_port, 0);
        let encoded = serde_json::to_value(&manifest).unwrap();
        assert!(encoded.get("kind").is_none());
        assert!(encoded.get("query_port").is_none());
        assert!(encoded.get("cpu_millis").is_none());

        let vrising = InstanceManifest {
            schema_version: MANIFEST_VERSION,
            kind: GameKind::VRising,
            id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_owned(),
            name: "Castle".to_owned(),
            instance_name: "castle".to_owned(),
            container_name: "helix-game-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_owned(),
            software: MinecraftSoftware::Vanilla,
            minecraft_version: "dedicated".to_owned(),
            build: "1829350".to_owned(),
            java_version: 0,
            runtime_image: vrising::RUNTIME_IMAGE.to_owned(),
            artifact_url: vrising::ARTIFACT_URL.to_owned(),
            artifact_sha256: vrising::empty_artifact_sha256().to_owned(),
            memory_mb: 4096,
            cpu_millis: 0,
            max_players: 40,
            game_port: 9876,
            query_port: 9877,
            rcon_port: 0,
            rcon_password: String::new(),
            start_on_boot: true,
            run_uid: 20_000,
            created_at_unix_ms: 1,
            unix_args: None,
            backup_keep_count: 0,
            backup_keep_days: 0,
            modpack: None,
        };
        let encoded = serde_json::to_value(&vrising).unwrap();
        assert_eq!(encoded["kind"], "vrising");
        assert_eq!(encoded["query_port"], 9877);
        let loaded: InstanceManifest = serde_json::from_value(encoded).unwrap();
        assert!(loaded.is_vrising());
        assert_eq!(
            assigned_game_ports(std::slice::from_ref(&loaded)),
            [9876_u16, 9877].into_iter().collect()
        );
    }

    #[test]
    fn minecraft_status_checks_are_parallel_and_worker_bounded() {
        let targets = (0..24_u16)
            .map(|index| (format!("server-{index}"), 20_000 + index))
            .collect::<Vec<_>>();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let statuses = collect_minecraft_statuses(&targets, |port| {
            let current = active.fetch_add(1, Ordering::AcqRel) + 1;
            peak.fetch_max(current, Ordering::AcqRel);
            thread::sleep(Duration::from_millis(10));
            active.fetch_sub(1, Ordering::AcqRel);
            Some(MinecraftStatus {
                players_online: u64::from(port),
                max_players: 100,
                version: None,
            })
        });

        assert_eq!(statuses.len(), targets.len());
        assert!(peak.load(Ordering::Acquire) > 1);
        assert!(peak.load(Ordering::Acquire) <= MAX_MINECRAFT_STATUS_WORKERS);
    }

    #[test]
    fn parse_tps_response_reads_paper_spark_and_rejects_unknown() {
        assert_eq!(
            parse_tps_response("TPS from last 1m, 5m, 15m: 20.0, 20.0, 19.87"),
            Some(20.0)
        );
        assert_eq!(
            parse_tps_response("§6TPS from last 1m, 5m, 15m: §a*20.0, §a*20.0, §a*20.0"),
            Some(20.0)
        );
        assert_eq!(
            parse_tps_response("TPS from last 1m, 5m, 15m: 20.0*, 20.0*, 20.0*"),
            Some(20.0)
        );
        assert_eq!(
            parse_tps_response(
                "TPS from last 5 seconds, 10 seconds, 1 minute, 5 minutes, 15 minutes:\n19.98, 20.0, 20.0, 20.0, 20.0"
            ),
            Some(19.98)
        );
        assert_eq!(
            parse_tps_response("TPS from last 1m, 5m, 15m: §x§E§E§F§F§0§020.01, 20.0, 20.0"),
            Some(20.01)
        );
        assert_eq!(
            parse_tps_response("Unknown or incomplete command, see below for error"),
            None
        );
        assert_eq!(
            parse_tps_response("Unknown command. Type \"/help\" for help."),
            None
        );
        assert_eq!(parse_tps_response(""), None);
    }

    #[test]
    fn minecraft_tps_checks_are_parallel_and_worker_bounded() {
        let targets = (0..24_u16)
            .map(|index| {
                (
                    format!("server-{index}"),
                    30_000 + index,
                    "secret".to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let samples = collect_minecraft_tps(&targets, |port, password| {
            assert_eq!(password, "secret");
            let current = active.fetch_add(1, Ordering::AcqRel) + 1;
            peak.fetch_max(current, Ordering::AcqRel);
            thread::sleep(Duration::from_millis(10));
            active.fetch_sub(1, Ordering::AcqRel);
            TpsProbeResult {
                tps: Some(f64::from(port % 21)),
                ttl: MINECRAFT_TPS_CACHE_HIT,
            }
        });

        assert_eq!(samples.len(), targets.len());
        assert!(peak.load(Ordering::Acquire) > 1);
        assert!(peak.load(Ordering::Acquire) <= MAX_MINECRAFT_STATUS_WORKERS);
    }

    #[test]
    fn custom_server_import_accepts_only_managed_regular_jars_and_explicit_versions() {
        let temporary = tempfile::tempdir().unwrap();
        let managed = temporary.path().join("storage");
        let outside = temporary.path().join("outside");
        fs::create_dir_all(&managed).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let jar = managed.join("PrivateServer.JAR");
        let outside_jar = outside.join("server.jar");
        fs::write(&jar, vec![0x5a; 16 * 1024]).unwrap();
        fs::write(&outside_jar, vec![0x5a; 16 * 1024]).unwrap();
        let manager = NativeManager {
            state_root: temporary.path().join("state"),
            instance_root: temporary.path().join("instances"),
            backup_root: temporary.path().join("backups"),
            docker_binary: PathBuf::from("/usr/bin/docker"),
            console_retention: ConsoleRetention {
                maximum_bytes: default_console_history_max_bytes(),
                files: default_console_history_files(),
            },
            backup_trash_retention_days: 30,
            custom_artifact_roots: vec![fs::canonicalize(&managed).unwrap()],
            operations: Mutex::new(HashSet::new()),
            port_policies: Mutex::new(()),
            console_archives: Mutex::new(HashMap::new()),
            console_stops: Mutex::new(HashMap::new()),
            tps_cache: Mutex::new(HashMap::new()),
            custom_import_root: managed.clone(),
            uploads: Mutex::new(HashMap::new()),
            amp: None,
        };
        let mut spec = MinecraftCreateSpec {
            pumpkin_bedrock_port: None,
            name: "Private build".to_owned(),
            software: MinecraftSoftware::Custom,
            version: "1.21.8".to_owned(),
            memory_mb: 4096,
            cpu_millis: 0,
            max_players: 20,
            game_port: Some(25566),
            network_exposure: helix_privd::ServerNetworkExposure::Private,
            start_on_boot: false,
            eula_accepted: true,
            custom_jar: Some(helix_privd::CustomMinecraftJarSpec {
                source_path: jar.to_string_lossy().into_owned(),
                java_version: 21,
            }),
        };

        validate_create_spec(&spec).unwrap();
        let artifact = manager.resolve_custom_artifact(&spec).unwrap();
        assert!(matches!(artifact.software, MinecraftSoftware::Custom));
        assert_eq!(artifact.version, "1.21.8");
        assert_eq!(artifact.java_version, 21);
        assert_eq!(artifact.local_source, Some(fs::canonicalize(&jar).unwrap()));

        spec.version = "latest".to_owned();
        assert!(validate_create_spec(&spec).is_err());
        spec.version = "1.21.8".to_owned();
        spec.custom_jar.as_mut().unwrap().source_path = outside_jar.to_string_lossy().into_owned();
        let outside_error = match manager.resolve_custom_artifact(&spec) {
            Ok(_) => panic!("outside JAR was accepted"),
            Err(error) => error,
        };
        assert!(outside_error.contains("Storage root"));
        spec.custom_jar.as_mut().unwrap().source_path = jar.to_string_lossy().into_owned();
        spec.custom_jar.as_mut().unwrap().java_version = 22;
        let java_error = match manager.resolve_custom_artifact(&spec) {
            Ok(_) => panic!("unsupported Java was accepted"),
            Err(error) => error,
        };
        assert!(java_error.contains("Java 17, 21, or 25"));
    }

    #[test]
    fn directory_sync_requires_an_existing_directory() {
        let temporary = tempfile::tempdir().unwrap();
        sync_directory(temporary.path()).unwrap();
        assert!(sync_directory(&temporary.path().join("missing")).is_err());
    }

    #[test]
    fn download_hosts_are_exact_and_https_only() {
        assert!(
            require_https_host("https://fill-data.papermc.io/a", &["fill-data.papermc.io"]).is_ok()
        );
        assert!(
            require_https_host("http://fill-data.papermc.io/a", &["fill-data.papermc.io"]).is_err()
        );
        assert!(
            require_https_host(
                "https://fill-data.papermc.io.evil.test/a",
                &["fill-data.papermc.io"]
            )
            .is_err()
        );
        assert!(
            require_https_host(
                "https://user@fill-data.papermc.io/a",
                &["fill-data.papermc.io"]
            )
            .is_err()
        );
        assert!(
            require_https_host(
                "https://meta.fabricmc.net/v2/versions/loader/1.21.11/0.19.3/1.1.2/server/jar",
                &["meta.fabricmc.net"]
            )
            .is_ok()
        );
        assert!(
            require_https_host(
                "https://meta.fabricmc.net.evil.test/server.jar",
                &["meta.fabricmc.net"]
            )
            .is_err()
        );
    }

    #[test]
    fn fabric_catalog_selects_only_stable_bounded_versions() {
        let loaders = json!([
            {"loader": {"version": "0.19.4", "stable": false}},
            {"loader": {"version": "0.19.3", "stable": true}}
        ]);
        let installers = json!([
            {"version": "1.1.2", "stable": true},
            {"version": "1.1.1", "stable": false}
        ]);
        assert_eq!(select_stable_fabric_loader(&loaders).unwrap(), "0.19.3");
        assert_eq!(
            select_stable_fabric_installer(&installers).unwrap(),
            "1.1.2"
        );
        assert!(
            select_stable_fabric_loader(
                &json!([{"loader": {"version": "../../escape", "stable": true}}])
            )
            .is_err()
        );
        assert!(
            select_stable_fabric_installer(
                &json!([{"version": "https://evil.test/x", "stable": true}])
            )
            .is_err()
        );
    }

    #[test]
    fn software_catalog_distinguishes_runnable_and_explained_choices() {
        let catalog = minecraft_software_catalog();
        let ready = catalog
            .iter()
            .filter(|entry| entry["installable"] == true)
            .filter_map(|entry| entry["id"].as_str())
            .collect::<HashSet<_>>();
        assert_eq!(
            ready,
            HashSet::from([
                "pumpkin",
                "paper",
                "purpur",
                "folia",
                "leaves",
                "vanilla",
                "fabric",
                "neoforge",
                "forge",
                "quilt",
                "pufferfish",
                "custom"
            ])
        );
        assert!(catalog.iter().any(|entry| {
            entry["id"] == "custom"
                && entry["kind"] == "custom_server"
                && entry["status"] == "ready"
        }));
        assert!(catalog.iter().any(|entry| {
            entry["id"] == "leaves" && entry["status"] == "ready" && entry["installable"] == true
        }));
        assert!(catalog.iter().all(|entry| {
            entry["appeal"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
                && entry["note"].as_str().is_some_and(|text| !text.is_empty())
        }));
        assert!(catalog.iter().any(|entry| {
            entry["id"] == "waterfall"
                && entry["status"] == "retired"
                && entry["installable"] == false
        }));
        assert!(catalog.iter().any(|entry| {
            entry["id"] == "neoforge" && entry["status"] == "ready" && entry["installable"] == true
        }));
    }

    #[test]
    fn fill_catalog_orders_versions_and_rejects_beta_as_latest() {
        let project = json!({
            "versions": {
                "1.21": ["1.21.11", "1.21.8"],
                "26.2": ["26.2"],
                "26.1": ["26.1.2"]
            }
        });
        assert_eq!(
            fill_project_versions(&project),
            vec!["26.2", "26.1.2", "1.21.11", "1.21.8"]
        );
        assert!(
            stable_fill_build(&json!([{
                "id": 7,
                "channel": "BETA"
            }]))
            .is_none()
        );
        assert_eq!(
            stable_fill_build(&json!([
                {"id": 7, "channel": "BETA"},
                {"id": 6, "channel": "STABLE"}
            ]))
            .unwrap()["id"],
            6
        );
    }

    #[test]
    fn version_catalogs_keep_releases_and_reject_leaves_experimental() {
        let vanilla = json!({
            "versions": [
                {"id": "26.1", "type": "snapshot"},
                {"id": "1.21.8", "type": "release"},
                {"id": "../../x", "type": "release"}
            ]
        });
        assert_eq!(mojang_release_versions(&vanilla), vec!["1.21.8"]);
        assert_eq!(
            bounded_version_list((0..200).map(|n| format!("1.0.{n}")).collect()).len(),
            MAX_MINECRAFT_VERSION_CATALOG
        );
        assert_eq!(
            versions_not_newer_than(
                vec![
                    "1.21.11".into(),
                    "1.21.10".into(),
                    "1.21.8".into(),
                    "1.21.4".into()
                ],
                "1.21.8"
            ),
            vec!["1.21.8", "1.21.4"]
        );
        let fabric = json!([
            {"version": "1.21.8", "stable": true},
            {"version": "1.21.9-rc.1", "stable": false}
        ]);
        assert_eq!(fabric_stable_game_versions(&fabric), vec!["1.21.8"]);
        assert!(leaves_build_is_release(
            &json!({"channel": "default", "promoted": false})
        ));
        assert!(!leaves_build_is_release(
            &json!({"channel": "experimental", "promoted": false})
        ));
        let temporary = tempfile::tempdir().unwrap();
        let unused = HashSet::new();
        assert_eq!(
            unique_jar_name(temporary.path(), "server.jar", &unused).unwrap(),
            "server.jar"
        );
        fs::write(temporary.path().join("server.jar"), b"exists").unwrap();
        let renamed = unique_jar_name(temporary.path(), "server.jar", &unused).unwrap();
        assert!(renamed.starts_with("server-") && renamed.ends_with(".jar"));
        let reserved = HashSet::from([temporary.path().join("other.jar")]);
        let colliding = unique_jar_name(temporary.path(), "other.jar", &reserved).unwrap();
        assert!(colliding.starts_with("other-") && colliding.ends_with(".jar"));
        assert!(unique_jar_name(temporary.path(), "../escape.jar", &unused).is_err());
        assert!(unique_jar_name(temporary.path(), "notes.txt", &unused).is_err());
    }

    #[test]
    fn native_config_keeps_safe_console_and_trash_defaults() {
        let config: NativeConfig = serde_json::from_value(json!({
            "state_root": "/var/lib/helix",
            "instance_root": "/srv/helix/instances",
            "backup_root": "/srv/helix/backups"
        }))
        .unwrap();
        assert_eq!(
            config.console_history_max_bytes,
            default_console_history_max_bytes()
        );
        assert_eq!(
            config.console_history_files,
            default_console_history_files()
        );
        assert_eq!(
            config.backup_trash_retention_days,
            default_backup_trash_retention_days()
        );
    }

    #[test]
    fn java_fallback_tracks_current_minecraft_requirements() {
        assert_eq!(fallback_java_version("26.2"), 25);
        assert_eq!(fallback_java_version("1.21.11"), 21);
        assert_eq!(fallback_java_version("1.20.4"), 17);
        assert_eq!(fallback_java_version("1.16.5"), 16);
    }

    #[test]
    fn names_and_ids_cannot_escape_managed_paths() {
        let id = "f2210f81-6b2d-4f4f-badc-c9cf2f80b7ca";
        assert_eq!(instance_name("My Server!", id), "myserver-f2210f81");
        assert!(validate_id(id).is_ok());
        assert!(validate_id("../../etc").is_err());
        assert!(validate_version("26.2").is_ok());
        assert!(validate_version("../../latest").is_err());
    }

    #[test]
    fn docker_memory_values_parse_without_locale_assumptions() {
        assert_eq!(parse_human_bytes("512MiB"), Some(512 * 1024 * 1024));
        assert_eq!(parse_human_bytes("1.5GiB"), Some(1_610_612_736));
        assert_eq!(parse_human_bytes("watts"), None);
    }

    #[test]
    fn varint_round_trips_status_lengths() {
        for value in [0, 1, 127, 128, 255, 2_097_152, -1] {
            let mut encoded = Vec::new();
            write_varint(&mut encoded, value);
            assert_eq!(read_varint(&mut encoded.as_slice()).unwrap(), value);
        }
    }

    #[test]
    fn settings_update_preserves_unmanaged_properties_and_does_not_overwrite_rcon() {
        let original = "# custom comment\nmotd=Old\nonline-mode=true\nrcon.password=secret\ncustom-setting=yes\nserver-port=25565\nquery.port=25565\n";
        let settings = MinecraftSettingsPatch {
            expected_revision: "a".repeat(64),
            motd: "Survival night".to_owned(),
            game_mode: MinecraftGameMode::Survival,
            difficulty: MinecraftDifficulty::Hard,
            max_players: 42,
            view_distance: 12,
            simulation_distance: 8,
            player_idle_timeout: 15,
            online_mode: true,
            pvp: true,
            allow_flight: false,
            white_list: true,
            enforce_white_list: true,
            spawn_protection: 8,
            game_port: 25_565,
            memory_mb: 4_096,
        };
        assert!(validate_settings(&settings).is_ok());
        let updated = update_properties(original, &settings);
        assert!(updated.contains("# custom comment"));
        assert!(updated.contains("custom-setting=yes"));
        assert!(updated.contains("rcon.password=secret"));
        assert!(updated.contains("motd=Survival night"));
        assert!(updated.contains("difficulty=hard"));
        assert_eq!(updated.matches("motd=").count(), 1);
        let changed = changed_setting_fields(&parse_properties(original), &settings);
        assert!(changed.contains(&"motd"));
        assert!(changed.contains(&"difficulty"));
        assert!(changed.contains(&"max_players"));
        assert!(!changed.contains(&"online_mode"));
        assert!(!changed.contains(&"game_port"));
        let mut retarget = settings.clone();
        retarget.game_port = 25_567;
        let retargeted = update_properties(original, &retarget);
        assert!(retargeted.contains("server-port=25567"));
        assert!(retargeted.contains("query.port=25567"));
        assert!(retargeted.contains("rcon.password=secret"));
        assert!(
            changed_setting_fields(&parse_properties(original), &retarget).contains(&"game_port")
        );
    }

    #[test]
    fn backup_ids_and_settings_boundaries_are_strict() {
        assert!(valid_backup_id("1787795182689"));
        assert!(!valid_backup_id("../../1787795182689"));
        assert!(!valid_backup_id("1787795182689.tar.gz"));
        let mut settings = MinecraftSettingsPatch {
            expected_revision: "b".repeat(64),
            motd: "Server".to_owned(),
            game_mode: MinecraftGameMode::Creative,
            difficulty: MinecraftDifficulty::Normal,
            max_players: 1,
            view_distance: 2,
            simulation_distance: 32,
            player_idle_timeout: 0,
            online_mode: true,
            pvp: false,
            allow_flight: true,
            white_list: false,
            enforce_white_list: false,
            spawn_protection: 0,
            game_port: 25_565,
            memory_mb: 4_096,
        };
        assert!(validate_settings(&settings).is_ok());
        settings.view_distance = 33;
        assert!(validate_settings(&settings).is_err());
        settings.view_distance = 2;
        settings.game_port = 80;
        assert!(validate_settings(&settings).is_err());
        settings.game_port = 25_565;
        settings.memory_mb = 512;
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn allocated_memory_bounds_match_create_limits() {
        assert_eq!(
            allocated_memory_bounds(GameKind::Minecraft),
            (1_024, 24_576)
        );
        assert_eq!(allocated_memory_bounds(GameKind::VRising), (2_048, 24_576));
        assert_eq!(allocated_memory_bounds(GameKind::Valheim), (1_024, 16_384));
        assert_eq!(allocated_memory_bounds(GameKind::Terraria), (512, 8_192));
        assert!(validate_allocated_memory(GameKind::Minecraft, 512).is_err());
        assert!(validate_allocated_memory(GameKind::Terraria, 512).is_ok());
        assert!(validate_allocated_memory(GameKind::VRising, 1_024).is_err());
        assert!(validate_allocated_memory(GameKind::Valheim, 24_576).is_err());
    }

    #[test]
    fn docker_cli_home_lives_under_native_state() {
        let temporary = tempfile::tempdir().unwrap();
        let state_root = temporary.path().join("state");
        fs::create_dir_all(&state_root).unwrap();
        let home = prepare_docker_cli_home(&state_root).unwrap();
        assert_eq!(home, state_root.join(".docker-cli"));
        assert!(home.join("buildx").join("activity").is_dir());
        assert!(home.join("tmp").is_dir());
    }

    #[test]
    fn console_archive_rotates_and_reads_a_stable_tail() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("console");
        let mut archive = ConsoleArchiveWriter::open(
            root.clone(),
            ConsoleRetention {
                maximum_bytes: 360,
                files: 3,
            },
        )
        .unwrap();
        for index in 0..20 {
            archive
                .append(&format!(
                    "2026-08-27T01:02:{index:02}.000000000Z server line {index:02}"
                ))
                .unwrap();
        }
        let segments = console_segment_paths(&root).unwrap();
        assert!(segments.len() <= 3);
        assert_eq!(
            read_console_tail(&root, 4).unwrap(),
            vec![
                "2026-08-27T01:02:16.000000000Z server line 16",
                "2026-08-27T01:02:17.000000000Z server line 17",
                "2026-08-27T01:02:18.000000000Z server line 18",
                "2026-08-27T01:02:19.000000000Z server line 19",
            ]
        );
    }

    #[test]
    fn console_resume_cursor_ignores_helix_markers() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("console");
        let mut archive = ConsoleArchiveWriter::open(
            root.clone(),
            ConsoleRetention {
                maximum_bytes: 1024 * 1024,
                files: 2,
            },
        )
        .unwrap();
        archive
            .append("2026-08-27T01:02:03.123456789Z Minecraft ready")
            .unwrap();
        archive
            .append("[helix 1787800000000] ---- Minecraft boot marker ----")
            .unwrap();
        assert_eq!(
            latest_docker_log(&root).unwrap(),
            Some((
                "2026-08-27T01:02:03.123456789Z".to_owned(),
                "2026-08-27T01:02:03.123456789Z Minecraft ready".to_owned()
            ))
        );
    }

    #[test]
    fn bounded_console_reader_discards_an_overlong_line_before_continuing() {
        let mut input = vec![b'x'; 64];
        input.extend_from_slice(b"\nnext line\n");
        let mut reader = BufReader::new(std::io::Cursor::new(input));
        let mut line = Vec::new();

        assert_eq!(read_bounded_line(&mut reader, &mut line, 8).unwrap(), 65);
        assert_eq!(line, b"xxxxxxxx");
        assert_eq!(read_bounded_line(&mut reader, &mut line, 8).unwrap(), 10);
        assert_eq!(line, b"next lin");
        assert_eq!(read_bounded_line(&mut reader, &mut line, 8).unwrap(), 0);
    }

    #[test]
    fn console_history_cursor_pages_retained_segments_without_duplicates() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("console");
        let mut archive = ConsoleArchiveWriter::open(
            root.clone(),
            ConsoleRetention {
                maximum_bytes: 420,
                files: 8,
            },
        )
        .unwrap();
        for index in 0..20 {
            archive
                .append(&format!(
                    "2026-08-27T01:02:{index:02}.000000000Z history {index:02}"
                ))
                .unwrap();
        }

        let mut cursor = None;
        let mut collected = Vec::new();
        loop {
            let page = read_console_history_page(&root, cursor.as_deref(), 7).unwrap();
            collected.extend(page.lines);
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(collected.len(), 20);
        let unique = collected.iter().collect::<HashSet<_>>();
        assert_eq!(unique.len(), 20);
        assert_eq!(
            collected.first().map(String::as_str),
            Some("2026-08-27T01:02:13.000000000Z history 13")
        );
        assert_eq!(
            collected.last().map(String::as_str),
            Some("2026-08-27T01:02:05.000000000Z history 05")
        );
    }

    #[test]
    fn console_history_cursor_rejects_invalid_and_expired_positions() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("console");
        let mut archive = ConsoleArchiveWriter::open(
            root.clone(),
            ConsoleRetention {
                maximum_bytes: 1024 * 1024,
                files: 2,
            },
        )
        .unwrap();
        archive.append("one").unwrap();
        assert!(read_console_history_page(&root, Some("not-a-cursor"), 10).is_err());
        assert!(
            read_console_history_page(
                &root,
                Some(&encode_console_history_cursor(ConsoleHistoryCursor {
                    sequence: 99,
                    offset: 1,
                })),
                10,
            )
            .unwrap_err()
            .contains("expired")
        );
        let sequence = console_segment_sequence(&console_segment_paths(&root).unwrap()[0]).unwrap();
        assert!(
            read_console_history_page(
                &root,
                Some(&encode_console_history_cursor(ConsoleHistoryCursor {
                    sequence,
                    offset: 1,
                })),
                10,
            )
            .unwrap_err()
            .contains("invalid")
        );
        assert!(
            read_console_history_page(
                &root,
                Some(&encode_console_history_cursor(ConsoleHistoryCursor {
                    sequence,
                    offset: 99,
                })),
                10,
            )
            .unwrap_err()
            .contains("invalid")
        );
    }

    #[test]
    fn console_history_entries_expose_boot_and_command_metadata() {
        let boot = console_history_entry(
            "[helix 1787800000000] ---- Minecraft boot 2026-08-27T01:02:03.123456789Z (server-id) ----",
        );
        assert_eq!(boot["kind"], "boot");
        assert_eq!(boot["timestamp_unix_ms"], 1_787_800_000_000_u64);
        assert_eq!(boot["boot_started_at"], "2026-08-27T01:02:03.123456789Z");
        let command = console_history_entry("[helix 1787800000001] > /say hello");
        assert_eq!(command["kind"], "command");
        let output =
            console_history_entry("2026-08-27T01:02:04.123456789Z [Server thread/INFO]: Done");
        assert_eq!(output["kind"], "output");
        assert_eq!(output["timestamp"], "2026-08-27T01:02:04.123456789Z");
    }

    #[test]
    fn instance_operations_reject_overlap_and_release_on_drop() {
        let operations = Mutex::new(HashSet::new());
        let key = "instance-operation-key".to_owned();
        operations.lock().unwrap().insert(key.clone());
        let guard = InstanceOperationGuard {
            operations: &operations,
            key: key.clone(),
        };
        assert!(operations.lock().unwrap().contains(&key));
        drop(guard);
        assert!(!operations.lock().unwrap().contains(&key));
    }

    #[test]
    fn creation_rollback_removes_an_ambiguously_committed_exact_container() {
        let temporary = tempfile::tempdir().unwrap();
        let state_root = temporary.path().join("state");
        let instance_root = temporary.path().join("instances");
        let backup_root = temporary.path().join("backups");
        let failed_root = instance_root.join(".failed");
        fs::create_dir_all(&state_root).unwrap();
        fs::create_dir_all(&failed_root).unwrap();
        fs::create_dir_all(&backup_root).unwrap();

        let calls = temporary.path().join("docker-calls");
        let docker = temporary.path().join("docker-test");
        fs::write(
            &docker,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nif [ \"$1\" = container ]; then exit 0; fi\nexit 1\n",
                calls.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&docker, fs::Permissions::from_mode(0o700)).unwrap();

        let manager = NativeManager {
            state_root: state_root.clone(),
            instance_root: instance_root.clone(),
            backup_root,
            docker_binary: docker,
            console_retention: ConsoleRetention {
                maximum_bytes: default_console_history_max_bytes(),
                files: default_console_history_files(),
            },
            backup_trash_retention_days: 30,
            custom_artifact_roots: Vec::new(),
            operations: Mutex::new(HashSet::new()),
            port_policies: Mutex::new(()),
            console_archives: Mutex::new(HashMap::new()),
            console_stops: Mutex::new(HashMap::new()),
            tps_cache: Mutex::new(HashMap::new()),
            custom_import_root: state_root.join("imports"),
            uploads: Mutex::new(HashMap::new()),
            amp: None,
        };
        let id = "6f8303a9-ab56-4724-8580-b769dafefd22";
        let container_name = format!("helix-game-{id}");
        let data_path = instance_root.join(id);
        let manifest_path = state_root.join(format!("{id}.json"));
        fs::create_dir(&data_path).unwrap();
        fs::write(data_path.join("server.jar"), b"preserved").unwrap();
        fs::write(&manifest_path, b"incomplete").unwrap();

        manager
            .rollback_creation(id, &container_name, &data_path, &manifest_path, true)
            .unwrap();

        let calls = fs::read_to_string(calls).unwrap();
        assert!(
            calls
                .lines()
                .any(|line| line == format!("rm --force {container_name}"))
        );
        assert!(!manifest_path.exists());
        assert!(!data_path.exists());
        let recovered = fs::read_dir(failed_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::read(recovered.join("server.jar")).unwrap(),
            b"preserved"
        );
    }

    #[test]
    fn backup_delete_is_path_opaque_and_recoverable() {
        let temporary = tempfile::tempdir().unwrap();
        let state_root = temporary.path().join("state");
        let instance_root = temporary.path().join("instances");
        let backup_root = temporary.path().join("backups");
        fs::create_dir_all(state_root.join("console")).unwrap();
        fs::create_dir_all(&instance_root).unwrap();
        fs::create_dir_all(backup_root.join(".trash")).unwrap();
        let manager = NativeManager {
            state_root: state_root.clone(),
            instance_root,
            backup_root: backup_root.clone(),
            docker_binary: PathBuf::from("/bin/true"),
            console_retention: ConsoleRetention {
                maximum_bytes: default_console_history_max_bytes(),
                files: default_console_history_files(),
            },
            backup_trash_retention_days: 30,
            custom_artifact_roots: Vec::new(),
            operations: Mutex::new(HashSet::new()),
            port_policies: Mutex::new(()),
            console_archives: Mutex::new(HashMap::new()),
            console_stops: Mutex::new(HashMap::new()),
            tps_cache: Mutex::new(HashMap::new()),
            custom_import_root: state_root.join("imports"),
            uploads: Mutex::new(HashMap::new()),
            amp: None,
        };
        let id = "f2210f81-6b2d-4f4f-badc-c9cf2f80b7ca";
        let backup_id = "1787799939239";
        let manifest = InstanceManifest {
            schema_version: MANIFEST_VERSION,
            id: id.to_owned(),
            name: "Test".to_owned(),
            instance_name: "test-f2210f81".to_owned(),
            container_name: format!("helix-game-{id}"),
            software: MinecraftSoftware::Paper,
            minecraft_version: "1.21.8".to_owned(),
            build: "1".to_owned(),
            java_version: 21,
            runtime_image: "eclipse-temurin@sha256:test".to_owned(),
            artifact_url: "https://example.invalid/server.jar".to_owned(),
            artifact_sha256: "a".repeat(64),
            memory_mb: 4096,
            cpu_millis: 0,
            max_players: 20,
            game_port: 25565,
            rcon_port: 30000,
            rcon_password: "secret".to_owned(),
            start_on_boot: true,
            run_uid: 20_000,
            created_at_unix_ms: 1,
            kind: GameKind::Minecraft,
            query_port: 0,
            unix_args: None,
            backup_keep_count: 0,
            backup_keep_days: 0,
            modpack: None,
        };
        write_manifest(&state_root.join(format!("{id}.json")), &manifest).unwrap();
        let active = backup_root.join(id);
        fs::create_dir_all(&active).unwrap();
        fs::write(active.join(format!("{backup_id}.tar.gz")), b"archive").unwrap();
        let definition = active.join(format!("{backup_id}.json"));
        write_manifest(&definition, &manifest).unwrap();

        let real_definition = active.join("definition-real.json");
        fs::rename(&definition, &real_definition).unwrap();
        std::os::unix::fs::symlink(&real_definition, &definition).unwrap();
        assert!(
            manager
                .trash_backup(&format!("helix:{id}"), backup_id)
                .unwrap_err()
                .contains("definition is invalid")
        );
        assert!(active.join(format!("{backup_id}.tar.gz")).is_file());
        fs::remove_file(&definition).unwrap();
        fs::rename(real_definition, &definition).unwrap();

        let trashed = manager
            .trash_backup(&format!("helix:{id}"), backup_id)
            .unwrap();
        let trash_id = trashed["trash_id"].as_str().unwrap();
        assert!(Uuid::parse_str(trash_id).is_ok());
        assert!(trashed.get("path").is_none());
        assert!(!active.join(format!("{backup_id}.tar.gz")).exists());
        let trash_directory = backup_root.join(".trash").join(id).join(trash_id);
        assert!(trash_directory.is_dir());

        fs::write(
            active.join(format!("{backup_id}.tar.gz")),
            b"foreign backup",
        )
        .unwrap();
        let collision = manager
            .restore_trashed_backup(&format!("helix:{id}"), trash_id)
            .unwrap_err();
        assert!(collision.contains("different backup"));
        fs::remove_file(active.join(format!("{backup_id}.tar.gz"))).unwrap();

        // A crash after removing only the archive leaves the definition hard-linked in
        // both catalogs. Undo must recognize that exact inode instead of treating it as
        // an unrelated collision.
        fs::hard_link(
            trash_directory.join(format!("{backup_id}.json")),
            active.join(format!("{backup_id}.json")),
        )
        .unwrap();

        let restored = manager
            .restore_trashed_backup(&format!("helix:{id}"), trash_id)
            .unwrap();
        assert_eq!(restored["backup_id"], backup_id);
        assert!(active.join(format!("{backup_id}.tar.gz")).is_file());
        assert!(active.join(format!("{backup_id}.json")).is_file());
        assert!(!backup_root.join(".trash").join(id).join(trash_id).exists());
    }

    #[test]
    fn rcon_session_noise_is_dropped_from_persistent_console() {
        assert!(is_rcon_session_noise(
            "2026-08-29T08:00:00.000000000Z [12:00:00] [RCON Client /127.0.0.1 #1/INFO]: Thread RCON Client /127.0.0.1 started"
        ));
        assert!(is_rcon_session_noise(
            "[12:00:01] [RCON Client /127.0.0.1 #1/INFO]: Thread RCON Client /127.0.0.1 shutting down"
        ));
        assert!(is_rcon_session_noise(
            "[Not Secure] [RCON Client /127.0.0.1 #1/INFO]: [Rcon: TPS from last 1m, 5m, 15m: 20.0, 20.0, 20.0]"
        ));
        assert!(!is_rcon_session_noise(
            "[12:00:02] [Server thread/INFO]: Done (4.2s)! For help, type \"help\""
        ));
        assert!(!is_rcon_session_noise(
            "[12:00:03] [Server thread/INFO]: RCON running on 0.0.0.0:25575"
        ));
    }

    #[test]
    fn backup_retention_moves_oldest_active_copies_to_trash() {
        let temporary = tempfile::tempdir().unwrap();
        let state_root = temporary.path().join("state");
        let instance_root = temporary.path().join("instances");
        let backup_root = temporary.path().join("backups");
        fs::create_dir_all(state_root.join("console")).unwrap();
        fs::create_dir_all(&instance_root).unwrap();
        fs::create_dir_all(backup_root.join(".trash")).unwrap();
        let manager = NativeManager {
            state_root: state_root.clone(),
            instance_root,
            backup_root: backup_root.clone(),
            docker_binary: PathBuf::from("/bin/true"),
            console_retention: ConsoleRetention {
                maximum_bytes: default_console_history_max_bytes(),
                files: default_console_history_files(),
            },
            backup_trash_retention_days: 30,
            custom_artifact_roots: Vec::new(),
            operations: Mutex::new(HashSet::new()),
            port_policies: Mutex::new(()),
            console_archives: Mutex::new(HashMap::new()),
            console_stops: Mutex::new(HashMap::new()),
            tps_cache: Mutex::new(HashMap::new()),
            custom_import_root: state_root.join("imports"),
            uploads: Mutex::new(HashMap::new()),
            amp: None,
        };
        let id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        let mut manifest = InstanceManifest {
            schema_version: MANIFEST_VERSION,
            id: id.to_owned(),
            name: "Retention".to_owned(),
            instance_name: "retention".to_owned(),
            container_name: format!("helix-game-{id}"),
            software: MinecraftSoftware::Paper,
            minecraft_version: "1.21.8".to_owned(),
            build: "1".to_owned(),
            java_version: 21,
            runtime_image: "eclipse-temurin@sha256:test".to_owned(),
            artifact_url: "https://example.invalid/server.jar".to_owned(),
            artifact_sha256: "a".repeat(64),
            memory_mb: 4096,
            cpu_millis: 0,
            max_players: 20,
            game_port: 25565,
            rcon_port: 30000,
            rcon_password: "secret".to_owned(),
            start_on_boot: true,
            run_uid: 20_000,
            created_at_unix_ms: 1,
            kind: GameKind::Minecraft,
            query_port: 0,
            unix_args: None,
            backup_keep_count: 2,
            backup_keep_days: 0,
            modpack: None,
        };
        write_manifest(&state_root.join(format!("{id}.json")), &manifest).unwrap();
        let active = backup_root.join(id);
        fs::create_dir_all(&active).unwrap();
        for backup_id in ["1000000000001", "1000000000002", "1000000000003"] {
            fs::write(
                active.join(format!("{backup_id}.tar.gz")),
                backup_id.as_bytes(),
            )
            .unwrap();
            write_manifest(&active.join(format!("{backup_id}.json")), &manifest).unwrap();
        }
        let pruned = manager.prune_backups(&format!("helix:{id}")).unwrap();
        assert_eq!(pruned["backups"].as_array().unwrap().len(), 2);
        assert_eq!(pruned["trash"].as_array().unwrap().len(), 1);
        assert!(!active.join("1000000000001.tar.gz").exists());
        assert!(active.join("1000000000003.tar.gz").is_file());

        let trash_id = pruned["trash"][0]["trash_id"].as_str().unwrap().to_owned();
        manager
            .purge_backup_trash(&format!("helix:{id}"), &trash_id)
            .unwrap();
        let after_purge = manager.list_backups(&format!("helix:{id}")).unwrap();
        assert!(after_purge["trash"].as_array().unwrap().is_empty());

        fs::write(active.join("1000000000004.tar.gz"), b"4").unwrap();
        write_manifest(&active.join("1000000000004.json"), &manifest).unwrap();
        let protected = manager
            .enforce_backup_retention(&manifest, "1000000000004")
            .unwrap();
        assert_eq!(protected, vec!["1000000000002".to_owned()]);
        assert!(active.join("1000000000004.tar.gz").is_file());
        assert!(active.join("1000000000003.tar.gz").is_file());
        assert!(!active.join("1000000000002.tar.gz").exists());

        manifest.backup_keep_count = 0;
        write_manifest(&state_root.join(format!("{id}.json")), &manifest).unwrap();
        let policy = manager
            .set_backup_policy(&format!("helix:{id}"), 1, 0)
            .unwrap();
        assert_eq!(policy["policy"]["keep_count"], 1);
        assert_eq!(policy["backups"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn purge_trashed_server_wipes_recovery_data_after_typed_confirmation() {
        let temporary = tempfile::tempdir().unwrap();
        let state_root = temporary.path().join("state");
        let instance_root = temporary.path().join("instances");
        let backup_root = temporary.path().join("backups");
        fs::create_dir_all(state_root.join("console")).unwrap();
        fs::create_dir_all(state_root.join("server-trash")).unwrap();
        fs::create_dir_all(instance_root.join(".trash")).unwrap();
        fs::create_dir_all(backup_root.join(".trash")).unwrap();
        let manager = NativeManager {
            state_root: state_root.clone(),
            instance_root: instance_root.clone(),
            backup_root: backup_root.clone(),
            docker_binary: PathBuf::from("/bin/true"),
            console_retention: ConsoleRetention {
                maximum_bytes: default_console_history_max_bytes(),
                files: default_console_history_files(),
            },
            backup_trash_retention_days: 30,
            custom_artifact_roots: Vec::new(),
            operations: Mutex::new(HashSet::new()),
            port_policies: Mutex::new(()),
            console_archives: Mutex::new(HashMap::new()),
            console_stops: Mutex::new(HashMap::new()),
            tps_cache: Mutex::new(HashMap::new()),
            custom_import_root: state_root.join("imports"),
            uploads: Mutex::new(HashMap::new()),
            amp: None,
        };
        let id = "6f55caa9-1264-4baf-8335-d3f31a704614";
        let trash_id = "8953dc16-3891-42bf-802f-711b3ba2965a";
        let manifest = InstanceManifest {
            schema_version: MANIFEST_VERSION,
            id: id.to_owned(),
            name: "Survival".to_owned(),
            instance_name: "survival".to_owned(),
            container_name: format!("helix-game-{id}"),
            software: MinecraftSoftware::Paper,
            minecraft_version: "1.21.8".to_owned(),
            build: "1".to_owned(),
            java_version: 21,
            runtime_image: "eclipse-temurin@sha256:test".to_owned(),
            artifact_url: "https://example.invalid/server.jar".to_owned(),
            artifact_sha256: "a".repeat(64),
            memory_mb: 4096,
            cpu_millis: 0,
            max_players: 20,
            game_port: 25565,
            rcon_port: 30000,
            rcon_password: "secret".to_owned(),
            start_on_boot: true,
            run_uid: 20_000,
            created_at_unix_ms: 1,
            kind: GameKind::Minecraft,
            query_port: 0,
            unix_args: None,
            backup_keep_count: 0,
            backup_keep_days: 0,
            modpack: None,
        };
        let record_root = state_root.join("server-trash").join(trash_id);
        let data_root = instance_root.join(".trash").join(trash_id);
        fs::create_dir_all(&record_root).unwrap();
        fs::create_dir_all(&data_root).unwrap();
        write_manifest(&record_root.join("manifest.json"), &manifest).unwrap();
        write_server_trash_record(
            &record_root.join("record.json"),
            &ServerTrashRecord {
                schema_version: 1,
                trash_id: trash_id.to_owned(),
                instance_id: id.to_owned(),
                name: "Survival".to_owned(),
                trashed_at_unix_ms: 1,
                was_running: false,
            },
        )
        .unwrap();
        fs::write(data_root.join("world.dat"), b"world").unwrap();
        let backups = backup_root.join(id);
        fs::create_dir_all(&backups).unwrap();
        fs::write(backups.join("1787799939239.tar.gz"), b"archive").unwrap();
        let backup_trash = backup_root.join(".trash").join(id);
        fs::create_dir_all(&backup_trash).unwrap();
        fs::write(backup_trash.join("stale"), b"old").unwrap();
        let console = state_root.join("console").join(id);
        fs::create_dir_all(&console).unwrap();
        fs::write(console.join("current.log"), b"log").unwrap();

        let rejected = manager.purge_trashed_server(trash_id, "Wrong").unwrap_err();
        assert!(rejected.contains("exact server name"));
        assert!(data_root.join("world.dat").is_file());

        let purged = manager.purge_trashed_server(trash_id, "Survival").unwrap();
        assert_eq!(purged["purged"], true);
        assert_eq!(purged["instance_id"], format!("helix:{id}"));
        assert_eq!(purged["trash_id"], trash_id);
        assert!(purged.get("path").is_none());
        assert!(!record_root.exists());
        assert!(!data_root.exists());
        assert!(!backups.exists());
        assert!(!backup_trash.exists());
        assert!(!console.exists());
        assert_eq!(
            manager.list_trashed_servers().unwrap()["servers"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
