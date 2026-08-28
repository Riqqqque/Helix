#[cfg(target_os = "linux")]
mod amp;
#[cfg(target_os = "linux")]
mod bounded_command;
#[cfg(target_os = "linux")]
mod files;
#[cfg(target_os = "linux")]
mod hook_install;
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
mod inventory;
#[cfg(target_os = "linux")]
mod native;
#[cfg(target_os = "linux")]
mod network;
#[cfg(target_os = "linux")]
mod packages;
#[cfg(target_os = "linux")]
mod upnp;

#[cfg(target_os = "linux")]
use amp::{AmpClient, AmpInventory, AmpInventoryIssue, AmpServer};
#[cfg(target_os = "linux")]
use clap::Parser;
#[cfg(target_os = "linux")]
use files::{FileManager, MAX_CONFIGURED_ROOTS, StorageAnalysisManager};
#[cfg(target_os = "linux")]
use helix_privd::{
    BrokerClient, BrokerRequest, BrokerResponse, HookServiceAction, MinecraftCreateSpec,
    MinecraftModpackCreateSpec, PackageUpdateCandidate, ServerNetworkExposure, read_frame,
    write_frame,
};
#[cfg(target_os = "linux")]
use hook_install::{HookInstaller, HookInstallerConfig};
#[cfg(target_os = "linux")]
use host::{HostControl, HostControlConfig};
#[cfg(target_os = "linux")]
use native::{NativeConfig, NativeManager};
#[cfg(target_os = "linux")]
use network::{GamePortMapping, NetworkConfig, NetworkManager};
#[cfg(target_os = "linux")]
use packages::{PackageConfig, PackageManager};
#[cfg(target_os = "linux")]
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use serde_json::{Value, json};
#[cfg(target_os = "linux")]
use std::{
    collections::HashMap,
    error::Error,
    fs, io,
    os::unix::{fs::PermissionsExt as _, net::UnixListener},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[cfg(target_os = "linux")]
use uuid::Uuid;

#[cfg(target_os = "linux")]
const MAX_CONNECTIONS: usize = 32;
#[cfg(target_os = "linux")]
const MAX_JOBS: usize = 64;

#[cfg(target_os = "linux")]
#[derive(Debug, Parser)]
#[command(
    name = "helix-privd",
    version,
    about = "Typed privileged host broker for Helix"
)]
struct Args {
    #[arg(long, default_value = "/etc/helix/privd.json")]
    config: PathBuf,
    #[arg(long, value_name = "SCHEDULE_ID", hide = true)]
    trigger_recurring_reboot: Option<String>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerConfig {
    socket: PathBuf,
    amp_credentials: Option<PathBuf>,
    managed_roots: Vec<PathBuf>,
    #[serde(default)]
    analysis_roots: Vec<PathBuf>,
    native: Option<NativeConfig>,
    #[serde(default)]
    host_control: HostControlConfig,
    #[serde(default)]
    network: NetworkConfig,
    #[serde(default)]
    packages: PackageConfig,
    #[serde(default)]
    hook_installer: HookInstallerConfig,
}

#[cfg(target_os = "linux")]
struct BrokerContext {
    files: FileManager,
    storage: StorageAnalysisManager,
    amp: Option<Arc<AmpClient>>,
    native: Option<Arc<NativeManager>>,
    host: Option<Arc<HostControl>>,
    network: NetworkManager,
    packages: PackageManager,
    hook_installer: Arc<HookInstaller>,
    power_gate: Mutex<()>,
    jobs: Mutex<HashMap<String, JobRecord>>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Serialize)]
struct JobRecord {
    id: String,
    kind: String,
    status: JobState,
    stage: String,
    progress_percent: u8,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
    result: Option<Value>,
    error: Option<String>,
    #[serde(skip)]
    resource_key: Option<String>,
    #[serde(skip)]
    reuse_key: Option<String>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum JobState {
    Queued,
    Running,
    Complete,
    Failed,
}

#[cfg(target_os = "linux")]
impl BrokerContext {
    fn dispatch(self: &Arc<Self>, request: BrokerRequest) -> BrokerResponse {
        let result = match request {
            BrokerRequest::HostInventory {} => inventory::collect().and_then(to_value),
            BrokerRequest::NetworkInventory {} => self.network_inventory(),
            BrokerRequest::GamePortPolicy { game } => self
                .native
                .as_deref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())
                .and_then(|native| native.game_port_policy(game)),
            BrokerRequest::SetGamePortPolicy { policy } => self
                .native
                .as_deref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())
                .and_then(|native| native.set_game_port_policy(policy)),
            BrokerRequest::SetServerNetworkExposure {
                instance_id,
                enabled,
            } => self.set_server_network_exposure(&instance_id, enabled),
            BrokerRequest::CreateFirewallRule { rule } => {
                self.firewall_mutation(|network| network.create_rule(rule))
            }
            BrokerRequest::DeleteFirewallRule { rule_id } => {
                self.firewall_mutation(|network| network.delete_rule(&rule_id))
            }
            BrokerRequest::RestoreFirewallRule { rule_id } => {
                self.firewall_mutation(|network| network.restore_rule(&rule_id))
            }
            BrokerRequest::EnableFirewall {
                ssh_port,
                confirmation,
            } => self.firewall_mutation(|network| network.enable_ufw(ssh_port, &confirmation)),
            BrokerRequest::SystemPackageInventory {} => self.packages.inventory(),
            BrokerRequest::RefreshSystemPackageLists {} => self.start_package_refresh_job(),
            BrokerRequest::ApplySystemPackageUpdates {
                packages,
                confirmation,
                disruption_acknowledged,
            } => self.start_package_apply_job(packages, confirmation, disruption_acknowledged),
            BrokerRequest::HookInventory {} => self.hook_inventory(),
            BrokerRequest::HookInstallPreflight { hook_id } => {
                self.hook_installer.preflight(&hook_id)
            }
            BrokerRequest::InstallHook {
                hook_id,
                confirmation,
                repository_change_acknowledged,
            } => self.start_hook_install_job(hook_id, confirmation, repository_change_acknowledged),
            BrokerRequest::ManageHookService { hook_id, action } => {
                self.manage_hook_service(&hook_id, action)
            }
            BrokerRequest::HostIntegrationStatus {} => self.host_integration_status(),
            BrokerRequest::SetHelixStartOnBoot { enabled } => self
                .host_control()
                .and_then(|host| host.set_start_on_boot(enabled)),
            BrokerRequest::HostRebootPreflight {} => self.host_reboot_preflight(),
            BrokerRequest::ScheduleHostReboot {
                confirmation_hostname,
                delay_seconds,
                disruption_acknowledged,
            } => self.schedule_host_reboot(
                &confirmation_hostname,
                delay_seconds,
                disruption_acknowledged,
            ),
            BrokerRequest::CancelHostReboot { operation_id } => {
                self.cancel_host_reboot(&operation_id)
            }
            BrokerRequest::SetRecurringHostReboot { schedule } => self
                .host_control()
                .and_then(|host| host.set_recurring_reboot(schedule)),
            BrokerRequest::DeleteRecurringHostReboot {
                confirmation_hostname,
            } => self
                .host_control()
                .and_then(|host| host.delete_recurring_reboot(&confirmation_hostname)),
            BrokerRequest::ExecuteRecurringHostReboot { schedule_id } => {
                self.execute_recurring_host_reboot(&schedule_id)
            }
            BrokerRequest::ListDirectory {
                path,
                cursor,
                limit,
            } => self
                .files
                .list(&path, cursor.as_deref(), limit)
                .and_then(to_value),
            BrokerRequest::CreateDirectory { parent, name } => self
                .files
                .create_directory(&parent, &name)
                .and_then(to_value),
            BrokerRequest::CreateFile { parent, name } => {
                self.files.create_file(&parent, &name).and_then(to_value)
            }
            BrokerRequest::ReadText { path } => self.files.read_text(&path).and_then(to_value),
            BrokerRequest::WriteText {
                path,
                content,
                expected_modified_unix_ms,
            } => self
                .files
                .write_text(&path, &content, expected_modified_unix_ms)
                .and_then(to_value),
            BrokerRequest::Rename { path, new_name } => {
                self.files.rename(&path, &new_name).and_then(to_value)
            }
            BrokerRequest::Trash { path } => self.files.trash(&path).and_then(to_value),
            BrokerRequest::StartStorageAnalysis { path, mode } => {
                self.storage.start_with_mode(&path, mode).and_then(to_value)
            }
            BrokerRequest::StorageAnalysisStatus { job_id } => {
                self.storage.status(&job_id).and_then(to_value)
            }
            BrokerRequest::CancelStorageAnalysis { job_id } => {
                self.storage.cancel(&job_id).and_then(to_value)
            }
            BrokerRequest::ListServers {} => self.list_servers(),
            BrokerRequest::ListTrashedServers {} => self
                .native
                .as_deref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())
                .and_then(NativeManager::list_trashed_servers),
            BrokerRequest::TrashNativeServer {
                instance_id,
                confirmation_name,
            } => self
                .native_manager(&instance_id)
                .and_then(|native| native.trash_server(&instance_id, &confirmation_name)),
            BrokerRequest::RestoreTrashedServer { trash_id } => self
                .native
                .as_deref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())
                .and_then(|native| native.restore_trashed_server(&trash_id)),
            BrokerRequest::ServerInventoryHealth {} => self.server_inventory_health(),
            BrokerRequest::ServerManagerReadiness {} => self.server_manager_readiness(),
            BrokerRequest::ServerDetail { instance_id } => self
                .native_manager(&instance_id)
                .and_then(|native| native.server_detail(&instance_id)),
            BrokerRequest::ServerLogs { instance_id, lines } => self
                .native_manager(&instance_id)
                .and_then(|native| native.server_logs(&instance_id, lines)),
            BrokerRequest::ServerLogHistory {
                instance_id,
                cursor,
                lines,
            } => self.native_manager(&instance_id).and_then(|native| {
                native.server_log_history(&instance_id, cursor.as_deref(), lines)
            }),
            BrokerRequest::ServerConsole {
                instance_id,
                command,
            } => self
                .native_manager(&instance_id)
                .and_then(|native| native.server_console(&instance_id, &command)),
            BrokerRequest::ServerSettings { instance_id } => self
                .native_manager(&instance_id)
                .and_then(|native| native.server_settings(&instance_id)),
            BrokerRequest::ServerMarketplaceSearch {
                instance_id,
                query,
                offset,
                limit,
            } => self
                .native_manager(&instance_id)
                .and_then(|native| native.marketplace_search(&instance_id, &query, offset, limit)),
            BrokerRequest::ServerMarketplaceProject {
                instance_id,
                project_id,
            } => self
                .native_manager(&instance_id)
                .and_then(|native| native.marketplace_project(&instance_id, &project_id)),
            BrokerRequest::MinecraftModpackSearch {
                query,
                offset,
                limit,
            } => self
                .native
                .as_deref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())
                .and_then(|native| native.minecraft_modpack_search(&query, offset, limit)),
            BrokerRequest::MinecraftModpackProject { project_id } => self
                .native
                .as_deref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())
                .and_then(|native| native.minecraft_modpack_project(&project_id)),
            BrokerRequest::InstallServerMarketplaceContent {
                instance_id,
                project_id,
                version_id,
            } => self.start_marketplace_install_job(instance_id, project_id, version_id),
            BrokerRequest::UpdateServerSettings {
                instance_id,
                settings,
            } => self
                .native_manager(&instance_id)
                .and_then(|native| native.update_server_settings(&instance_id, &settings)),
            BrokerRequest::ListBackups { instance_id } => self
                .native_manager(&instance_id)
                .and_then(|native| native.list_backups(&instance_id)),
            BrokerRequest::RestoreBackup {
                instance_id,
                backup_id,
            } => self.start_restore_job(instance_id, backup_id),
            BrokerRequest::TrashBackup {
                instance_id,
                backup_id,
            } => self
                .native_manager(&instance_id)
                .and_then(|native| native.trash_backup(&instance_id, &backup_id)),
            BrokerRequest::RestoreTrashedBackup {
                instance_id,
                trash_id,
            } => self
                .native_manager(&instance_id)
                .and_then(|native| native.restore_trashed_backup(&instance_id, &trash_id)),
            BrokerRequest::ServerAction {
                instance_id,
                action,
            } => self.server_action(&instance_id, action),
            BrokerRequest::CreateMinecraft { spec } => self.start_minecraft_job(spec),
            BrokerRequest::CreateMinecraftModpack { spec } => {
                self.start_minecraft_modpack_job(spec)
            }
            BrokerRequest::JobStatus { job_id } => self.job_status(&job_id),
        };

        match result {
            Ok(value) => BrokerResponse::success(value),
            Err(message) => BrokerResponse::failure(problem_code(&message), message),
        }
    }

    fn list_servers(&self) -> Result<Value, String> {
        let mut servers = if let Some(native) = &self.native {
            native.list_servers()?
        } else {
            Vec::new()
        };
        if let Some(amp) = &self.amp {
            match amp.list_servers() {
                Ok(imported) => servers.extend(imported.servers),
                Err(error) => eprintln!("AMP compatibility inventory unavailable: {error}"),
            }
        }
        to_value(servers)
    }

    fn server_inventory_health(&self) -> Result<Value, String> {
        let amp = match &self.amp {
            Some(amp) => amp_inventory_health(Some(amp.list_servers())),
            None => amp_inventory_health(None),
        };
        Ok(json!({
            "schema_version": 1,
            "managers": [amp],
            "checked_at_unix_ms": now_unix_ms()
        }))
    }

    fn network_inventory(&self) -> Result<Value, String> {
        let mut mappings = Vec::new();
        let mut errors = Vec::new();
        if let Some(native) = &self.native {
            match native.list_servers() {
                Ok(servers) => append_game_port_mappings(&mut mappings, servers),
                Err(error) => errors.push(json!({
                    "manager": "helix_native",
                    "message": error
                })),
            }
        }
        if let Some(amp) = &self.amp {
            match amp.list_servers() {
                Ok(inventory) => {
                    if inventory.issue_count > 0 {
                        errors.push(json!({
                            "manager": "amp",
                            "code": "amp_inventory_degraded",
                            "message": "One or more AMP instance records could not be verified.",
                            "unverified_instance_count": inventory.issue_count
                        }));
                    }
                    append_game_port_mappings(&mut mappings, inventory.servers);
                }
                Err(error) => errors.push(json!({
                    "manager": "amp",
                    "code": "amp_inventory_unavailable",
                    "message": error
                })),
            }
        }
        let mut inventory = self.network.inventory(&mappings)?;
        inventory
            .as_object_mut()
            .ok_or_else(|| "network inventory response was invalid".to_owned())?
            .insert(
                "game_port_inventory_errors".to_owned(),
                Value::Array(errors),
            );
        Ok(inventory)
    }

    fn set_server_network_exposure(
        &self,
        instance_id: &str,
        enabled: bool,
    ) -> Result<Value, String> {
        let native = self
            .native
            .as_deref()
            .ok_or_else(|| "the Helix server manager is not configured".to_owned())?;
        let server = native
            .list_servers()?
            .into_iter()
            .find(|server| server.id == instance_id)
            .ok_or_else(|| "the Helix-owned server does not exist".to_owned())?;
        let port = server
            .game_port
            .ok_or_else(|| "the server does not report a game port".to_owned())?;
        self.network.set_server_exposure(
            &GamePortMapping {
                instance_id: server.id,
                name: server.name,
                manager: server.manager.to_owned(),
                port,
                running: server.panel_running,
            },
            enabled,
        )
    }

    fn apply_creation_exposure(
        &self,
        mut value: Value,
        server_name: &str,
        requested: ServerNetworkExposure,
    ) -> Value {
        let Some(instance_id) = value
            .get("instance_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return value;
        };
        let Some(port) = value
            .get("game_port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
        else {
            return value;
        };
        let network = if requested == ServerNetworkExposure::Public {
            self.network
                .set_server_exposure(
                    &GamePortMapping {
                        instance_id,
                        name: server_name.to_owned(),
                        manager: "helix".to_owned(),
                        port,
                        running: true,
                    },
                    true,
                )
                .unwrap_or_else(|error| {
                    json!({
                        "enabled": false,
                        "state": "needs_attention",
                        "server_created": true,
                        "error": error,
                        "note": "The server is online, but automatic public access could not be confirmed. Retry from the server's Join section."
                    })
                })
        } else {
            json!({
                "enabled": false,
                "state": "private",
                "note": "The server was created for LAN or private-network access."
            })
        };
        if let Some(object) = value.as_object_mut() {
            object.insert("network_exposure".to_owned(), network);
        }
        value
    }

    fn hook_inventory(&self) -> Result<Value, String> {
        let mut host_inventory = self.host_control()?.hook_inventory()?;
        let hooks = host_inventory
            .get_mut("hooks")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "host hook inventory was invalid".to_owned())?;
        let amp = match &self.amp {
            Some(amp) => match amp.list_servers() {
                Ok(inventory) => json!({
                    "id": "amp",
                    "kind": "api",
                    "installed": true,
                    "active": true,
                    "active_state": if inventory.issue_count == 0 { "connected" } else { "degraded" },
                    "enabled": true,
                    "enabled_state": "configured",
                    "controllable": false,
                    "actions": [],
                    "instance_count": inventory.servers.len(),
                    "unverified_instance_count": inventory.issue_count,
                    "panel_port": amp.public_panel_port(),
                    "error": Value::Null
                }),
                Err(error) => json!({
                    "id": "amp",
                    "kind": "api",
                    "installed": true,
                    "active": false,
                    "active_state": "unavailable",
                    "enabled": true,
                    "enabled_state": "configured",
                    "controllable": false,
                    "actions": [],
                    "panel_port": amp.public_panel_port(),
                    "error": error
                }),
            },
            None => json!({
                "id": "amp",
                "kind": "api",
                "installed": false,
                "active": false,
                "active_state": "not_configured",
                "enabled": false,
                "enabled_state": "not_configured",
                "controllable": false,
                "actions": [],
                "error": Value::Null
            }),
        };
        hooks.push(amp);
        Ok(host_inventory)
    }

    fn manage_hook_service(
        &self,
        hook_id: &str,
        action: HookServiceAction,
    ) -> Result<Value, String> {
        let _power_gate = self
            .power_gate
            .lock()
            .map_err(|_| "host power coordination failed".to_owned())?;
        if let Some(host) = self.host.as_deref()
            && host.reboot_pending()?
        {
            return Err(
                "a host reboot is scheduled; hook changes are temporarily unavailable".to_owned(),
            );
        }
        self.host_control()?.manage_hook_service(hook_id, action)
    }

    fn firewall_mutation(
        &self,
        operation: impl FnOnce(&NetworkManager) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let _power_gate = self
            .power_gate
            .lock()
            .map_err(|_| "host power coordination failed".to_owned())?;
        if let Some(host) = self.host.as_deref()
            && host.reboot_pending()?
        {
            return Err(
                "a host reboot is scheduled; firewall changes are temporarily unavailable"
                    .to_owned(),
            );
        }
        operation(&self.network)
    }

    fn host_integration_status(&self) -> Result<Value, String> {
        let mut status = self.host_control()?.status()?;
        let preflight = self.host_reboot_preflight()?;
        status
            .as_object_mut()
            .ok_or_else(|| "host integration status was invalid".to_owned())?
            .insert("reboot_preflight".to_owned(), preflight);
        Ok(status)
    }

    fn host_reboot_preflight(&self) -> Result<Value, String> {
        let mut blockers = Vec::new();
        let mut active_servers = Vec::new();
        let mut active_server_count = 0_u64;
        let mut active_players = 0_u64;
        let mut unverified_server_count = 0_u64;
        let mut unverified_servers = Vec::new();

        if let Some(native) = &self.native {
            match native.list_servers() {
                Ok(servers) => collect_active_players(
                    servers,
                    "helix_native",
                    &mut active_players,
                    &mut active_server_count,
                    &mut active_servers,
                    &mut unverified_server_count,
                    &mut unverified_servers,
                ),
                Err(_) => blockers.push(json!({
                    "code": "native_player_status_unavailable",
                    "message": "Helix could not verify native Minecraft player activity."
                })),
            }
        }
        if let Some(amp) = &self.amp {
            match amp.list_servers() {
                Ok(inventory) => {
                    collect_amp_inventory_issues(
                        inventory.issue_count,
                        inventory.issues,
                        &mut unverified_server_count,
                        &mut unverified_servers,
                    );
                    collect_active_players(
                        inventory.servers,
                        "amp_import",
                        &mut active_players,
                        &mut active_server_count,
                        &mut active_servers,
                        &mut unverified_server_count,
                        &mut unverified_servers,
                    );
                }
                Err(_) => blockers.push(json!({
                    "code": "amp_player_status_unavailable",
                    "message": "Helix could not verify AMP player activity."
                })),
            }
        }

        let jobs = self
            .jobs
            .lock()
            .map_err(|_| "job registry failed".to_owned())?;
        let mut active_jobs_total = 0_u64;
        let mut active_jobs = Vec::new();
        for job in jobs
            .values()
            .filter(|job| matches!(job.status, JobState::Queued | JobState::Running))
        {
            active_jobs_total = active_jobs_total.saturating_add(1);
            if active_jobs.len() < 32 {
                active_jobs.push(json!({
                    "job_id": job.id,
                    "kind": job.kind,
                    "status": job.status,
                    "stage": job.stage
                }));
            }
        }
        drop(jobs);
        append_player_status_blockers(&mut blockers, active_players, unverified_server_count);
        if active_jobs_total > 0 {
            blockers.push(json!({
                "code": "jobs_running",
                "message": "One or more Helix server jobs are still running."
            }));
        }

        Ok(json!({
            "schema_version": 1,
            "can_schedule": blockers.is_empty(),
            "active_players": active_players,
            "active_server_count": active_server_count,
            "active_servers": active_servers,
            "active_servers_truncated": active_server_count > 32,
            "unverified_server_count": unverified_server_count,
            "unverified_servers": unverified_servers,
            "unverified_servers_truncated": unverified_server_count > 32,
            "active_jobs_total": active_jobs_total,
            "active_jobs": active_jobs,
            "blockers": blockers,
            "checked_at_unix_ms": now_unix_ms()
        }))
    }

    fn schedule_host_reboot(
        &self,
        confirmation_hostname: &str,
        delay_seconds: u16,
        disruption_acknowledged: bool,
    ) -> Result<Value, String> {
        let _power_gate = self
            .power_gate
            .lock()
            .map_err(|_| "host power coordination failed".to_owned())?;
        let preflight = self.host_reboot_preflight()?;
        if preflight["can_schedule"] != true {
            return Err("host reboot preflight is blocked by active players, running jobs, or unavailable player status".to_owned());
        }
        let mut scheduled = self.host_control()?.schedule_reboot(
            confirmation_hostname,
            delay_seconds,
            disruption_acknowledged,
        )?;
        scheduled
            .as_object_mut()
            .ok_or_else(|| "host reboot result was invalid".to_owned())?
            .insert("preflight".to_owned(), preflight);
        Ok(scheduled)
    }

    fn cancel_host_reboot(&self, operation_id: &str) -> Result<Value, String> {
        let _power_gate = self
            .power_gate
            .lock()
            .map_err(|_| "host power coordination failed".to_owned())?;
        self.host_control()?.cancel_reboot(operation_id)
    }

    fn execute_recurring_host_reboot(&self, schedule_id: &str) -> Result<Value, String> {
        let _power_gate = self
            .power_gate
            .lock()
            .map_err(|_| "host power coordination failed".to_owned())?;
        let hostname = self.host_control()?.verify_recurring_trigger(schedule_id)?;
        let preflight = self.host_reboot_preflight()?;
        if preflight["can_schedule"] != true {
            return Err(
                "the recurring reboot was skipped because active players, running jobs, or unavailable player status blocked the live safety check"
                    .to_owned(),
            );
        }
        let mut scheduled = self.host_control()?.schedule_reboot(&hostname, 30, true)?;
        let result = scheduled
            .as_object_mut()
            .ok_or_else(|| "host reboot result was invalid".to_owned())?;
        result.insert("preflight".to_owned(), preflight);
        result.insert("recurring_trigger".to_owned(), Value::Bool(true));
        result.insert(
            "schedule_id".to_owned(),
            Value::String(schedule_id.to_owned()),
        );
        Ok(scheduled)
    }

    fn host_control(&self) -> Result<&HostControl, String> {
        self.host
            .as_deref()
            .ok_or_else(|| "host control is not configured".to_owned())
    }

    fn server_manager_readiness(&self) -> Result<Value, String> {
        match &self.native {
            Some(native) => native.readiness(),
            None => Ok(json!({
                "schema_version": 1,
                "availability": "unavailable",
                "manager": "helix",
                "blockers": ["native_manager_not_configured"],
                "collected_at_unix_ms": now_unix_ms()
            })),
        }
    }

    fn native_manager(&self, instance_id: &str) -> Result<&NativeManager, String> {
        if !instance_id.starts_with("helix:") {
            return Err("deep management is available only for Helix-owned servers; this server remains separate in AMP".to_owned());
        }
        self.native
            .as_deref()
            .ok_or_else(|| "the Helix server manager is not configured".to_owned())
    }

    fn server_action(
        self: &Arc<Self>,
        instance_id: &str,
        action: helix_privd::ServerAction,
    ) -> Result<Value, String> {
        if instance_id.starts_with("helix:") {
            return self.start_server_action_job(instance_id.to_owned(), action);
        }
        if instance_id.starts_with("amp:") {
            let _power_gate = self
                .power_gate
                .lock()
                .map_err(|_| "host power coordination failed".to_owned())?;
            if let Some(host) = self.host.as_deref()
                && host.reboot_pending()?
            {
                return Err(
                    "a host reboot is scheduled; AMP server actions are temporarily unavailable"
                        .to_owned(),
                );
            }
            return self
                .amp
                .as_ref()
                .ok_or_else(|| "the AMP compatibility adapter is not configured".to_owned())?
                .server_action(instance_id, action);
        }
        Err("the server manager identifier is invalid".to_owned())
    }

    fn start_hook_install_job(
        self: &Arc<Self>,
        hook_id: String,
        confirmation: String,
        repository_change_acknowledged: bool,
    ) -> Result<Value, String> {
        let plan = self.hook_installer.preflight(&hook_id)?;
        if plan["install_available"] != true {
            return Err("this hook is not ready for a one-click install on this host".to_owned());
        }
        let reuse_key = format!("hook:install:{hook_id}");
        let (job_id, reused) =
            self.queue_job("hook_install", Some("system:packages"), Some(&reuse_key))?;
        if reused {
            return Ok(json!({"job_id": job_id, "reused": true}));
        }
        let context = Arc::clone(self);
        let worker_job_id = job_id.clone();
        let spawn = thread::Builder::new()
            .name(format!("hook-install-{}", &job_id[..8]))
            .spawn(move || {
                context.update_job(&worker_job_id, |job| {
                    job.status = JobState::Running;
                    job.stage = "Installing from the verified official repository".to_owned();
                    job.progress_percent = 12;
                });
                let result = context.hook_installer.install(
                    &hook_id,
                    &confirmation,
                    repository_change_acknowledged,
                    &|stage, percent| {
                        context.update_job(&worker_job_id, |job| {
                            job.stage = stage.to_owned();
                            job.progress_percent = percent;
                        });
                    },
                );
                context.finish_job(
                    &worker_job_id,
                    result,
                    "Hook installed and service verified",
                );
            });
        if let Err(error) = spawn {
            self.update_job(&job_id, |job| {
                job.status = JobState::Failed;
                job.stage = "Failed".to_owned();
                job.error = Some(format!(
                    "could not start the hook installer worker: {error}"
                ));
            });
            return Err("could not start the hook installer worker".to_owned());
        }
        Ok(json!({"job_id": job_id, "reused": false}))
    }

    fn start_package_refresh_job(self: &Arc<Self>) -> Result<Value, String> {
        let (job_id, reused) = self.queue_job(
            "system_package_lists_refresh",
            Some("system:packages"),
            Some("refresh"),
        )?;
        if reused {
            return Ok(json!({"job_id": job_id, "reused": true}));
        }
        let context = Arc::clone(self);
        let worker_job_id = job_id.clone();
        if thread::Builder::new()
            .name(format!("package-refresh-{}", &job_id[..8]))
            .spawn(move || {
                context.update_job(&worker_job_id, |job| {
                    job.status = JobState::Running;
                    job.stage = "Refreshing signed APT package lists".to_owned();
                    job.progress_percent = 10;
                });
                let result = context.packages.refresh_lists();
                context.finish_job(&worker_job_id, result, "Package lists refreshed");
            })
            .is_err()
        {
            self.finish_job(
                &job_id,
                Err("could not start the package-list refresh job".to_owned()),
                "",
            );
            return Err("could not start the package-list refresh job".to_owned());
        }
        Ok(json!({"job_id": job_id, "reused": false}))
    }

    fn start_package_apply_job(
        self: &Arc<Self>,
        packages: Vec<PackageUpdateCandidate>,
        confirmation: String,
        disruption_acknowledged: bool,
    ) -> Result<Value, String> {
        let (job_id, _) = self.queue_job("system_package_apply", Some("system:packages"), None)?;
        let context = Arc::clone(self);
        let worker_job_id = job_id.clone();
        if thread::Builder::new()
            .name(format!("package-apply-{}", &job_id[..8]))
            .spawn(move || {
                context.update_job(&worker_job_id, |job| {
                    job.status = JobState::Running;
                    job.stage = "Revalidating exact package candidates and disk space".to_owned();
                    job.progress_percent = 5;
                });
                let result = context.packages.apply_updates(
                    &packages,
                    &confirmation,
                    disruption_acknowledged,
                );
                context.finish_job(&worker_job_id, result, "Selected updates verified");
            })
            .is_err()
        {
            self.finish_job(
                &job_id,
                Err("could not start the package update job".to_owned()),
                "",
            );
            return Err("could not start the package update job".to_owned());
        }
        Ok(json!({"job_id": job_id, "reused": false}))
    }

    fn start_server_action_job(
        self: &Arc<Self>,
        instance_id: String,
        action: helix_privd::ServerAction,
    ) -> Result<Value, String> {
        let native = Arc::clone(
            self.native
                .as_ref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())?,
        );
        let action_name = match action {
            helix_privd::ServerAction::Start => "start",
            helix_privd::ServerAction::Stop => "stop",
            helix_privd::ServerAction::Restart => "restart",
            helix_privd::ServerAction::Update => "update",
            helix_privd::ServerAction::Backup => "backup",
        };
        let resource_key = format!("server:{instance_id}");
        let reuse_key = format!("action:{action_name}");
        let (job_id, reused) = self.queue_job(
            &format!("server_{action_name}"),
            Some(&resource_key),
            Some(&reuse_key),
        )?;
        if reused {
            return Ok(json!({"job_id": job_id, "reused": true}));
        }
        let context = Arc::clone(self);
        let worker_job_id = job_id.clone();
        if thread::Builder::new()
            .name(format!("server-job-{}", &job_id[..8]))
            .spawn(move || {
                context.update_job(&worker_job_id, |job| {
                    job.status = JobState::Running;
                    job.stage = match action {
                        helix_privd::ServerAction::Start => "Starting Minecraft",
                        helix_privd::ServerAction::Stop => "Stopping Minecraft cleanly",
                        helix_privd::ServerAction::Restart => "Restarting and checking Minecraft",
                        helix_privd::ServerAction::Update => {
                            "Backing up and checking for an update"
                        }
                        helix_privd::ServerAction::Backup => "Creating a consistent backup",
                    }
                    .to_owned();
                    job.progress_percent = 10;
                });
                let result = native.server_action(&instance_id, action);
                let completion_stage = server_action_completion_stage(action, &result);
                context.finish_job(&worker_job_id, result, completion_stage);
            })
            .is_err()
        {
            self.finish_job(
                &job_id,
                Err("could not start the server action job".to_owned()),
                "",
            );
            return Err("could not start the server action job".to_owned());
        }
        Ok(json!({"job_id": job_id, "reused": false}))
    }

    fn start_restore_job(
        self: &Arc<Self>,
        instance_id: String,
        backup_id: String,
    ) -> Result<Value, String> {
        let native = Arc::clone(
            self.native
                .as_ref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())?,
        );
        if !instance_id.starts_with("helix:") {
            return Err("backups can be restored only to Helix-owned servers".to_owned());
        }
        let resource_key = format!("server:{instance_id}");
        let reuse_key = format!("restore:{backup_id}");
        let (job_id, reused) =
            self.queue_job("server_restore", Some(&resource_key), Some(&reuse_key))?;
        if reused {
            return Ok(json!({"job_id": job_id, "reused": true}));
        }
        let context = Arc::clone(self);
        let worker_job_id = job_id.clone();
        if thread::Builder::new()
            .name(format!("restore-job-{}", &job_id[..8]))
            .spawn(move || {
                context.update_job(&worker_job_id, |job| {
                    job.status = JobState::Running;
                    job.stage = "Creating a safety backup and restoring files".to_owned();
                    job.progress_percent = 10;
                });
                let result = native.restore_backup(&instance_id, &backup_id);
                context.finish_job(&worker_job_id, result, "Restore complete");
            })
            .is_err()
        {
            self.finish_job(
                &job_id,
                Err("could not start the restore job".to_owned()),
                "",
            );
            return Err("could not start the restore job".to_owned());
        }
        Ok(json!({"job_id": job_id, "reused": false}))
    }

    fn start_minecraft_job(self: &Arc<Self>, spec: MinecraftCreateSpec) -> Result<Value, String> {
        let native = Arc::clone(
            self.native
                .as_ref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())?,
        );
        let (job_id, _) = self.queue_job("minecraft_create", Some("minecraft:create"), None)?;

        let context = Arc::clone(self);
        let worker_job_id = job_id.clone();
        if thread::Builder::new()
            .name(format!("minecraft-job-{}", &job_id[..8]))
            .spawn(move || {
                context.update_job(&worker_job_id, |job| {
                    job.status = JobState::Running;
                    job.stage = "Preparing".to_owned();
                    job.progress_percent = 2;
                });
                let result = native
                    .create_minecraft(&spec, |stage, progress| {
                        context.update_job(&worker_job_id, |job| {
                            job.stage = stage.to_owned();
                            job.progress_percent = progress;
                        });
                    })
                    .map(|value| {
                        context.apply_creation_exposure(value, &spec.name, spec.network_exposure)
                    });
                context.update_job(&worker_job_id, |job| match result {
                    Ok(value) => {
                        job.status = JobState::Complete;
                        job.stage = "Online".to_owned();
                        job.progress_percent = 100;
                        job.result = Some(value);
                    }
                    Err(message) => {
                        job.status = JobState::Failed;
                        job.stage = "Failed".to_owned();
                        job.error = Some(message);
                    }
                });
            })
            .is_err()
        {
            self.finish_job(
                &job_id,
                Err("could not start the Minecraft installation job".to_owned()),
                "",
            );
            return Err("could not start the Minecraft installation job".to_owned());
        }

        Ok(json!({"job_id": job_id, "reused": false}))
    }

    fn start_minecraft_modpack_job(
        self: &Arc<Self>,
        spec: MinecraftModpackCreateSpec,
    ) -> Result<Value, String> {
        let native = Arc::clone(
            self.native
                .as_ref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())?,
        );
        let (job_id, _) =
            self.queue_job("minecraft_modpack_create", Some("minecraft:create"), None)?;
        let context = Arc::clone(self);
        let worker_job_id = job_id.clone();
        if thread::Builder::new()
            .name(format!("modpack-job-{}", &job_id[..8]))
            .spawn(move || {
                context.update_job(&worker_job_id, |job| {
                    job.status = JobState::Running;
                    job.stage = "Resolving the selected Modrinth release".to_owned();
                    job.progress_percent = 2;
                });
                let result = native
                    .create_minecraft_modpack(&spec, |stage, progress| {
                        context.update_job(&worker_job_id, |job| {
                            job.stage = stage.to_owned();
                            job.progress_percent = progress;
                        });
                    })
                    .map(|value| {
                        context.apply_creation_exposure(value, &spec.name, spec.network_exposure)
                    });
                context.update_job(&worker_job_id, |job| match result {
                    Ok(value) => {
                        job.status = JobState::Complete;
                        job.stage = "Online".to_owned();
                        job.progress_percent = 100;
                        job.result = Some(value);
                    }
                    Err(message) => {
                        job.status = JobState::Failed;
                        job.stage = "Failed and rolled back".to_owned();
                        job.error = Some(message);
                    }
                });
            })
            .is_err()
        {
            self.finish_job(
                &job_id,
                Err("could not start the Modrinth modpack installation job".to_owned()),
                "",
            );
            return Err("could not start the Modrinth modpack installation job".to_owned());
        }
        Ok(json!({"job_id": job_id, "reused": false}))
    }

    fn start_marketplace_install_job(
        self: &Arc<Self>,
        instance_id: String,
        project_id: String,
        version_id: Option<String>,
    ) -> Result<Value, String> {
        let native = Arc::clone(
            self.native
                .as_ref()
                .ok_or_else(|| "the Helix server manager is not configured".to_owned())?,
        );
        if !instance_id.starts_with("helix:") {
            return Err(
                "marketplace content can be installed only on Helix-owned servers".to_owned(),
            );
        }
        let resource_key = format!("server:{instance_id}");
        let reuse_key = format!(
            "marketplace:{project_id}:{}",
            version_id.as_deref().unwrap_or("latest-release")
        );
        let (job_id, reused) = self.queue_job(
            "server_marketplace_install",
            Some(&resource_key),
            Some(&reuse_key),
        )?;
        if reused {
            return Ok(json!({"job_id": job_id, "reused": true}));
        }
        let context = Arc::clone(self);
        let worker_job_id = job_id.clone();
        if thread::Builder::new()
            .name(format!("content-job-{}", &job_id[..8]))
            .spawn(move || {
                context.update_job(&worker_job_id, |job| {
                    job.status = JobState::Running;
                    job.stage = "Resolving compatible content and dependencies".to_owned();
                    job.progress_percent = 5;
                });
                let result = native.install_marketplace_content(
                    &instance_id,
                    &project_id,
                    version_id.as_deref(),
                );
                context.finish_job(&worker_job_id, result, "Content installed");
            })
            .is_err()
        {
            self.finish_job(
                &job_id,
                Err("could not start the marketplace install job".to_owned()),
                "",
            );
            return Err("could not start the marketplace install job".to_owned());
        }
        Ok(json!({"job_id": job_id, "reused": false}))
    }

    fn queue_job(
        &self,
        kind: &str,
        resource_key: Option<&str>,
        reuse_key: Option<&str>,
    ) -> Result<(String, bool), String> {
        let _power_gate = self
            .power_gate
            .lock()
            .map_err(|_| "host power coordination failed".to_owned())?;
        if let Some(host) = self.host.as_deref()
            && host.reboot_pending()?
        {
            return Err(
                "a host reboot is scheduled; new server jobs are temporarily unavailable"
                    .to_owned(),
            );
        }
        let job_id = Uuid::new_v4().to_string();
        let now = now_unix_ms();
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "job registry failed".to_owned())?;
        if let Some(resource_key) = resource_key
            && let Some(active) = jobs.values().find(|job| {
                job.resource_key.as_deref() == Some(resource_key)
                    && matches!(job.status, JobState::Queued | JobState::Running)
            })
        {
            if reuse_key.is_some() && active.reuse_key.as_deref() == reuse_key {
                return Ok((active.id.clone(), true));
            }
            return Err("another operation is already in progress for this resource".to_owned());
        }
        if jobs.len() >= MAX_JOBS {
            let oldest_terminal = jobs
                .values()
                .filter(|job| matches!(job.status, JobState::Complete | JobState::Failed))
                .min_by_key(|job| job.updated_at_unix_ms)
                .map(|job| job.id.clone());
            if let Some(id) = oldest_terminal {
                jobs.remove(&id);
            } else {
                return Err("too many background jobs are active".to_owned());
            }
        }
        jobs.insert(
            job_id.clone(),
            JobRecord {
                id: job_id.clone(),
                kind: kind.to_owned(),
                status: JobState::Queued,
                stage: "Queued".to_owned(),
                progress_percent: 0,
                created_at_unix_ms: now,
                updated_at_unix_ms: now,
                result: None,
                error: None,
                resource_key: resource_key.map(str::to_owned),
                reuse_key: reuse_key.map(str::to_owned),
            },
        );
        Ok((job_id, false))
    }

    fn finish_job(&self, job_id: &str, result: Result<Value, String>, success_stage: &str) {
        self.update_job(job_id, |job| match result {
            Ok(value) => {
                job.status = JobState::Complete;
                job.stage = success_stage.to_owned();
                job.progress_percent = 100;
                job.result = Some(value);
            }
            Err(message) => {
                job.status = JobState::Failed;
                job.stage = "Failed".to_owned();
                job.error = Some(message);
            }
        });
    }

    fn job_status(&self, job_id: &str) -> Result<Value, String> {
        if Uuid::parse_str(job_id).is_err() {
            return Err("job ID is invalid".to_owned());
        }
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| "job registry failed".to_owned())?;
        let job = jobs
            .get(job_id)
            .ok_or_else(|| "the selected job does not exist".to_owned())?;
        to_value(job)
    }

    fn update_job(&self, job_id: &str, update: impl FnOnce(&mut JobRecord)) {
        if let Ok(mut jobs) = self.jobs.lock()
            && let Some(job) = jobs.get_mut(job_id)
        {
            update(job);
            job.updated_at_unix_ms = now_unix_ms();
        }
    }
}

#[cfg(target_os = "linux")]
fn server_action_completion_stage(
    action: helix_privd::ServerAction,
    result: &Result<Value, String>,
) -> &'static str {
    if matches!(action, helix_privd::ServerAction::Update) {
        if result
            .as_ref()
            .is_ok_and(|value| value["detail"]["already_current"] == true)
        {
            return "Already current";
        }
        if result
            .as_ref()
            .is_ok_and(|value| value["detail"]["runtime_validation_performed"] == true)
        {
            return "Update installed and verified";
        }
        return "Update staged; first start not verified";
    }
    match action {
        helix_privd::ServerAction::Start | helix_privd::ServerAction::Restart => "Online",
        helix_privd::ServerAction::Stop => "Stopped",
        helix_privd::ServerAction::Backup => "Backup ready",
        helix_privd::ServerAction::Update => unreachable!(),
    }
}

#[cfg(target_os = "linux")]
fn collect_active_players(
    servers: Vec<AmpServer>,
    manager: &str,
    active_players: &mut u64,
    active_server_count: &mut u64,
    active_servers: &mut Vec<Value>,
    unverified_server_count: &mut u64,
    unverified_servers: &mut Vec<Value>,
) {
    for server in servers {
        if !server.player_count_verified {
            *unverified_server_count = unverified_server_count.saturating_add(1);
            if unverified_servers.len() < 32 {
                unverified_servers.push(json!({
                    "instance_id": server.id.as_str(),
                    "name": server.name.as_str(),
                    "manager": manager,
                    "reported_status": server.status.as_str(),
                    "issue_code": "player_count_unverified"
                }));
            }
        }
        if server.players_online > 0 {
            *active_players = active_players.saturating_add(server.players_online);
            *active_server_count = active_server_count.saturating_add(1);
            if active_servers.len() < 32 {
                active_servers.push(json!({
                    "instance_id": server.id.as_str(),
                    "name": server.name.as_str(),
                    "manager": manager,
                    "players_online": server.players_online
                }));
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn append_player_status_blockers(
    blockers: &mut Vec<Value>,
    active_players: u64,
    unverified_server_count: u64,
) {
    if active_players > 0 {
        blockers.push(json!({
            "code": "players_online",
            "message": "Players are currently connected to one or more game servers."
        }));
    }
    if unverified_server_count > 0 {
        blockers.push(json!({
            "code": "player_status_unverified",
            "message": "One or more game servers did not provide a trustworthy player count."
        }));
    }
}

#[cfg(target_os = "linux")]
fn collect_amp_inventory_issues(
    issue_count: u64,
    issues: Vec<AmpInventoryIssue>,
    unverified_server_count: &mut u64,
    unverified_servers: &mut Vec<Value>,
) {
    *unverified_server_count = unverified_server_count.saturating_add(issue_count);
    for issue in issues {
        if unverified_servers.len() >= 32 {
            break;
        }
        unverified_servers.push(json!({
            "instance_id": issue.instance_id,
            "name": issue.instance_name,
            "manager": "amp_import",
            "reported_status": "inventory_invalid",
            "issue_code": issue.code,
            "message": issue.message
        }));
    }
}

#[cfg(target_os = "linux")]
fn amp_inventory_health(result: Option<Result<AmpInventory, String>>) -> Value {
    let Some(result) = result else {
        return json!({
            "manager": "amp_import",
            "configured": false,
            "status": "not_configured",
            "server_count": 0,
            "unverified_instance_count": 0,
            "issues": [],
            "issues_truncated": false,
            "error": null
        });
    };
    let inventory = match result {
        Ok(inventory) => inventory,
        Err(_) => {
            return json!({
                "manager": "amp_import",
                "configured": true,
                "status": "unavailable",
                "server_count": 0,
                "unverified_instance_count": 0,
                "issues": [],
                "issues_truncated": false,
                "error": {
                    "code": "amp_inventory_unavailable",
                    "message": "Helix could not load the AMP compatibility inventory."
                }
            });
        }
    };
    let server_count = inventory.servers.len();
    let unverified_player_counts = inventory
        .servers
        .iter()
        .filter(|server| !server.player_count_verified)
        .count();
    let unverified_instance_count = inventory
        .issue_count
        .saturating_add(u64::try_from(unverified_player_counts).unwrap_or(u64::MAX));
    let mut issues = inventory
        .issues
        .into_iter()
        .map(|issue| {
            json!({
                "code": issue.code,
                "instance_id": issue.instance_id,
                "instance_name": issue.instance_name,
                "message": issue.message
            })
        })
        .collect::<Vec<_>>();
    for server in inventory
        .servers
        .into_iter()
        .filter(|server| !server.player_count_verified)
    {
        if issues.len() >= 64 {
            break;
        }
        issues.push(json!({
            "code": "player_count_unverified",
            "instance_id": server.id,
            "instance_name": server.instance_name,
            "message": "AMP did not provide a trustworthy player count for this Minecraft instance."
        }));
    }
    json!({
        "manager": "amp_import",
        "configured": true,
        "status": if unverified_instance_count == 0 { "healthy" } else { "degraded" },
        "server_count": server_count,
        "unverified_instance_count": unverified_instance_count,
        "issues_truncated": unverified_instance_count > issues.len() as u64,
        "issues": issues,
        "error": null
    })
}

#[cfg(target_os = "linux")]
fn append_game_port_mappings(mappings: &mut Vec<GamePortMapping>, servers: Vec<AmpServer>) {
    mappings.extend(servers.into_iter().filter_map(|server| {
        server.game_port.map(|port| GamePortMapping {
            instance_id: server.id,
            name: server.name,
            manager: server.manager.to_owned(),
            port,
            running: server.panel_running,
        })
    }));
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn Error>> {
    if !rustix::process::geteuid().is_root() {
        return Err("helix-privd must run as root behind its protected Unix socket".into());
    }
    let args = Args::parse();
    let config = load_config(&args.config)?;
    if let Some(schedule_id) = args.trigger_recurring_reboot {
        let result = BrokerClient::new(config.socket.clone())
            .request(&BrokerRequest::ExecuteRecurringHostReboot { schedule_id })?;
        eprintln!(
            "Helix accepted the recurring reboot safety-gate request: {}",
            result["operation_id"].as_str().unwrap_or("unknown")
        );
        return Ok(());
    }
    let socket_parent = config
        .socket
        .parent()
        .ok_or("the broker socket must have a parent directory")?;
    if !socket_parent.is_absolute() {
        return Err("the broker socket path must be absolute".into());
    }
    fs::create_dir_all(socket_parent)?;
    if config.socket.exists() {
        match std::os::unix::net::UnixStream::connect(&config.socket) {
            Ok(_) => return Err("another helix-privd process is already listening".into()),
            Err(_) => fs::remove_file(&config.socket)?,
        }
    }
    let managed_roots = config.managed_roots.clone();
    let analysis_roots = configured_analysis_roots(&config);
    let storage = StorageAnalysisManager::new(analysis_roots).map_err(io::Error::other)?;
    let files = FileManager::new(managed_roots.clone()).map_err(io::Error::other)?;
    let amp = config
        .amp_credentials
        .as_deref()
        .map(AmpClient::from_file)
        .transpose()
        .map_err(io::Error::other)?
        .map(Arc::new);
    let native = config
        .native
        .map(|mut native| {
            apply_custom_artifact_root_defaults(&mut native, &managed_roots);
            native
        })
        .map(NativeManager::new)
        .transpose()
        .map_err(io::Error::other)?
        .map(Arc::new);
    let host = Some(Arc::new(
        HostControl::new(config.host_control).map_err(io::Error::other)?,
    ));
    let network = NetworkManager::new(config.network).map_err(io::Error::other)?;
    let packages = PackageManager::new(config.packages).map_err(io::Error::other)?;
    let hook_installer =
        Arc::new(HookInstaller::new(config.hook_installer).map_err(io::Error::other)?);
    let context = Arc::new(BrokerContext {
        files,
        storage,
        amp,
        native,
        host,
        network,
        packages,
        hook_installer,
        power_gate: Mutex::new(()),
        jobs: Mutex::new(HashMap::new()),
    });

    let listener = UnixListener::bind(&config.socket)?;
    fs::set_permissions(&config.socket, fs::Permissions::from_mode(0o660))?;
    let active = Arc::new(AtomicUsize::new(0));
    eprintln!("helix-privd ready on {}", config.socket.display());
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        if active.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::AcqRel);
            let response = BrokerResponse::failure("broker_busy", "The host broker is busy.");
            if let Ok(body) = serde_json::to_vec(&response) {
                let _ = write_frame(&mut stream, &body);
            }
            continue;
        }
        let context = Arc::clone(&context);
        let active = Arc::clone(&active);
        thread::spawn(move || {
            let _guard = ConnectionGuard(active);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(35)));
            let response = match read_frame(&mut stream)
                .ok()
                .and_then(|body| serde_json::from_slice::<BrokerRequest>(&body).ok())
            {
                Some(request) => context.dispatch(request),
                None => {
                    BrokerResponse::failure("invalid_request", "The broker request is invalid.")
                }
            };
            if let Ok(body) = serde_json::to_vec(&response) {
                let _ = write_frame(&mut stream, &body);
            }
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_custom_artifact_root_defaults(native: &mut NativeConfig, managed_roots: &[PathBuf]) {
    if !native.custom_artifact_roots.is_empty() {
        return;
    }
    native.custom_artifact_roots.extend(
        managed_roots
            .iter()
            .filter(|root| native::custom_artifact_root_is_safe(root))
            .cloned(),
    );
}

#[cfg(target_os = "linux")]
struct ConnectionGuard(Arc<AtomicUsize>);

#[cfg(target_os = "linux")]
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(target_os = "linux")]
fn to_value(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "the broker could not encode its response".to_owned())
}

#[cfg(target_os = "linux")]
fn load_config(path: &Path) -> Result<BrokerConfig, Box<dyn Error>> {
    use std::os::unix::fs::MetadataExt as _;

    if !path.is_absolute() {
        return Err("the broker config path must be absolute".into());
    }
    let metadata = fs::metadata(path)?;
    if metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
        return Err("the broker config must be root-owned and inaccessible to other users".into());
    }
    if metadata.len() > 64 * 1024 {
        return Err("the broker config is too large".into());
    }
    let config: BrokerConfig = serde_json::from_slice(&fs::read(path)?)?;
    if !config.socket.is_absolute()
        || config.managed_roots.is_empty()
        || config.managed_roots.len() > MAX_CONFIGURED_ROOTS
        || config.analysis_roots.len() > MAX_CONFIGURED_ROOTS
        || config.managed_roots.iter().any(|root| !root.is_absolute())
        || config.analysis_roots.iter().any(|root| !root.is_absolute())
        || config
            .amp_credentials
            .as_ref()
            .is_some_and(|credentials| !credentials.is_absolute())
        || config.native.as_ref().is_some_and(|native| {
            !native.state_root.is_absolute()
                || !native.instance_root.is_absolute()
                || !native.backup_root.is_absolute()
                || !native.docker_binary.is_absolute()
        })
    {
        return Err(
            "broker paths must be absolute and at least one managed root is required".into(),
        );
    }
    Ok(config)
}

#[cfg(target_os = "linux")]
fn configured_analysis_roots(config: &BrokerConfig) -> Vec<PathBuf> {
    if config.analysis_roots.is_empty() {
        config.managed_roots.clone()
    } else {
        config.analysis_roots.clone()
    }
}

#[cfg(target_os = "linux")]
fn problem_code(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("outside a managed") || lower.contains("intentionally unavailable") {
        "path_restricted"
    } else if lower.contains("does not exist") {
        "not_found"
    } else if lower.contains("already exists")
        || lower.contains("already in use")
        || lower.contains("already in progress")
    {
        "conflict"
    } else if lower.contains("unavailable") || lower.contains("did not become reachable") {
        "dependency_unavailable"
    } else {
        "operation_failed"
    }
}

#[cfg(target_os = "linux")]
fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("helix-privd is only available on Linux");
    std::process::exit(1);
}

#[cfg(all(target_os = "linux", test))]
mod tests {
    use super::*;

    fn context() -> BrokerContext {
        let root = tempfile::tempdir().unwrap().keep();
        let network = NetworkManager::new(NetworkConfig {
            state_root: root.join("network-state"),
            ..NetworkConfig::default()
        })
        .unwrap();
        BrokerContext {
            files: FileManager::new(vec![root.clone()]).unwrap(),
            storage: StorageAnalysisManager::new(vec![root]).unwrap(),
            amp: None,
            native: None,
            host: None,
            network,
            packages: PackageManager::new(PackageConfig::default()).unwrap(),
            hook_installer: Arc::new(HookInstaller::new(HookInstallerConfig::default()).unwrap()),
            power_gate: Mutex::new(()),
            jobs: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn update_completion_stage_distinguishes_staged_from_runtime_verified() {
        let stopped = Ok(json!({
            "detail": {
                "already_current": false,
                "runtime_validation_performed": false
            }
        }));
        let running = Ok(json!({
            "detail": {
                "already_current": false,
                "runtime_validation_performed": true
            }
        }));
        let current = Ok(json!({
            "detail": {
                "already_current": true,
                "runtime_validation_performed": false
            }
        }));

        assert_eq!(
            server_action_completion_stage(helix_privd::ServerAction::Update, &stopped),
            "Update staged; first start not verified"
        );
        assert_eq!(
            server_action_completion_stage(helix_privd::ServerAction::Update, &running),
            "Update installed and verified"
        );
        assert_eq!(
            server_action_completion_stage(helix_privd::ServerAction::Update, &current),
            "Already current"
        );
    }

    #[test]
    fn active_resource_jobs_are_reused_only_for_the_same_request() {
        let context = context();
        let (first, reused) = context
            .queue_job(
                "server_restart",
                Some("server:helix:test"),
                Some("action:restart"),
            )
            .unwrap();
        assert!(!reused);
        let (second, reused) = context
            .queue_job(
                "server_restart",
                Some("server:helix:test"),
                Some("action:restart"),
            )
            .unwrap();
        assert!(reused);
        assert_eq!(first, second);
        assert!(
            context
                .queue_job(
                    "server_stop",
                    Some("server:helix:test"),
                    Some("action:stop")
                )
                .is_err()
        );
        context.finish_job(&first, Ok(json!({"online": true})), "Online");
        let (third, reused) = context
            .queue_job(
                "server_stop",
                Some("server:helix:test"),
                Some("action:stop"),
            )
            .unwrap();
        assert!(!reused);
        assert_ne!(first, third);
    }

    #[test]
    fn reboot_preflight_blocks_active_jobs_and_reports_a_bounded_reason() {
        let context = context();
        assert_eq!(
            context.host_reboot_preflight().unwrap()["can_schedule"],
            true
        );
        let (job_id, reused) = context
            .queue_job(
                "server_backup",
                Some("server:helix:test"),
                Some("action:backup"),
            )
            .unwrap();
        assert!(!reused);
        let preflight = context.host_reboot_preflight().unwrap();
        assert_eq!(preflight["can_schedule"], false);
        assert_eq!(preflight["active_jobs_total"], 1);
        assert_eq!(preflight["active_jobs"][0]["job_id"], job_id);
        assert_eq!(preflight["blockers"][0]["code"], "jobs_running");
    }

    #[test]
    fn legacy_broker_config_gets_backward_compatible_host_network_and_package_defaults() {
        let config: BrokerConfig = serde_json::from_value(json!({
            "socket": "/run/helix/privd.sock",
            "amp_credentials": null,
            "managed_roots": ["/srv"],
            "native": null
        }))
        .unwrap();
        assert_eq!(config.host_control.dashboard_container, "server-dashboard");
        assert_eq!(
            config.host_control.gateway_container,
            "server-dashboard-gateway"
        );
        assert_eq!(
            config.network.state_root,
            PathBuf::from("/var/lib/helix/network")
        );
        assert_eq!(
            config.packages.apt_get_binary,
            PathBuf::from("/usr/bin/apt-get")
        );
        assert!(config.analysis_roots.is_empty());
        assert_eq!(
            configured_analysis_roots(&config),
            vec![PathBuf::from("/srv")]
        );
    }

    #[test]
    fn analysis_roots_are_separate_from_writable_file_roots() {
        let config: BrokerConfig = serde_json::from_value(json!({
            "socket": "/run/helix/privd.sock",
            "amp_credentials": null,
            "managed_roots": ["/srv/storage"],
            "analysis_roots": ["/"],
            "native": null
        }))
        .unwrap();

        assert_eq!(config.managed_roots, vec![PathBuf::from("/srv/storage")]);
        assert_eq!(config.analysis_roots, vec![PathBuf::from("/")]);
        assert_eq!(configured_analysis_roots(&config), vec![PathBuf::from("/")]);
    }

    #[test]
    fn broad_browse_roots_never_become_custom_executable_roots() {
        let temporary = tempfile::tempdir().unwrap();
        let storage = temporary.path().join("storage");
        fs::create_dir(&storage).unwrap();
        let mut native: NativeConfig = serde_json::from_value(json!({
            "state_root": temporary.path().join("state"),
            "instance_root": temporary.path().join("instances"),
            "backup_root": temporary.path().join("backups"),
            "docker_binary": "/usr/bin/docker"
        }))
        .unwrap();

        apply_custom_artifact_root_defaults(&mut native, &[PathBuf::from("/"), storage.clone()]);
        assert_eq!(native.custom_artifact_roots, vec![storage]);
    }

    #[test]
    fn running_server_without_a_player_query_is_not_treated_as_empty() {
        let server = AmpServer {
            id: "helix:test".to_owned(),
            name: "Test".to_owned(),
            instance_name: "test".to_owned(),
            software: "Paper".to_owned(),
            version: "1.21.8".to_owned(),
            status: "offline".to_owned(),
            panel_running: true,
            start_on_boot: true,
            players_online: 0,
            player_count_verified: false,
            max_players: 20,
            cpu_percent: 0.0,
            memory_used_mb: 0,
            memory_limit_mb: 4096,
            tps: None,
            manager_panel_port: 0,
            panel_port: 0,
            game_port: Some(25_565),
            path: "/srv/test".to_owned(),
            warnings: Vec::new(),
            manager: "helix",
            execution_backend: "docker",
        };
        let mut active_players = 0;
        let mut active_server_count = 0;
        let mut active_servers = Vec::new();
        let mut unverified_server_count = 0;
        let mut unverified_servers = Vec::new();
        collect_active_players(
            vec![server],
            "helix_native",
            &mut active_players,
            &mut active_server_count,
            &mut active_servers,
            &mut unverified_server_count,
            &mut unverified_servers,
        );
        assert_eq!(active_players, 0);
        assert_eq!(unverified_server_count, 1);
        assert_eq!(unverified_servers[0]["reported_status"], "offline");
        let mut blockers = Vec::new();
        append_player_status_blockers(&mut blockers, active_players, unverified_server_count);
        assert_eq!(blockers[0]["code"], "player_status_unverified");
    }

    #[test]
    fn malformed_amp_inventory_items_block_reboot_and_surface_typed_health() {
        let issue = AmpInventoryIssue {
            code: "invalid_instance_panel_state",
            instance_id: Some("amp-instance".to_owned()),
            instance_name: Some("Minecraft01".to_owned()),
            message: "AMP returned incomplete or invalid Minecraft panel state.",
        };
        let health = amp_inventory_health(Some(Ok(AmpInventory {
            servers: Vec::new(),
            issue_count: 1,
            issues: vec![AmpInventoryIssue {
                code: issue.code,
                instance_id: issue.instance_id.clone(),
                instance_name: issue.instance_name.clone(),
                message: issue.message,
            }],
        })));
        assert_eq!(health["status"], "degraded");
        assert_eq!(health["unverified_instance_count"], 1);
        assert_eq!(health["issues"][0]["code"], issue.code);

        let mut unverified_server_count = 0;
        let mut unverified_servers = Vec::new();
        collect_amp_inventory_issues(
            1,
            vec![issue],
            &mut unverified_server_count,
            &mut unverified_servers,
        );
        let mut blockers = Vec::new();
        append_player_status_blockers(&mut blockers, 0, unverified_server_count);
        assert_eq!(unverified_server_count, 1);
        assert_eq!(
            unverified_servers[0]["issue_code"],
            "invalid_instance_panel_state"
        );
        assert_eq!(blockers[0]["code"], "player_status_unverified");
    }

    #[test]
    fn amp_inventory_outage_has_a_bounded_typed_error() {
        let health = amp_inventory_health(Some(Err("upstream detail".to_owned())));
        assert_eq!(health["status"], "unavailable");
        assert_eq!(health["error"]["code"], "amp_inventory_unavailable");
        assert!(!health.to_string().contains("upstream detail"));
    }
}
