use crate::bounded_command::run_bounded_command;
use crate::upnp::{ExternalAddressKind, UpnpGateway, classify_external_address};
use helix_privd::{FirewallProtocol, FirewallRuleSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Write as _,
    net::{Ipv4Addr, Ipv6Addr},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const MAX_COMMAND_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_LISTENERS: usize = 4_096;
const MAX_PROCESSES: usize = 4_096;
const MAX_PROCESS_FDS: usize = 65_536;
const MAX_DOCKER_CONTAINERS: usize = 512;
const MAX_FIREWALL_RULES: usize = 2_048;
const MAX_RULE_RANGE_WIDTH: u16 = 1_024;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const UNDO_WINDOW_MS: u64 = 15 * 60 * 1_000;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    #[serde(default = "default_docker_binary")]
    pub docker_binary: PathBuf,
    #[serde(default = "default_ufw_binary")]
    pub ufw_binary: PathBuf,
    #[serde(default = "default_timeout_binary")]
    pub timeout_binary: PathBuf,
    #[serde(default = "default_proc_root")]
    pub proc_root: PathBuf,
    #[serde(default = "default_state_root")]
    pub state_root: PathBuf,
    #[serde(default = "default_mutation_cooldown_seconds")]
    pub mutation_cooldown_seconds: u8,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            docker_binary: default_docker_binary(),
            ufw_binary: default_ufw_binary(),
            timeout_binary: default_timeout_binary(),
            proc_root: default_proc_root(),
            state_root: default_state_root(),
            mutation_cooldown_seconds: default_mutation_cooldown_seconds(),
        }
    }
}

pub struct NetworkManager {
    config: NetworkConfig,
    runner: Arc<dyn NetworkCommandRunner>,
    mutation: Mutex<()>,
    last_mutation: Mutex<Option<Instant>>,
    mutation_cooldown: Duration,
    assume_binaries_available: bool,
    exposure_mutation: Mutex<()>,
    router_cache: Mutex<Option<(Instant, Result<UpnpGateway, String>)>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GamePortMapping {
    pub instance_id: String,
    pub name: String,
    pub manager: String,
    pub port: u16,
    pub running: bool,
}

#[derive(Clone, Debug)]
struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

trait NetworkCommandRunner: Send + Sync {
    fn run(
        &self,
        program: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<CommandOutput, String>;
}

struct ProcessRunner {
    timeout_binary: PathBuf,
}

impl NetworkCommandRunner for ProcessRunner {
    fn run(
        &self,
        program: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<CommandOutput, String> {
        let output = run_bounded_command(
            &self.timeout_binary,
            program,
            args,
            timeout,
            &[],
            MAX_COMMAND_OUTPUT_BYTES,
        )?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8(output.stdout)
                .map_err(|_| format!("{} returned invalid output", program.display()))?,
            stderr: String::from_utf8(output.stderr)
                .map_err(|_| format!("{} returned invalid output", program.display()))?,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct ListenerRecord {
    protocol: &'static str,
    family: &'static str,
    address: String,
    port: u16,
    wildcard: bool,
    uid: u32,
    inode: u64,
    process: Option<ProcessOwner>,
}

#[derive(Clone, Debug, Serialize)]
struct ProcessOwner {
    pid: u32,
    name: String,
}

#[derive(Clone, Debug, Serialize)]
struct DockerPublication {
    container_id: String,
    container_name: String,
    compose_service: Option<String>,
    protocol: String,
    container_port: u16,
    host_address: String,
    host_port: u16,
}

#[derive(Clone, Debug, Serialize)]
struct ParsedUfwRule {
    number: u32,
    display: String,
    action: String,
    source: String,
    protocol: Option<String>,
    port_start: Option<u16>,
    port_end: Option<u16>,
    comment: Option<String>,
    helix_rule_id: Option<String>,
}

#[derive(Clone, Debug)]
struct UfwSnapshot {
    installed: bool,
    active: bool,
    status: String,
    default_incoming: Option<String>,
    default_outgoing: Option<String>,
    default_routed: Option<String>,
    rules: Vec<ParsedUfwRule>,
    error: Option<String>,
}

struct ParsedUfwStatus {
    active: bool,
    status: String,
    incoming: Option<String>,
    outgoing: Option<String>,
    routed: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FirewallRecord {
    schema_version: u32,
    rule_id: String,
    marker: String,
    spec: FirewallRuleSpec,
    state: FirewallRecordState,
    created_at_unix_ms: u64,
    trashed_at_unix_ms: Option<u64>,
    undo_expires_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ServerExposureRecord {
    schema_version: u32,
    instance_id: String,
    port: u16,
    protocol: FirewallProtocol,
    local_ip: Ipv4Addr,
    external_ip: Ipv4Addr,
    mapping_description: String,
    firewall_rule_id: Option<String>,
    created_at_unix_ms: u64,
    verified_at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FirewallRecordState {
    CreatePending,
    Active,
    Trashed,
    DeletePending,
    RestorePending,
}

impl NetworkManager {
    pub fn new(config: NetworkConfig) -> Result<Self, String> {
        validate_config(&config)?;
        prepare_state_root(&config.state_root)?;
        Ok(Self {
            mutation_cooldown: Duration::from_secs(u64::from(config.mutation_cooldown_seconds)),
            runner: Arc::new(ProcessRunner {
                timeout_binary: config.timeout_binary.clone(),
            }),
            config,
            mutation: Mutex::new(()),
            last_mutation: Mutex::new(None),
            assume_binaries_available: false,
            exposure_mutation: Mutex::new(()),
            router_cache: Mutex::new(None),
        })
    }

    #[cfg(test)]
    fn with_runner(
        mut config: NetworkConfig,
        runner: Arc<dyn NetworkCommandRunner>,
    ) -> Result<Self, String> {
        validate_config_shape(&config)?;
        fs::create_dir_all(&config.state_root)
            .map_err(|_| "could not create test firewall state".to_owned())?;
        config.state_root = fs::canonicalize(&config.state_root)
            .map_err(|_| "could not resolve test firewall state".to_owned())?;
        Ok(Self {
            config,
            runner,
            mutation: Mutex::new(()),
            last_mutation: Mutex::new(None),
            mutation_cooldown: Duration::ZERO,
            assume_binaries_available: true,
            exposure_mutation: Mutex::new(()),
            router_cache: Mutex::new(None),
        })
    }

    pub fn inventory(&self, game_ports: &[GamePortMapping]) -> Result<Value, String> {
        let (mut listeners, listener_truncated) = collect_listeners(&self.config.proc_root)?;
        attach_process_owners(&self.config.proc_root, &mut listeners);
        let (docker_installed, docker_publications, docker_truncated, docker_error) =
            self.docker_publications();
        let firewall = self.ufw_snapshot()?;
        let firewall_state_verified =
            firewall.installed && firewall.active && firewall.error.is_none();
        let managed_records = self.load_records_bounded();
        let exposure_records = self.load_exposure_records_bounded();
        let router = self.router_snapshot(false);
        let private_ipv4 = router
            .as_ref()
            .ok()
            .map(|gateway| gateway.local_ip)
            .or_else(detect_private_ipv4);
        let router_inventory = router_inventory_value(&router);
        let game_port_rows = game_ports
            .iter()
            .flat_map(|mapping| {
                ["tcp", "udp"].into_iter().map(|protocol| {
                    let listening = listeners.iter().any(|listener| {
                        listener.protocol == protocol && listener.port == mapping.port
                    });
                    let publications = docker_publications
                        .iter()
                        .filter(|publication| {
                            publication.protocol == protocol
                                && (publication.host_port == mapping.port
                                    || publication.container_port == mapping.port)
                        })
                        .collect::<Vec<_>>();
                    let firewall_allowed = if firewall_state_verified {
                        Some(firewall.rules.iter().any(|rule| {
                            rule.action.eq_ignore_ascii_case("ALLOW IN")
                                && rule.protocol.as_deref() == Some(protocol)
                                && port_in_rule(mapping.port, rule)
                        }))
                    } else {
                        None
                    };
                    let exposure = exposure_records.get(&mapping.instance_id).filter(|record| {
                        protocol == "tcp"
                            && record.port == mapping.port
                            && record.protocol == FirewallProtocol::Tcp
                    });
                    let external_reachability = if let Some(record) = exposure {
                        json!({
                            "state": "router_mapping_confirmed",
                            "reachable": Value::Null,
                            "tested_from_external_network": false,
                            "router_mapping_verified": true,
                            "external_ip": record.external_ip,
                            "join_address": format_join_address(record.external_ip, record.port),
                            "verified_at_unix_ms": record.verified_at_unix_ms,
                            "note": "The router confirmed Helix's exact TCP mapping. Helix has not tested this address from a separate external network."
                        })
                    } else if protocol == "udp" {
                        json!({
                            "state": "not_requested",
                            "reachable": Value::Null,
                            "tested_from_external_network": false,
                            "note": "Minecraft player connections use TCP. UDP is not forwarded unless a separate feature explicitly needs it."
                        })
                    } else {
                        match &router {
                            Ok(gateway) if classify_external_address(gateway.external_ip) == ExternalAddressKind::Public => json!({
                                "state": "setup_available",
                                "reachable": Value::Null,
                                "tested_from_external_network": false,
                                "router_mapping_verified": false,
                                "external_ip": gateway.external_ip,
                                "join_address": format_join_address(gateway.external_ip, mapping.port),
                                "note": "The router supports automatic setup, but this server does not have a Helix-owned mapping yet."
                            }),
                            Ok(gateway) => json!({
                                "state": classify_external_address(gateway.external_ip).as_str(),
                                "reachable": false,
                                "tested_from_external_network": false,
                                "router_mapping_verified": false,
                                "external_ip": gateway.external_ip,
                                "join_address": Value::Null,
                                "note": "The router's WAN address is not globally routable, so port forwarding alone cannot provide a public join address."
                            }),
                            Err(error) => json!({
                                "state": "automatic_setup_unavailable",
                                "reachable": Value::Null,
                                "tested_from_external_network": false,
                                "router_mapping_verified": false,
                                "external_ip": Value::Null,
                                "join_address": Value::Null,
                                "note": error
                            })
                        }
                    };
                    json!({
                        "instance_id": mapping.instance_id,
                        "name": mapping.name,
                        "manager": mapping.manager,
                        "port": mapping.port,
                        "protocol": protocol,
                        "server_reported_running": mapping.running,
                        "listener_bound": listening,
                        "docker_published": !publications.is_empty(),
                        "docker_publications": publications,
                        "firewall_input_allowance": {
                            "applicable": firewall_state_verified,
                            "allowed": firewall_allowed,
                            "state": if !firewall.installed {
                                "ufw_unavailable"
                            } else if firewall.error.is_some() {
                                "ufw_state_unverified"
                            } else if !firewall.active {
                                "ufw_inactive"
                            } else if firewall_allowed == Some(true) {
                                "allowed"
                            } else {
                                "not_allowed_by_matching_rule"
                            }
                        },
                        "private_join_address": private_ipv4.map(|address| format_join_address(address, mapping.port)),
                        "external_reachability": external_reachability
                    })
                })
            })
            .collect::<Vec<_>>();
        let ufw_rules = firewall
            .rules
            .iter()
            .map(|rule| {
                let record = rule
                    .helix_rule_id
                    .as_deref()
                    .and_then(|id| managed_records.get(id));
                json!({
                    "number": rule.number,
                    "display": rule.display,
                    "action": rule.action,
                    "source": rule.source,
                    "protocol": rule.protocol,
                    "port_start": rule.port_start,
                    "port_end": rule.port_end,
                    "comment": rule.comment,
                    "helix_owned": rule.helix_rule_id.is_some(),
                    "rule_id": rule.helix_rule_id,
                    "managed": record.is_some(),
                    "management_state": record.map(|record| record.state),
                    "name": record.map(|record| record.spec.name.as_str()),
                    "description": record.map(|record| record.spec.description.as_str())
                })
            })
            .collect::<Vec<_>>();
        let mut managed_rule_state = managed_records
            .values()
            .map(|record| {
                let marker_present = snapshot_has_marker(&firewall, &record.marker);
                json!({
                    "rule_id": record.rule_id.as_str(),
                    "name": record.spec.name.as_str(),
                    "description": record.spec.description.as_str(),
                    "protocol": record.spec.protocol,
                    "port_start": record.spec.port_start,
                    "port_end": record.spec.port_end,
                    "state": record.state,
                    "created_at_unix_ms": record.created_at_unix_ms,
                    "trashed_at_unix_ms": record.trashed_at_unix_ms,
                    "undo_available": record.state == FirewallRecordState::Trashed
                        && record.undo_expires_at_unix_ms.is_some_and(|expires| now_unix_ms() <= expires),
                    "undo_expires_at_unix_ms": record.undo_expires_at_unix_ms,
                    "observed_in_ufw": marker_present,
                    "exact_body_verified": snapshot_has_exact_rule(&firewall, record)
                })
            })
            .collect::<Vec<_>>();
        managed_rule_state
            .sort_by(|left, right| left["rule_id"].as_str().cmp(&right["rule_id"].as_str()));
        Ok(json!({
            "schema_version": 1,
            "collected_at_unix_ms": now_unix_ms(),
            "addresses": {
                "private_ipv4": private_ipv4,
                "source": if router.is_ok() { "router_path" } else { "host_route" },
                "note": "The private address is intended for devices on the same LAN. A Tailscale address, when present elsewhere in Helix, is separate."
            },
            "router": router_inventory,
            "listeners": {
                "source": "linux_proc_net",
                "items": listeners,
                "truncated": listener_truncated,
                "owner_process_best_effort": true
            },
            "docker": {
                "installed": docker_installed,
                "publications": docker_publications,
                "containers_truncated": docker_truncated,
                "error": docker_error,
                "note": "A Docker publication is not the same as a host listener or a firewall allowance. Docker DNAT may bypass the UFW INPUT chain depending on Docker and firewall configuration."
            },
            "firewall": {
                "backend": "ufw",
                "installed": firewall.installed,
                "active": firewall.active,
                "status": firewall.status,
                "default_policy": {
                    "incoming": firewall.default_incoming,
                    "outgoing": firewall.default_outgoing,
                    "routed": firewall.default_routed
                },
                "rules": ufw_rules,
                "rules_truncated": firewall.rules.len() == MAX_FIREWALL_RULES,
                "helix_managed_rule_state": managed_rule_state,
                "error": firewall.error,
                "mutations_supported": firewall_state_verified,
                "mutation_scope": "Helix creates, deletes, and restores only exact UUID-commented allow rules. Enabling an inactive firewall is a separate confirmed flow that first preserves the selected listening SSH port. Public server setup creates only an exact Helix-owned TCP router mapping and a matching UFW rule when UFW is already active; it never enables UFW automatically.",
                "inactive_note": if firewall.installed && firewall.error.is_none() && !firewall.active {
                    Value::String("UFW is inactive, so Helix does not claim any firewall rule makes a port open.".to_owned())
                } else {
                    Value::Null
                }
            },
            "game_ports": game_port_rows,
            "external_reachability": {
                "state": "unverified",
                "tested_from_external_network": false
            }
        }))
    }

    pub fn set_server_exposure(
        &self,
        mapping: &GamePortMapping,
        enabled: bool,
        amp_claimed_ports: &HashSet<u16>,
    ) -> Result<Value, String> {
        let id = mapping.instance_id.strip_prefix("helix:").ok_or_else(|| {
            "automatic public access is available only for Helix-owned servers".to_owned()
        })?;
        validate_rule_id(id)?;
        if mapping.port < 1_024 {
            return Err("the server game port is invalid".to_owned());
        }
        let _exposure = self
            .exposure_mutation
            .lock()
            .map_err(|_| "the server exposure lock is unavailable".to_owned())?;
        let path = self.exposure_record_path(id)?;
        if !enabled {
            let record = read_exposure_record(&path, &mapping.instance_id)?;
            if record.port != mapping.port || record.protocol != FirewallProtocol::Tcp {
                return Err("the protected router mapping record does not match this server; nothing was removed".to_owned());
            }
            let gateway = self.router_snapshot(true)?;
            if gateway.local_ip != record.local_ip
                || !gateway.verify_tcp_mapping(record.port, &record.mapping_description)?
            {
                return Err("the router no longer reports the exact Helix-owned mapping; nothing was removed".to_owned());
            }
            gateway.delete_tcp_mapping(record.port)?;
            if gateway
                .verify_tcp_mapping(record.port, &record.mapping_description)
                .unwrap_or(false)
            {
                return Err("the router still reports the Helix mapping after deletion".to_owned());
            }
            let mut firewall_warning = None;
            if let Some(rule_id) = record.firewall_rule_id.as_deref()
                && let Err(error) = self.delete_rule(rule_id)
            {
                firewall_warning = Some(format!(
                    "the router mapping was removed, but the Helix UFW rule still needs attention: {error}"
                ));
            }
            remove_record_durable(&path)?;
            self.invalidate_router_cache();
            return Ok(json!({
                "instance_id": mapping.instance_id,
                "enabled": false,
                "router_mapping_removed": true,
                "firewall_warning": firewall_warning,
                "external_reachability": {
                    "state": "not_configured",
                    "reachable": Value::Null,
                    "tested_from_external_network": false
                }
            }));
        }

        if path.exists() {
            let record = read_exposure_record(&path, &mapping.instance_id)?;
            let gateway = self.router_snapshot(true)?;
            if record.port == mapping.port
                && gateway.local_ip == record.local_ip
                && gateway.verify_tcp_mapping(record.port, &record.mapping_description)?
            {
                return Ok(exposure_result(
                    mapping,
                    &record,
                    true,
                    "already_configured",
                    None,
                ));
            }
            return Err("a protected public-access record exists, but the router mapping has drifted; Helix did not overwrite it".to_owned());
        }

        if amp_claimed_ports.contains(&mapping.port) {
            return Err(crate::amp::amp_port_claimed_message(mapping.port));
        }

        let gateway = self.router_snapshot(true)?;
        let address_kind = classify_external_address(gateway.external_ip);
        if address_kind != ExternalAddressKind::Public {
            return Err(match address_kind {
                ExternalAddressKind::CarrierGradeNat => format!(
                    "the router reports CGNAT address {}; automatic port forwarding cannot create a public join address through carrier-grade NAT",
                    gateway.external_ip
                ),
                ExternalAddressKind::PrivateOrReserved => format!(
                    "the router reports non-public WAN address {}; another router or upstream NAT must be configured first",
                    gateway.external_ip
                ),
                ExternalAddressKind::Public => unreachable!("public handled above"),
            });
        }
        let description = format!("Helix Minecraft {}", &id[..8]);
        if gateway.tcp_mapping_exists(mapping.port)? {
            return Err(unowned_router_mapping_message(
                mapping.port,
                amp_claimed_ports.contains(&mapping.port),
            ));
        }
        gateway.add_tcp_mapping(mapping.port, &description)?;
        if !gateway.verify_tcp_mapping(mapping.port, &description)? {
            return Err(
                "the router accepted the request but did not return the exact Helix mapping; Helix did not delete an unverified router rule"
                    .to_owned(),
            );
        }

        let firewall = self.ufw_snapshot()?;
        let mut firewall_rule_id = None;
        let firewall_state = if firewall.installed && firewall.error.is_none() && firewall.active {
            match self.create_rule(FirewallRuleSpec {
                name: format!("{} public server", mapping.name),
                description: format!(
                    "TCP game traffic for {} on port {}",
                    mapping.name, mapping.port
                ),
                protocol: FirewallProtocol::Tcp,
                port_start: mapping.port,
                port_end: mapping.port,
            }) {
                Ok(value) => {
                    firewall_rule_id = value
                        .get("rule_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    "helix_rule_verified"
                }
                Err(error) => {
                    let _ = gateway.delete_tcp_mapping(mapping.port);
                    return Err(format!(
                        "the router mapping was rolled back because the active host firewall rule could not be verified: {error}"
                    ));
                }
            }
        } else if firewall.installed && firewall.error.is_none() {
            "ufw_inactive_not_blocking"
        } else if !firewall.installed {
            "ufw_unavailable"
        } else {
            let _ = gateway.delete_tcp_mapping(mapping.port);
            return Err("the router mapping was rolled back because the host firewall state could not be verified".to_owned());
        };
        let now = now_unix_ms();
        let record = ServerExposureRecord {
            schema_version: 1,
            instance_id: mapping.instance_id.clone(),
            port: mapping.port,
            protocol: FirewallProtocol::Tcp,
            local_ip: gateway.local_ip,
            external_ip: gateway.external_ip,
            mapping_description: description,
            firewall_rule_id: firewall_rule_id.clone(),
            created_at_unix_ms: now,
            verified_at_unix_ms: now,
        };
        if let Err(error) = write_exposure_record(&path, &record) {
            let _ = gateway.delete_tcp_mapping(mapping.port);
            if let Some(rule_id) = firewall_rule_id.as_deref() {
                let _ = self.delete_rule(rule_id);
            }
            return Err(format!(
                "public access was rolled back because its protected record could not be saved: {error}"
            ));
        }
        self.invalidate_router_cache();
        Ok(exposure_result(
            mapping,
            &record,
            false,
            "router_mapping_confirmed",
            Some(firewall_state),
        ))
    }

    fn router_snapshot(&self, force: bool) -> Result<UpnpGateway, String> {
        let mut cache = self
            .router_cache
            .lock()
            .map_err(|_| "the router discovery cache is unavailable".to_owned())?;
        if !force
            && let Some((collected, result)) = cache.as_ref()
            && collected.elapsed() < Duration::from_secs(60)
        {
            return result.clone();
        }
        let result = UpnpGateway::discover();
        *cache = Some((Instant::now(), result.clone()));
        result
    }

    fn invalidate_router_cache(&self) {
        if let Ok(mut cache) = self.router_cache.lock() {
            *cache = None;
        }
    }

    pub fn enable_ufw(&self, ssh_port: u16, confirmation: &str) -> Result<Value, String> {
        if confirmation != "ENABLE UFW" {
            return Err("type ENABLE UFW to confirm this host-wide firewall change".to_owned());
        }
        if ssh_port == 0 {
            return Err("the SSH safety port must be between 1 and 65535".to_owned());
        }
        let (mut listeners, _) = collect_listeners(&self.config.proc_root)?;
        attach_process_owners(&self.config.proc_root, &mut listeners);
        if !listeners
            .iter()
            .any(|listener| listener.protocol == "tcp" && listener.port == ssh_port)
        {
            return Err(format!(
                "no listening TCP socket was found on SSH safety port {ssh_port}; UFW was not enabled"
            ));
        }

        let _mutation = self.begin_mutation()?;
        let before = self.ufw_snapshot()?;
        if !before.installed {
            return Err("UFW is unavailable; Helix did not change the firewall".to_owned());
        }
        if let Some(error) = &before.error {
            return Err(format!("UFW state could not be verified: {error}"));
        }
        if before.active {
            return Err("UFW is already active; no change was made".to_owned());
        }

        let rule_id = Uuid::new_v4().to_string();
        let marker = format!("helix:{rule_id}");
        let spec = FirewallRuleSpec {
            name: "SSH safety before firewall enable".to_owned(),
            description: format!(
                "Preserves the confirmed listening SSH port {ssh_port} before Helix enables UFW."
            ),
            protocol: FirewallProtocol::Tcp,
            port_start: ssh_port,
            port_end: ssh_port,
        };
        let mut record = FirewallRecord {
            schema_version: 1,
            rule_id: rule_id.clone(),
            marker: marker.clone(),
            spec: spec.clone(),
            state: FirewallRecordState::CreatePending,
            created_at_unix_ms: now_unix_ms(),
            trashed_at_unix_ms: None,
            undo_expires_at_unix_ms: None,
        };
        let record_path = self.record_path(&rule_id)?;
        write_record(&record_path, &record)?;

        let add_result = self
            .runner
            .run(
                &self.config.ufw_binary,
                &add_rule_args(&spec, &marker),
                Duration::from_secs(5),
            )
            .and_then(require_success);
        if let Err(error) = add_result {
            let _ = remove_record_durable(&record_path);
            return Err(format!(
                "the SSH safety rule could not be staged, so UFW was not enabled: {error}"
            ));
        }

        let enable_result = self
            .runner
            .run(
                &self.config.ufw_binary,
                &["--force".to_owned(), "enable".to_owned()],
                Duration::from_secs(15),
            )
            .and_then(require_success);
        let after = self.ufw_snapshot();
        if enable_result.is_err()
            || after.as_ref().is_err()
            || !after.as_ref().is_ok_and(|snapshot| {
                snapshot.active
                    && snapshot.error.is_none()
                    && snapshot_has_exact_rule(snapshot, &record)
            })
        {
            let _ = self.runner.run(
                &self.config.ufw_binary,
                &["--force".to_owned(), "disable".to_owned()],
                Duration::from_secs(15),
            );
            let cleanup = self
                .runner
                .run(
                    &self.config.ufw_binary,
                    &delete_rule_args(&spec, &marker),
                    Duration::from_secs(5),
                )
                .and_then(require_success);
            if cleanup.is_ok() {
                let _ = remove_record_durable(&record_path);
            }
            return Err(format!(
                "UFW did not become active with the exact SSH safety rule, so Helix attempted to restore the inactive state{}",
                enable_result
                    .err()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            ));
        }
        let after = match after {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(format!(
                    "UFW changed state, but Helix could not retain the verification snapshot: {error}"
                ));
            }
        };
        record.state = FirewallRecordState::Active;
        write_record(&record_path, &record).map_err(|error| {
            format!(
                "UFW is active with the verified SSH safety rule, but its protected journal needs attention: {error}"
            )
        })?;
        Ok(json!({
            "enabled": true,
            "verified": true,
            "ssh_port": ssh_port,
            "ssh_rule_id": rule_id,
            "before_evidence": ufw_evidence(&before),
            "after_evidence": ufw_evidence(&after),
            "note": "UFW is active and the selected listening SSH port has an exact Helix-owned allow rule. Router and upstream reachability remain separate."
        }))
    }

    pub fn create_rule(&self, spec: FirewallRuleSpec) -> Result<Value, String> {
        let spec = normalize_rule_spec(spec)?;
        let _mutation = self.begin_mutation()?;
        let before = self.require_active_ufw()?;
        let rule_id = Uuid::new_v4().to_string();
        let marker = format!("helix:{rule_id}");
        if before
            .rules
            .iter()
            .any(|rule| rule.comment.as_deref() == Some(&marker))
        {
            return Err("the generated firewall rule identity already exists".to_owned());
        }
        let created_at_unix_ms = now_unix_ms();
        let mut record = FirewallRecord {
            schema_version: 1,
            rule_id: rule_id.clone(),
            marker: marker.clone(),
            spec: spec.clone(),
            state: FirewallRecordState::CreatePending,
            created_at_unix_ms,
            trashed_at_unix_ms: None,
            undo_expires_at_unix_ms: None,
        };
        let record_path = self.record_path(&rule_id)?;
        write_record(&record_path, &record)?;
        let args = add_rule_args(&spec, &marker);
        let command = self
            .runner
            .run(&self.config.ufw_binary, &args, Duration::from_secs(5));
        let command_error = match command {
            Ok(output) => require_success(output).err(),
            Err(error) => Some(error),
        };
        let after = self.ufw_snapshot().map_err(|error| {
            format!(
                "the firewall add outcome could not be verified; its protected record was retained: {error}"
            )
        })?;
        if !snapshot_has_exact_rule(&after, &record) {
            let cleanup = self.runner.run(
                &self.config.ufw_binary,
                &delete_rule_args(&spec, &marker),
                Duration::from_secs(5),
            );
            let cleanup_error = cleanup.and_then(require_success).err();
            let cleanup_snapshot = self.ufw_snapshot().map_err(|error| {
                format!(
                    "UFW did not report the new exact rule and cleanup could not be verified; its protected record was retained: {error}"
                )
            })?;
            if snapshot_has_marker(&cleanup_snapshot, &marker) {
                return Err(format!(
                    "UFW did not report the new exact rule and the UUID marker remains after cleanup; its protected record was retained{}",
                    cleanup_error
                        .as_deref()
                        .map_or_else(String::new, |error| format!(": {error}"))
                ));
            }
            remove_record_durable(&record_path)?;
            return Err(command_error.map_or_else(
                || "UFW did not report the new rule after writing; Helix verified that no UUID-marked rule remains".to_owned(),
                |error| format!(
                    "the firewall rule was not added and Helix verified that no UUID-marked rule remains: {error}"
                ),
            ));
        }
        record.state = FirewallRecordState::Active;
        write_record(&record_path, &record).map_err(|error| {
            format!(
                "the firewall rule was created, but its protected journal remains create_pending: {error}"
            )
        })?;
        Ok(json!({
            "rule_id": rule_id,
            "state": "active",
            "rule": spec,
            "marker": marker,
            "created_at_unix_ms": created_at_unix_ms,
            "before_evidence": ufw_evidence(&before),
            "after_evidence": ufw_evidence(&after),
            "verified": true,
            "command_warning": command_error,
            "undo_available": false
        }))
    }

    pub fn delete_rule(&self, rule_id: &str) -> Result<Value, String> {
        validate_rule_id(rule_id)?;
        let _mutation = self.begin_mutation()?;
        let path = self.record_path(rule_id)?;
        let mut record = read_record(&path, rule_id)?;
        let before = self.require_active_ufw()?;
        if record.state == FirewallRecordState::CreatePending {
            if !snapshot_has_marker(&before, &record.marker) {
                remove_record_durable(&path)?;
                return Ok(json!({
                    "rule_id": rule_id,
                    "state": "absent",
                    "reconciled_create_pending": true,
                    "verified": true,
                    "undo_available": false,
                    "before_evidence": ufw_evidence(&before),
                    "after_evidence": ufw_evidence(&before)
                }));
            }
            if !snapshot_has_exact_rule(&before, &record) {
                return Err("a create-pending firewall rule has this Helix identity but its body does not match; nothing was deleted".to_owned());
            }
            record.state = FirewallRecordState::Active;
            write_record(&path, &record)?;
        }
        match record.state {
            FirewallRecordState::CreatePending => unreachable!("create-pending state reconciled"),
            FirewallRecordState::Trashed => {
                return Err("the firewall rule is already in Helix trash".to_owned());
            }
            FirewallRecordState::RestorePending => {
                return Err(
                    "a previous firewall restore has an unverified outcome; reconcile restore before deleting"
                        .to_owned(),
                );
            }
            FirewallRecordState::DeletePending if !snapshot_has_marker(&before, &record.marker) => {
                record.state = FirewallRecordState::Trashed;
                write_record(&path, &record)?;
                return Ok(deleted_rule_response(rule_id, &record, &before, &before));
            }
            FirewallRecordState::Active | FirewallRecordState::DeletePending => {}
        }
        if !snapshot_has_exact_rule(&before, &record) {
            return Err("the exact Helix-owned firewall rule body did not match its protected record; nothing was deleted".to_owned());
        }
        if record.state == FirewallRecordState::Active {
            let trashed_at_unix_ms = now_unix_ms();
            record.state = FirewallRecordState::DeletePending;
            record.trashed_at_unix_ms = Some(trashed_at_unix_ms);
            record.undo_expires_at_unix_ms =
                Some(trashed_at_unix_ms.saturating_add(UNDO_WINDOW_MS));
            write_record(&path, &record)?;
        }
        let command = self.runner.run(
            &self.config.ufw_binary,
            &delete_rule_args(&record.spec, &record.marker),
            Duration::from_secs(5),
        );
        let command_error = match command {
            Ok(output) => require_success(output).err(),
            Err(error) => Some(error),
        };
        let after = match self.ufw_snapshot() {
            Ok(after) => after,
            Err(error) => {
                return Err(format!(
                    "the firewall delete outcome could not be verified and remains delete_pending: {error}"
                ));
            }
        };
        if snapshot_has_marker(&after, &record.marker) {
            record.state = FirewallRecordState::Active;
            record.trashed_at_unix_ms = None;
            record.undo_expires_at_unix_ms = None;
            write_record(&path, &record)?;
            return Err(command_error.map_or_else(
                || "UFW still reports the exact firewall rule after deletion".to_owned(),
                |error| format!("the firewall rule was not deleted: {error}"),
            ));
        }
        record.state = FirewallRecordState::Trashed;
        write_record(&path, &record)?;
        Ok(deleted_rule_response(rule_id, &record, &before, &after))
    }

    pub fn restore_rule(&self, rule_id: &str) -> Result<Value, String> {
        validate_rule_id(rule_id)?;
        let _mutation = self.begin_mutation()?;
        let path = self.record_path(rule_id)?;
        let mut record = read_record(&path, rule_id)?;
        if record.state == FirewallRecordState::Active {
            return Err("the firewall rule is not in Helix trash".to_owned());
        }
        if record.state == FirewallRecordState::CreatePending {
            return Err(
                "the firewall rule creation has an unverified outcome; reconcile it before restoring"
                    .to_owned(),
            );
        }
        if record.state == FirewallRecordState::DeletePending {
            return Err(
                "a previous firewall delete has an unverified outcome; reconcile delete before restoring"
                    .to_owned(),
            );
        }
        let expires = record
            .undo_expires_at_unix_ms
            .ok_or_else(|| "the firewall undo record is invalid".to_owned())?;
        if now_unix_ms() > expires {
            return Err("the firewall rule undo window has expired".to_owned());
        }
        let before = self.require_active_ufw()?;
        if snapshot_has_exact_rule(&before, &record) {
            if record.state == FirewallRecordState::RestorePending {
                record.state = FirewallRecordState::Active;
                record.trashed_at_unix_ms = None;
                record.undo_expires_at_unix_ms = None;
                write_record(&path, &record)?;
                return Ok(restored_rule_response(rule_id, &record, &before, &before));
            }
            return Err(
                "the exact firewall rule already exists; restore was not attempted".to_owned(),
            );
        }
        if snapshot_has_marker(&before, &record.marker) {
            return Err("a firewall rule with this Helix identity exists but its body does not match; restore was not attempted".to_owned());
        }
        if record.state == FirewallRecordState::Trashed {
            record.state = FirewallRecordState::RestorePending;
            write_record(&path, &record)?;
        }
        let command = self.runner.run(
            &self.config.ufw_binary,
            &add_rule_args(&record.spec, &record.marker),
            Duration::from_secs(5),
        );
        let command_error = match command {
            Ok(output) => require_success(output).err(),
            Err(error) => Some(error),
        };
        let after = match self.ufw_snapshot() {
            Ok(after) => after,
            Err(error) => {
                return Err(format!(
                    "the firewall restore outcome could not be verified and remains restore_pending: {error}"
                ));
            }
        };
        if !snapshot_has_exact_rule(&after, &record) {
            if snapshot_has_marker(&after, &record.marker) {
                return Err(command_error.map_or_else(
                    || "a UUID-marked firewall rule remained after restore, but its body did not match; the record remains restore_pending".to_owned(),
                    |error| format!("the firewall restore outcome is ambiguous and remains restore_pending: {error}"),
                ));
            }
            record.state = FirewallRecordState::Trashed;
            write_record(&path, &record)?;
            return Err(command_error.map_or_else(
                || "UFW did not report the exact firewall rule after restore".to_owned(),
                |error| format!("the firewall rule was not restored: {error}"),
            ));
        }
        record.state = FirewallRecordState::Active;
        record.trashed_at_unix_ms = None;
        record.undo_expires_at_unix_ms = None;
        write_record(&path, &record)?;
        Ok(restored_rule_response(rule_id, &record, &before, &after))
    }

    fn begin_mutation(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        let guard = self
            .mutation
            .try_lock()
            .map_err(|_| "another firewall mutation is already in progress".to_owned())?;
        let mut last = self
            .last_mutation
            .lock()
            .map_err(|_| "firewall mutation rate state failed".to_owned())?;
        if last.is_some_and(|last| last.elapsed() < self.mutation_cooldown) {
            return Err("firewall mutations are temporarily rate limited".to_owned());
        }
        *last = Some(Instant::now());
        drop(last);
        Ok(guard)
    }

    fn require_active_ufw(&self) -> Result<UfwSnapshot, String> {
        let snapshot = self.ufw_snapshot()?;
        if !snapshot.installed {
            return Err("UFW is unavailable; Helix did not change the firewall".to_owned());
        }
        if let Some(error) = &snapshot.error {
            return Err(format!("UFW state could not be verified: {error}"));
        }
        if !snapshot.active {
            return Err("UFW is inactive; Helix did not change the firewall".to_owned());
        }
        Ok(snapshot)
    }

    fn ufw_snapshot(&self) -> Result<UfwSnapshot, String> {
        if !self.binary_available(&self.config.ufw_binary) {
            return Ok(UfwSnapshot {
                installed: false,
                active: false,
                status: "unavailable".to_owned(),
                default_incoming: None,
                default_outgoing: None,
                default_routed: None,
                rules: Vec::new(),
                error: Some("the configured UFW binary is unavailable".to_owned()),
            });
        }
        let verbose = match self.runner.run(
            &self.config.ufw_binary,
            &["status".to_owned(), "verbose".to_owned()],
            Duration::from_secs(2),
        ) {
            Ok(output) => output,
            Err(error) => {
                return Ok(UfwSnapshot {
                    installed: true,
                    active: false,
                    status: "error".to_owned(),
                    default_incoming: None,
                    default_outgoing: None,
                    default_routed: None,
                    rules: Vec::new(),
                    error: Some(error),
                });
            }
        };
        if !verbose.success {
            return Ok(UfwSnapshot {
                installed: true,
                active: false,
                status: "error".to_owned(),
                default_incoming: None,
                default_outgoing: None,
                default_routed: None,
                rules: Vec::new(),
                error: Some(bounded_command_error(&verbose)),
            });
        }
        let ParsedUfwStatus {
            active,
            status,
            incoming,
            outgoing,
            routed,
        } = match parse_ufw_verbose(&verbose.stdout) {
            Ok(parsed) => parsed,
            Err(error) => {
                return Ok(UfwSnapshot {
                    installed: true,
                    active: false,
                    status: "error".to_owned(),
                    default_incoming: None,
                    default_outgoing: None,
                    default_routed: None,
                    rules: Vec::new(),
                    error: Some(error),
                });
            }
        };
        if !active {
            return Ok(UfwSnapshot {
                installed: true,
                active: false,
                status,
                default_incoming: incoming,
                default_outgoing: outgoing,
                default_routed: routed,
                rules: Vec::new(),
                error: None,
            });
        }
        let numbered = match self.runner.run(
            &self.config.ufw_binary,
            &["status".to_owned(), "numbered".to_owned()],
            Duration::from_secs(2),
        ) {
            Ok(output) => output,
            Err(error) => {
                return Ok(UfwSnapshot {
                    installed: true,
                    active: true,
                    status,
                    default_incoming: incoming,
                    default_outgoing: outgoing,
                    default_routed: routed,
                    rules: Vec::new(),
                    error: Some(error),
                });
            }
        };
        if !numbered.success {
            return Ok(UfwSnapshot {
                installed: true,
                active: true,
                status,
                default_incoming: incoming,
                default_outgoing: outgoing,
                default_routed: routed,
                rules: Vec::new(),
                error: Some(bounded_command_error(&numbered)),
            });
        }
        Ok(UfwSnapshot {
            installed: true,
            active: true,
            status,
            default_incoming: incoming,
            default_outgoing: outgoing,
            default_routed: routed,
            rules: parse_ufw_numbered(&numbered.stdout),
            error: None,
        })
    }

    fn docker_publications(&self) -> (bool, Vec<DockerPublication>, bool, Option<String>) {
        if !self.binary_available(&self.config.docker_binary) {
            return (
                false,
                Vec::new(),
                false,
                Some("the configured Docker binary is unavailable".to_owned()),
            );
        }
        let ps = match self.runner.run(
            &self.config.docker_binary,
            &[
                "ps".to_owned(),
                "--quiet".to_owned(),
                "--no-trunc".to_owned(),
            ],
            Duration::from_secs(5),
        ) {
            Ok(output) if output.success => output,
            Ok(output) => {
                return (
                    true,
                    Vec::new(),
                    false,
                    Some(bounded_command_error(&output)),
                );
            }
            Err(error) => return (true, Vec::new(), false, Some(error)),
        };
        let mut ids = ps
            .stdout
            .lines()
            .map(str::trim)
            .filter(|id| valid_container_id(id))
            .take(MAX_DOCKER_CONTAINERS.saturating_add(1))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let truncated = ids.len() > MAX_DOCKER_CONTAINERS;
        ids.truncate(MAX_DOCKER_CONTAINERS);
        let mut publications = Vec::new();
        for chunk in ids.chunks(MAX_DOCKER_CONTAINERS) {
            let mut args = vec![
                "inspect".to_owned(),
                "--type".to_owned(),
                "container".to_owned(),
            ];
            args.extend(chunk.iter().cloned());
            match self
                .runner
                .run(&self.config.docker_binary, &args, Duration::from_secs(10))
            {
                Ok(output) if output.success => {
                    if let Err(error) = parse_docker_inspect(&output.stdout, &mut publications) {
                        return (true, publications, truncated, Some(error));
                    }
                }
                Ok(output) => {
                    return (
                        true,
                        publications,
                        truncated,
                        Some(bounded_command_error(&output)),
                    );
                }
                Err(error) => return (true, publications, truncated, Some(error)),
            }
        }
        (true, publications, truncated, None)
    }

    fn binary_available(&self, path: &Path) -> bool {
        self.assume_binaries_available || path.is_file()
    }

    fn record_path(&self, rule_id: &str) -> Result<PathBuf, String> {
        validate_rule_id(rule_id)?;
        Ok(self.config.state_root.join(format!("{rule_id}.json")))
    }

    fn exposure_record_path(&self, instance_uuid: &str) -> Result<PathBuf, String> {
        validate_rule_id(instance_uuid)?;
        Ok(self
            .config
            .state_root
            .join(format!("exposure-{instance_uuid}.json")))
    }

    fn load_exposure_records_bounded(&self) -> HashMap<String, ServerExposureRecord> {
        let Ok(entries) = fs::read_dir(&self.config.state_root) else {
            return HashMap::new();
        };
        entries
            .flatten()
            .take(MAX_DOCKER_CONTAINERS)
            .filter_map(|entry| {
                let file_name = entry.file_name();
                let uuid = file_name
                    .to_str()?
                    .strip_prefix("exposure-")?
                    .strip_suffix(".json")?;
                let instance_id = format!("helix:{uuid}");
                read_exposure_record(&entry.path(), &instance_id)
                    .ok()
                    .map(|record| (instance_id, record))
            })
            .collect()
    }

    fn load_records_bounded(&self) -> HashMap<String, FirewallRecord> {
        let Ok(entries) = fs::read_dir(&self.config.state_root) else {
            return HashMap::new();
        };
        entries
            .flatten()
            .take(MAX_FIREWALL_RULES)
            .filter_map(|entry| {
                let id = entry
                    .file_name()
                    .to_str()?
                    .strip_suffix(".json")?
                    .to_owned();
                read_record(&entry.path(), &id)
                    .ok()
                    .map(|record| (id, record))
            })
            .collect()
    }
}

fn normalize_rule_spec(mut spec: FirewallRuleSpec) -> Result<FirewallRuleSpec, String> {
    spec.name = spec.name.trim().to_owned();
    spec.description = spec.description.trim().to_owned();
    if spec.name.is_empty() || spec.name.len() > 80 || spec.name.chars().any(char::is_control) {
        return Err("firewall rule name must be 1 to 80 safe characters".to_owned());
    }
    if spec.description.len() > 300 || spec.description.chars().any(char::is_control) {
        return Err("firewall rule description must be at most 300 safe characters".to_owned());
    }
    if spec.port_start == 0 || spec.port_end < spec.port_start {
        return Err("firewall port or range is invalid".to_owned());
    }
    let width = spec
        .port_end
        .saturating_sub(spec.port_start)
        .saturating_add(1);
    if width > MAX_RULE_RANGE_WIDTH {
        return Err(format!(
            "firewall port ranges may contain at most {MAX_RULE_RANGE_WIDTH} ports"
        ));
    }
    Ok(spec)
}

fn add_rule_args(spec: &FirewallRuleSpec, marker: &str) -> Vec<String> {
    vec![
        "allow".to_owned(),
        "in".to_owned(),
        "proto".to_owned(),
        protocol_name(spec.protocol).to_owned(),
        "from".to_owned(),
        "any".to_owned(),
        "to".to_owned(),
        "any".to_owned(),
        "port".to_owned(),
        port_argument(spec),
        "comment".to_owned(),
        marker.to_owned(),
    ]
}

fn delete_rule_args(spec: &FirewallRuleSpec, marker: &str) -> Vec<String> {
    let mut args = vec!["--force".to_owned(), "delete".to_owned()];
    args.extend(add_rule_args(spec, marker));
    args
}

fn protocol_name(protocol: FirewallProtocol) -> &'static str {
    match protocol {
        FirewallProtocol::Tcp => "tcp",
        FirewallProtocol::Udp => "udp",
    }
}

fn port_argument(spec: &FirewallRuleSpec) -> String {
    if spec.port_start == spec.port_end {
        spec.port_start.to_string()
    } else {
        format!("{}:{}", spec.port_start, spec.port_end)
    }
}

fn snapshot_has_exact_rule(snapshot: &UfwSnapshot, record: &FirewallRecord) -> bool {
    snapshot.rules.iter().any(|rule| {
        rule.comment.as_deref() == Some(&record.marker)
            && rule.action.eq_ignore_ascii_case("ALLOW IN")
            && rule.protocol.as_deref() == Some(protocol_name(record.spec.protocol))
            && rule.port_start == Some(record.spec.port_start)
            && rule.port_end == Some(record.spec.port_end)
    })
}

fn snapshot_has_marker(snapshot: &UfwSnapshot, marker: &str) -> bool {
    snapshot
        .rules
        .iter()
        .any(|rule| rule.comment.as_deref() == Some(marker))
}

fn deleted_rule_response(
    rule_id: &str,
    record: &FirewallRecord,
    before: &UfwSnapshot,
    after: &UfwSnapshot,
) -> Value {
    let expires = record.undo_expires_at_unix_ms;
    json!({
        "rule_id": rule_id,
        "state": "trashed",
        "original_rule": &record.spec,
        "trashed_at_unix_ms": record.trashed_at_unix_ms,
        "undo_available": expires.is_some_and(|expires| now_unix_ms() <= expires),
        "undo_expires_at_unix_ms": expires,
        "before_evidence": ufw_evidence(before),
        "after_evidence": ufw_evidence(after),
        "verified": true
    })
}

fn restored_rule_response(
    rule_id: &str,
    record: &FirewallRecord,
    before: &UfwSnapshot,
    after: &UfwSnapshot,
) -> Value {
    json!({
        "rule_id": rule_id,
        "state": "active",
        "rule": &record.spec,
        "restored_at_unix_ms": now_unix_ms(),
        "undo_available": false,
        "before_evidence": ufw_evidence(before),
        "after_evidence": ufw_evidence(after),
        "verified": true
    })
}

fn ufw_evidence(snapshot: &UfwSnapshot) -> Value {
    json!({
        "installed": snapshot.installed,
        "active": snapshot.active,
        "status": snapshot.status,
        "rule_count": snapshot.rules.len(),
        "captured_at_unix_ms": now_unix_ms()
    })
}

fn parse_ufw_verbose(output: &str) -> Result<ParsedUfwStatus, String> {
    let status_line = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Status:"))
        .map(str::trim)
        .ok_or_else(|| "UFW did not report its status".to_owned())?;
    let status = sanitize_text(status_line, 40);
    let active = status.eq_ignore_ascii_case("active");
    let mut incoming = None;
    let mut outgoing = None;
    let mut routed = None;
    if let Some(defaults) = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Default:"))
    {
        for part in defaults.split(',') {
            let part = part.trim();
            let Some((value, scope)) = part.split_once(' ') else {
                continue;
            };
            let value = Some(sanitize_text(value, 20));
            match scope.trim_matches(['(', ')']) {
                "incoming" => incoming = value,
                "outgoing" => outgoing = value,
                "routed" => routed = value,
                _ => {}
            }
        }
    }
    Ok(ParsedUfwStatus {
        active,
        status,
        incoming,
        outgoing,
        routed,
    })
}

fn parse_ufw_numbered(output: &str) -> Vec<ParsedUfwRule> {
    output
        .lines()
        .filter_map(parse_ufw_rule_line)
        .take(MAX_FIREWALL_RULES)
        .collect()
}

fn parse_ufw_rule_line(line: &str) -> Option<ParsedUfwRule> {
    let trimmed = line.trim();
    let end = trimmed.find(']')?;
    if !trimmed.starts_with('[') {
        return None;
    }
    let number = trimmed[1..end].trim().parse().ok()?;
    let body = trimmed[end + 1..].trim();
    let (rule_text, comment) = body
        .split_once(" # ")
        .map_or((body, None), |(rule, comment)| {
            (rule.trim(), Some(sanitize_text(comment.trim(), 120)))
        });
    let columns = rule_text
        .split("  ")
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .collect::<Vec<_>>();
    if columns.len() < 3 {
        return None;
    }
    let destination = columns[0].trim_end_matches(" (v6)").trim();
    let action = sanitize_text(columns[1], 32);
    let source = sanitize_text(columns[2], 160);
    let (port_start, port_end, protocol) = parse_ufw_destination(destination);
    let helix_rule_id = comment.as_deref().and_then(parse_helix_marker);
    Some(ParsedUfwRule {
        number,
        display: sanitize_text(rule_text, 500),
        action,
        source,
        protocol,
        port_start,
        port_end,
        comment,
        helix_rule_id,
    })
}

fn parse_ufw_destination(destination: &str) -> (Option<u16>, Option<u16>, Option<String>) {
    let Some((ports, protocol)) = destination.rsplit_once('/') else {
        return (None, None, None);
    };
    let protocol = protocol.trim().to_ascii_lowercase();
    if protocol != "tcp" && protocol != "udp" {
        return (None, None, None);
    }
    let range = if let Some((start, end)) = ports.split_once(':') {
        start
            .trim()
            .parse::<u16>()
            .ok()
            .zip(end.trim().parse::<u16>().ok())
    } else {
        ports.trim().parse::<u16>().ok().map(|port| (port, port))
    };
    match range {
        Some((start, end)) if start > 0 && end >= start => (Some(start), Some(end), Some(protocol)),
        _ => (None, None, Some(protocol)),
    }
}

fn parse_helix_marker(comment: &str) -> Option<String> {
    let id = comment.strip_prefix("helix:")?;
    let parsed = Uuid::parse_str(id).ok()?;
    (parsed.to_string() == id).then(|| id.to_owned())
}

fn port_in_rule(port: u16, rule: &ParsedUfwRule) -> bool {
    rule.port_start
        .zip(rule.port_end)
        .is_some_and(|(start, end)| (start..=end).contains(&port))
}

fn parse_docker_inspect(
    output: &str,
    publications: &mut Vec<DockerPublication>,
) -> Result<(), String> {
    let containers: Vec<Value> = serde_json::from_str(output)
        .map_err(|_| "Docker returned invalid container inspection data".to_owned())?;
    for container in containers.into_iter().take(MAX_DOCKER_CONTAINERS) {
        let id = container
            .get("Id")
            .and_then(Value::as_str)
            .filter(|id| valid_container_id(id))
            .unwrap_or("unknown")
            .to_owned();
        let name = container
            .get("Name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .trim_start_matches('/')
            .to_owned();
        let compose_service = container
            .pointer("/Config/Labels/com.docker.compose.service")
            .and_then(Value::as_str)
            .map(|value| sanitize_text(value, 120));
        let Some(ports) = container
            .pointer("/NetworkSettings/Ports")
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (container_socket, bindings) in ports {
            let Some((port, protocol)) = parse_socket_name(container_socket) else {
                continue;
            };
            let Some(bindings) = bindings.as_array() else {
                continue;
            };
            for binding in bindings {
                let Some(host_port) = binding
                    .get("HostPort")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u16>().ok())
                else {
                    continue;
                };
                let host_address = binding
                    .get("HostIp")
                    .and_then(Value::as_str)
                    .map_or_else(|| "0.0.0.0".to_owned(), |value| sanitize_text(value, 64));
                publications.push(DockerPublication {
                    container_id: id.clone(),
                    container_name: name.clone(),
                    compose_service: compose_service.clone(),
                    protocol: protocol.to_owned(),
                    container_port: port,
                    host_address,
                    host_port,
                });
            }
        }
    }
    Ok(())
}

fn parse_socket_name(value: &str) -> Option<(u16, &str)> {
    let (port, protocol) = value.split_once('/')?;
    let port = port.parse().ok()?;
    matches!(protocol, "tcp" | "udp").then_some((port, protocol))
}

fn valid_container_id(id: &str) -> bool {
    (12..=64).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn collect_listeners(proc_root: &Path) -> Result<(Vec<ListenerRecord>, bool), String> {
    let mut listeners = Vec::new();
    for (relative, protocol, family, state) in [
        ("net/tcp", "tcp", "ipv4", "0A"),
        ("net/tcp6", "tcp", "ipv6", "0A"),
        ("net/udp", "udp", "ipv4", "07"),
        ("net/udp6", "udp", "ipv6", "07"),
    ] {
        let path = proc_root.join(relative);
        let Ok(body) = fs::read_to_string(path) else {
            continue;
        };
        for line in body.lines().skip(1) {
            if listeners.len() >= MAX_LISTENERS {
                return Ok((listeners, true));
            }
            if let Some(listener) = parse_proc_socket_line(line, protocol, family, state) {
                listeners.push(listener);
            }
        }
    }
    Ok((listeners, false))
}

fn parse_proc_socket_line(
    line: &str,
    protocol: &'static str,
    family: &'static str,
    required_state: &str,
) -> Option<ListenerRecord> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 10 || fields[3] != required_state {
        return None;
    }
    let (address_hex, port_hex) = fields[1].split_once(':')?;
    let address = if family == "ipv4" {
        parse_proc_ipv4(address_hex)?
    } else {
        parse_proc_ipv6(address_hex)?
    };
    let port = u16::from_str_radix(port_hex, 16).ok()?;
    if port == 0 {
        return None;
    }
    let wildcard = address == "0.0.0.0" || address == "::";
    Some(ListenerRecord {
        protocol,
        family,
        address,
        port,
        wildcard,
        uid: fields[7].parse().ok()?,
        inode: fields[9].parse().ok()?,
        process: None,
    })
}

fn parse_proc_ipv4(value: &str) -> Option<String> {
    let raw = u32::from_str_radix(value, 16).ok()?;
    Some(Ipv4Addr::from(raw.to_le_bytes()).to_string())
}

fn parse_proc_ipv6(value: &str) -> Option<String> {
    if value.len() != 32 {
        return None;
    }
    let mut bytes = [0_u8; 16];
    for index in 0..4 {
        let start = index * 8;
        let chunk = &value.as_bytes()[start..start + 8];
        let text = std::str::from_utf8(chunk).ok()?;
        let word = u32::from_str_radix(text, 16).ok()?;
        bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    Some(Ipv6Addr::from(bytes).to_string())
}

fn attach_process_owners(proc_root: &Path, listeners: &mut [ListenerRecord]) {
    let wanted = listeners
        .iter()
        .map(|entry| entry.inode)
        .collect::<HashSet<_>>();
    if wanted.is_empty() {
        return;
    }
    let Ok(entries) = fs::read_dir(proc_root) else {
        return;
    };
    let mut owners = HashMap::new();
    let mut inspected_fds = 0_usize;
    for entry in entries.flatten().take(MAX_PROCESSES) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        let name = fs::read_to_string(entry.path().join("comm"))
            .ok()
            .map(|value| sanitize_text(value.trim(), 80))
            .unwrap_or_else(|| "unknown".to_owned());
        for fd in fds.flatten() {
            inspected_fds = inspected_fds.saturating_add(1);
            if inspected_fds > MAX_PROCESS_FDS {
                break;
            }
            let Ok(target) = fs::read_link(fd.path()) else {
                continue;
            };
            let Some(inode) = target
                .to_str()
                .and_then(|value| value.strip_prefix("socket:["))
                .and_then(|value| value.strip_suffix(']'))
                .and_then(|value| value.parse::<u64>().ok())
            else {
                continue;
            };
            if wanted.contains(&inode) {
                owners.entry(inode).or_insert_with(|| ProcessOwner {
                    pid,
                    name: name.clone(),
                });
            }
        }
        if inspected_fds > MAX_PROCESS_FDS || owners.len() == wanted.len() {
            break;
        }
    }
    for listener in listeners {
        listener.process = owners.get(&listener.inode).cloned();
    }
}

fn detect_private_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(address) if !address.is_unspecified() && !address.is_loopback() => {
            Some(address)
        }
        _ => None,
    }
}

fn format_join_address(address: Ipv4Addr, port: u16) -> String {
    if port == 25_565 {
        address.to_string()
    } else {
        format!("{address}:{port}")
    }
}

fn router_inventory_value(router: &Result<UpnpGateway, String>) -> Value {
    match router {
        Ok(gateway) => {
            let kind = classify_external_address(gateway.external_ip);
            json!({
                "automatic_port_forwarding_available": kind == ExternalAddressKind::Public,
                "discovery": "upnp_igd",
                "state": if kind == ExternalAddressKind::Public { "available" } else { kind.as_str() },
                "external_ipv4": gateway.external_ip,
                "external_address_kind": kind.as_str(),
                "private_ipv4": gateway.local_ip,
                "error": Value::Null,
                "note": if kind == ExternalAddressKind::Public {
                    "The router supports a Helix-owned TCP mapping. This does not prove reachability from an external network."
                } else {
                    "The router answered, but its WAN address is not globally routable. Port forwarding alone cannot provide public access."
                }
            })
        }
        Err(error) => json!({
            "automatic_port_forwarding_available": false,
            "discovery": "upnp_igd",
            "state": "unavailable",
            "external_ipv4": Value::Null,
            "external_address_kind": "unknown",
            "private_ipv4": detect_private_ipv4(),
            "error": error,
            "note": "Helix did not change the router. Enable UPnP on a trusted LAN router or configure a manual TCP port forward."
        }),
    }
}

fn exposure_result(
    mapping: &GamePortMapping,
    record: &ServerExposureRecord,
    reused: bool,
    state: &str,
    firewall_state: Option<&str>,
) -> Value {
    json!({
        "instance_id": mapping.instance_id,
        "enabled": true,
        "reused": reused,
        "protocol": "tcp",
        "port": record.port,
        "private_ipv4": record.local_ip,
        "private_join_address": format_join_address(record.local_ip, record.port),
        "public_ipv4": record.external_ip,
        "public_join_address": format_join_address(record.external_ip, record.port),
        "firewall_state": firewall_state,
        "external_reachability": {
            "state": state,
            "reachable": Value::Null,
            "tested_from_external_network": false,
            "router_mapping_verified": true,
            "verified_at_unix_ms": record.verified_at_unix_ms,
            "note": "The router confirmed Helix's exact TCP mapping. Test from a separate external network before treating it as end-to-end verified."
        }
    })
}

fn unowned_router_mapping_message(port: u16, amp_claimed: bool) -> String {
    if amp_claimed {
        crate::amp::amp_port_claimed_message(port)
    } else {
        format!(
            "router TCP port {port} already has a mapping; Helix will not overwrite an unowned router rule"
        )
    }
}

fn write_exposure_record(path: &Path, record: &ServerExposureRecord) -> Result<(), String> {
    if record.schema_version != 1
        || record.port < 1_024
        || !record.instance_id.starts_with("helix:")
        || record.mapping_description.is_empty()
        || record.mapping_description.len() > 64
    {
        return Err("the public-access record is invalid".to_owned());
    }
    let body = serde_json::to_vec_pretty(record)
        .map_err(|_| "could not encode public-access state".to_owned())?;
    let temporary = path.with_extension(format!("partial.{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "could not stage public-access state".to_owned())?;
        file.write_all(&body)
            .and_then(|()| file.sync_all())
            .map_err(|_| "could not persist public-access state".to_owned())?;
        fs::rename(&temporary, path)
            .map_err(|_| "could not commit public-access state".to_owned())?;
        fs::File::open(
            path.parent()
                .ok_or_else(|| "public-access path is invalid".to_owned())?,
        )
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "could not sync public-access state".to_owned())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn read_exposure_record(path: &Path, instance_id: &str) -> Result<ServerExposureRecord, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "the Helix public-access record does not exist".to_owned())?;
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_RECORD_BYTES
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err("the Helix public-access record is invalid".to_owned());
    }
    let record: ServerExposureRecord = serde_json::from_slice(
        &fs::read(path).map_err(|_| "could not read the Helix public-access record".to_owned())?,
    )
    .map_err(|_| "the Helix public-access record is invalid".to_owned())?;
    if record.schema_version != 1
        || record.instance_id != instance_id
        || record.protocol != FirewallProtocol::Tcp
        || record.port < 1_024
        || record.mapping_description.is_empty()
        || record.mapping_description.len() > 64
    {
        return Err("the Helix public-access record does not match this server".to_owned());
    }
    Ok(record)
}

fn prepare_state_root(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| "could not create firewall state storage".to_owned())?;
    let canonical = fs::canonicalize(path)
        .map_err(|_| "could not resolve firewall state storage".to_owned())?;
    if canonical != path {
        return Err(
            "firewall state storage must not contain symlinks or path traversal".to_owned(),
        );
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "could not inspect firewall state storage".to_owned())?;
    if !metadata.file_type().is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err("firewall state storage is invalid".to_owned());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| "could not protect firewall state storage".to_owned())?;
    let protected = fs::symlink_metadata(path)
        .map_err(|_| "could not verify firewall state storage".to_owned())?;
    if protected.mode() & 0o077 != 0 {
        return Err("firewall state storage is not private".to_owned());
    }
    Ok(())
}

fn write_record(path: &Path, record: &FirewallRecord) -> Result<(), String> {
    validate_record(record, &record.rule_id)?;
    let body = serde_json::to_vec_pretty(record)
        .map_err(|_| "could not encode firewall rule state".to_owned())?;
    let temporary = path.with_extension(format!("partial.{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "could not stage firewall rule state".to_owned())?;
        file.write_all(&body)
            .and_then(|()| file.sync_all())
            .map_err(|_| "could not persist firewall rule state".to_owned())?;
        fs::rename(&temporary, path)
            .map_err(|_| "could not commit firewall rule state".to_owned())?;
        let parent = path
            .parent()
            .ok_or_else(|| "firewall rule state path is invalid".to_owned())?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "could not sync firewall rule state storage".to_owned())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn remove_record_durable(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|_| "could not remove firewall rule state".to_owned())?;
    let parent = path
        .parent()
        .ok_or_else(|| "firewall rule state path is invalid".to_owned())?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "could not sync firewall rule state storage".to_owned())
}

fn read_record(path: &Path, rule_id: &str) -> Result<FirewallRecord, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "the Helix firewall rule record does not exist".to_owned())?;
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_RECORD_BYTES
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err("the Helix firewall rule record is invalid".to_owned());
    }
    let record: FirewallRecord = serde_json::from_slice(
        &fs::read(path).map_err(|_| "could not read the Helix firewall rule record".to_owned())?,
    )
    .map_err(|_| "the Helix firewall rule record is invalid".to_owned())?;
    validate_record(&record, rule_id)?;
    Ok(record)
}

fn validate_record(record: &FirewallRecord, rule_id: &str) -> Result<(), String> {
    validate_rule_id(rule_id)?;
    normalize_rule_spec(record.spec.clone())?;
    if record.schema_version != 1
        || record.rule_id != rule_id
        || record.marker != format!("helix:{rule_id}")
        || (matches!(
            record.state,
            FirewallRecordState::CreatePending | FirewallRecordState::Active
        ) && (record.trashed_at_unix_ms.is_some() || record.undo_expires_at_unix_ms.is_some()))
        || (matches!(
            record.state,
            FirewallRecordState::Trashed
                | FirewallRecordState::DeletePending
                | FirewallRecordState::RestorePending
        ) && (record.trashed_at_unix_ms.is_none() || record.undo_expires_at_unix_ms.is_none()))
    {
        return Err("the Helix firewall rule record is invalid".to_owned());
    }
    Ok(())
}

fn validate_rule_id(rule_id: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(rule_id).map_err(|_| "firewall rule ID is invalid".to_owned())?;
    if parsed.to_string() != rule_id {
        return Err("firewall rule ID is invalid".to_owned());
    }
    Ok(())
}

fn validate_config(config: &NetworkConfig) -> Result<(), String> {
    validate_config_shape(config)?;
    if !config.proc_root.is_dir() {
        return Err("the configured proc filesystem is unavailable".to_owned());
    }
    Ok(())
}

fn validate_config_shape(config: &NetworkConfig) -> Result<(), String> {
    if [
        &config.docker_binary,
        &config.ufw_binary,
        &config.timeout_binary,
        &config.proc_root,
        &config.state_root,
    ]
    .iter()
    .any(|path| !path.is_absolute())
        || !(1..=30).contains(&config.mutation_cooldown_seconds)
        || config.state_root == Path::new("/")
    {
        return Err(
            "network paths must be absolute and the mutation cooldown must be 1 to 30 seconds"
                .to_owned(),
        );
    }
    Ok(())
}

fn require_success(output: CommandOutput) -> Result<(), String> {
    if output.success {
        Ok(())
    } else {
        Err(bounded_command_error(&output))
    }
}

fn bounded_command_error(output: &CommandOutput) -> String {
    let message = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    if message.is_empty() {
        "the command failed without details".to_owned()
    } else {
        sanitize_text(message, 500)
    }
}

fn sanitize_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == ' ')
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn default_docker_binary() -> PathBuf {
    PathBuf::from("/usr/bin/docker")
}

fn default_ufw_binary() -> PathBuf {
    PathBuf::from("/usr/sbin/ufw")
}

fn default_timeout_binary() -> PathBuf {
    PathBuf::from("/usr/bin/timeout")
}

fn default_proc_root() -> PathBuf {
    PathBuf::from("/proc")
}

fn default_state_root() -> PathBuf {
    PathBuf::from("/var/lib/helix/network")
}

fn default_mutation_cooldown_seconds() -> u8 {
    2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct MockRunner {
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
        outputs: Mutex<VecDeque<Result<CommandOutput, String>>>,
    }

    impl MockRunner {
        fn push(&self, stdout: &str) {
            self.outputs.lock().unwrap().push_back(Ok(CommandOutput {
                success: true,
                stdout: stdout.to_owned(),
                stderr: String::new(),
            }));
        }

        fn calls(&self) -> Vec<(PathBuf, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl NetworkCommandRunner for MockRunner {
        fn run(
            &self,
            program: &Path,
            args: &[String],
            _timeout: Duration,
        ) -> Result<CommandOutput, String> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_owned(), args.to_vec()));
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err("unexpected command".to_owned()))
        }
    }

    #[derive(Clone)]
    struct StatefulRule {
        marker: String,
        protocol: String,
        ports: String,
    }

    #[derive(Default)]
    struct StatefulUfwRunner {
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
        rule: Mutex<Option<StatefulRule>>,
    }

    impl NetworkCommandRunner for StatefulUfwRunner {
        fn run(
            &self,
            program: &Path,
            args: &[String],
            _timeout: Duration,
        ) -> Result<CommandOutput, String> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_owned(), args.to_vec()));
            let stdout = if args == ["status", "verbose"] {
                active_verbose().to_owned()
            } else if args == ["status", "numbered"] {
                self.rule.lock().unwrap().as_ref().map_or_else(
                    || "Status: active\n".to_owned(),
                    |rule| {
                        format!(
                            "Status: active\n\n[ 1] {}/{}  ALLOW IN  Anywhere  # {}\n",
                            rule.ports, rule.protocol, rule.marker
                        )
                    },
                )
            } else if args.first().is_some_and(|argument| argument == "allow") {
                let marker = argument_after(args, "comment")?;
                let protocol = argument_after(args, "proto")?;
                let ports = argument_after(args, "port")?;
                *self.rule.lock().unwrap() = Some(StatefulRule {
                    marker,
                    protocol,
                    ports,
                });
                "Rule added\n".to_owned()
            } else if args.starts_with(&["--force".to_owned(), "delete".to_owned()]) {
                let marker = argument_after(args, "comment")?;
                let mut rule = self.rule.lock().unwrap();
                if rule.as_ref().is_some_and(|rule| rule.marker == marker) {
                    *rule = None;
                }
                "Rule deleted\n".to_owned()
            } else {
                return Err(format!("unexpected command arguments: {args:?}"));
            };
            Ok(CommandOutput {
                success: true,
                stdout,
                stderr: String::new(),
            })
        }
    }

    fn argument_after(args: &[String], name: &str) -> Result<String, String> {
        args.iter()
            .position(|argument| argument == name)
            .and_then(|index| args.get(index.saturating_add(1)))
            .cloned()
            .ok_or_else(|| format!("missing {name} argument"))
    }

    fn active_verbose() -> &'static str {
        "Status: active\nLogging: on (low)\nDefault: deny (incoming), allow (outgoing), disabled (routed)\n"
    }

    fn config(root: PathBuf) -> NetworkConfig {
        NetworkConfig {
            state_root: root,
            mutation_cooldown_seconds: 1,
            ..NetworkConfig::default()
        }
    }

    fn spec() -> FirewallRuleSpec {
        FirewallRuleSpec {
            name: "Minecraft".to_owned(),
            description: "Survival server".to_owned(),
            protocol: FirewallProtocol::Tcp,
            port_start: 25_565,
            port_end: 25_565,
        }
    }

    #[test]
    fn proc_listener_parser_decodes_ipv4_and_filters_non_listening_tcp() {
        let line = "0: 0100007F:63DD 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 4242 1";
        let listener = parse_proc_socket_line(line, "tcp", "ipv4", "0A").unwrap();
        assert_eq!(listener.address, "127.0.0.1");
        assert_eq!(listener.port, 25_565);
        assert_eq!(listener.inode, 4242);
        assert!(parse_proc_socket_line(line, "tcp", "ipv4", "01").is_none());
    }

    #[test]
    fn ufw_parser_keeps_status_defaults_ranges_and_exact_helix_identity() {
        let parsed = parse_ufw_verbose(active_verbose()).unwrap();
        assert!(parsed.active);
        assert_eq!(parsed.incoming.as_deref(), Some("deny"));
        assert_eq!(parsed.outgoing.as_deref(), Some("allow"));
        assert_eq!(parsed.routed.as_deref(), Some("disabled"));
        let id = "8953dc16-3891-42bf-802f-711b3ba2965a";
        let rules = parse_ufw_numbered(&format!(
            "Status: active\n\n[ 1] 25565:25570/tcp          ALLOW IN    Anywhere                   # helix:{id}\n[ 2] 22/tcp (v6)              ALLOW IN    Anywhere (v6)\n"
        ));
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].port_start, Some(25_565));
        assert_eq!(rules[0].port_end, Some(25_570));
        assert_eq!(rules[0].helix_rule_id.as_deref(), Some(id));
        assert!(rules[1].helix_rule_id.is_none());
    }

    #[test]
    fn docker_parser_preserves_bind_address_and_both_ports() {
        let mut publications = Vec::new();
        parse_docker_inspect(
            r#"[{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Name":"/mc","Config":{"Labels":{"com.docker.compose.service":"minecraft"}},"NetworkSettings":{"Ports":{"25565/tcp":[{"HostIp":"192.168.1.20","HostPort":"25570"}],"19132/udp":null}}}]"#,
            &mut publications,
        )
        .unwrap();
        assert_eq!(publications.len(), 1);
        assert_eq!(publications[0].container_port, 25_565);
        assert_eq!(publications[0].host_port, 25_570);
        assert_eq!(publications[0].host_address, "192.168.1.20");
    }

    #[test]
    fn validation_rejects_wide_ranges_and_control_characters() {
        let mut invalid = spec();
        invalid.port_end = invalid.port_start + MAX_RULE_RANGE_WIDTH;
        assert!(normalize_rule_spec(invalid).is_err());
        let mut invalid = spec();
        invalid.name = "bad\nname".to_owned();
        assert!(normalize_rule_spec(invalid).is_err());
    }

    #[test]
    fn create_delete_restore_uses_only_exact_typed_ufw_rules() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        // create: before verbose + numbered, add, after verbose + numbered
        runner.push(active_verbose());
        runner.push("Status: active\n");
        runner.push("Rule added\nRule added (v6)\n");
        runner.push(active_verbose());
        let manager =
            NetworkManager::with_runner(config(temporary.path().join("state")), runner.clone())
                .unwrap();
        // The ID is generated during create, so provide the verified numbered output lazily by
        // extracting it from the add call after a first intentionally ambiguous verification.
        runner.push("Status: active\n");
        let error = manager.create_rule(spec()).unwrap_err();
        assert!(error.contains("did not report"));
        let calls = runner.calls();
        let add = calls
            .iter()
            .find(|(_, args)| args.first().is_some_and(|arg| arg == "allow"))
            .unwrap();
        assert_eq!(
            add.1[1..11],
            [
                "in", "proto", "tcp", "from", "any", "to", "any", "port", "25565", "comment"
            ]
        );
        assert!(add.1[11].starts_with("helix:"));
        assert!(calls.iter().all(|(_, args)| {
            !args
                .iter()
                .any(|arg| matches!(arg.as_str(), "enable" | "disable" | "reset" | "default"))
        }));
    }

    #[test]
    fn verified_delete_is_recoverable_and_restore_reuses_the_exact_uuid_rule() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(StatefulUfwRunner::default());
        let manager =
            NetworkManager::with_runner(config(temporary.path().join("state")), runner.clone())
                .unwrap();

        let created = manager.create_rule(spec()).unwrap();
        let rule_id = created["rule_id"].as_str().unwrap();
        let marker = format!("helix:{rule_id}");
        assert_eq!(created["verified"], true);

        let deleted = manager.delete_rule(rule_id).unwrap();
        assert_eq!(deleted["state"], "trashed");
        assert_eq!(deleted["undo_available"], true);
        assert!(deleted["undo_expires_at_unix_ms"].as_u64().is_some());
        let record = read_record(&manager.record_path(rule_id).unwrap(), rule_id).unwrap();
        assert_eq!(record.state, FirewallRecordState::Trashed);

        let restored = manager.restore_rule(rule_id).unwrap();
        assert_eq!(restored["state"], "active");
        assert_eq!(restored["verified"], true);
        let calls = runner.calls.lock().unwrap();
        assert!(calls.iter().any(|(_, args)| {
            args.starts_with(&["--force".to_owned(), "delete".to_owned()])
                && args.last() == Some(&marker)
        }));
        assert_eq!(
            calls
                .iter()
                .filter(|(_, args)| {
                    args.first().is_some_and(|argument| argument == "allow")
                        && args.last() == Some(&marker)
                })
                .count(),
            2
        );
        assert!(calls.iter().all(|(_, args)| {
            !args.iter().any(|argument| {
                matches!(
                    argument.as_str(),
                    "enable" | "disable" | "reset" | "default"
                )
            })
        }));
    }

    #[test]
    fn create_pending_journal_without_a_rule_can_be_reconciled_safely() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(StatefulUfwRunner::default());
        let manager =
            NetworkManager::with_runner(config(temporary.path().join("state")), runner.clone())
                .unwrap();
        let rule_id = Uuid::new_v4().to_string();
        let record = FirewallRecord {
            schema_version: 1,
            rule_id: rule_id.clone(),
            marker: format!("helix:{rule_id}"),
            spec: spec(),
            state: FirewallRecordState::CreatePending,
            created_at_unix_ms: now_unix_ms(),
            trashed_at_unix_ms: None,
            undo_expires_at_unix_ms: None,
        };
        let path = manager.record_path(&rule_id).unwrap();
        write_record(&path, &record).unwrap();

        let result = manager.delete_rule(&rule_id).unwrap();
        assert_eq!(result["state"], "absent");
        assert_eq!(result["reconciled_create_pending"], true);
        assert!(!path.exists());
        assert!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|(_, args)| { matches!(args.as_slice(), [verb, _] if verb == "status") })
        );
    }

    #[test]
    fn mismatched_marker_after_restore_keeps_the_pending_journal() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        let manager =
            NetworkManager::with_runner(config(temporary.path().join("state")), runner.clone())
                .unwrap();
        let rule_id = Uuid::new_v4().to_string();
        let now = now_unix_ms();
        let record = FirewallRecord {
            schema_version: 1,
            rule_id: rule_id.clone(),
            marker: format!("helix:{rule_id}"),
            spec: spec(),
            state: FirewallRecordState::Trashed,
            created_at_unix_ms: now,
            trashed_at_unix_ms: Some(now),
            undo_expires_at_unix_ms: Some(now.saturating_add(UNDO_WINDOW_MS)),
        };
        let path = manager.record_path(&rule_id).unwrap();
        write_record(&path, &record).unwrap();
        runner.push(active_verbose());
        runner.push("Status: active\n");
        runner.push("Rule added\n");
        runner.push(active_verbose());
        runner.push(&format!(
            "Status: active\n\n[ 1] 25565/udp  ALLOW IN    Anywhere  # helix:{rule_id}\n"
        ));

        let error = manager.restore_rule(&rule_id).unwrap_err();
        assert!(error.contains("remains restore_pending"));
        assert_eq!(
            read_record(&path, &rule_id).unwrap().state,
            FirewallRecordState::RestorePending
        );
    }

    #[test]
    fn inactive_ufw_never_runs_a_mutation_or_claims_support() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push("Status: inactive\n");
        let manager =
            NetworkManager::with_runner(config(temporary.path().join("state")), runner.clone())
                .unwrap();
        assert!(
            manager
                .create_rule(spec())
                .unwrap_err()
                .contains("inactive")
        );
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn router_mapping_conflict_names_amp_when_amp_already_claimed_the_port() {
        assert_eq!(
            unowned_router_mapping_message(25_566, true),
            "AMP already has port 25566 claimed"
        );
        assert_eq!(
            unowned_router_mapping_message(25_566, false),
            "router TCP port 25566 already has a mapping; Helix will not overwrite an unowned router rule"
        );
    }
}
