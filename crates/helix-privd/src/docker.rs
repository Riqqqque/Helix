use crate::host::{HostControl, parse_human_bytes, require_success, write_atomic_file};
use helix_privd::DockerContainerActionKind;
use rusqlite::{Connection, OpenFlags, backup::Backup};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const DASHBOARD_ICONS_PNG: &str = "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/png";

const MAX_CONTAINERS: usize = 128;
const MAX_HOMARR_WIDGETS: usize = 64;
const MAX_HOMARR_DB_BYTES: u64 = 32 * 1024 * 1024;
const MIN_DOCKER_CLEANUP_RETENTION_HOURS: u16 = 24;
const MAX_DOCKER_CLEANUP_RETENTION_HOURS: u16 = 8_760;
const DOCKER_CLEANUP_LAST_RUN: &str = "last-run.json";
const MAX_DOCKER_CLEANUP_RECORD_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug)]
struct DockerDiskUsage {
    storage: Value,
    categories: Vec<Value>,
    reclaimable_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DockerCleanupRunRecord {
    schema_version: u32,
    run_id: String,
    trigger: String,
    status: String,
    retention_hours: u16,
    started_at_unix_ms: u64,
    finished_at_unix_ms: Option<u64>,
    reclaimable_before_bytes: u64,
    available_before_bytes: u64,
    available_after_bytes: Option<u64>,
    completed_steps: Vec<String>,
    error: Option<String>,
}

#[derive(Debug)]
enum HomarrSqliteRead {
    Absent,
    Found(Vec<Value>),
    Unreadable(String),
}

struct HomarrCatalogScan {
    widgets: Vec<Value>,
    source: Option<&'static str>,
    sqlite_note: Option<String>,
    sqlite_empty: bool,
}

impl HostControl {
    pub fn docker_inventory(&self) -> Result<Value, String> {
        self.collect_docker_inventory(true)
    }

    pub fn docker_inventory_listing(&self) -> Result<Value, String> {
        self.collect_docker_inventory(false)
    }

    pub fn docker_cleanup_status(&self) -> Result<Value, String> {
        let schedule = self.docker_cleanup_schedule_status()?;
        let last_run = self.read_docker_cleanup_last_run().unwrap_or_else(|error| {
            json!({
                "status": "unavailable",
                "error": sanitize_label(&error, 500)
            })
        });
        match self.collect_docker_disk_usage() {
            Ok(usage) => Ok(json!({
                "schema_version": 1,
                "availability": "ready",
                "docker_installed": true,
                "storage": usage.storage,
                "usage": usage.categories,
                "reclaimable_bytes": usage.reclaimable_bytes,
                "policy": docker_cleanup_policy(),
                "schedule": schedule,
                "last_run": last_run,
                "collected_at_unix_ms": now_unix_ms()
            })),
            Err(error) => Ok(json!({
                "schema_version": 1,
                "availability": "unavailable",
                "docker_installed": false,
                "storage": Value::Null,
                "usage": [],
                "reclaimable_bytes": 0,
                "policy": docker_cleanup_policy(),
                "schedule": schedule,
                "last_run": last_run,
                "error": sanitize_label(&error, 500),
                "collected_at_unix_ms": now_unix_ms()
            })),
        }
    }

    pub fn run_safe_docker_cleanup(
        &self,
        retention_hours: u16,
        trigger: &str,
    ) -> Result<Value, String> {
        validate_docker_cleanup_retention(retention_hours)?;
        if !matches!(trigger, "manual" | "scheduled") {
            return Err("Docker cleanup trigger is invalid".to_owned());
        }
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "Docker cleanup lock failed".to_owned())?;
        let before = self.collect_docker_disk_usage()?;
        let started = now_unix_ms();
        let mut record = DockerCleanupRunRecord {
            schema_version: 1,
            run_id: Uuid::new_v4().to_string(),
            trigger: trigger.to_owned(),
            status: "running".to_owned(),
            retention_hours,
            started_at_unix_ms: started,
            finished_at_unix_ms: None,
            reclaimable_before_bytes: before.reclaimable_bytes,
            available_before_bytes: storage_available_bytes(&before.storage),
            available_after_bytes: None,
            completed_steps: Vec::new(),
            error: None,
        };
        self.write_docker_cleanup_run(&record)?;
        let steps = docker_cleanup_steps(retention_hours);
        for (step, args) in steps {
            if let Err(error) = self.docker_command(&args, Duration::from_secs(10 * 60)) {
                record.status = "failed".to_owned();
                record.finished_at_unix_ms = Some(now_unix_ms());
                let error = sanitize_label(&error, 500);
                record.error = Some(if error.is_empty() {
                    "Docker command failed".to_owned()
                } else {
                    error
                });
                let history = self.write_docker_cleanup_run(&record);
                return Err(match history {
                    Ok(()) => format!(
                        "Docker cleanup stopped after {} safe step(s): {}",
                        record.completed_steps.len(),
                        record.error.as_deref().unwrap_or("Docker command failed")
                    ),
                    Err(history) => format!(
                        "Docker cleanup stopped after {} safe step(s), and its result could not be recorded: {history}",
                        record.completed_steps.len()
                    ),
                });
            }
            record.completed_steps.push(step.to_owned());
        }
        let after = self.collect_docker_disk_usage().ok();
        record.status = "complete".to_owned();
        record.finished_at_unix_ms = Some(now_unix_ms());
        record.available_after_bytes = after
            .as_ref()
            .map(|usage| storage_available_bytes(&usage.storage));
        self.write_docker_cleanup_run(&record).map_err(|error| {
            format!("Docker cleanup finished, but Helix could not record its final result: {error}")
        })?;
        let available_increase_bytes = record
            .available_after_bytes
            .unwrap_or(record.available_before_bytes)
            .saturating_sub(record.available_before_bytes);
        Ok(json!({
            "schema_version": 1,
            "run_id": record.run_id,
            "status": "complete",
            "trigger": trigger,
            "retention_hours": retention_hours,
            "completed_steps": record.completed_steps,
            "reclaimable_before_bytes": record.reclaimable_before_bytes,
            "available_before_bytes": record.available_before_bytes,
            "available_after_bytes": record.available_after_bytes,
            "available_increase_bytes": available_increase_bytes,
            "history_recorded": true,
            "excluded": ["containers", "volumes", "named_images", "active_resources"],
            "finished_at_unix_ms": record.finished_at_unix_ms
        }))
    }

    fn collect_docker_disk_usage(&self) -> Result<DockerDiskUsage, String> {
        let info = self.docker_command(
            &[
                "info".to_owned(),
                "--format".to_owned(),
                "{{.DockerRootDir}}\t{{.Driver}}".to_owned(),
            ],
            Duration::from_secs(20),
        )?;
        let mut fields = info.stdout.trim().splitn(2, '\t');
        let root = fields.next().unwrap_or_default().trim();
        let driver = sanitize_label(fields.next().unwrap_or_default(), 64);
        if root.is_empty()
            || root.len() > 4_096
            || !Path::new(root).is_absolute()
            || root.chars().any(char::is_control)
        {
            return Err("Docker returned an invalid data-root path".to_owned());
        }
        let root_path = Path::new(root);
        let stats = rustix::fs::statvfs(root_path)
            .map_err(|_| "Helix could not measure Docker's data-root filesystem".to_owned())?;
        let (mount_point, source) = mount_for_path(root_path)
            .unwrap_or_else(|| ("unavailable".to_owned(), "unavailable".to_owned()));
        let storage = json!({
            "data_root": root,
            "storage_driver": driver,
            "mount_point": mount_point,
            "source": source,
            "total_bytes": stats.f_blocks.saturating_mul(stats.f_frsize),
            "available_bytes": stats.f_bavail.saturating_mul(stats.f_frsize),
            "location_controlled_by": "docker_daemon"
        });
        let usage_output = self.docker_command(
            &[
                "system".to_owned(),
                "df".to_owned(),
                "--format".to_owned(),
                "{{json .}}".to_owned(),
            ],
            Duration::from_secs(30),
        )?;
        let categories = parse_docker_disk_usage(&usage_output.stdout)?;
        let reclaimable_bytes = categories.iter().fold(0_u64, |total, item| {
            total.saturating_add(
                item.get("reclaimable_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            )
        });
        Ok(DockerDiskUsage {
            storage,
            categories,
            reclaimable_bytes,
        })
    }

    fn docker_cleanup_last_run_path(&self) -> PathBuf {
        self.config
            .docker_cleanup_state_root
            .join(DOCKER_CLEANUP_LAST_RUN)
    }

    fn read_docker_cleanup_last_run(&self) -> Result<Value, String> {
        let path = self.docker_cleanup_last_run_path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Value::Null);
            }
            Err(_) => return Err("could not inspect the Docker cleanup history".to_owned()),
        };
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if !metadata.file_type().is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.len() == 0
            || metadata.len() > MAX_DOCKER_CLEANUP_RECORD_BYTES
        {
            return Err("the Docker cleanup history is unsafe".to_owned());
        }
        let record = serde_json::from_slice::<DockerCleanupRunRecord>(
            &fs::read(path).map_err(|_| "could not read Docker cleanup history".to_owned())?,
        )
        .map_err(|_| "the Docker cleanup history is invalid".to_owned())?;
        validate_docker_cleanup_run(&record)?;
        serde_json::to_value(record)
            .map_err(|_| "could not encode Docker cleanup history".to_owned())
    }

    fn write_docker_cleanup_run(&self, record: &DockerCleanupRunRecord) -> Result<(), String> {
        validate_docker_cleanup_run(record)?;
        let body = serde_json::to_vec_pretty(record)
            .map_err(|_| "could not encode Docker cleanup history".to_owned())?;
        write_atomic_file(
            &self.docker_cleanup_last_run_path(),
            &body,
            0o600,
            "Docker cleanup history",
        )
    }

    fn collect_docker_inventory(&self, include_stats: bool) -> Result<Value, String> {
        let listed = match self.list_docker_containers() {
            Ok(output) => output,
            Err(error) => {
                return Ok(json!({
                    "schema_version": 1,
                    "availability": "unavailable",
                    "docker_installed": false,
                    "containers": [],
                    "truncated": false,
                    "portainer": { "detected": false, "panel_port": Value::Null, "panel_scheme": Value::Null, "container": Value::Null },
                    "error": error,
                    "collected_at_unix_ms": now_unix_ms()
                }));
            }
        };
        let mut containers = parse_docker_listing(&listed);
        let truncated = containers.len() > MAX_CONTAINERS;
        containers.truncate(MAX_CONTAINERS);
        let running: Vec<String> = containers
            .iter()
            .filter(|item| item.get("running").and_then(Value::as_bool) == Some(true))
            .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_owned))
            .take(MAX_CONTAINERS)
            .collect();
        if include_stats && !running.is_empty() {
            let mut args = vec![
                "stats".to_owned(),
                "--no-stream".to_owned(),
                "--format".to_owned(),
                "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.PIDs}}".to_owned(),
            ];
            args.extend(running);
            if let Ok(stats) = self.docker_command(&args, Duration::from_secs(12)) {
                apply_docker_stats(&mut containers, &stats.stdout);
            }
        }
        for container in &mut containers {
            if let Some(object) = container.as_object_mut() {
                let name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                object.insert(
                    "protected".to_owned(),
                    Value::Bool(self.container_is_protected(&name)),
                );
            }
        }
        let portainer = detect_portainer(&containers);
        Ok(json!({
            "schema_version": 1,
            "availability": "ready",
            "docker_installed": true,
            "containers": containers,
            "truncated": truncated,
            "portainer": portainer,
            "error": Value::Null,
            "note": "Helix lists Docker Engine containers on this host, including ones Portainer also shows. Start, stop, and restart require typing the exact container name. Helix dashboard, gateway, and native game containers cannot be stopped here.",
            "collected_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn docker_container_action(
        &self,
        name: &str,
        action: DockerContainerActionKind,
        confirmation: &str,
    ) -> Result<Value, String> {
        validate_container_name(name)?;
        if confirmation != name {
            return Err("type the exact container name to confirm this action".to_owned());
        }
        if self.container_is_protected(name) {
            return Err(if is_helix_game_container(name) {
                "Helix will not start, stop, or restart a native game container from this page; use Servers so the stop is graceful and health-checked"
                    .to_owned()
            } else {
                "Helix will not start, stop, or restart its own dashboard or gateway container from this page"
                    .to_owned()
            });
        }
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "Docker mutation lock failed".to_owned())?;
        let verb = match action {
            DockerContainerActionKind::Start => "start",
            DockerContainerActionKind::Stop => "stop",
            DockerContainerActionKind::Restart => "restart",
        };
        let output =
            self.docker_command(&[verb.to_owned(), name.to_owned()], Duration::from_secs(45))?;
        let _ = require_success(output)?;
        let inventory = self.docker_inventory()?;
        let container = inventory
            .get("containers")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
            })
            .cloned()
            .unwrap_or(Value::Null);
        Ok(json!({
            "schema_version": 1,
            "name": name,
            "action": verb,
            "verified": true,
            "container": container,
            "updated_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn homarr_widget_catalog(&self) -> Result<Value, String> {
        let inventory = self.docker_inventory()?;
        let containers = inventory
            .get("containers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let Some(container) = containers.iter().find(|item| {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let image = item
                .get("image")
                .and_then(Value::as_str)
                .unwrap_or_default();
            looks_like_homarr(name, image)
        }) else {
            return Ok(json!({
                "schema_version": 1,
                "availability": "not_found",
                "container": Value::Null,
                "widgets": [],
                "note": "Helix did not find a Homarr container on this host. Start Homarr in Docker, then try again.",
                "collected_at_unix_ms": now_unix_ms()
            }));
        };
        let name = container
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let inspect = self.docker_command(
            &[
                "inspect".to_owned(),
                "--format".to_owned(),
                "{{json .Mounts}}".to_owned(),
                name.clone(),
            ],
            Duration::from_secs(15),
        )?;
        let mounts: Value = serde_json::from_str(inspect.stdout.trim())
            .map_err(|_| "Docker returned invalid Homarr mount metadata".to_owned())?;
        let scan = scan_homarr_mounts(&mounts);
        if scan.sqlite_empty {
            return Ok(json!({
                "schema_version": 1,
                "availability": "ready",
                "container": name,
                "source": scan.source,
                "widgets": [],
                "note": "Homarr's app catalog has no http(s) addresses Helix can import. Relative links and Homarr-only apps stay in Homarr.",
                "collected_at_unix_ms": now_unix_ms()
            }));
        }
        if scan.widgets.is_empty() {
            let note = scan.sqlite_note.unwrap_or_else(|| {
                "Homarr is running, but Helix could not read its app list from the container mounts. Helix looks for classic Homarr JSON and for a SQLite app catalog.".to_owned()
            });
            return Ok(json!({
                "schema_version": 1,
                "availability": "unsupported_format",
                "container": name,
                "widgets": [],
                "note": note,
                "collected_at_unix_ms": now_unix_ms()
            }));
        }
        Ok(json!({
            "schema_version": 1,
            "availability": "ready",
            "container": name,
            "source": scan.source,
            "widgets": scan.widgets,
            "note": "Helix places these on a Homarr Home in Homarr layout order and matches icons from names, links, or Homarr icon slugs. Uploaded Homarr files stay in Homarr.",
            "collected_at_unix_ms": now_unix_ms()
        }))
    }

    fn list_docker_containers(&self) -> Result<String, String> {
        let compact = self.docker_command(
            &[
                "ps".to_owned(),
                "-a".to_owned(),
                "--format".to_owned(),
                "{{.Names}}\t{{.Image}}\t{{.State}}\t{{.Status}}\t{{.Ports}}".to_owned(),
            ],
            Duration::from_secs(20),
        );
        match compact {
            Ok(output) => Ok(output.stdout),
            Err(compact_error) => {
                let json = self.docker_command(
                    &[
                        "ps".to_owned(),
                        "-a".to_owned(),
                        "--format".to_owned(),
                        "{{json .}}".to_owned(),
                    ],
                    Duration::from_secs(20),
                );
                match json {
                    Ok(output) => Ok(output.stdout),
                    Err(_) => {
                        let names = self.docker_command(
                            &[
                                "ps".to_owned(),
                                "-a".to_owned(),
                                "--format".to_owned(),
                                "{{.Names}}".to_owned(),
                            ],
                            Duration::from_secs(20),
                        );
                        match names {
                            Ok(output) => Ok(output.stdout),
                            Err(_) => Err(compact_error),
                        }
                    }
                }
            }
        }
    }

    fn docker_command(
        &self,
        args: &[String],
        timeout: Duration,
    ) -> Result<crate::host::CommandOutput, String> {
        self.runner
            .run(&self.config.docker_binary, args, timeout)
            .and_then(require_success)
    }

    fn container_is_protected(&self, name: &str) -> bool {
        name == self.config.dashboard_container
            || name == self.config.gateway_container
            || is_helix_game_container(name)
    }
}

fn is_helix_game_container(name: &str) -> bool {
    name.strip_prefix("helix-game-")
        .is_some_and(|id| Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id))
}

#[allow(clippy::collapsible_if)]
fn parse_docker_listing(stdout: &str) -> Vec<Value> {
    let trimmed = stdout.trim();
    if trimmed.starts_with('[') {
        if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(trimmed) {
            return items
                .iter()
                .filter_map(container_from_json)
                .take(MAX_CONTAINERS)
                .collect();
        }
    }
    let mut containers = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        if containers.len() >= MAX_CONTAINERS {
            break;
        }
        if let Some(container) = container_from_line(line) {
            containers.push(container);
        }
    }
    containers
}

fn container_from_line(line: &str) -> Option<Value> {
    let line = line.trim();
    if line.starts_with('{') {
        let value: Value = serde_json::from_str(line).ok()?;
        return container_from_json(&value);
    }
    let mut parts = line.splitn(5, '\t');
    let name = sanitize_container_name(parts.next().unwrap_or_default())?;
    let image = sanitize_label(parts.next().unwrap_or_default(), 180);
    let state = sanitize_label(parts.next().unwrap_or_default(), 32).to_ascii_lowercase();
    let status = sanitize_label(parts.next().unwrap_or_default(), 80);
    let ports = sanitize_label(parts.next().unwrap_or_default(), 240);
    Some(container_record(name, image, state, status, ports))
}

fn container_from_json(value: &Value) -> Option<Value> {
    let name = sanitize_container_name(
        value
            .get("Names")
            .or_else(|| value.get("Name"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )?;
    let image = sanitize_label(
        value.get("Image").and_then(Value::as_str).unwrap_or(""),
        180,
    );
    let state = sanitize_label(value.get("State").and_then(Value::as_str).unwrap_or(""), 32)
        .to_ascii_lowercase();
    let status = sanitize_label(
        value.get("Status").and_then(Value::as_str).unwrap_or(""),
        80,
    );
    let ports = sanitize_label(
        value.get("Ports").and_then(Value::as_str).unwrap_or(""),
        240,
    );
    Some(container_record(name, image, state, status, ports))
}

fn container_record(
    name: String,
    image: String,
    state: String,
    status: String,
    ports: String,
) -> Value {
    let running = state == "running";
    json!({
        "name": name,
        "image": image,
        "state": state,
        "status": status,
        "ports": ports,
        "running": running,
        "cpu_percent": Value::Null,
        "memory_used_bytes": Value::Null,
        "memory_limit_bytes": Value::Null,
        "pids": Value::Null,
        "panel_port": published_tcp_port(&ports),
    })
}

#[cfg(test)]
fn parse_docker_ps(stdout: &str) -> Result<Vec<Value>, String> {
    Ok(parse_docker_listing(stdout))
}

fn apply_docker_stats(containers: &mut [Value], stdout: &str) {
    let mut by_name = Map::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let (name, cpu_percent, memory_used_bytes, memory_limit_bytes, pids) =
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                let Some(name) = sanitize_container_name(
                    value.get("Name").and_then(Value::as_str).unwrap_or(""),
                ) else {
                    continue;
                };
                let cpu_percent = value
                    .get("CPUPerc")
                    .and_then(Value::as_str)
                    .and_then(parse_percent);
                let (memory_used_bytes, memory_limit_bytes) = value
                    .get("MemUsage")
                    .and_then(Value::as_str)
                    .and_then(parse_mem_usage)
                    .unwrap_or((None, None));
                let pids = value
                    .get("PIDs")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u64>().ok());
                (
                    name,
                    cpu_percent,
                    memory_used_bytes,
                    memory_limit_bytes,
                    pids,
                )
            } else {
                let mut parts = line.split('\t');
                let Some(name) = sanitize_container_name(parts.next().unwrap_or_default()) else {
                    continue;
                };
                let cpu_percent = parts.next().and_then(parse_percent);
                let (memory_used_bytes, memory_limit_bytes) = parts
                    .next()
                    .and_then(parse_mem_usage)
                    .unwrap_or((None, None));
                let pids = parts.next().and_then(|value| value.parse::<u64>().ok());
                (
                    name,
                    cpu_percent,
                    memory_used_bytes,
                    memory_limit_bytes,
                    pids,
                )
            };
        by_name.insert(
            name,
            json!({
                "cpu_percent": cpu_percent,
                "memory_used_bytes": memory_used_bytes,
                "memory_limit_bytes": memory_limit_bytes,
                "pids": pids
            }),
        );
    }
    for container in containers {
        let Some(name) = container.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(stats) = by_name.get(name) else {
            continue;
        };
        if let Some(object) = container.as_object_mut() {
            object.insert(
                "cpu_percent".to_owned(),
                stats.get("cpu_percent").cloned().unwrap_or(Value::Null),
            );
            object.insert(
                "memory_used_bytes".to_owned(),
                stats
                    .get("memory_used_bytes")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            object.insert(
                "memory_limit_bytes".to_owned(),
                stats
                    .get("memory_limit_bytes")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            object.insert(
                "pids".to_owned(),
                stats.get("pids").cloned().unwrap_or(Value::Null),
            );
        }
    }
}

fn detect_portainer(containers: &[Value]) -> Value {
    for container in containers {
        let name = container.get("name").and_then(Value::as_str).unwrap_or("");
        let image = container.get("image").and_then(Value::as_str).unwrap_or("");
        if !looks_like_portainer(name, image) {
            continue;
        }
        let ports = container.get("ports").and_then(Value::as_str).unwrap_or("");
        let panel_port = portainer_ui_port(ports);
        return json!({
            "detected": true,
            "container": name,
            "running": container.get("running").and_then(Value::as_bool).unwrap_or(false),
            "panel_port": panel_port,
            "panel_scheme": panel_port.map(|port| if port == 9443 { "https" } else { "http" })
        });
    }
    json!({
        "detected": false,
        "container": Value::Null,
        "running": false,
        "panel_port": Value::Null,
        "panel_scheme": Value::Null
    })
}

fn looks_like_portainer(name: &str, image: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let image = image.to_ascii_lowercase();
    name.contains("portainer") || image.contains("portainer")
}

fn looks_like_homarr(name: &str, image: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let image = image.to_ascii_lowercase();
    name.contains("homarr") || image.contains("homarr")
}

#[allow(clippy::collapsible_if)]
fn published_tcp_ports(ports: &str) -> Vec<u16> {
    let mut found = Vec::new();
    for part in ports.split(',') {
        let part = part.trim();
        let Some((_, rest)) = part.split_once(':') else {
            continue;
        };
        let host = rest.split("->").next().unwrap_or(rest);
        let host = host.split('/').next().unwrap_or(host);
        if let Ok(port) = host.parse::<u16>() {
            if port >= 1 && !found.contains(&port) {
                found.push(port);
            }
        }
    }
    found
}

fn portainer_ui_port(ports: &str) -> Option<u16> {
    let published = published_tcp_ports(ports);
    for preferred in [9443_u16, 9000, 9001] {
        if published.contains(&preferred) {
            return Some(preferred);
        }
    }
    published
        .into_iter()
        .find(|port| !matches!(port, 8000 | 2375 | 2376 | 2377))
}

fn published_tcp_port(ports: &str) -> Value {
    published_tcp_ports(ports)
        .into_iter()
        .next()
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn parse_percent(value: &str) -> Option<f64> {
    value
        .trim_end_matches('%')
        .parse::<f64>()
        .ok()
        .filter(|value| *value >= 0.0 && *value <= 10_000.0)
}

fn parse_mem_usage(value: &str) -> Option<(Option<u64>, Option<u64>)> {
    let (used, limit) = value.split_once('/')?;
    Some((
        parse_human_bytes(used.trim()),
        parse_human_bytes(limit.trim()),
    ))
}

fn sanitize_container_name(value: &str) -> Option<String> {
    let name = value
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches('/');
    if regex_container_name(name) {
        Some(name.to_owned())
    } else {
        None
    }
}

fn regex_container_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    let bytes = name.as_bytes();
    bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-' | b'.'))
}

fn validate_container_name(name: &str) -> Result<(), String> {
    if regex_container_name(name) {
        Ok(())
    } else {
        Err("the container name is invalid".to_owned())
    }
}

fn sanitize_label(value: &str, maximum: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(maximum)
        .collect()
}

fn safe_host_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('\0')
        && !path.contains("/../")
        && path.len() <= 512
        && !path.ends_with("/..")
}

fn homarr_mount_roots(mounts: &Value) -> Vec<String> {
    let mut roots = Vec::new();
    let Some(items) = mounts.as_array() else {
        return roots;
    };
    for mount in items.iter().take(16) {
        let Some(path) = homarr_mount_source(mount) else {
            continue;
        };
        if !roots.iter().any(|existing| existing == path) {
            roots.push(path.to_owned());
        }
    }
    roots
}

fn homarr_mount_source(mount: &Value) -> Option<&str> {
    let mount_type = mount.get("Type").and_then(Value::as_str)?;
    if mount_type != "bind" && mount_type != "volume" {
        return None;
    }
    let path = mount.get("Source").and_then(Value::as_str)?;
    if safe_host_path(path) {
        Some(path)
    } else {
        None
    }
}

fn scan_homarr_mounts(mounts: &Value) -> HomarrCatalogScan {
    let mut scan = HomarrCatalogScan {
        widgets: Vec::new(),
        source: None,
        sqlite_note: None,
        sqlite_empty: false,
    };
    for path in homarr_mount_roots(mounts) {
        if let Some(found) = read_homarr_widgets(&path) {
            scan.widgets = found;
            scan.source = Some("json");
            scan.sqlite_empty = false;
            scan.sqlite_note = None;
            break;
        }
        match read_homarr_sqlite_catalog(&path) {
            HomarrSqliteRead::Found(mut found) => {
                sort_homarr_widgets(&mut found);
                scan.widgets = finalize_homarr_widgets(found);
                scan.source = Some("sqlite");
                scan.sqlite_empty = scan.widgets.is_empty();
                scan.sqlite_note = None;
                break;
            }
            HomarrSqliteRead::Unreadable(note) => {
                scan.sqlite_note = Some(note);
            }
            HomarrSqliteRead::Absent => {}
        }
    }
    scan
}

#[allow(clippy::collapsible_if)]
fn read_homarr_widgets(root: &str) -> Option<Vec<Value>> {
    let candidates = [
        format!("{root}/configs/default.json"),
        format!("{root}/default.json"),
        format!("{root}/data/configs/default.json"),
        format!("{root}/app/data/configs/default.json"),
    ];
    for path in candidates {
        if let Some(widgets) = parse_homarr_file(&path) {
            if !widgets.is_empty() {
                return Some(widgets);
            }
        }
    }
    let dir = format!("{root}/configs");
    let entries = fs::read_dir(&dir).ok()?;
    for entry in entries.flatten().take(16) {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if let Some(widgets) = parse_homarr_file(path.to_str()?) {
            if !widgets.is_empty() {
                return Some(widgets);
            }
        }
    }
    None
}

fn parse_homarr_file(path: &str) -> Option<Vec<Value>> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    parse_homarr_config(&text)
}

pub(crate) fn parse_homarr_config(text: &str) -> Option<Vec<Value>> {
    let value: Value = serde_json::from_str(text).ok()?;
    let mut widgets = Vec::new();
    collect_homarr_services(&value, &mut widgets);
    sort_homarr_widgets(&mut widgets);
    let widgets = finalize_homarr_widgets(widgets);
    if widgets.is_empty() {
        None
    } else {
        Some(widgets)
    }
}

fn collect_homarr_services(value: &Value, widgets: &mut Vec<Value>) {
    if widgets.len() >= MAX_HOMARR_WIDGETS {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_homarr_services(item, widgets);
            }
        }
        Value::Object(object) => {
            if let Some(widget) = homarr_shortcut(object) {
                widgets.push(widget);
            }
            for (key, nested) in object {
                if matches!(
                    key.as_str(),
                    "services" | "apps" | "items" | "widgets" | "config" | "data"
                ) {
                    collect_homarr_services(nested, widgets);
                }
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy)]
struct HomarrPlacement {
    x: i64,
    y: i64,
    width: i64,
    breakpoint: i64,
}

fn homarr_shortcut(object: &Map<String, Value>) -> Option<Value> {
    let name = object
        .get("name")
        .or_else(|| object.get("title"))
        .and_then(Value::as_str)?;
    let url = object
        .get("href")
        .or_else(|| object.get("url"))
        .or_else(|| object.get("link"))
        .and_then(Value::as_str)?;
    let icon = object
        .get("icon")
        .or_else(|| object.get("iconUrl"))
        .or_else(|| object.get("icon_url"))
        .and_then(Value::as_str);
    homarr_http_shortcut(name, url, icon, homarr_json_placement(object).as_ref())
}

fn homarr_http_shortcut(
    name: &str,
    url: &str,
    icon: Option<&str>,
    placement: Option<&HomarrPlacement>,
) -> Option<Value> {
    let name = name.trim();
    let url = url.trim();
    if name.is_empty() {
        return None;
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 2_048 {
        return None;
    }
    if name.chars().count() > 80 || name.chars().any(char::is_control) {
        return None;
    }
    let icon = resolve_homarr_icon(icon);
    let mut widget = json!({
        "name": name,
        "url": url,
        "icon": icon
    });
    if let Some(placement) = placement {
        widget["x"] = json!(placement.x);
        widget["y"] = json!(placement.y);
        widget["width"] = json!(placement.width);
    }
    Some(widget)
}

fn resolve_homarr_icon(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() || raw.len() > 2_048 {
        return None;
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Some(raw.to_owned());
    }
    let slug = homarr_icon_slug(raw)?;
    Some(format!("{DASHBOARD_ICONS_PNG}/{slug}.png"))
}

fn homarr_icon_slug(raw: &str) -> Option<String> {
    let last = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .split('?')
        .next()
        .unwrap_or(raw);
    let stem = match last.rsplit_once('.') {
        Some((name, extension))
            if matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "svg" | "webp" | "jpg" | "jpeg" | "gif" | "ico"
            ) =>
        {
            name
        }
        _ => last,
    };
    let slug = stem
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '_'], "-")
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect::<String>();
    let slug = slug.trim_matches('-').to_owned();
    if slug.len() < 2 || slug.len() > 80 {
        return None;
    }
    if generic_homarr_icon_slug(&slug) {
        return None;
    }
    Some(slug)
}

fn generic_homarr_icon_slug(slug: &str) -> bool {
    matches!(
        slug,
        "avatar"
            | "blank"
            | "default"
            | "file"
            | "icon"
            | "image"
            | "img"
            | "logo"
            | "media"
            | "placeholder"
            | "thumbnail"
            | "upload"
    ) || (slug.len() >= 32
        && slug
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-'))
}

fn json_grid_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        _ => None,
    }
}

fn nested_grid_int(object: &Map<String, Value>, path: &[&str]) -> Option<i64> {
    let mut current = object;
    for (index, key) in path.iter().enumerate() {
        let value = current.get(*key)?;
        if index + 1 == path.len() {
            return json_grid_int(value);
        }
        current = value.as_object()?;
    }
    None
}

fn homarr_json_placement(object: &Map<String, Value>) -> Option<HomarrPlacement> {
    let x = object
        .get("x")
        .and_then(json_grid_int)
        .or_else(|| nested_grid_int(object, &["location", "x"]))
        .or_else(|| nested_grid_int(object, &["position", "x"]))
        .or_else(|| nested_grid_int(object, &["shape", "lg", "location", "x"]))
        .or_else(|| nested_grid_int(object, &["grid", "x"]))?;
    let y = object
        .get("y")
        .and_then(json_grid_int)
        .or_else(|| nested_grid_int(object, &["location", "y"]))
        .or_else(|| nested_grid_int(object, &["position", "y"]))
        .or_else(|| nested_grid_int(object, &["shape", "lg", "location", "y"]))
        .or_else(|| nested_grid_int(object, &["grid", "y"]))?;
    let width = object
        .get("width")
        .and_then(json_grid_int)
        .or_else(|| nested_grid_int(object, &["size", "width"]))
        .or_else(|| nested_grid_int(object, &["shape", "lg", "size", "width"]))
        .or_else(|| nested_grid_int(object, &["grid", "width"]))
        .unwrap_or(1);
    Some(HomarrPlacement {
        x,
        y,
        width,
        breakpoint: 0,
    })
}

fn finalize_homarr_widgets(mut widgets: Vec<Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    widgets.retain(|widget| {
        widget
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| seen.insert(url.to_owned()))
    });
    widgets.truncate(MAX_HOMARR_WIDGETS);
    widgets
}

fn sort_homarr_widgets(widgets: &mut [Value]) {
    let has_layout = widgets
        .iter()
        .any(|widget| widget.get("y").and_then(json_grid_int).is_some());
    if !has_layout {
        return;
    }
    widgets.sort_by(|left, right| {
        let left_y = left.get("y").and_then(json_grid_int).unwrap_or(i64::MAX);
        let right_y = right.get("y").and_then(json_grid_int).unwrap_or(i64::MAX);
        left_y
            .cmp(&right_y)
            .then_with(|| {
                let left_x = left.get("x").and_then(json_grid_int).unwrap_or(i64::MAX);
                let right_x = right.get("x").and_then(json_grid_int).unwrap_or(i64::MAX);
                left_x.cmp(&right_x)
            })
            .then_with(|| {
                left.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .cmp(right.get("name").and_then(Value::as_str).unwrap_or(""))
            })
    });
}

fn read_homarr_sqlite_catalog(root: &str) -> HomarrSqliteRead {
    match find_homarr_sqlite(root) {
        Ok(None) => HomarrSqliteRead::Absent,
        Err(note) => HomarrSqliteRead::Unreadable(note),
        Ok(Some(path)) => match query_homarr_sqlite(&path) {
            Ok(widgets) => HomarrSqliteRead::Found(widgets),
            Err(note) => HomarrSqliteRead::Unreadable(note),
        },
    }
}

fn find_homarr_sqlite(root: &str) -> Result<Option<PathBuf>, String> {
    let candidates = [
        format!("{root}/db/db.sqlite"),
        format!("{root}/db.sqlite"),
        format!("{root}/data/db.sqlite"),
        format!("{root}/appdata/db/db.sqlite"),
    ];
    for path in candidates {
        let path = PathBuf::from(path);
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() == 0 {
            continue;
        }
        if metadata.len() > MAX_HOMARR_DB_BYTES {
            return Err("the Homarr SQLite catalog is larger than Helix will read".to_owned());
        }
        return Ok(Some(path));
    }
    Ok(None)
}

fn query_homarr_sqlite(path: &Path) -> Result<Vec<Value>, String> {
    let snapshot = snapshot_homarr_sqlite(path)?;
    let mut widgets = read_homarr_apps(&snapshot)?;
    sort_homarr_widgets(&mut widgets);
    Ok(widgets)
}

fn snapshot_homarr_sqlite(path: &Path) -> Result<Connection, String> {
    let source = open_homarr_readonly(path)?;
    match backup_homarr_to_memory(&source) {
        Ok(snapshot) => Ok(snapshot),
        Err(_) => Ok(source),
    }
}

fn open_homarr_readonly(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "could not open the Homarr SQLite catalog".to_owned())?;
    let _ = connection.busy_timeout(Duration::from_millis(800));
    Ok(connection)
}

fn backup_homarr_to_memory(source: &Connection) -> Result<Connection, String> {
    let mut snapshot = Connection::open_in_memory()
        .map_err(|_| "could not snapshot the Homarr SQLite catalog".to_owned())?;
    {
        let backup = Backup::new(source, &mut snapshot)
            .map_err(|_| "could not snapshot the Homarr SQLite catalog".to_owned())?;
        backup
            .run_to_completion(64, Duration::from_millis(10), None)
            .map_err(|_| "could not snapshot the Homarr SQLite catalog".to_owned())?;
    }
    Ok(snapshot)
}

fn read_homarr_apps(connection: &Connection) -> Result<Vec<Value>, String> {
    let (table, columns) = homarr_app_table(connection)?;
    if !column_named(&columns, "name") || !column_named(&columns, "href") {
        return Err("the Homarr SQLite catalog does not have the expected app columns".to_owned());
    }
    let icon_column = if column_named(&columns, "icon_url") {
        Some("icon_url")
    } else if column_named(&columns, "iconUrl") {
        Some("iconUrl")
    } else {
        None
    };
    let has_id = column_named(&columns, "id");
    let sql = homarr_app_select_sql(table, icon_column, has_id).ok_or_else(|| {
        "the Homarr SQLite catalog does not have the expected app columns".to_owned()
    })?;
    let layouts = read_homarr_item_layouts(connection);
    let mut statement = connection
        .prepare(sql)
        .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
    let mut rows = statement
        .query([])
        .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
    let mut widgets = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?
    {
        if widgets.len() >= MAX_HOMARR_WIDGETS {
            break;
        }
        let id = row
            .get::<_, Option<String>>(0)
            .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
        let name = row
            .get::<_, String>(1)
            .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
        let href = row
            .get::<_, Option<String>>(2)
            .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
        let icon = row
            .get::<_, Option<String>>(3)
            .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
        let Some(href) = href.as_deref() else {
            continue;
        };
        let placement = id.as_deref().and_then(|value| layouts.get(value));
        if let Some(widget) = homarr_http_shortcut(&name, href, icon.as_deref(), placement) {
            widgets.push(widget);
        }
    }
    Ok(widgets)
}

fn sqlite_table_exists(connection: &Connection, table: &str) -> bool {
    let sql = match table {
        "item" => "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'item' LIMIT 1",
        "item_layout" => {
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'item_layout' LIMIT 1"
        }
        "layout" => "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'layout' LIMIT 1",
        _ => return false,
    };
    connection.query_row(sql, [], |_| Ok(())).is_ok()
}

fn homarr_options_app_id(options: &str) -> Option<String> {
    let value: Value = serde_json::from_str(options).ok()?;
    value
        .pointer("/json/appId")
        .or_else(|| value.get("appId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn better_homarr_placement(candidate: &HomarrPlacement, current: &HomarrPlacement) -> bool {
    candidate.breakpoint > current.breakpoint
}

fn read_homarr_item_layouts(connection: &Connection) -> HashMap<String, HomarrPlacement> {
    if !sqlite_table_exists(connection, "item") || !sqlite_table_exists(connection, "item_layout") {
        return HashMap::new();
    }
    let sql = if sqlite_table_exists(connection, "layout") {
        "SELECT i.options, l.x_offset, l.y_offset, l.width, COALESCE(ly.breakpoint, 0)
         FROM item i
         INNER JOIN item_layout l ON l.item_id = i.id
         LEFT JOIN layout ly ON ly.id = l.layout_id"
    } else {
        "SELECT i.options, l.x_offset, l.y_offset, l.width, 0
         FROM item i
         INNER JOIN item_layout l ON l.item_id = i.id"
    };
    let Ok(mut statement) = connection.prepare(sql) else {
        return HashMap::new();
    };
    let Ok(mut rows) = statement.query([]) else {
        return HashMap::new();
    };
    let mut layouts = HashMap::new();
    while let Ok(Some(row)) = rows.next() {
        let Ok(options) = row.get::<_, String>(0) else {
            continue;
        };
        let Some(app_id) = homarr_options_app_id(&options) else {
            continue;
        };
        let Ok(x) = row.get::<_, i64>(1) else {
            continue;
        };
        let Ok(y) = row.get::<_, i64>(2) else {
            continue;
        };
        let Ok(width) = row.get::<_, i64>(3) else {
            continue;
        };
        let breakpoint = row.get::<_, i64>(4).unwrap_or(0);
        let candidate = HomarrPlacement {
            x,
            y,
            width,
            breakpoint,
        };
        layouts
            .entry(app_id)
            .and_modify(|current| {
                if better_homarr_placement(&candidate, current) {
                    *current = candidate;
                }
            })
            .or_insert(candidate);
    }
    layouts
}

fn homarr_app_table(connection: &Connection) -> Result<(&'static str, Vec<String>), String> {
    for table in ["app", "apps"] {
        let columns = table_columns(connection, table)?;
        if !columns.is_empty() {
            return Ok((table, columns));
        }
    }
    Err("the Homarr SQLite catalog is missing the app table".to_owned())
}

fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, String> {
    let pragma = match table {
        "app" => "PRAGMA table_info(app)",
        "apps" => "PRAGMA table_info(apps)",
        _ => return Err("the Homarr SQLite catalog is missing the app table".to_owned()),
    };
    let mut statement = connection
        .prepare(pragma)
        .map_err(|_| "could not inspect the Homarr SQLite catalog".to_owned())?;
    let mut rows = statement
        .query([])
        .map_err(|_| "could not inspect the Homarr SQLite catalog".to_owned())?;
    let mut names = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|_| "could not inspect the Homarr SQLite catalog".to_owned())?
    {
        let name: String = row
            .get(1)
            .map_err(|_| "could not inspect the Homarr SQLite catalog".to_owned())?;
        names.push(name);
    }
    Ok(names)
}

fn column_named(columns: &[String], wanted: &str) -> bool {
    columns.iter().any(|name| name == wanted)
}

fn homarr_app_select_sql(
    table: &str,
    icon_column: Option<&str>,
    has_id: bool,
) -> Option<&'static str> {
    match (table, icon_column, has_id) {
        ("app", Some("icon_url"), true) => {
            Some("SELECT id, name, href, icon_url FROM app LIMIT 96")
        }
        ("app", Some("icon_url"), false) => {
            Some("SELECT NULL, name, href, icon_url FROM app LIMIT 96")
        }
        ("app", Some("iconUrl"), true) => {
            Some(r#"SELECT id, name, href, "iconUrl" FROM app LIMIT 96"#)
        }
        ("app", Some("iconUrl"), false) => {
            Some(r#"SELECT NULL, name, href, "iconUrl" FROM app LIMIT 96"#)
        }
        ("app", None, true) => Some("SELECT id, name, href, NULL FROM app LIMIT 96"),
        ("app", None, false) => Some("SELECT NULL, name, href, NULL FROM app LIMIT 96"),
        ("apps", Some("icon_url"), true) => {
            Some("SELECT id, name, href, icon_url FROM apps LIMIT 96")
        }
        ("apps", Some("icon_url"), false) => {
            Some("SELECT NULL, name, href, icon_url FROM apps LIMIT 96")
        }
        ("apps", Some("iconUrl"), true) => {
            Some(r#"SELECT id, name, href, "iconUrl" FROM apps LIMIT 96"#)
        }
        ("apps", Some("iconUrl"), false) => {
            Some(r#"SELECT NULL, name, href, "iconUrl" FROM apps LIMIT 96"#)
        }
        ("apps", None, true) => Some("SELECT id, name, href, NULL FROM apps LIMIT 96"),
        ("apps", None, false) => Some("SELECT NULL, name, href, NULL FROM apps LIMIT 96"),
        _ => None,
    }
}

fn now_unix_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn docker_cleanup_policy() -> Value {
    json!({
        "profile": "safe",
        "default_retention_hours": 168,
        "minimum_retention_hours": MIN_DOCKER_CLEANUP_RETENTION_HOURS,
        "maximum_retention_hours": MAX_DOCKER_CLEANUP_RETENTION_HOURS,
        "removes": ["old_build_cache", "dangling_images", "unused_networks"],
        "preserves": ["running_and_stopped_containers", "named_images", "all_volumes", "active_resources"],
        "automatic_volume_cleanup": false,
        "automatic_container_cleanup": false
    })
}

fn validate_docker_cleanup_retention(retention_hours: u16) -> Result<(), String> {
    if (MIN_DOCKER_CLEANUP_RETENTION_HOURS..=MAX_DOCKER_CLEANUP_RETENTION_HOURS)
        .contains(&retention_hours)
    {
        Ok(())
    } else {
        Err("Docker cleanup retention must be between 24 hours and one year".to_owned())
    }
}

fn docker_cleanup_steps(retention_hours: u16) -> Vec<(&'static str, Vec<String>)> {
    let filter = format!("until={retention_hours}h");
    [
        (
            "old_build_cache",
            ["builder", "prune", "--force", "--filter"],
        ),
        ("dangling_images", ["image", "prune", "--force", "--filter"]),
        (
            "unused_networks",
            ["network", "prune", "--force", "--filter"],
        ),
    ]
    .into_iter()
    .map(|(step, command)| {
        let mut arguments = command.map(str::to_owned).to_vec();
        arguments.push(filter.clone());
        (step, arguments)
    })
    .collect()
}

fn validate_docker_cleanup_run(record: &DockerCleanupRunRecord) -> Result<(), String> {
    let parsed = Uuid::parse_str(&record.run_id)
        .map_err(|_| "the Docker cleanup history is invalid".to_owned())?;
    let allowed_steps = ["old_build_cache", "dangling_images", "unused_networks"];
    let steps_are_prefix = record.completed_steps.len() <= allowed_steps.len()
        && record
            .completed_steps
            .iter()
            .zip(allowed_steps)
            .all(|(actual, expected)| actual == expected);
    let terminal = record
        .finished_at_unix_ms
        .is_some_and(|finished| finished >= record.started_at_unix_ms);
    let state_valid = match record.status.as_str() {
        "running" => record.finished_at_unix_ms.is_none() && record.error.is_none(),
        "complete" => terminal && record.error.is_none() && record.completed_steps.len() == 3,
        "failed" => {
            terminal
                && record.error.as_ref().is_some_and(|error| {
                    !error.is_empty() && error.len() <= 500 && !error.chars().any(char::is_control)
                })
        }
        _ => false,
    };
    if record.schema_version != 1
        || parsed.to_string() != record.run_id
        || !matches!(record.trigger.as_str(), "manual" | "scheduled")
        || validate_docker_cleanup_retention(record.retention_hours).is_err()
        || !steps_are_prefix
        || !state_valid
    {
        return Err("the Docker cleanup history is invalid".to_owned());
    }
    Ok(())
}

fn parse_docker_disk_usage(stdout: &str) -> Result<Vec<Value>, String> {
    let mut categories = Vec::new();
    let mut seen = HashSet::new();
    for line in stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(16)
    {
        let value = serde_json::from_str::<Value>(line)
            .map_err(|_| "Docker returned invalid disk usage data".to_owned())?;
        let kind = match value.get("Type").and_then(Value::as_str) {
            Some("Images") => "images",
            Some("Containers") => "containers",
            Some("Local Volumes") => "local_volumes",
            Some("Build Cache") => "build_cache",
            _ => continue,
        };
        if !seen.insert(kind) {
            return Err("Docker returned duplicate disk usage categories".to_owned());
        }
        let count = |field: &str| {
            value
                .get(field)
                .and_then(Value::as_str)
                .and_then(|text| text.parse::<u64>().ok())
                .filter(|count| *count <= 1_000_000_000)
        };
        let bytes = |field: &str| {
            value
                .get(field)
                .and_then(Value::as_str)
                .and_then(|text| text.split_whitespace().next())
                .and_then(parse_human_bytes)
        };
        let total_count = count("TotalCount")
            .ok_or_else(|| "Docker returned an invalid object count".to_owned())?;
        let active_count =
            count("Active").ok_or_else(|| "Docker returned an invalid active count".to_owned())?;
        let size_bytes =
            bytes("Size").ok_or_else(|| "Docker returned an invalid disk size".to_owned())?;
        let reclaimable_bytes = bytes("Reclaimable")
            .ok_or_else(|| "Docker returned invalid reclaimable space".to_owned())?;
        categories.push(json!({
            "kind": kind,
            "total_count": total_count,
            "active_count": active_count,
            "size_bytes": size_bytes,
            "reclaimable_bytes": reclaimable_bytes
        }));
    }
    if categories.is_empty() {
        return Err("Docker did not report any disk usage categories".to_owned());
    }
    Ok(categories)
}

fn storage_available_bytes(storage: &Value) -> u64 {
    storage
        .get("available_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn mount_for_path(path: &Path) -> Option<(String, String)> {
    let body = fs::read_to_string("/proc/self/mountinfo").ok()?;
    if body.len() > 2 * 1024 * 1024 {
        return None;
    }
    let mut best: Option<(PathBuf, String)> = None;
    for line in body.lines().take(8_192) {
        let Some((prefix, suffix)) = line.split_once(" - ") else {
            continue;
        };
        let mut fields = prefix.split_whitespace();
        let Some(mount) = fields.nth(4).and_then(decode_mountinfo_field) else {
            continue;
        };
        let mount_path = PathBuf::from(&mount);
        if !path.starts_with(&mount_path)
            || best.as_ref().is_some_and(|(current, _)| {
                current.components().count() >= mount_path.components().count()
            })
        {
            continue;
        }
        let source = suffix
            .split_whitespace()
            .nth(1)
            .and_then(decode_mountinfo_field)
            .unwrap_or_else(|| "unavailable".to_owned());
        best = Some((mount_path, sanitize_label(&source, 512)));
    }
    best.map(|(mount, source)| (mount.to_string_lossy().into_owned(), source))
}

fn decode_mountinfo_field(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 4_096 || value.chars().any(char::is_control) {
        return None;
    }
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 3 < bytes.len() {
            let octal = &bytes[index + 1..index + 4];
            if octal.iter().all(|byte| matches!(byte, b'0'..=b'7')) {
                let byte = (octal[0] - b'0') * 64 + (octal[1] - b'0') * 8 + (octal[2] - b'0');
                if byte == 0 || byte.is_ascii_control() {
                    return None;
                }
                decoded.push(byte);
                index += 4;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_disk_usage_parser_keeps_bounded_reclaimable_totals() {
        let usage = parse_docker_disk_usage(
            r#"{"Active":"26","Reclaimable":"3.196GB (14%)","Size":"21.5GB","TotalCount":"91","Type":"Images"}
{"Active":"23","Reclaimable":"10.46GB","Size":"15.97GB","TotalCount":"546","Type":"Build Cache"}"#,
        )
        .unwrap();
        assert_eq!(usage.len(), 2);
        assert_eq!(usage[0]["kind"], "images");
        assert_eq!(usage[0]["reclaimable_bytes"], 3_196_000_000_u64);
        assert_eq!(usage[1]["kind"], "build_cache");
        assert_eq!(usage[1]["reclaimable_bytes"], 10_460_000_000_u64);
    }

    #[test]
    fn cleanup_commands_never_include_containers_volumes_or_all_images() {
        let steps = docker_cleanup_steps(168);
        assert_eq!(steps.len(), 3);
        assert_eq!(
            steps[0],
            (
                "old_build_cache",
                vec!["builder", "prune", "--force", "--filter", "until=168h"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            )
        );
        assert_eq!(steps[1].1[0], "image");
        assert_eq!(steps[2].1[0], "network");
        for (_, arguments) in steps {
            assert!(!arguments.iter().any(|argument| argument == "container"));
            assert!(!arguments.iter().any(|argument| argument == "volume"));
            assert!(!arguments.iter().any(|argument| argument == "--all"));
        }
    }

    #[test]
    fn cleanup_history_rejects_skipped_steps_and_unsafe_errors() {
        let valid = DockerCleanupRunRecord {
            schema_version: 1,
            run_id: Uuid::new_v4().to_string(),
            trigger: "manual".to_owned(),
            status: "complete".to_owned(),
            retention_hours: 168,
            started_at_unix_ms: 10,
            finished_at_unix_ms: Some(20),
            reclaimable_before_bytes: 100,
            available_before_bytes: 1_000,
            available_after_bytes: Some(1_100),
            completed_steps: vec![
                "old_build_cache".to_owned(),
                "dangling_images".to_owned(),
                "unused_networks".to_owned(),
            ],
            error: None,
        };
        assert!(validate_docker_cleanup_run(&valid).is_ok());
        let mut skipped = valid.clone();
        skipped.status = "failed".to_owned();
        skipped.completed_steps = vec!["unused_networks".to_owned()];
        skipped.error = Some("failed".to_owned());
        assert!(validate_docker_cleanup_run(&skipped).is_err());
        let mut unsafe_error = skipped;
        unsafe_error.completed_steps.clear();
        unsafe_error.error = Some("bad\nmessage".to_owned());
        assert!(validate_docker_cleanup_run(&unsafe_error).is_err());
    }

    #[test]
    fn mountinfo_decoder_handles_spaces_without_accepting_controls() {
        assert_eq!(
            decode_mountinfo_field("/mnt/docker\\040data").as_deref(),
            Some("/mnt/docker data")
        );
        assert!(decode_mountinfo_field("/mnt/bad\\000path").is_none());
    }

    #[test]
    fn parse_docker_ps_keeps_running_state_and_published_ports() {
        let stdout = r#"{"ID":"abc","Names":"/plex","Image":"plexinc/pms-docker","State":"running","Status":"Up 2 hours","Ports":"0.0.0.0:32400->32400/tcp"}
{"ID":"def","Names":"portainer","Image":"portainer/portainer-ce","State":"exited","Status":"Exited (0)","Ports":"0.0.0.0:9000->9000/tcp"}
"#;
        let containers = parse_docker_ps(stdout).unwrap();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0]["name"], "plex");
        assert_eq!(containers[0]["running"], true);
        assert_eq!(containers[0]["panel_port"], 32400);
        assert_eq!(containers[1]["name"], "portainer");
        assert_eq!(containers[1]["running"], false);
        let portainer = detect_portainer(&containers);
        assert_eq!(portainer["detected"], true);
        assert_eq!(portainer["panel_port"], 9000);
        assert_eq!(portainer["panel_scheme"], "http");
    }

    #[test]
    fn parse_docker_tsv_and_skips_invalid_names() {
        let stdout = "plex\tplexinc/pms-docker\trunning\tUp 2 hours\t0.0.0.0:32400->32400/tcp\nbad;name\timage\trunning\tUp\t\nportainer\tportainer/portainer-ce\trunning\tUp\t0.0.0.0:8000->8000/tcp, 0.0.0.0:9443->9443/tcp\n";
        let containers = parse_docker_listing(stdout);
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0]["name"], "plex");
        assert_eq!(containers[1]["name"], "portainer");
        let portainer = detect_portainer(&containers);
        assert_eq!(portainer["panel_port"], 9443);
        assert_eq!(portainer["panel_scheme"], "https");
    }

    #[test]
    fn parse_docker_keeps_exited_containers_with_empty_ports() {
        let stdout = "AMP_AllTheMons01\tcubecoders/ampbase\trunning\tUp 3 days\t\nhomarr\tghcr.io/homarr-labs/homarr:latest\texited\tExited (0) 2 hours ago\t\n";
        let containers = parse_docker_listing(stdout);
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0]["name"], "AMP_AllTheMons01");
        assert_eq!(containers[0]["running"], true);
        assert_eq!(containers[0]["ports"], "");
        assert_eq!(containers[1]["name"], "homarr");
        assert_eq!(containers[1]["running"], false);
        assert_eq!(containers[1]["image"], "ghcr.io/homarr-labs/homarr:latest");
    }

    #[test]
    fn parse_docker_names_only_still_lists_containers() {
        let stdout = "server-dashboard\nAMP_AllTheMons01\nplex\n";
        let containers = parse_docker_listing(stdout);
        assert_eq!(containers.len(), 3);
        assert_eq!(containers[0]["name"], "server-dashboard");
        assert_eq!(containers[0]["running"], false);
        assert_eq!(containers[0]["image"], "");
    }

    #[test]
    fn portainer_prefers_ui_port_over_edge_agent() {
        assert_eq!(
            portainer_ui_port("0.0.0.0:8000->8000/tcp, 0.0.0.0:9000->9000/tcp"),
            Some(9000)
        );
        assert_eq!(portainer_ui_port("0.0.0.0:8000->8000/tcp"), None);
    }

    #[test]
    fn homarr_parser_keeps_http_shortcuts_and_drops_scripts() {
        let widgets = parse_homarr_config(
            r#"{
                "services": [
                    {"name": "Plex", "href": "http://192.168.1.10:32400/web", "icon": "https://example.test/plex.png"},
                    {"name": "Evil", "url": "javascript:alert(1)"},
                    {"name": "Notes"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0]["name"], "Plex");
        assert_eq!(widgets[0]["url"], "http://192.168.1.10:32400/web");
        assert_eq!(widgets[0]["icon"], "https://example.test/plex.png");
    }

    #[test]
    fn homarr_parser_dedupes_identical_urls() {
        let widgets = parse_homarr_config(
            r#"{
                "services": [
                    {"name": "Plex", "href": "http://192.168.1.10:32400/web"},
                    {"name": "Plex copy", "href": "http://192.168.1.10:32400/web"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0]["name"], "Plex");
    }

    #[test]
    fn homarr_parser_keeps_grid_order_and_icon_slugs() {
        let widgets = parse_homarr_config(
            r#"{
                "services": [
                    {"name": "Zebra", "href": "http://192.168.1.10:9090", "icon": "sonarr", "x": 4, "y": 1, "width": 2},
                    {"name": "Apple", "href": "http://192.168.1.10:8081", "x": 0, "y": 0, "width": 1}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(widgets.len(), 2);
        assert_eq!(widgets[0]["name"], "Apple");
        assert_eq!(widgets[0]["y"], 0);
        assert_eq!(widgets[1]["name"], "Zebra");
        assert_eq!(
            widgets[1]["icon"],
            format!("{DASHBOARD_ICONS_PNG}/sonarr.png")
        );
        assert_eq!(widgets[1]["width"], 2);
    }

    #[test]
    fn homarr_mounts_accept_bind_and_volume_sources() {
        let mounts = json!([
            {"Type": "bind", "Source": "/home/owner/homarr/appdata", "Destination": "/appdata"},
            {"Type": "volume", "Source": "/var/lib/docker/volumes/homarr_data/_data", "Destination": "/data"},
            {"Type": "tmpfs", "Source": "", "Destination": "/tmp"},
            {"Type": "bind", "Source": "/home/owner/homarr/appdata", "Destination": "/appdata"}
        ]);
        assert_eq!(
            homarr_mount_roots(&mounts),
            vec![
                "/home/owner/homarr/appdata".to_owned(),
                "/var/lib/docker/volumes/homarr_data/_data".to_owned()
            ]
        );
    }

    #[test]
    fn homarr_scan_prefers_classic_json_over_sqlite() {
        let root = tempfile::tempdir().expect("homarr mixed fixture");
        let configs = root.path().join("configs");
        fs::create_dir(&configs).expect("homarr configs");
        fs::write(
            configs.join("default.json"),
            r#"{"services":[{"name":"Plex","href":"http://192.168.1.10:32400/web"}]}"#,
        )
        .expect("write homarr json");
        let db_dir = root.path().join("db");
        fs::create_dir(&db_dir).expect("homarr db dir");
        let connection = Connection::open(db_dir.join("db.sqlite")).expect("create sqlite");
        connection
            .execute_batch(
                r#"
                CREATE TABLE app (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    icon_url TEXT NOT NULL,
                    href TEXT
                );
                INSERT INTO app VALUES
                    ('1', 'Sonarr', 'https://example.test/sonarr.png', 'http://192.168.1.10:8989');
                "#,
            )
            .expect("seed sqlite");
        drop(connection);
        let path = root.path().to_str().expect("utf8 path");
        let mounts = json!([{ "Type": "bind", "Source": path, "Destination": "/appdata" }]);
        let scan = scan_homarr_mounts(&mounts);
        assert_eq!(scan.source, Some("json"));
        assert_eq!(scan.widgets.len(), 1);
        assert_eq!(scan.widgets[0]["name"], "Plex");
    }

    #[test]
    fn homarr_scan_reads_sqlite_when_json_is_absent() {
        let root = tempfile::tempdir().expect("homarr sqlite scan fixture");
        let db_dir = root.path().join("db");
        fs::create_dir(&db_dir).expect("homarr db dir");
        let connection = Connection::open(db_dir.join("db.sqlite")).expect("create sqlite");
        connection
            .execute_batch(
                r#"
                CREATE TABLE app (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    icon_url TEXT NOT NULL,
                    href TEXT
                );
                INSERT INTO app VALUES
                    ('1', 'Sonarr', 'https://example.test/sonarr.png', 'http://192.168.1.10:8989');
                "#,
            )
            .expect("seed sqlite");
        drop(connection);
        let path = root.path().to_str().expect("utf8 path");
        let mounts = json!([{ "Type": "bind", "Source": path, "Destination": "/appdata" }]);
        let scan = scan_homarr_mounts(&mounts);
        assert_eq!(scan.source, Some("sqlite"));
        assert_eq!(scan.widgets.len(), 1);
        assert_eq!(scan.widgets[0]["name"], "Sonarr");
        assert!(!scan.sqlite_empty);
    }

    #[test]
    fn homarr_sqlite_reads_http_apps_and_drops_relative_links() {
        let root = tempfile::tempdir().expect("homarr sqlite fixture");
        let db_dir = root.path().join("db");
        fs::create_dir(&db_dir).expect("homarr db dir");
        let db = db_dir.join("db.sqlite");
        let connection = Connection::open(&db).expect("create homarr sqlite");
        connection
            .execute_batch(
                r#"
                CREATE TABLE app (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT,
                    icon_url TEXT NOT NULL,
                    href TEXT,
                    ping_url TEXT
                );
                INSERT INTO app VALUES
                    ('1', 'Plex', NULL, 'https://example.test/plex.png', 'http://192.168.1.10:32400/web', NULL),
                    ('2', 'Notes', NULL, '/imgs/logo/logo.png', NULL, NULL),
                    ('3', 'Evil', NULL, 'https://example.test/x.png', 'javascript:alert(1)', NULL),
                    ('4', 'Plex copy', NULL, 'https://example.test/plex.png', 'http://192.168.1.10:32400/web', NULL),
                    ('5', 'Radarr', NULL, '/api/user-medias/icon.png', 'http://192.168.1.10:7878', NULL);
                "#,
            )
            .expect("seed homarr sqlite");
        drop(connection);

        let HomarrSqliteRead::Found(widgets) =
            read_homarr_sqlite_catalog(root.path().to_str().expect("utf8 path"))
        else {
            panic!("expected a Homarr sqlite catalog");
        };
        let widgets = finalize_homarr_widgets(widgets);
        assert_eq!(widgets.len(), 2);
        assert_eq!(widgets[0]["name"], "Plex");
        assert_eq!(widgets[0]["url"], "http://192.168.1.10:32400/web");
        assert_eq!(widgets[0]["icon"], "https://example.test/plex.png");
        assert_eq!(widgets[1]["name"], "Radarr");
        assert_eq!(widgets[1]["url"], "http://192.168.1.10:7878");
        assert_eq!(widgets[1]["icon"], Value::Null);
    }

    #[test]
    fn homarr_sqlite_reads_camel_case_apps_table() {
        let root = tempfile::tempdir().expect("homarr camel sqlite fixture");
        let db = root.path().join("db.sqlite");
        let connection = Connection::open(&db).expect("create camel homarr sqlite");
        connection
            .execute_batch(
                r#"
                CREATE TABLE apps (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    "iconUrl" TEXT NOT NULL,
                    href TEXT
                );
                INSERT INTO apps VALUES
                    ('1', 'Sonarr', 'https://example.test/sonarr.png', 'https://example.test:8989');
                "#,
            )
            .expect("seed camel homarr sqlite");
        drop(connection);

        let HomarrSqliteRead::Found(widgets) =
            read_homarr_sqlite_catalog(root.path().to_str().expect("utf8 path"))
        else {
            panic!("expected a Homarr sqlite catalog");
        };
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0]["name"], "Sonarr");
        assert_eq!(widgets[0]["icon"], "https://example.test/sonarr.png");
    }

    #[test]
    fn homarr_sqlite_uses_item_layout_and_icon_slugs() {
        let root = tempfile::tempdir().expect("homarr layout sqlite fixture");
        let db_dir = root.path().join("db");
        fs::create_dir(&db_dir).expect("homarr db dir");
        let db = db_dir.join("db.sqlite");
        let connection = Connection::open(&db).expect("create layout sqlite");
        connection
            .execute_batch(
                r#"
                CREATE TABLE app (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    icon_url TEXT NOT NULL,
                    href TEXT
                );
                CREATE TABLE item (
                    id TEXT PRIMARY KEY,
                    board_id TEXT,
                    kind TEXT,
                    options TEXT NOT NULL,
                    advanced_options TEXT
                );
                CREATE TABLE layout (
                    id TEXT PRIMARY KEY,
                    name TEXT,
                    board_id TEXT,
                    column_count INTEGER,
                    breakpoint INTEGER
                );
                CREATE TABLE item_layout (
                    item_id TEXT,
                    section_id TEXT,
                    layout_id TEXT,
                    x_offset INTEGER,
                    y_offset INTEGER,
                    width INTEGER,
                    height INTEGER
                );
                INSERT INTO app VALUES
                    ('app-plex', 'Plex', 'https://example.test/plex.png', 'http://192.168.1.10:32400/web'),
                    ('app-radarr', 'Radarr', 'radarr', 'http://192.168.1.10:7878');
                INSERT INTO item VALUES
                    ('item-plex', 'board', 'app', '{"json":{"appId":"app-plex"}}', '{"json":{}}'),
                    ('item-radarr', 'board', 'app', '{"json":{"appId":"app-radarr"}}', '{"json":{}}');
                INSERT INTO layout VALUES ('lg', 'lg', 'board', 12, 1200);
                INSERT INTO item_layout VALUES
                    ('item-radarr', 'section', 'lg', 0, 0, 1, 1),
                    ('item-plex', 'section', 'lg', 4, 2, 2, 1);
                "#,
            )
            .expect("seed layout sqlite");
        drop(connection);

        let HomarrSqliteRead::Found(widgets) =
            read_homarr_sqlite_catalog(root.path().to_str().expect("utf8 path"))
        else {
            panic!("expected a Homarr sqlite catalog");
        };
        let widgets = finalize_homarr_widgets(widgets);
        assert_eq!(widgets.len(), 2);
        assert_eq!(widgets[0]["name"], "Radarr");
        assert_eq!(widgets[0]["y"], 0);
        assert_eq!(
            widgets[0]["icon"],
            format!("{DASHBOARD_ICONS_PNG}/radarr.png")
        );
        assert_eq!(widgets[1]["name"], "Plex");
        assert_eq!(widgets[1]["y"], 2);
        assert_eq!(widgets[1]["icon"], "https://example.test/plex.png");
    }

    #[test]
    fn homarr_sqlite_fails_closed_without_app_table() {
        let root = tempfile::tempdir().expect("empty homarr sqlite fixture");
        let db_dir = root.path().join("db");
        fs::create_dir(&db_dir).expect("homarr db dir");
        let db = db_dir.join("db.sqlite");
        let connection = Connection::open(&db).expect("create empty sqlite");
        connection
            .execute_batch("CREATE TABLE item (id TEXT PRIMARY KEY, kind TEXT);")
            .expect("seed unrelated table");
        drop(connection);

        let HomarrSqliteRead::Unreadable(note) =
            read_homarr_sqlite_catalog(root.path().to_str().expect("utf8 path"))
        else {
            panic!("expected Homarr sqlite to fail closed");
        };
        assert!(note.contains("app table"));
    }

    #[test]
    fn container_names_reject_shell_metacharacters() {
        assert!(validate_container_name("plex").is_ok());
        assert!(validate_container_name("server-dashboard").is_ok());
        assert!(validate_container_name("plex; reboot").is_err());
        assert!(validate_container_name("../escape").is_err());
        assert!(validate_container_name("").is_err());
    }

    #[test]
    fn native_game_containers_are_protected_from_the_docker_page() {
        assert!(is_helix_game_container(
            "helix-game-2876a033-11d1-4035-9734-236bc7723792"
        ));
        assert!(!is_helix_game_container("plex"));
        assert!(!is_helix_game_container("server-dashboard"));
        assert!(!is_helix_game_container("helix-game-not-a-uuid"));
        assert!(!is_helix_game_container(
            "helix-game-2876A033-11D1-4035-9734-236BC7723792"
        ));
    }
}
