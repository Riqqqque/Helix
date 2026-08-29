use helix_privd::ServerAction;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString},
    fs,
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpStream},
    os::fd::OwnedFd,
    os::unix::ffi::OsStrExt as _,
    os::unix::fs::MetadataExt as _,
    path::{Component, Path, PathBuf},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_HTTP_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LOCAL_INSTANCES: usize = 4_096;
const MAX_KVP_FILES_PER_INSTANCE: usize = 32;
const MAX_INVENTORY_ISSUE_DETAILS: usize = 64;
const API_TIMEOUT: Duration = Duration::from_secs(20);
const AMP_KILL_UNSUPPORTED: &str = "Helix cannot force-kill AMP instances; they remain under AMP. Use Stop, or kill from the AMP panel.";

pub(crate) fn amp_port_claimed_message(port: u16) -> String {
    format!("AMP already has port {port} claimed")
}

pub struct AmpClient {
    endpoint: SocketAddr,
    public_panel_port: u16,
    username: String,
    password: SecretString,
    instance_root: PathBuf,
    sessions: Mutex<HashMap<u16, String>>,
    operations: Mutex<HashSet<String>>,
}

struct AmpOperationGuard<'a> {
    operations: &'a Mutex<HashSet<String>>,
    instance_id: String,
}

impl Drop for AmpOperationGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.operations.lock() {
            operations.remove(&self.instance_id);
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AmpServer {
    pub id: String,
    pub name: String,
    pub instance_name: String,
    pub kind: &'static str,
    pub software: String,
    pub version: String,
    pub status: String,
    pub panel_running: bool,
    pub start_on_boot: bool,
    pub players_online: u64,
    pub player_count_verified: bool,
    pub max_players: u64,
    pub cpu_percent: f64,
    pub memory_used_mb: u64,
    pub memory_limit_mb: u64,
    pub tps: Option<f64>,
    pub manager_panel_port: u16,
    pub panel_port: u16,
    pub game_port: Option<u16>,
    pub path: String,
    pub warnings: Vec<String>,
    pub manager: &'static str,
    pub execution_backend: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AmpInventoryIssue {
    pub code: &'static str,
    pub instance_id: Option<String>,
    pub instance_name: Option<String>,
    pub message: &'static str,
}

#[derive(Debug)]
pub struct AmpInventory {
    pub servers: Vec<AmpServer>,
    pub issue_count: u64,
    pub issues: Vec<AmpInventoryIssue>,
}

impl AmpInventory {
    fn new() -> Self {
        Self {
            servers: Vec::new(),
            issue_count: 0,
            issues: Vec::new(),
        }
    }

    fn record_issue(&mut self, instance: &Value, code: &'static str, message: &'static str) {
        self.issue_count = self.issue_count.saturating_add(1);
        if self.issues.len() < MAX_INVENTORY_ISSUE_DETAILS {
            self.issues.push(AmpInventoryIssue {
                code,
                instance_id: bounded_identity(instance, "InstanceID", 128),
                instance_name: bounded_identity(instance, "InstanceName", 255),
                message,
            });
        }
    }
}

impl AmpClient {
    #[must_use]
    pub const fn public_panel_port(&self) -> u16 {
        self.public_panel_port
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        let metadata =
            fs::metadata(path).map_err(|_| "AMP credentials are unavailable".to_owned())?;
        if metadata.len() > 16 * 1024 {
            return Err("AMP credential file is too large".to_owned());
        }
        if metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
            return Err(
                "AMP credentials must be root-owned and inaccessible to other users".to_owned(),
            );
        }
        let value: Value = serde_json::from_slice(
            &fs::read(path).map_err(|_| "AMP credentials are unavailable".to_owned())?,
        )
        .map_err(|_| "AMP credential file is invalid".to_owned())?;
        let endpoint = value
            .get("endpoint")
            .and_then(Value::as_str)
            .unwrap_or("127.0.0.1:8080")
            .parse::<SocketAddr>()
            .map_err(|_| "AMP endpoint is invalid".to_owned())?;
        if !endpoint.ip().is_loopback() {
            return Err("AMP endpoint must remain on loopback".to_owned());
        }
        let public_panel_port = match value.get("public_panel_port") {
            Some(value) => value
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port > 0)
                .ok_or_else(|| "AMP public panel port is invalid".to_owned())?,
            None => endpoint.port(),
        };
        let username = required_text(&value, "username", 128)?;
        let password = required_text(&value, "password", 1024)?;
        let instance_root = PathBuf::from(
            value
                .get("instance_root")
                .and_then(Value::as_str)
                .unwrap_or("/home/amp/.ampdata/instances"),
        );
        if !instance_root.is_absolute() {
            return Err("AMP instance root must be absolute".to_owned());
        }
        Ok(Self {
            endpoint,
            public_panel_port,
            username,
            password: SecretString::from(password),
            instance_root,
            sessions: Mutex::new(HashMap::new()),
            operations: Mutex::new(HashSet::new()),
        })
    }

    pub fn list_servers(&self) -> Result<AmpInventory, String> {
        let instances = self.call_ads("ADSModule", "GetLocalInstances", json!({}))?;
        self.parse_inventory(&instances)
    }

    pub fn occupied_ports(&self) -> HashSet<u16> {
        let mut ports = HashSet::new();
        insert_nonzero_port(&mut ports, self.public_panel_port);
        insert_nonzero_port(&mut ports, self.endpoint.port());
        ports.extend(self.scan_instance_root_ports());
        if let Ok(instances) = self.local_instances() {
            for instance in instances.into_iter().take(MAX_LOCAL_INSTANCES) {
                if let Ok(port) = required_u16(&instance, "Port") {
                    insert_nonzero_port(&mut ports, port);
                }
                if let Some(name) = text(&instance, "InstanceName") {
                    ports.extend(self.instance_config_ports(&name));
                }
            }
        }
        ports
    }

    fn scan_instance_root_ports(&self) -> HashSet<u16> {
        let mut ports = HashSet::new();
        let Ok(entries) = fs::read_dir(&self.instance_root) else {
            return ports;
        };
        let mut seen = 0usize;
        for entry in entries {
            if seen >= MAX_LOCAL_INSTANCES {
                break;
            }
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if validate_instance_name(&name).is_err() {
                continue;
            }
            seen += 1;
            ports.extend(self.instance_config_ports(&name));
        }
        ports
    }

    fn instance_config_ports(&self, instance_name: &str) -> HashSet<u16> {
        let mut ports = HashSet::new();
        let names = match list_instance_kvp_names(&self.instance_root, instance_name) {
            Ok(names) if !names.is_empty() => names,
            _ => vec!["MinecraftModule.kvp".to_owned()],
        };
        for name in names {
            if let Ok(config) = read_instance_kvp_file(&self.instance_root, instance_name, &name) {
                collect_ports_from_kvp(&config, &mut ports);
            }
        }
        ports
    }

    fn parse_inventory(&self, instances: &Value) -> Result<AmpInventory, String> {
        let instances = instances
            .as_array()
            .ok_or_else(|| "AMP returned an invalid instance list".to_owned())?;
        if instances.len() > MAX_LOCAL_INSTANCES {
            return Err("AMP returned too many local instances".to_owned());
        }
        let mut inventory = AmpInventory::new();
        for instance in instances {
            match instance.get("Module").and_then(Value::as_str) {
                Some("Minecraft") => match self.map_server(instance) {
                    Ok(server) => inventory.servers.push(server),
                    Err(MapServerError { code, message }) => {
                        inventory.record_issue(instance, code, message);
                    }
                },
                Some(_) => {}
                None => inventory.record_issue(
                    instance,
                    "invalid_instance_module",
                    "AMP returned an instance whose workload type could not be verified.",
                ),
            }
        }
        inventory
            .servers
            .sort_by_key(|server| server.name.to_lowercase());
        Ok(inventory)
    }

    pub fn server_action(&self, instance_id: &str, action: ServerAction) -> Result<Value, String> {
        let instance_id = instance_id.strip_prefix("amp:").unwrap_or(instance_id);
        let _operation = self.begin_operation(instance_id)?;
        let instances = self.local_instances()?;
        let instance = instances
            .iter()
            .find(|instance| text(instance, "InstanceID").as_deref() == Some(instance_id))
            .ok_or_else(|| "the selected server no longer exists".to_owned())?;
        if text(instance, "Module").as_deref() != Some("Minecraft") {
            return Err("the selected instance is not a Minecraft server".to_owned());
        }
        let instance_name = text(instance, "InstanceName")
            .ok_or_else(|| "the selected AMP instance is invalid".to_owned())?;
        validate_instance_name(&instance_name)
            .map_err(|_| "the selected AMP instance is invalid".to_owned())?;
        let port = required_u16(instance, "Port")
            .map_err(|_| "the selected AMP instance has an invalid port".to_owned())?;
        let panel_was_running = required_boolean(instance, "Running")
            .map_err(|_| "the selected AMP instance has invalid runtime state".to_owned())?;
        if !panel_was_running && action == ServerAction::Stop {
            return Ok(json!({
                "instance_id": format!("amp:{instance_id}"),
                "action": action,
                "accepted": true,
                "already_stopped": true,
                "panel_started_by_helix": false
            }));
        }
        if action == ServerAction::Kill {
            return Err(AMP_KILL_UNSUPPORTED.to_owned());
        }
        if !panel_was_running && matches!(action, ServerAction::Update | ServerAction::Backup) {
            return Err(
                "the AMP manager is stopped; start this instance before updating or backing it up"
                    .to_owned(),
            );
        }
        if !panel_was_running {
            let start_result = self.call_ads(
                "ADSModule",
                "StartInstance",
                json!({"InstanceName": instance_name.clone()}),
            )?;
            ensure_action_result(&start_result)?;
            self.wait_for_api(port, Duration::from_secs(60))?;
        }
        let result = match action {
            ServerAction::Start => self.call(port, "Core", "Start", json!({})),
            ServerAction::Stop => self.call(port, "Core", "Stop", json!({})),
            ServerAction::Kill => return Err(AMP_KILL_UNSUPPORTED.to_owned()),
            ServerAction::Restart if panel_was_running => {
                self.call(port, "Core", "Restart", json!({}))
            }
            ServerAction::Restart => self.call(port, "Core", "Start", json!({})),
            ServerAction::Update => self.call(port, "Core", "UpdateApplication", json!({})),
            ServerAction::Backup => self.call(
                port,
                "LocalFileBackupPlugin",
                "TakeBackup",
                json!({
                    "Title": format!("Helix {}", chrono_free_timestamp()),
                    "Description": "Created from Helix",
                    "Sticky": false,
                    "Local": true,
                    "S3": false,
                    "WasCreatedAutomatically": false,
                    "DirtyOnly": false,
                    "BackupWhileRunning": true
                }),
            ),
        }?;
        ensure_action_result(&result)?;
        Ok(json!({
            "instance_id": format!("amp:{instance_id}"),
            "action": action,
            "accepted": true,
            "panel_started_by_helix": !panel_was_running
        }))
    }

    fn map_server(&self, instance: &Value) -> Result<AmpServer, MapServerError> {
        let id = text(instance, "InstanceID").ok_or(MapServerError::invalid_identity())?;
        validate_instance_identifier(&id).map_err(|_| MapServerError::invalid_identity())?;
        let instance_name =
            text(instance, "InstanceName").ok_or(MapServerError::invalid_instance_path())?;
        validate_instance_name(&instance_name)
            .map_err(|_| MapServerError::invalid_instance_path())?;
        let name = text(instance, "FriendlyName").unwrap_or_else(|| instance_name.clone());
        let panel_port =
            required_u16(instance, "Port").map_err(|_| MapServerError::invalid_panel_state())?;
        let panel_running = required_boolean(instance, "Running")
            .map_err(|_| MapServerError::invalid_panel_state())?;
        let metrics = instance.get("Metrics").and_then(Value::as_object);
        let online = panel_running && metrics.is_some_and(|metrics| !metrics.is_empty());
        let (config, config_warning) = match read_instance_kvp(&self.instance_root, &instance_name)
        {
            Ok(config) => (config, None),
            Err(ConfigReadError::Unavailable) => (
                HashMap::new(),
                Some("Instance configuration is unavailable".to_owned()),
            ),
            Err(ConfigReadError::UnsafePath) => {
                return Err(MapServerError::invalid_instance_path());
            }
        };
        let software = config
            .get("Minecraft.ServerType")
            .map(|value| display_server_type(value))
            .unwrap_or_else(|| "Minecraft".to_owned());
        let version = server_version(&config, &software);
        let max_players = metric(metrics, "Active Users", "MaxValue")
            .or_else(|| {
                config
                    .get("Limits.MaxPlayers")
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or(0);
        let memory_limit_mb = metric(metrics, "Memory Usage", "MaxValue")
            .or_else(|| {
                config
                    .get("Java.MaxHeapSizeMB")
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or(0);
        let path = self.instance_root.join(&instance_name);
        let mut warnings = Vec::new();
        if let Some(warning) = config_warning {
            warnings.push(warning);
        }
        if !path.exists() {
            warnings.push("Instance files are unavailable".to_owned());
        }
        let (players_online, player_count_verified) = match player_count_metric(metrics) {
            Some(players) => (players, true),
            None => {
                warnings.push("AMP did not provide a trustworthy player count".to_owned());
                (0, false)
            }
        };
        Ok(AmpServer {
            id: format!("amp:{id}"),
            name,
            instance_name,
            kind: "imported",
            software,
            version,
            status: if online {
                "online"
            } else if panel_running {
                "offline"
            } else {
                "manager_stopped"
            }
            .to_owned(),
            panel_running,
            start_on_boot: boolean(instance, "DaemonAutostart"),
            players_online,
            player_count_verified,
            max_players,
            cpu_percent: metric_f64(metrics, "CPU Usage", "RawValue").unwrap_or(0.0),
            memory_used_mb: metric(metrics, "Memory Usage", "RawValue").unwrap_or(0),
            memory_limit_mb,
            tps: metric_f64(metrics, "TPS", "RawValue"),
            manager_panel_port: self.public_panel_port,
            panel_port,
            game_port: config
                .get("Minecraft.PortNumber")
                .and_then(|value| value.parse().ok()),
            path: path.to_string_lossy().into_owned(),
            warnings,
            manager: "amp_import",
            execution_backend: "external",
        })
    }

    fn local_instances(&self) -> Result<Vec<Value>, String> {
        self.call_ads("ADSModule", "GetLocalInstances", json!({}))?
            .as_array()
            .cloned()
            .ok_or_else(|| "AMP returned an invalid instance list".to_owned())
    }

    fn begin_operation(&self, instance_id: &str) -> Result<AmpOperationGuard<'_>, String> {
        if instance_id.is_empty()
            || instance_id.len() > 128
            || instance_id
                .chars()
                .any(|character| character.is_control() || matches!(character, '/' | '\\'))
        {
            return Err("the AMP instance identifier is invalid".to_owned());
        }
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "AMP operation registry failed".to_owned())?;
        if !operations.insert(instance_id.to_owned()) {
            return Err("another AMP operation is already in progress for this server".to_owned());
        }
        drop(operations);
        Ok(AmpOperationGuard {
            operations: &self.operations,
            instance_id: instance_id.to_owned(),
        })
    }

    fn wait_for_api(&self, port: u16, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if TcpStream::connect_timeout(
                &SocketAddr::new(self.endpoint.ip(), port),
                Duration::from_secs(1),
            )
            .is_ok()
            {
                return Ok(());
            }
            thread::sleep(Duration::from_secs(2));
        }
        Err("the AMP instance did not become reachable in time".to_owned())
    }

    fn call_ads(&self, module: &str, method: &str, parameters: Value) -> Result<Value, String> {
        self.call(self.endpoint.port(), module, method, parameters)
    }

    fn call(
        &self,
        port: u16,
        module: &str,
        method: &str,
        parameters: Value,
    ) -> Result<Value, String> {
        let session = self.session(port)?;
        match self.post(port, module, method, &parameters, Some(&session)) {
            Ok(value) => Ok(value),
            Err(HttpError::Unauthorized) => {
                self.sessions
                    .lock()
                    .map_err(|_| "AMP session cache failed".to_owned())?
                    .remove(&port);
                let session = self.session(port)?;
                self.post(port, module, method, &parameters, Some(&session))
                    .map_err(http_error)
            }
            Err(error) => Err(http_error(error)),
        }
    }

    fn session(&self, port: u16) -> Result<String, String> {
        if let Some(session) = self
            .sessions
            .lock()
            .map_err(|_| "AMP session cache failed".to_owned())?
            .get(&port)
            .cloned()
        {
            return Ok(session);
        }
        let login = self
            .post(
                port,
                "Core",
                "Login",
                &json!({
                    "username": self.username,
                    "password": self.password.expose_secret(),
                    "token": "",
                    "rememberMe": false
                }),
                None,
            )
            .map_err(http_error)?;
        if !boolean(&login, "success") {
            return Err("AMP rejected the configured service login".to_owned());
        }
        let session =
            text(&login, "sessionID").ok_or_else(|| "AMP login returned no session".to_owned())?;
        self.sessions
            .lock()
            .map_err(|_| "AMP session cache failed".to_owned())?
            .insert(port, session.clone());
        Ok(session)
    }

    fn post(
        &self,
        port: u16,
        module: &str,
        method: &str,
        parameters: &Value,
        session: Option<&str>,
    ) -> Result<Value, HttpError> {
        if !module.bytes().all(|byte| byte.is_ascii_alphanumeric())
            || !method.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(HttpError::Protocol);
        }
        let body = serde_json::to_vec(parameters).map_err(|_| HttpError::Protocol)?;
        let address = SocketAddr::new(self.endpoint.ip(), port);
        let mut stream = TcpStream::connect_timeout(&address, API_TIMEOUT)
            .map_err(|_| HttpError::Unavailable)?;
        stream
            .set_read_timeout(Some(API_TIMEOUT))
            .map_err(|_| HttpError::Unavailable)?;
        stream
            .set_write_timeout(Some(API_TIMEOUT))
            .map_err(|_| HttpError::Unavailable)?;
        let authorization = session
            .map(|session| format!("Authorization: Bearer {session}\r\n"))
            .unwrap_or_default();
        let header = format!(
            "POST /API/{module}/{method} HTTP/1.1\r\nHost: {}:{port}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{authorization}\r\n",
            self.endpoint.ip(),
            body.len(),
        );
        stream
            .write_all(header.as_bytes())
            .map_err(|_| HttpError::Unavailable)?;
        stream
            .write_all(&body)
            .map_err(|_| HttpError::Unavailable)?;
        let mut response = Vec::new();
        stream
            .take(MAX_HTTP_RESPONSE_BYTES + 1)
            .read_to_end(&mut response)
            .map_err(|_| HttpError::Unavailable)?;
        if response.len() as u64 > MAX_HTTP_RESPONSE_BYTES {
            return Err(HttpError::Protocol);
        }
        parse_http_response(&response)
    }
}

#[derive(Debug)]
enum HttpError {
    Unauthorized,
    Unavailable,
    Rejected(String),
    Protocol,
}

#[derive(Debug)]
struct MapServerError {
    code: &'static str,
    message: &'static str,
}

impl MapServerError {
    const fn invalid_identity() -> Self {
        Self {
            code: "invalid_instance_identity",
            message: "AMP returned a Minecraft instance with an invalid identity.",
        }
    }

    const fn invalid_panel_state() -> Self {
        Self {
            code: "invalid_instance_panel_state",
            message: "AMP returned incomplete or invalid Minecraft panel state.",
        }
    }

    const fn invalid_instance_path() -> Self {
        Self {
            code: "invalid_instance_path",
            message: "AMP returned a Minecraft instance whose file path was not safe to inspect.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfigReadError {
    Unavailable,
    UnsafePath,
}

fn parse_http_response(response: &[u8]) -> Result<Value, HttpError> {
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(HttpError::Protocol)?;
    let header = std::str::from_utf8(&response[..separator]).map_err(|_| HttpError::Protocol)?;
    let mut lines = header.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(HttpError::Protocol)?;
    if status == 401 || status == 403 {
        return Err(HttpError::Unauthorized);
    }
    let chunked = lines.clone().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
        })
    });
    let raw_body = &response[separator + 4..];
    let body = if chunked {
        decode_chunked(raw_body)?
    } else {
        raw_body.to_vec()
    };
    if !(200..300).contains(&status) {
        let message = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| text(&value, "Message").or_else(|| text(&value, "Title")))
            .unwrap_or_else(|| format!("AMP returned HTTP {status}"));
        return Err(HttpError::Rejected(message));
    }
    let body = body.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&body);
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(body).map_err(|_| HttpError::Protocol)
}

fn decode_chunked(input: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut remaining = input;
    let mut output = Vec::new();
    loop {
        let line_end = remaining
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or(HttpError::Protocol)?;
        let size_text =
            std::str::from_utf8(&remaining[..line_end]).map_err(|_| HttpError::Protocol)?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or(""), 16)
            .map_err(|_| HttpError::Protocol)?;
        remaining = &remaining[line_end + 2..];
        if size == 0 {
            break;
        }
        if remaining.len() < size + 2 || &remaining[size..size + 2] != b"\r\n" {
            return Err(HttpError::Protocol);
        }
        output.extend_from_slice(&remaining[..size]);
        if output.len() as u64 > MAX_HTTP_RESPONSE_BYTES {
            return Err(HttpError::Protocol);
        }
        remaining = &remaining[size + 2..];
    }
    Ok(output)
}

fn http_error(error: HttpError) -> String {
    match error {
        HttpError::Unauthorized => "AMP authorization failed".to_owned(),
        HttpError::Unavailable => "AMP is unavailable".to_owned(),
        HttpError::Rejected(message) => message,
        HttpError::Protocol => "AMP returned an invalid response".to_owned(),
    }
}

fn ensure_action_result(value: &Value) -> Result<(), String> {
    if value.is_null() || value == &Value::Bool(true) {
        return Ok(());
    }
    if let Some(status) = value.get("Status").and_then(Value::as_bool) {
        if status {
            return Ok(());
        }
        return Err(text(value, "Message")
            .or_else(|| text(value, "Reason"))
            .unwrap_or_else(|| "AMP rejected the operation".to_owned()));
    }
    if value.get("Id").is_some() {
        return Ok(());
    }
    Err("AMP returned an unexpected operation result".to_owned())
}

fn read_instance_kvp(
    instance_root: &Path,
    instance_name: &str,
) -> Result<HashMap<String, String>, ConfigReadError> {
    read_instance_kvp_file(instance_root, instance_name, "MinecraftModule.kvp")
}

fn list_instance_kvp_names(
    instance_root: &Path,
    instance_name: &str,
) -> Result<Vec<String>, ConfigReadError> {
    validate_instance_name(instance_name).map_err(|_| ConfigReadError::UnsafePath)?;
    reject_symlink(instance_root)?;
    let instance_path = instance_root.join(instance_name);
    reject_symlink(&instance_path)?;
    let mut names = Vec::new();
    let entries = fs::read_dir(&instance_path).map_err(|_| ConfigReadError::Unavailable)?;
    for entry in entries {
        if names.len() >= MAX_KVP_FILES_PER_INSTANCE {
            break;
        }
        let entry = entry.map_err(|_| ConfigReadError::Unavailable)?;
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|_| ConfigReadError::Unavailable)?;
        if !metadata.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_kvp_file_name(&name).is_ok() {
            names.push(name);
        }
    }
    Ok(names)
}

fn read_instance_kvp_file(
    instance_root: &Path,
    instance_name: &str,
    file_name: &str,
) -> Result<HashMap<String, String>, ConfigReadError> {
    validate_instance_name(instance_name).map_err(|_| ConfigReadError::UnsafePath)?;
    validate_kvp_file_name(file_name).map_err(|_| ConfigReadError::UnsafePath)?;
    reject_symlink(instance_root)?;
    let instance_path = instance_root.join(instance_name);
    reject_symlink(&instance_path)?;
    reject_symlink(&instance_path.join(file_name))?;

    let root = open_absolute_directory_without_symlinks(instance_root)?;
    let instance_name =
        CString::new(instance_name.as_bytes()).map_err(|_| ConfigReadError::UnsafePath)?;
    let instance = open_directory_at(&root, instance_name.as_c_str())?;
    let config_name =
        CString::new(file_name.as_bytes()).map_err(|_| ConfigReadError::UnsafePath)?;
    let descriptor = rustix::fs::openat(
        &instance,
        config_name.as_c_str(),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )
    .map_err(map_config_open_error)?;
    let metadata = rustix::fs::fstat(&descriptor).map_err(|_| ConfigReadError::Unavailable)?;
    if !rustix::fs::FileType::from_raw_mode(metadata.st_mode).is_file() {
        return Err(ConfigReadError::UnsafePath);
    }
    let length = u64::try_from(metadata.st_size).map_err(|_| ConfigReadError::UnsafePath)?;
    if length > MAX_CONFIG_BYTES {
        return Err(ConfigReadError::Unavailable);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    fs::File::from(descriptor)
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ConfigReadError::Unavailable)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ConfigReadError::Unavailable);
    }
    let text = String::from_utf8(bytes).map_err(|_| ConfigReadError::Unavailable)?;
    Ok(text
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| !key.is_empty() && key.len() <= 256)
        .map(|(key, value)| (key.to_owned(), value.trim().to_owned()))
        .collect())
}

fn validate_kvp_file_name(file_name: &str) -> Result<(), ()> {
    if file_name.len() < 5
        || file_name.len() > 255
        || !file_name.ends_with(".kvp")
        || file_name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(());
    }
    let mut components = Path::new(file_name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(());
    }
    Ok(())
}

fn insert_nonzero_port(ports: &mut HashSet<u16>, port: u16) {
    if port != 0 {
        ports.insert(port);
    }
}

fn collect_ports_from_kvp(config: &HashMap<String, String>, ports: &mut HashSet<u16>) {
    for (key, value) in config {
        if key.to_ascii_lowercase().contains("port") {
            collect_ports_from_amp_value(value, ports);
        }
    }
}

fn collect_ports_from_amp_value(value: &str, ports: &mut HashSet<u16>) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Ok(port) = trimmed.parse::<u16>() {
        insert_nonzero_port(ports, port);
        return;
    }
    if (trimmed.starts_with('[') || trimmed.starts_with('{'))
        && let Ok(json) = serde_json::from_str::<Value>(trimmed)
    {
        collect_ports_from_amp_json(&json, ports, true);
        return;
    }
    for part in trimmed.split(|character: char| matches!(character, ',' | ';' | ' ' | '\t' | '/')) {
        if let Ok(port) = part.trim().parse::<u16>() {
            insert_nonzero_port(ports, port);
        }
    }
}

fn collect_ports_from_amp_json(value: &Value, ports: &mut HashSet<u16>, port_context: bool) {
    match value {
        Value::Number(number) if port_context => {
            if let Some(port) = number.as_u64().and_then(|value| u16::try_from(value).ok()) {
                insert_nonzero_port(ports, port);
            }
        }
        Value::String(text) if port_context => collect_ports_from_amp_value(text, ports),
        Value::Array(items) => {
            for item in items {
                collect_ports_from_amp_json(item, ports, port_context);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                collect_ports_from_amp_json(item, ports, key.to_ascii_lowercase().contains("port"));
            }
        }
        _ => {}
    }
}

fn validate_instance_name(instance_name: &str) -> Result<(), ()> {
    if instance_name.is_empty()
        || instance_name.len() > 255
        || instance_name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(());
    }
    let mut components = Path::new(instance_name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(());
    }
    Ok(())
}

fn validate_instance_identifier(instance_id: &str) -> Result<(), ()> {
    if instance_id.is_empty()
        || instance_id.len() > 128
        || instance_id
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(());
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), ConfigReadError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(ConfigReadError::UnsafePath),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(ConfigReadError::Unavailable)
        }
        Err(_) => Err(ConfigReadError::Unavailable),
    }
}

fn open_absolute_directory_without_symlinks(path: &Path) -> Result<OwnedFd, ConfigReadError> {
    if !path.is_absolute() {
        return Err(ConfigReadError::UnsafePath);
    }
    let mut descriptor = rustix::fs::open("/", directory_open_flags(), rustix::fs::Mode::empty())
        .map_err(|_| ConfigReadError::Unavailable)?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let name =
                    CString::new(name.as_bytes()).map_err(|_| ConfigReadError::UnsafePath)?;
                descriptor = open_directory_at(&descriptor, name.as_c_str())?;
            }
            _ => return Err(ConfigReadError::UnsafePath),
        }
    }
    Ok(descriptor)
}

fn open_directory_at<Fd: std::os::fd::AsFd>(
    descriptor: Fd,
    name: &CStr,
) -> Result<OwnedFd, ConfigReadError> {
    rustix::fs::openat(
        descriptor,
        name,
        directory_open_flags(),
        rustix::fs::Mode::empty(),
    )
    .map_err(map_config_open_error)
}

fn directory_open_flags() -> rustix::fs::OFlags {
    rustix::fs::OFlags::RDONLY
        | rustix::fs::OFlags::DIRECTORY
        | rustix::fs::OFlags::CLOEXEC
        | rustix::fs::OFlags::NOFOLLOW
}

fn map_config_open_error(error: rustix::io::Errno) -> ConfigReadError {
    if matches!(error, rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) {
        ConfigReadError::UnsafePath
    } else {
        ConfigReadError::Unavailable
    }
}

fn display_server_type(value: &str) -> String {
    match value {
        "Official" => "Vanilla",
        "Paper_Spigot" | "Paper" => "Paper",
        "Purpur" => "Purpur",
        "Fabric" => "Fabric",
        "NeoForge" => "NeoForge",
        other => other,
    }
    .to_owned()
}

fn server_version(config: &HashMap<String, String>, software: &str) -> String {
    let key = match software {
        "Paper" => "Minecraft.SpecificPaperVersion",
        "Purpur" => "Minecraft.SpecificPurpurVersion",
        "Fabric" => "Minecraft.FabricMCVersion",
        "NeoForge" => "Minecraft.SpecificNeoForgeVersion",
        _ => "Minecraft.SpecificVersion",
    };
    config
        .get(key)
        .or_else(|| config.get("Minecraft.SpecificVersion"))
        .cloned()
        .unwrap_or_else(|| "Managed by AMP".to_owned())
}

fn metric(
    metrics: Option<&serde_json::Map<String, Value>>,
    name: &str,
    field: &str,
) -> Option<u64> {
    metric_f64(metrics, name, field).map(|value| value.max(0.0).round() as u64)
}

fn metric_f64(
    metrics: Option<&serde_json::Map<String, Value>>,
    name: &str,
    field: &str,
) -> Option<f64> {
    metrics?.get(name)?.get(field)?.as_f64()
}

fn player_count_metric(metrics: Option<&serde_json::Map<String, Value>>) -> Option<u64> {
    let value = metrics?.get("Active Users")?.get("RawValue")?;
    if let Some(value) = value.as_u64() {
        return Some(value);
    }
    let value = value.as_f64()?;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > u64::MAX as f64 {
        return None;
    }
    Some(value as u64)
}

fn required_text(value: &Value, key: &str, maximum: usize) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= maximum)
        .ok_or_else(|| format!("AMP {key} is missing or invalid"))?;
    Ok(text.to_owned())
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .map(str::to_owned)
}

fn bounded_identity(value: &Value, key: &str, maximum: usize) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
}

fn required_u16(value: &Value, key: &str) -> Result<u16, ()> {
    let value = value.get(key).and_then(Value::as_u64).ok_or(())?;
    let value = u16::try_from(value).map_err(|_| ())?;
    if value == 0 {
        return Err(());
    }
    Ok(value)
}

fn required_boolean(value: &Value, key: &str) -> Result<bool, ()> {
    value.get(key).and_then(Value::as_bool).ok_or(())
}

fn boolean(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn chrono_free_timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("backup-{seconds}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client(instance_root: &Path) -> AmpClient {
        AmpClient {
            endpoint: "127.0.0.1:1".parse().unwrap(),
            public_panel_port: 8080,
            username: "test".to_owned(),
            password: SecretString::from("test".to_owned()),
            instance_root: instance_root.to_owned(),
            sessions: Mutex::new(HashMap::new()),
            operations: Mutex::new(HashSet::new()),
        }
    }

    fn valid_instance(instance_name: &str) -> Value {
        json!({
            "Module": "Minecraft",
            "InstanceID": "71b629b7-5861-47b8-907b-acde40dadc9e",
            "InstanceName": instance_name,
            "FriendlyName": "Survival",
            "Port": 8081,
            "Running": true,
            "DaemonAutostart": true,
            "Metrics": {
                "Active Users": {"RawValue": 0, "MaxValue": 20},
                "CPU Usage": {"RawValue": 1.5},
                "Memory Usage": {"RawValue": 512, "MaxValue": 4096}
            }
        })
    }

    fn create_instance_root(instance_name: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let instance = root.path().join(instance_name);
        fs::create_dir(&instance).unwrap();
        fs::write(
            instance.join("MinecraftModule.kvp"),
            "Minecraft.ServerType=Paper\nMinecraft.SpecificPaperVersion=1.21.8\nMinecraft.PortNumber=25565\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn http_parser_handles_content_length_and_empty_success() {
        let value = parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\n\r\n{\"Status\":true}",
        )
        .expect("parse response");
        assert_eq!(value["Status"], true);
        assert_eq!(
            parse_http_response(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .expect("empty response"),
            Value::Null
        );
    }

    #[test]
    fn http_parser_handles_chunked_json() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"ok\":1\r\n1\r\n}\r\n0\r\n\r\n";
        assert_eq!(
            parse_http_response(response).expect("chunked response")["ok"],
            1
        );
    }

    #[test]
    fn amp_operation_guard_releases_the_instance() {
        let operations = Mutex::new(HashSet::new());
        operations.lock().unwrap().insert("instance".to_owned());
        let guard = AmpOperationGuard {
            operations: &operations,
            instance_id: "instance".to_owned(),
        };
        assert!(operations.lock().unwrap().contains("instance"));
        drop(guard);
        assert!(!operations.lock().unwrap().contains("instance"));
    }

    #[test]
    fn amp_action_results_reject_ambiguous_payloads() {
        assert!(ensure_action_result(&Value::Null).is_ok());
        assert!(ensure_action_result(&json!({"Status": true})).is_ok());
        assert!(ensure_action_result(&json!({"Status": false})).is_err());
        assert!(ensure_action_result(&json!({"unexpected": true})).is_err());
    }

    #[test]
    fn malformed_minecraft_items_are_reported_instead_of_dropped() {
        let root = create_instance_root("valid");
        let client = test_client(root.path());
        let mut malformed = valid_instance("valid");
        malformed.as_object_mut().unwrap().remove("InstanceID");
        let inventory = client
            .parse_inventory(&json!([
                valid_instance("valid"),
                malformed,
                {"InstanceID": "unknown-module"},
                {"Module": "GenericModule", "InstanceID": "ignored"}
            ]))
            .unwrap();

        assert_eq!(inventory.servers.len(), 1);
        assert_eq!(inventory.servers[0].manager_panel_port, 8080);
        assert_eq!(inventory.issue_count, 2);
        assert_eq!(inventory.issues[0].code, "invalid_instance_identity");
        assert_eq!(inventory.issues[1].code, "invalid_instance_module");
    }

    #[test]
    fn missing_or_unparseable_player_metrics_are_explicitly_unverified() {
        let root = create_instance_root("players");
        let client = test_client(root.path());
        for raw_value in [Value::Null, Value::String("zero".to_owned()), json!(-1)] {
            let mut instance = valid_instance("players");
            instance["Metrics"]["Active Users"]["RawValue"] = raw_value;
            let inventory = client.parse_inventory(&json!([instance])).unwrap();
            assert_eq!(inventory.servers.len(), 1);
            assert_eq!(inventory.servers[0].players_online, 0);
            assert!(!inventory.servers[0].player_count_verified);
            assert!(
                inventory.servers[0]
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("player count"))
            );
        }

        let mut stopped_without_metrics = valid_instance("players");
        stopped_without_metrics["Running"] = Value::Bool(false);
        stopped_without_metrics
            .as_object_mut()
            .unwrap()
            .remove("Metrics");
        let inventory = client
            .parse_inventory(&json!([stopped_without_metrics]))
            .unwrap();
        assert_eq!(inventory.servers[0].status, "manager_stopped");
        assert!(!inventory.servers[0].player_count_verified);
    }

    #[test]
    fn traversal_and_absolute_instance_names_are_never_inspected() {
        let root = tempfile::tempdir().unwrap();
        let client = test_client(root.path());
        for instance_name in [
            "../outside",
            "/outside",
            "nested/server",
            "nested\\server",
            ".",
            "..",
        ] {
            let inventory = client
                .parse_inventory(&json!([valid_instance(instance_name)]))
                .unwrap();
            assert!(inventory.servers.is_empty(), "{instance_name}");
            assert_eq!(inventory.issue_count, 1, "{instance_name}");
            assert_eq!(inventory.issues[0].code, "invalid_instance_path");
        }
    }

    #[test]
    fn symlinked_instance_and_configuration_paths_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(
            outside.path().join("MinecraftModule.kvp"),
            "Minecraft.PortNumber=25565\n",
        )
        .unwrap();
        symlink(outside.path(), root.path().join("linked-instance")).unwrap();
        let client = test_client(root.path());
        let inventory = client
            .parse_inventory(&json!([valid_instance("linked-instance")]))
            .unwrap();
        assert!(inventory.servers.is_empty());
        assert_eq!(inventory.issues[0].code, "invalid_instance_path");

        let instance = root.path().join("linked-config");
        fs::create_dir(&instance).unwrap();
        symlink(
            outside.path().join("MinecraftModule.kvp"),
            instance.join("MinecraftModule.kvp"),
        )
        .unwrap();
        let inventory = client
            .parse_inventory(&json!([valid_instance("linked-config")]))
            .unwrap();
        assert!(inventory.servers.is_empty());
        assert_eq!(inventory.issues[0].code, "invalid_instance_path");
    }

    #[test]
    fn kvp_port_collection_keeps_amp_ports_and_ignores_unrelated_numbers() {
        let mut config = HashMap::new();
        config.insert("Minecraft.PortNumber".to_owned(), "25566".to_owned());
        config.insert("Minecraft.QueryPort".to_owned(), "25567".to_owned());
        config.insert("Limits.MaxPlayers".to_owned(), "20".to_owned());
        config.insert("Java.MaxHeapSizeMB".to_owned(), "4096".to_owned());
        config.insert(
            "Minecraft.ApplicationPortBindings".to_owned(),
            r#"[{"Protocol":"TCP","Port":25575,"MaxPlayers":40}]"#.to_owned(),
        );
        let mut ports = HashSet::new();
        collect_ports_from_kvp(&config, &mut ports);
        assert_eq!(ports, HashSet::from([25566, 25567, 25575]));
    }

    #[test]
    fn occupied_ports_include_every_amp_instance_port_from_disk() {
        let root = tempfile::tempdir().unwrap();
        let instance = root.path().join("Survival01");
        fs::create_dir(&instance).unwrap();
        fs::write(
            instance.join("MinecraftModule.kvp"),
            "Minecraft.ServerType=Paper\nMinecraft.PortNumber=25566\nMinecraft.QueryPort=25567\nLimits.MaxPlayers=20\n",
        )
        .unwrap();
        fs::write(
            instance.join("FileManagerPlugin.kvp"),
            "FileManager.Port=8082\n",
        )
        .unwrap();
        let client = test_client(root.path());
        let ports = client.occupied_ports();
        assert!(ports.contains(&8080), "{ports:?}");
        assert!(ports.contains(&25566), "{ports:?}");
        assert!(ports.contains(&25567), "{ports:?}");
        assert!(ports.contains(&8082), "{ports:?}");
        assert!(!ports.contains(&20), "{ports:?}");
        assert_eq!(
            amp_port_claimed_message(25566),
            "AMP already has port 25566 claimed"
        );
    }
}
