use crate::host::{HostControl, parse_human_bytes, require_success};
use helix_privd::DockerContainerActionKind;
use rusqlite::{Connection, OpenFlags, backup::Backup};
use serde_json::{Map, Value, json};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_CONTAINERS: usize = 64;
const MAX_HOMARR_WIDGETS: usize = 64;
const MAX_HOMARR_DB_BYTES: u64 = 32 * 1024 * 1024;

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
        if !running.is_empty() {
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
        let protected = self.protected_container_names();
        for container in &mut containers {
            if let Some(object) = container.as_object_mut() {
                let name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                object.insert(
                    "protected".to_owned(),
                    Value::Bool(protected.iter().any(|item| item == &name)),
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
            "note": "Helix lists Docker Engine containers on this host, including ones Portainer also shows. Start, stop, and restart require typing the exact container name. Helix dashboard and gateway containers cannot be stopped here.",
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
        if self
            .protected_container_names()
            .iter()
            .any(|item| item == name)
        {
            return Err(
                "Helix will not start, stop, or restart its own dashboard or gateway container from this page"
                    .to_owned(),
            );
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
            "note": "Choose which Homarr links to place on this Home. Relative icons, notes, and Homarr-only apps stay in Homarr.",
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

    fn protected_container_names(&self) -> Vec<String> {
        vec![
            self.config.dashboard_container.clone(),
            self.config.gateway_container.clone(),
        ]
    }
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
    homarr_http_shortcut(name, url, icon)
}

fn homarr_http_shortcut(name: &str, url: &str, icon: Option<&str>) -> Option<Value> {
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
    let icon = icon
        .map(str::trim)
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
        .filter(|value| value.len() <= 2_048)
        .map(str::to_owned);
    Some(json!({
        "name": name,
        "url": url,
        "icon": icon
    }))
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
    widgets.sort_by(|left, right| {
        let left_name = left.get("name").and_then(Value::as_str).unwrap_or("");
        let right_name = right.get("name").and_then(Value::as_str).unwrap_or("");
        match left_name.cmp(right_name) {
            std::cmp::Ordering::Equal => left
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(right.get("url").and_then(Value::as_str).unwrap_or("")),
            order => order,
        }
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
    read_homarr_apps(&snapshot)
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
    let sql = homarr_app_select_sql(table, icon_column).ok_or_else(|| {
        "the Homarr SQLite catalog does not have the expected app columns".to_owned()
    })?;
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
        let name = row
            .get::<_, String>(0)
            .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
        let href = row
            .get::<_, Option<String>>(1)
            .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
        let icon = row
            .get::<_, Option<String>>(2)
            .map_err(|_| "could not read the Homarr SQLite catalog".to_owned())?;
        let Some(href) = href.as_deref() else {
            continue;
        };
        if let Some(widget) = homarr_http_shortcut(&name, href, icon.as_deref()) {
            widgets.push(widget);
        }
    }
    Ok(widgets)
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

fn homarr_app_select_sql(table: &str, icon_column: Option<&str>) -> Option<&'static str> {
    match (table, icon_column) {
        ("app", Some("icon_url")) => Some("SELECT name, href, icon_url FROM app LIMIT 96"),
        ("app", Some("iconUrl")) => Some(r#"SELECT name, href, "iconUrl" FROM app LIMIT 96"#),
        ("app", None) => Some("SELECT name, href, NULL FROM app LIMIT 96"),
        ("apps", Some("icon_url")) => Some("SELECT name, href, icon_url FROM apps LIMIT 96"),
        ("apps", Some("iconUrl")) => Some(r#"SELECT name, href, "iconUrl" FROM apps LIMIT 96"#),
        ("apps", None) => Some("SELECT name, href, NULL FROM apps LIMIT 96"),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
