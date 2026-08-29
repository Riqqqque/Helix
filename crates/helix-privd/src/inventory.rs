use crate::bounded_command::{BoundedCommandOutput, run_bounded_command};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_COMMAND_OUTPUT: usize = 8 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SERVICES: usize = 256;
const MAX_LISTENERS: usize = 512;
const MAX_PROCESSES: usize = 32;

#[derive(Debug, Serialize)]
pub struct HostInventory {
    pub disks: Vec<BlockDevice>,
    pub mounts: Vec<Mount>,
    pub interfaces: Vec<NetworkInterface>,
    pub routes: Vec<Route>,
    pub listeners: Vec<Listener>,
    pub services: Vec<Service>,
    pub processes: Vec<Process>,
    pub load_average: [f64; 3],
    pub process_count: u64,
    pub thread_count: u64,
    pub cpu_model: Option<String>,
    pub collected_at_unix_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct BlockDevice {
    pub name: String,
    pub path: Option<String>,
    pub parent: Option<String>,
    pub device_type: String,
    pub size_bytes: u64,
    pub file_system: Option<String>,
    pub label: Option<String>,
    pub mount_points: Vec<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub transport: Option<String>,
    pub rotational: bool,
    pub read_only: bool,
    pub hotplug: bool,
}

#[derive(Debug, Serialize)]
pub struct Mount {
    pub target: String,
    pub source: String,
    pub file_system: String,
    pub size_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub use_percent: u8,
    pub read_only: bool,
}

#[derive(Debug, Serialize)]
pub struct NetworkInterface {
    pub name: String,
    pub state: String,
    pub mac: Option<String>,
    pub mtu: u64,
    pub addresses: Vec<NetworkAddress>,
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
    pub received_packets: u64,
    pub transmitted_packets: u64,
    pub received_errors: u64,
    pub transmitted_errors: u64,
}

#[derive(Debug, Serialize)]
pub struct NetworkAddress {
    pub family: String,
    pub address: String,
    pub prefix_length: u8,
    pub scope: String,
}

#[derive(Debug, Serialize)]
pub struct Route {
    pub destination: String,
    pub gateway: Option<String>,
    pub interface: Option<String>,
    pub source: Option<String>,
    pub protocol: Option<String>,
    pub metric: Option<u64>,
    pub link_down: bool,
}

#[derive(Debug, Serialize)]
pub struct Listener {
    pub protocol: String,
    pub address: String,
    pub port: u16,
    pub process: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Service {
    pub unit: String,
    pub active: String,
    pub state: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct Process {
    pub pid: u32,
    pub user: String,
    pub name: String,
    pub cpu_percent: f32,
    pub resident_bytes: u64,
    pub uptime_seconds: u64,
}

pub fn collect() -> Result<HostInventory, String> {
    let block_devices = run_json(
        "lsblk",
        &[
            "-J",
            "-b",
            "-o",
            "NAME,PATH,SIZE,TYPE,FSTYPE,LABEL,MOUNTPOINTS,MODEL,SERIAL,ROTA,RO,HOTPLUG,TRAN",
        ],
    )?;
    let mounts = run_json(
        "findmnt",
        &[
            "-J",
            "-b",
            "--real",
            "-o",
            "TARGET,SOURCE,FSTYPE,OPTIONS,SIZE,USED,AVAIL,USE%",
        ],
    )?;
    let interfaces = run_json("ip", &["-j", "-s", "address"])?;
    let routes = run_json("ip", &["-j", "route", "show", "table", "main"])?;
    let services = run_json(
        "systemctl",
        &[
            "list-units",
            "--type=service",
            "--all",
            "--output=json",
            "--no-pager",
        ],
    )?;

    Ok(HostInventory {
        disks: parse_disks(&block_devices),
        mounts: parse_mounts(&mounts),
        interfaces: parse_interfaces(&interfaces),
        routes: parse_routes(&routes),
        listeners: collect_listeners().unwrap_or_default(),
        services: parse_services(&services),
        processes: collect_processes().unwrap_or_default(),
        load_average: read_load_average().unwrap_or([0.0; 3]),
        process_count: count_processes().unwrap_or(0),
        thread_count: read_thread_count().unwrap_or(0),
        cpu_model: read_cpu_model(),
        collected_at_unix_ms: now_unix_ms(),
    })
}

fn run_json(program: &str, arguments: &[&str]) -> Result<Value, String> {
    let output = run_command(program, arguments)?;
    serde_json::from_slice(&output.stdout).map_err(|_| format!("{program} returned malformed JSON"))
}

fn run_command(program: &str, arguments: &[&str]) -> Result<BoundedCommandOutput, String> {
    let arguments = arguments
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect::<Vec<_>>();
    let output = run_bounded_command(
        Path::new("timeout"),
        Path::new(program),
        &arguments,
        COMMAND_TIMEOUT,
        &[("LC_ALL", "C")],
        MAX_COMMAND_OUTPUT,
    )?;
    if !output.status.success() {
        if output.status.code() == Some(124) {
            return Err(format!("{program} timed out"));
        }
        return Err(format!("{program} failed"));
    }
    Ok(output)
}

fn parse_disks(root: &Value) -> Vec<BlockDevice> {
    let mut disks = Vec::new();
    if let Some(devices) = root.get("blockdevices").and_then(Value::as_array) {
        for device in devices {
            append_device(device, None, &mut disks);
        }
    }
    disks.truncate(128);
    disks
}

fn append_device(value: &Value, parent: Option<&str>, output: &mut Vec<BlockDevice>) {
    if output.len() >= 128 {
        return;
    }
    let name = text(value, "name").unwrap_or_else(|| "unknown".to_owned());
    output.push(BlockDevice {
        name: name.clone(),
        path: text(value, "path"),
        parent: parent.map(str::to_owned),
        device_type: text(value, "type").unwrap_or_else(|| "unknown".to_owned()),
        size_bytes: number(value, "size"),
        file_system: text(value, "fstype"),
        label: text(value, "label"),
        mount_points: value
            .get("mountpoints")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .take(32)
                    .collect()
            })
            .unwrap_or_default(),
        model: text(value, "model"),
        serial: text(value, "serial"),
        transport: text(value, "tran"),
        rotational: boolean(value, "rota"),
        read_only: boolean(value, "ro"),
        hotplug: boolean(value, "hotplug"),
    });
    if let Some(children) = value.get("children").and_then(Value::as_array) {
        for child in children {
            append_device(child, Some(&name), output);
        }
    }
}

fn parse_mounts(root: &Value) -> Vec<Mount> {
    let mut output = Vec::new();
    if let Some(items) = root.get("filesystems").and_then(Value::as_array) {
        for item in items {
            append_mount(item, &mut output);
        }
    }
    output.sort_by(|left, right| left.target.cmp(&right.target));
    output.truncate(128);
    output
}

fn append_mount(value: &Value, output: &mut Vec<Mount>) {
    if output.len() >= 128 {
        return;
    }
    if let (Some(target), Some(source), Some(file_system)) = (
        text(value, "target"),
        text(value, "source"),
        text(value, "fstype"),
    ) {
        let options = text(value, "options").unwrap_or_default();
        let size_bytes = json_u64(value, "size");
        let used_bytes = json_u64(value, "used");
        output.push(Mount {
            target,
            source,
            file_system,
            size_bytes,
            used_bytes,
            available_bytes: json_u64(value, "avail"),
            use_percent: json_percent(value, "use%").unwrap_or_else(|| {
                if size_bytes == 0 {
                    0
                } else {
                    ((used_bytes as f64 / size_bytes as f64) * 100.0)
                        .round()
                        .clamp(0.0, 100.0) as u8
                }
            }),
            read_only: options.split(',').any(|option| option == "ro"),
        });
    }
    if let Some(children) = value.get("children").and_then(Value::as_array) {
        for child in children {
            append_mount(child, output);
        }
    }
}

fn parse_interfaces(root: &Value) -> Vec<NetworkInterface> {
    let mut interfaces = root
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let name = text(value, "ifname")?;
            let addresses = value
                .get("addr_info")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|address| {
                    Some(NetworkAddress {
                        family: text(address, "family")?,
                        address: text(address, "local")?,
                        prefix_length: u8::try_from(number(address, "prefixlen")).ok()?,
                        scope: text(address, "scope").unwrap_or_else(|| "unknown".to_owned()),
                    })
                })
                .take(32)
                .collect();
            let rx = value.pointer("/stats64/rx").unwrap_or(&Value::Null);
            let tx = value.pointer("/stats64/tx").unwrap_or(&Value::Null);
            Some(NetworkInterface {
                name,
                state: text(value, "operstate").unwrap_or_else(|| "UNKNOWN".to_owned()),
                mac: text(value, "address"),
                mtu: number(value, "mtu"),
                addresses,
                received_bytes: number(rx, "bytes"),
                transmitted_bytes: number(tx, "bytes"),
                received_packets: number(rx, "packets"),
                transmitted_packets: number(tx, "packets"),
                received_errors: number(rx, "errors"),
                transmitted_errors: number(tx, "errors"),
            })
        })
        .take(128)
        .collect::<Vec<_>>();
    interfaces.sort_by(|left, right| left.name.cmp(&right.name));
    interfaces
}

fn parse_routes(root: &Value) -> Vec<Route> {
    root.as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            Some(Route {
                destination: text(value, "dst")?,
                gateway: text(value, "gateway"),
                interface: text(value, "dev"),
                source: text(value, "prefsrc"),
                protocol: text(value, "protocol"),
                metric: value.get("metric").and_then(Value::as_u64),
                link_down: value
                    .get("flags")
                    .and_then(Value::as_array)
                    .is_some_and(|flags| {
                        flags.iter().any(|flag| flag.as_str() == Some("linkdown"))
                    }),
            })
        })
        .take(256)
        .collect()
}

fn parse_services(root: &Value) -> Vec<Service> {
    let mut services = root
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            Some(Service {
                unit: text(value, "unit")?,
                active: text(value, "active").unwrap_or_else(|| "unknown".to_owned()),
                state: text(value, "sub").unwrap_or_else(|| "unknown".to_owned()),
                description: text(value, "description").unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    services.sort_by(|left, right| {
        service_rank(&left.active)
            .cmp(&service_rank(&right.active))
            .then_with(|| left.unit.cmp(&right.unit))
    });
    services.truncate(MAX_SERVICES);
    services
}

fn service_rank(active: &str) -> u8 {
    match active {
        "failed" => 0,
        "activating" | "deactivating" => 1,
        "active" => 2,
        _ => 3,
    }
}

fn collect_listeners() -> Result<Vec<Listener>, String> {
    let output = run_command("ss", &["-H", "-lntup"])?;
    let text =
        String::from_utf8(output.stdout).map_err(|_| "ss returned invalid text".to_owned())?;
    let mut listeners = Vec::new();
    for line in text.lines().take(MAX_LISTENERS) {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() < 5 {
            continue;
        }
        let protocol = columns[0].to_ascii_lowercase();
        let local = columns[4];
        let Some((address, port_text)) = local.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port_text.parse() else {
            continue;
        };
        let process = line
            .split("users:((\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        listeners.push(Listener {
            protocol,
            address: address.trim_matches(['[', ']']).to_owned(),
            port,
            process,
        });
    }
    listeners.sort_by(|left, right| {
        left.port
            .cmp(&right.port)
            .then_with(|| left.protocol.cmp(&right.protocol))
    });
    Ok(listeners)
}

fn collect_processes() -> Result<Vec<Process>, String> {
    let output = run_command(
        "ps",
        &["-eo", "pid=,user=,comm=,pcpu=,rss=,etimes=", "--sort=-pcpu"],
    )?;
    let text =
        String::from_utf8(output.stdout).map_err(|_| "ps returned invalid text".to_owned())?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some(Process {
                pid: fields.next()?.parse().ok()?,
                user: fields.next()?.to_owned(),
                name: fields.next()?.to_owned(),
                cpu_percent: fields.next()?.parse().ok()?,
                resident_bytes: fields.next()?.parse::<u64>().ok()?.saturating_mul(1024),
                uptime_seconds: fields.next()?.parse().ok()?,
            })
        })
        .take(MAX_PROCESSES)
        .collect())
}

fn read_load_average() -> Result<[f64; 3], String> {
    let text = fs::read_to_string("/proc/loadavg")
        .map_err(|_| "load average is unavailable".to_owned())?;
    let mut values = text.split_whitespace().take(3).map(str::parse::<f64>);
    Ok([
        values.next().and_then(Result::ok).unwrap_or(0.0),
        values.next().and_then(Result::ok).unwrap_or(0.0),
        values.next().and_then(Result::ok).unwrap_or(0.0),
    ])
}

fn read_thread_count() -> Result<u64, String> {
    let text = fs::read_to_string("/proc/loadavg")
        .map_err(|_| "thread count is unavailable".to_owned())?;
    parse_thread_count(&text)
}

fn parse_thread_count(loadavg: &str) -> Result<u64, String> {
    let token = loadavg
        .split_whitespace()
        .nth(3)
        .ok_or_else(|| "thread count is unavailable".to_owned())?;
    let count = token
        .split('/')
        .nth(1)
        .ok_or_else(|| "thread count is unavailable".to_owned())?;
    count
        .parse()
        .map_err(|_| "thread count is unavailable".to_owned())
}

fn count_processes() -> Result<u64, String> {
    let mut count = 0_u64;
    let entries = fs::read_dir("/proc").map_err(|_| "process list is unavailable".to_owned())?;
    for entry in entries {
        let entry = entry.map_err(|_| "process list is unavailable".to_owned())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if !file_type.is_dir() && !file_type.is_symlink() {
            continue;
        }
        count = count.saturating_add(1);
        if count >= 1_000_000 {
            break;
        }
    }
    Ok(count)
}

fn json_u64(value: &Value, key: &str) -> u64 {
    match value.get(key) {
        Some(Value::Number(number)) => number
            .as_u64()
            .or_else(|| {
                number.as_f64().and_then(|value| {
                    if value.is_finite() && value >= 0.0 {
                        Some(value.round() as u64)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn json_percent(value: &Value, key: &str) -> Option<u8> {
    let parsed = match value.get(key) {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().trim_end_matches('%').trim().parse::<f64>().ok(),
        _ => None,
    };
    parsed
        .filter(|value| value.is_finite())
        .map(|value| value.round().clamp(0.0, 100.0) as u8)
}

fn read_cpu_model() -> Option<String> {
    let text = fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() == "model name" {
            let model = value.trim();
            if !model.is_empty() && model.len() <= 180 && !model.chars().any(char::is_control) {
                return Some(model.to_owned());
            }
        }
    }
    None
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn boolean(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
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
    use super::{json_percent, json_u64, parse_mounts, parse_thread_count};
    use serde_json::json;

    #[test]
    fn loadavg_fourth_field_is_kernel_threads_not_processes() {
        assert_eq!(
            parse_thread_count("0.44 0.72 0.61 1/2261 4190839").unwrap(),
            2261
        );
        assert!(parse_thread_count("0.00 0.00 0.00").is_err());
        assert!(parse_thread_count("0.00 0.00 0.00 1 12").is_err());
    }

    #[test]
    fn mount_use_percent_accepts_string_or_number_and_falls_back_to_bytes() {
        let mounts = parse_mounts(&json!({
            "filesystems": [
                {
                    "target": "/",
                    "source": "/dev/sda2",
                    "fstype": "ext4",
                    "options": "rw",
                    "size": 1000,
                    "used": 250,
                    "avail": 750,
                    "use%": "25%"
                },
                {
                    "target": "/nvme",
                    "source": "/dev/nvme0n1p1",
                    "fstype": "ext4",
                    "options": "rw",
                    "size": 8000,
                    "used": 7200,
                    "avail": 800,
                    "use%": 90.4
                },
                {
                    "target": "/hdd",
                    "source": "/dev/sdb1",
                    "fstype": "ext4",
                    "options": "rw",
                    "size": "2000",
                    "used": "1500",
                    "avail": "500"
                }
            ]
        }));
        assert_eq!(mounts[0].target, "/");
        assert_eq!(mounts[0].use_percent, 25);
        assert_eq!(mounts[1].target, "/hdd");
        assert_eq!(mounts[1].size_bytes, 2000);
        assert_eq!(mounts[1].used_bytes, 1500);
        assert_eq!(mounts[1].use_percent, 75);
        assert_eq!(mounts[2].target, "/nvme");
        assert_eq!(mounts[2].use_percent, 90);
        assert_eq!(json_u64(&json!({"size": 12.4}), "size"), 12);
        assert_eq!(json_percent(&json!({"use%": "0%"}), "use%"), Some(0));
    }
}
