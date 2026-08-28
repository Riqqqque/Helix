use crate::host::{HostControl, parse_human_bytes, require_success};
use helix_privd::DockerContainerActionKind;
use serde_json::{Map, Value, json};
use std::{
    fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_CONTAINERS: usize = 64;
const MAX_HOMARR_WIDGETS: usize = 64;

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
        let mut widgets = Vec::new();
        let mut source = None;
        if let Some(items) = mounts.as_array() {
            for mount in items.iter().take(16) {
                if mount.get("Type").and_then(Value::as_str) != Some("bind") {
                    continue;
                }
                let Some(path) = mount.get("Source").and_then(Value::as_str) else {
                    continue;
                };
                if !safe_host_path(path) {
                    continue;
                }
                if let Some(found) = read_homarr_widgets(path) {
                    source = Some(path.to_owned());
                    widgets = found;
                    break;
                }
            }
        }
        if widgets.is_empty() {
            return Ok(json!({
                "schema_version": 1,
                "availability": "unsupported_format",
                "container": name,
                "widgets": [],
                "note": "Homarr is running, but Helix could not read a classic JSON widget list from its bind mounts. Newer Homarr stores apps in a database Helix does not parse. Export shortcuts from Homarr or add them by hand.",
                "collected_at_unix_ms": now_unix_ms()
            }));
        }
        Ok(json!({
            "schema_version": 1,
            "availability": "ready",
            "container": name,
            "source": source,
            "widgets": widgets,
            "note": "Helix imported Homarr shortcuts that already have an http(s) address. Notes and Homarr-only apps stay in Homarr.",
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
    if widgets.is_empty() {
        None
    } else {
        widgets.truncate(MAX_HOMARR_WIDGETS);
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
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let url = object
        .get("href")
        .or_else(|| object.get("url"))
        .or_else(|| object.get("link"))
        .and_then(Value::as_str)
        .map(str::trim)?;
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 2_048 {
        return None;
    }
    if name.chars().count() > 80 || name.chars().any(char::is_control) {
        return None;
    }
    let icon = object
        .get("icon")
        .or_else(|| object.get("iconUrl"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
        .map(|value| value.chars().take(2_048).collect::<String>());
    Some(json!({
        "name": name,
        "url": url,
        "icon": icon
    }))
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
