use crate::bounded_command::run_bounded_command;
use helix_privd::{HookServiceAction, RebootWeekday, RecurringRebootSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const MAX_COMMAND_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_POWER_RECORD_BYTES: u64 = 64 * 1024;
const MAX_POWER_RECORDS: usize = 64;
const RECURRING_RECORD_NAME: &str = "recurring.json";
const RECURRING_SERVICE_UNIT: &str = "helix-recurring-reboot.service";
const RECURRING_TIMER_UNIT: &str = "helix-recurring-reboot.timer";
const RECURRING_UNIT_MARKER: &str = "# Managed by Helix recurring reboot scheduler";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostControlConfig {
    #[serde(default = "default_dashboard_container")]
    pub dashboard_container: String,
    #[serde(default = "default_gateway_container")]
    pub gateway_container: String,
    #[serde(default = "default_docker_binary")]
    pub docker_binary: PathBuf,
    #[serde(default = "default_systemctl_binary")]
    pub systemctl_binary: PathBuf,
    #[serde(default = "default_systemd_run_binary")]
    pub systemd_run_binary: PathBuf,
    #[serde(default = "default_systemd_analyze_binary")]
    pub systemd_analyze_binary: PathBuf,
    #[serde(default = "default_timeout_binary")]
    pub timeout_binary: PathBuf,
    #[serde(default = "default_docker_unit")]
    pub docker_unit: String,
    #[serde(default = "default_broker_unit")]
    pub broker_unit: String,
    #[serde(default = "default_power_state_root")]
    pub power_state_root: PathBuf,
    #[serde(default = "default_recurring_state_root")]
    pub recurring_state_root: PathBuf,
    #[serde(default = "default_systemd_unit_root")]
    pub systemd_unit_root: PathBuf,
    #[serde(default = "default_broker_binary")]
    pub broker_binary: PathBuf,
    #[serde(default = "default_broker_config_path")]
    pub broker_config_path: PathBuf,
    #[serde(default = "default_hook_services")]
    pub hook_services: Vec<HookServiceConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookServiceConfig {
    pub id: String,
    pub unit: String,
}

impl Default for HostControlConfig {
    fn default() -> Self {
        Self {
            dashboard_container: default_dashboard_container(),
            gateway_container: default_gateway_container(),
            docker_binary: default_docker_binary(),
            systemctl_binary: default_systemctl_binary(),
            systemd_run_binary: default_systemd_run_binary(),
            systemd_analyze_binary: default_systemd_analyze_binary(),
            timeout_binary: default_timeout_binary(),
            docker_unit: default_docker_unit(),
            broker_unit: default_broker_unit(),
            power_state_root: default_power_state_root(),
            recurring_state_root: default_recurring_state_root(),
            systemd_unit_root: default_systemd_unit_root(),
            broker_binary: default_broker_binary(),
            broker_config_path: default_broker_config_path(),
            hook_services: default_hook_services(),
        }
    }
}

pub struct HostControl {
    pub(crate) config: HostControlConfig,
    pub(crate) runner: Arc<dyn HostCommandRunner>,
    pub(crate) mutation: Mutex<()>,
}

#[derive(Clone, Debug)]
pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) trait HostCommandRunner: Send + Sync {
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

impl HostCommandRunner for ProcessRunner {
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct RestartPolicy {
    name: String,
    maximum_retry_count: u64,
}

impl RestartPolicy {
    fn docker_argument(&self) -> Result<String, String> {
        match self.name.as_str() {
            "no" | "always" | "unless-stopped" => Ok(self.name.clone()),
            "on-failure" => Ok(if self.maximum_retry_count == 0 {
                "on-failure".to_owned()
            } else {
                format!("on-failure:{}", self.maximum_retry_count)
            }),
            _ => Err("the container has an unsupported restart policy".to_owned()),
        }
    }
}

#[derive(Clone, Debug)]
struct ContainerStatus {
    name: String,
    running: bool,
    health: Option<String>,
    restart_count: u64,
    state_error: Option<String>,
    oom_killed: bool,
    restart_policy: RestartPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PowerOperationRecord {
    schema_version: u32,
    operation_id: String,
    hostname: String,
    scheduled_at_unix_ms: u64,
    execute_at_unix_ms: u64,
    delay_seconds: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecurringRebootRecord {
    schema_version: u32,
    schedule_id: String,
    hostname: String,
    weekdays: Vec<RebootWeekday>,
    hour: u8,
    minute: u8,
    timezone: String,
    calendar_expression: String,
    service_sha256: String,
    timer_sha256: String,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
}

impl HostControl {
    pub fn new(config: HostControlConfig) -> Result<Self, String> {
        validate_config(&config)?;
        fs::create_dir_all(&config.power_state_root)
            .map_err(|_| "could not create the host power operation directory".to_owned())?;
        let power_metadata = fs::symlink_metadata(&config.power_state_root)
            .map_err(|_| "could not inspect the host power operation directory".to_owned())?;
        if !power_metadata.file_type().is_dir()
            || power_metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err("the host power operation directory is invalid".to_owned());
        }
        fs::set_permissions(&config.power_state_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the host power operation directory".to_owned())?;
        prepare_private_directory(&config.recurring_state_root, "recurring reboot state")?;
        let runner = Arc::new(ProcessRunner {
            timeout_binary: config.timeout_binary.clone(),
        });
        Ok(Self {
            config,
            runner,
            mutation: Mutex::new(()),
        })
    }

    #[cfg(test)]
    fn with_runner(
        mut config: HostControlConfig,
        runner: Arc<dyn HostCommandRunner>,
    ) -> Result<Self, String> {
        validate_config_shape(&config)?;
        fs::create_dir_all(&config.power_state_root)
            .map_err(|_| "could not create test power state".to_owned())?;
        config.power_state_root = fs::canonicalize(&config.power_state_root)
            .map_err(|_| "could not resolve test power state".to_owned())?;
        fs::create_dir_all(&config.recurring_state_root)
            .map_err(|_| "could not create test recurring state".to_owned())?;
        config.recurring_state_root = fs::canonicalize(&config.recurring_state_root)
            .map_err(|_| "could not resolve test recurring state".to_owned())?;
        fs::create_dir_all(&config.systemd_unit_root)
            .map_err(|_| "could not create test systemd unit root".to_owned())?;
        config.systemd_unit_root = fs::canonicalize(&config.systemd_unit_root)
            .map_err(|_| "could not resolve test systemd unit root".to_owned())?;
        Ok(Self {
            config,
            runner,
            mutation: Mutex::new(()),
        })
    }

    pub fn status(&self) -> Result<Value, String> {
        let mut errors = Vec::new();
        let docker_service = self.unit_status(&self.config.docker_unit, &mut errors);
        let broker_service = self.unit_status(&self.config.broker_unit, &mut errors);
        let dashboard = self.container_status_or_error(
            &self.config.dashboard_container,
            "dashboard",
            &mut errors,
        );
        let gateway =
            self.container_status_or_error(&self.config.gateway_container, "gateway", &mut errors);
        let resources = self.resource_status(&mut errors);
        let policies = [dashboard.as_ref(), gateway.as_ref()]
            .into_iter()
            .flatten()
            .map(|container| container.restart_policy.name.as_str())
            .collect::<Vec<_>>();
        let start_on_boot = if policies.len() == 2 {
            let enabled = policies
                .iter()
                .all(|policy| matches!(*policy, "always" | "unless-stopped"));
            let disabled = policies.iter().all(|policy| *policy == "no");
            json!({
                "state": if enabled { "enabled" } else if disabled { "disabled" } else { "mixed" },
                "enabled": if enabled { Value::Bool(true) } else if disabled { Value::Bool(false) } else { Value::Null },
                "reconciled": enabled || disabled,
                "persistence": "docker_restart_policy",
                "current_runtime_changed_by_toggle": false,
                "container_recreation_may_reset": true,
                "note": "The setting survives Docker daemon and host restarts. Recreating either container can reapply its Compose restart policy, so Helix reads both exact container policies whenever status is requested."
            })
        } else {
            json!({
                "state": "unavailable",
                "enabled": Value::Null,
                "reconciled": false,
                "persistence": "docker_restart_policy",
                "current_runtime_changed_by_toggle": false,
                "container_recreation_may_reset": true
            })
        };
        let scheduled_reboot = self.scheduled_reboot_status(&mut errors)?;
        let recurring_reboot = self.recurring_reboot_status(&mut errors)?;
        Ok(json!({
            "schema_version": 1,
            "availability": if errors.is_empty() { "ready" } else { "degraded" },
            "hostname": current_hostname()?,
            "timezone": host_timezone(),
            "configured_targets": {
                "dashboard_container": self.config.dashboard_container.as_str(),
                "gateway_container": self.config.gateway_container.as_str()
            },
            "services": {
                "docker": docker_service,
                "helix_privd": broker_service
            },
            "containers": {
                "dashboard": dashboard.map(container_json),
                "gateway": gateway.map(container_json)
            },
            "start_on_boot": start_on_boot,
            "resources": resources,
            "scheduled_reboot": scheduled_reboot,
            "recurring_reboot": recurring_reboot,
            "errors": errors,
            "collected_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn hook_inventory(&self) -> Result<Value, String> {
        let hooks = self
            .config
            .hook_services
            .iter()
            .map(|hook| {
                let enabled_state = self.systemctl_state("is-enabled", &hook.unit);
                let active_state = self.systemctl_state("is-active", &hook.unit);
                let installed = enabled_state.as_deref() != Ok("not-found")
                    && active_state.as_deref() != Ok("unknown")
                    && (enabled_state.is_ok() || active_state.is_ok());
                let active = active_state.as_deref() == Ok("active");
                let enabled = matches!(
                    enabled_state.as_deref(),
                    Ok("enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias")
                );
                json!({
                    "id": hook.id,
                    "kind": "systemd",
                    "unit": hook.unit,
                    "installed": installed,
                    "active": active,
                    "active_state": active_state.as_deref().unwrap_or("unavailable"),
                    "enabled": enabled,
                    "enabled_state": enabled_state.as_deref().unwrap_or("unavailable"),
                    "controllable": installed && enabled_state.is_ok() && active_state.is_ok(),
                    "actions": if installed {
                        json!(["start", "stop", "restart", "enable", "disable"])
                    } else {
                        json!([])
                    },
                    "memory_used_bytes": if installed {
                        self.unit_memory_bytes(&hook.unit)
                    } else {
                        None
                    },
                    "cpu_percent": if installed && active {
                        self.unit_cpu_percent(&hook.unit)
                    } else {
                        None
                    },
                    "error": if enabled_state.is_err() || active_state.is_err() {
                        Value::String("Helix could not verify every systemd state for this service.".to_owned())
                    } else {
                        Value::Null
                    }
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "schema_version": 1,
            "hooks": hooks,
            "collected_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn manage_hook_service(
        &self,
        hook_id: &str,
        action: HookServiceAction,
    ) -> Result<Value, String> {
        validate_hook_id(hook_id)?;
        let hook = self
            .config
            .hook_services
            .iter()
            .find(|hook| hook.id == hook_id)
            .ok_or_else(|| "the hook is not configured for host control".to_owned())?;
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "hook service mutation lock failed".to_owned())?;
        let enabled_before = self.systemctl_state("is-enabled", &hook.unit)?;
        let active_before = self.systemctl_state("is-active", &hook.unit)?;
        if enabled_before == "not-found" || active_before == "unknown" {
            return Err("the hook service is not installed on this host".to_owned());
        }
        let verb = match action {
            HookServiceAction::Start => "start",
            HookServiceAction::Stop => "stop",
            HookServiceAction::Restart => "restart",
            HookServiceAction::Enable => "enable",
            HookServiceAction::Disable => "disable",
        };
        let output = self.runner.run(
            &self.config.systemctl_binary,
            &[verb.to_owned(), hook.unit.clone()],
            Duration::from_secs(30),
        )?;
        require_success(output)?;
        let enabled_state = self.systemctl_state("is-enabled", &hook.unit)?;
        let active_state = self.systemctl_state("is-active", &hook.unit)?;
        let verified = match action {
            HookServiceAction::Start | HookServiceAction::Restart => active_state == "active",
            HookServiceAction::Stop => active_state != "active",
            HookServiceAction::Enable => matches!(
                enabled_state.as_str(),
                "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
            ),
            HookServiceAction::Disable => !matches!(
                enabled_state.as_str(),
                "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
            ),
        };
        if !verified {
            return Err("systemd did not reach the requested hook service state".to_owned());
        }
        Ok(json!({
            "hook_id": hook.id,
            "unit": hook.unit,
            "action": verb,
            "active": active_state == "active",
            "active_state": active_state,
            "enabled": matches!(enabled_state.as_str(), "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"),
            "enabled_state": enabled_state,
            "verified": true,
            "updated_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn set_start_on_boot(&self, enabled: bool) -> Result<Value, String> {
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "host integration mutation lock failed".to_owned())?;
        let names = [
            self.config.dashboard_container.as_str(),
            self.config.gateway_container.as_str(),
        ];
        let original = [
            self.inspect_container(names[0])?.restart_policy,
            self.inspect_container(names[1])?.restart_policy,
        ];
        for policy in &original {
            policy.docker_argument()?;
        }
        let desired = if enabled { "unless-stopped" } else { "no" };
        let mut changed = Vec::new();
        for (index, name) in names.iter().enumerate() {
            if original[index].name == desired {
                continue;
            }
            if let Err(error) = self.docker_update_restart(name, desired) {
                let rollback = self.rollback_restart_policies(&names, &original);
                return Err(match rollback {
                    Ok(()) => format!("start-on-boot was not changed: {error}"),
                    Err(rollback) => format!(
                        "start-on-boot change failed: {error}; rollback also failed: {rollback}"
                    ),
                });
            }
            changed.push(index);
        }
        for name in names {
            let verified = match self.inspect_container(name) {
                Ok(verified) => verified,
                Err(error) => {
                    let rollback = self.rollback_restart_policies(&names, &original);
                    return Err(match rollback {
                        Ok(()) => format!(
                            "Docker restart policy verification failed and the original policies were restored: {error}"
                        ),
                        Err(rollback) => format!(
                            "Docker restart policy verification failed: {error}; rollback also failed: {rollback}"
                        ),
                    });
                }
            };
            if verified.restart_policy.name != desired {
                let rollback = self.rollback_restart_policies(&names, &original);
                return Err(match rollback {
                    Ok(()) => "Docker did not persist the requested restart policy".to_owned(),
                    Err(rollback) => format!(
                        "Docker did not persist the requested restart policy; rollback also failed: {rollback}"
                    ),
                });
            }
        }
        Ok(json!({
            "enabled": enabled,
            "policy": desired,
            "containers": names,
            "changed_containers": changed.len(),
            "current_runtime_changed": false,
            "persisted": true,
            "reconciled": true,
            "container_recreation_may_reset": true,
            "persistence_note": "The policy survives daemon and host restarts, but later container recreation can reapply the Compose policy.",
            "updated_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn schedule_reboot(
        &self,
        confirmation_hostname: &str,
        delay_seconds: u16,
        disruption_acknowledged: bool,
    ) -> Result<Value, String> {
        if !(10..=300).contains(&delay_seconds) {
            return Err("reboot delay must be between 10 and 300 seconds".to_owned());
        }
        if !disruption_acknowledged {
            return Err("reboot disruption must be acknowledged".to_owned());
        }
        let hostname = current_hostname()?;
        if confirmation_hostname != hostname {
            return Err("hostname confirmation does not exactly match this host".to_owned());
        }
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "host power mutation lock failed".to_owned())?;
        if self.active_power_record()?.is_some() {
            return Err("a host reboot is already scheduled".to_owned());
        }
        let operation_id = Uuid::new_v4().to_string();
        let scheduled_at_unix_ms = now_unix_ms();
        let execute_at_unix_ms =
            scheduled_at_unix_ms.saturating_add(u64::from(delay_seconds).saturating_mul(1_000));
        let record = PowerOperationRecord {
            schema_version: 1,
            operation_id: operation_id.clone(),
            hostname: hostname.clone(),
            scheduled_at_unix_ms,
            execute_at_unix_ms,
            delay_seconds,
        };
        let record_path = self.power_record_path(&operation_id)?;
        write_power_record(&record_path, &record)?;
        let service_unit = reboot_service_unit(&operation_id)?;
        let timer_unit = format!("{service_unit}.timer");
        let args = vec![
            "--quiet".to_owned(),
            "--collect".to_owned(),
            format!("--unit={service_unit}"),
            format!("--on-active={delay_seconds}s"),
            "--timer-property=AccuracySec=1s".to_owned(),
            "--property=Type=oneshot".to_owned(),
            "--property=NoNewPrivileges=yes".to_owned(),
            "--property=ProtectSystem=strict".to_owned(),
            self.config.systemctl_binary.to_string_lossy().into_owned(),
            "reboot".to_owned(),
        ];
        let result = self.runner.run(
            &self.config.systemd_run_binary,
            &args,
            Duration::from_secs(15),
        );
        if let Err(error) = result.and_then(require_success) {
            return Err(self.failed_schedule_cleanup(
                &record_path,
                &timer_unit,
                &format!("could not schedule the host reboot: {error}"),
            ));
        }
        match self.unit_is_active(&timer_unit) {
            Ok(true) => {}
            Ok(false) => {
                return Err(self.failed_schedule_cleanup(
                    &record_path,
                    &timer_unit,
                    "the reboot timer was not active after scheduling",
                ));
            }
            Err(error) => {
                return Err(self.failed_schedule_cleanup(
                    &record_path,
                    &timer_unit,
                    &format!("the reboot timer could not be verified after scheduling: {error}"),
                ));
            }
        }
        Ok(json!({
            "operation_id": operation_id,
            "state": "scheduled",
            "hostname": hostname,
            "scheduled_at_unix_ms": scheduled_at_unix_ms,
            "execute_at_unix_ms": execute_at_unix_ms,
            "delay_seconds": delay_seconds,
            "cancellable": true,
            "timer_backend": "systemd_transient_timer"
        }))
    }

    pub fn cancel_reboot(&self, operation_id: &str) -> Result<Value, String> {
        validate_operation_id(operation_id)?;
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "host power mutation lock failed".to_owned())?;
        let record_path = self.power_record_path(operation_id)?;
        let record = read_power_record(&record_path)?;
        validate_power_record(&record, operation_id)?;
        let service_unit = reboot_service_unit(operation_id)?;
        let timer_unit = format!("{service_unit}.timer");
        if self.unit_is_active(&service_unit)? {
            return Err(
                "the reboot operation has already started and can no longer be cancelled"
                    .to_owned(),
            );
        }
        let was_active = self.unit_is_active(&timer_unit)?;
        if was_active {
            let output = self.runner.run(
                &self.config.systemctl_binary,
                &["stop".to_owned(), timer_unit.clone()],
                Duration::from_secs(15),
            )?;
            require_success(output)?;
        }
        if self.unit_is_active(&timer_unit)? {
            return Err("the reboot timer remained active after cancellation".to_owned());
        }
        remove_power_record_durable(&record_path).map_err(|_| {
            "the reboot timer stopped, but its Helix record could not be removed durably".to_owned()
        })?;
        Ok(json!({
            "operation_id": operation_id,
            "state": "cancelled",
            "was_active": was_active,
            "cancelled_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn set_recurring_reboot(&self, spec: RecurringRebootSpec) -> Result<Value, String> {
        let hostname = current_hostname()?;
        let timezone = read_host_timezone()?;
        let weekdays = validate_recurring_spec(&spec, &hostname, &timezone)?;
        let calendar_expression = recurring_calendar(&weekdays, spec.hour, spec.minute, &timezone);
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "host power mutation lock failed".to_owned())?;
        let validation = self.runner.run(
            &self.config.systemd_analyze_binary,
            &[
                "calendar".to_owned(),
                "--iterations=1".to_owned(),
                calendar_expression.clone(),
            ],
            Duration::from_secs(10),
        )?;
        require_success(validation).map_err(|error| {
            format!("systemd rejected the requested recurring schedule: {error}")
        })?;

        let previous = self.read_recurring_record_optional()?;
        let service_path = self.recurring_service_path()?;
        let timer_path = self.recurring_timer_path()?;
        if previous.is_none() && (service_path.exists() || timer_path.exists()) {
            return Err(
                "recurring reboot unit files already exist without a verified Helix record; nothing was overwritten"
                    .to_owned(),
            );
        }
        if let Some(record) = &previous {
            self.verify_recurring_unit_files(record)?;
        }
        let schedule_id = previous.as_ref().map_or_else(
            || Uuid::new_v4().to_string(),
            |record| record.schedule_id.clone(),
        );
        let now = now_unix_ms();
        let service = recurring_service_unit(&self.config, &schedule_id)?;
        let timer = recurring_timer_unit(&calendar_expression);
        let record = RecurringRebootRecord {
            schema_version: 1,
            schedule_id: schedule_id.clone(),
            hostname: hostname.clone(),
            weekdays,
            hour: spec.hour,
            minute: spec.minute,
            timezone: timezone.clone(),
            calendar_expression: calendar_expression.clone(),
            service_sha256: sha256_hex(service.as_bytes()),
            timer_sha256: sha256_hex(timer.as_bytes()),
            created_at_unix_ms: previous
                .as_ref()
                .map_or(now, |record| record.created_at_unix_ms),
            updated_at_unix_ms: now,
        };
        let previous_service = read_optional_regular_file(&service_path, 64 * 1024)?;
        let previous_timer = read_optional_regular_file(&timer_path, 64 * 1024)?;
        write_unit_file(&service_path, service.as_bytes())?;
        let activation = write_unit_file(&timer_path, timer.as_bytes())
            .and_then(|()| self.write_recurring_record(&record))
            .and_then(|()| self.systemctl(&["daemon-reload"]))
            .and_then(|()| self.systemctl(&["enable", "--now", RECURRING_TIMER_UNIT]))
            .and_then(|()| self.verify_recurring_timer_active())
            .and_then(|()| {
                self.recurring_next_elapse()?
                    .ok_or_else(|| "systemd did not report a next recurring reboot time".to_owned())
            });
        let next_at_unix_ms = match activation {
            Ok(next_at_unix_ms) => next_at_unix_ms,
            Err(error) => {
                let rollback = self.rollback_recurring_files(
                    previous.as_ref(),
                    previous_service.as_deref(),
                    previous_timer.as_deref(),
                );
                return Err(match rollback {
                    Ok(()) => format!("the recurring reboot schedule was not activated: {error}"),
                    Err(rollback) => format!(
                        "the recurring reboot schedule was not activated: {error}; rollback also needs attention: {rollback}"
                    ),
                });
            }
        };
        Ok(json!({
            "state": "scheduled",
            "schedule_id": schedule_id,
            "hostname": hostname,
            "weekdays": record.weekdays,
            "hour": record.hour,
            "minute": record.minute,
            "timezone": timezone,
            "calendar_expression": calendar_expression,
            "next_at_unix_ms": next_at_unix_ms,
            "timer_active": true,
            "timer_enabled": true,
            "missed_runs_catch_up": false,
            "execution_gate": "players_jobs_and_inventory_must_be_clear",
            "automatic_reboot_without_preflight": false,
            "updated_at_unix_ms": now
        }))
    }

    pub fn delete_recurring_reboot(&self, confirmation_hostname: &str) -> Result<Value, String> {
        let hostname = current_hostname()?;
        if confirmation_hostname != hostname {
            return Err("hostname confirmation does not exactly match this host".to_owned());
        }
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "host power mutation lock failed".to_owned())?;
        let record = self
            .read_recurring_record_optional()?
            .ok_or_else(|| "no recurring host reboot schedule exists".to_owned())?;
        self.verify_recurring_unit_files(&record)?;
        let service_path = self.recurring_service_path()?;
        let timer_path = self.recurring_timer_path()?;
        let service = read_optional_regular_file(&service_path, 64 * 1024)?;
        let timer = read_optional_regular_file(&timer_path, 64 * 1024)?;
        self.systemctl(&["disable", "--now", RECURRING_TIMER_UNIT])?;
        if self.unit_is_active(RECURRING_TIMER_UNIT)? {
            return Err(
                "the recurring reboot timer remained active; its files were retained".to_owned(),
            );
        }
        remove_file_and_sync(&service_path)?;
        if let Err(error) = remove_file_and_sync(&timer_path)
            .and_then(|()| remove_file_and_sync(&self.recurring_record_path()))
            .and_then(|()| self.systemctl(&["daemon-reload"]))
        {
            let rollback =
                self.rollback_recurring_files(Some(&record), service.as_deref(), timer.as_deref());
            return Err(match rollback {
                Ok(()) => format!("the recurring reboot schedule was not removed: {error}"),
                Err(rollback) => format!(
                    "the recurring reboot schedule removal failed: {error}; rollback also needs attention: {rollback}"
                ),
            });
        }
        Ok(json!({
            "state": "removed",
            "schedule_id": record.schedule_id,
            "hostname": hostname,
            "timer_active": false,
            "timer_enabled": false,
            "removed_at_unix_ms": now_unix_ms()
        }))
    }

    pub fn verify_recurring_trigger(&self, schedule_id: &str) -> Result<String, String> {
        validate_operation_id(schedule_id)?;
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "host power mutation lock failed".to_owned())?;
        let record = self
            .read_recurring_record_optional()?
            .ok_or_else(|| "the recurring reboot schedule no longer exists".to_owned())?;
        if record.schedule_id != schedule_id || record.hostname != current_hostname()? {
            return Err(
                "the recurring reboot trigger does not match the active schedule".to_owned(),
            );
        }
        self.verify_recurring_unit_files(&record)?;
        self.verify_recurring_timer_active()?;
        Ok(record.hostname)
    }

    pub fn reboot_pending(&self) -> Result<bool, String> {
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| "host power mutation lock failed".to_owned())?;
        Ok(self.active_power_record()?.is_some())
    }

    fn unit_status(&self, unit: &str, errors: &mut Vec<Value>) -> Value {
        let enabled = self.systemctl_state("is-enabled", unit);
        let active = self.systemctl_state("is-active", unit);
        if enabled.is_err() || active.is_err() {
            push_error(errors, "systemd", "unit_status_unavailable", unit);
        }
        json!({
            "unit": unit,
            "enabled_state": enabled.as_deref().unwrap_or("unavailable"),
            "enabled": enabled.as_deref() == Ok("enabled"),
            "active_state": active.as_deref().unwrap_or("unavailable"),
            "active": active.as_deref() == Ok("active")
        })
    }

    fn failed_schedule_cleanup(
        &self,
        record_path: &Path,
        timer_unit: &str,
        original_error: &str,
    ) -> String {
        let stop = self.runner.run(
            &self.config.systemctl_binary,
            &["stop".to_owned(), timer_unit.to_owned()],
            Duration::from_secs(15),
        );
        let inactive = self.unit_is_active(timer_unit).is_ok_and(|active| !active);
        let record_cleanup = inactive.then(|| remove_power_record_durable(record_path));
        match (stop.and_then(require_success), inactive, record_cleanup) {
            (_, true, Some(Ok(()))) => original_error.to_owned(),
            (_, true, Some(Err(_))) => format!(
                "{original_error}; the reboot timer was cancelled, but its protected record cleanup remains pending"
            ),
            (Err(cleanup), false, None) => format!(
                "{original_error}; Helix could not prove the reboot timer was cancelled: {cleanup}"
            ),
            (Ok(_), false, None) => {
                format!("{original_error}; Helix could not prove the reboot timer was cancelled")
            }
            (_, _, _) => {
                format!("{original_error}; Helix could not reconcile reboot cleanup safely")
            }
        }
    }

    pub(crate) fn systemctl_state(&self, verb: &str, unit: &str) -> Result<String, String> {
        let output = self.runner.run(
            &self.config.systemctl_binary,
            &[verb.to_owned(), unit.to_owned()],
            Duration::from_secs(10),
        )?;
        let state = first_line(&output.stdout)
            .or_else(|| first_line(&output.stderr))
            .map(sanitize_state)
            .filter(|state| !state.is_empty())
            .ok_or_else(|| format!("systemd returned no state for {unit}"))?;
        if recognized_systemctl_state(verb, &state) {
            Ok(state)
        } else {
            Err(format!("systemd returned an invalid state for {unit}"))
        }
    }

    fn unit_is_active(&self, unit: &str) -> Result<bool, String> {
        Ok(self.systemctl_state("is-active", unit)? == "active")
    }

    fn unit_memory_bytes(&self, unit: &str) -> Option<u64> {
        let output = self
            .runner
            .run(
                &self.config.systemctl_binary,
                &[
                    "show".to_owned(),
                    "-p".to_owned(),
                    "MemoryCurrent".to_owned(),
                    "--value".to_owned(),
                    unit.to_owned(),
                ],
                Duration::from_secs(5),
            )
            .ok()?;
        let text = first_line(&output.stdout)?;
        if text != "[not set]" && text != "[NotSet]" {
            if let Ok(bytes) = text.parse::<u64>() {
                if bytes > 0 && bytes != u64::MAX {
                    return Some(bytes);
                }
            }
        }
        self.cgroup_memory_bytes(unit)
    }

    fn unit_cpu_percent(&self, unit: &str) -> Option<f64> {
        let first = self.cgroup_cpu_usage_usec(unit)?;
        std::thread::sleep(Duration::from_millis(120));
        let second = self.cgroup_cpu_usage_usec(unit)?;
        let delta = second.saturating_sub(first);
        let percent = (delta as f64) / 1_200.0;
        if percent.is_finite() && percent >= 0.0 && percent <= 10_000.0 {
            Some(percent)
        } else {
            None
        }
    }

    fn cgroup_dir(&self, unit: &str) -> Option<PathBuf> {
        let output = self
            .runner
            .run(
                &self.config.systemctl_binary,
                &[
                    "show".to_owned(),
                    "-p".to_owned(),
                    "ControlGroup".to_owned(),
                    "--value".to_owned(),
                    unit.to_owned(),
                ],
                Duration::from_secs(5),
            )
            .ok()?;
        let group = first_line(&output.stdout)?.trim();
        if group.is_empty() || group.contains('\0') || group.contains("/../") {
            return None;
        }
        let path = PathBuf::from(format!("/sys/fs/cgroup{group}"));
        path.starts_with("/sys/fs/cgroup")
            .then_some(path)
            .filter(|path| path.is_dir())
    }

    fn cgroup_memory_bytes(&self, unit: &str) -> Option<u64> {
        let text = fs::read_to_string(self.cgroup_dir(unit)?.join("memory.current")).ok()?;
        let bytes = text.trim().parse::<u64>().ok()?;
        (bytes > 0 && bytes != u64::MAX).then_some(bytes)
    }

    fn cgroup_cpu_usage_usec(&self, unit: &str) -> Option<u64> {
        let text = fs::read_to_string(self.cgroup_dir(unit)?.join("cpu.stat")).ok()?;
        for line in text.lines() {
            let Some(("usage_usec", rest)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            return rest.trim().parse().ok();
        }
        None
    }

    fn inspect_container(&self, name: &str) -> Result<ContainerStatus, String> {
        let output = self.runner.run(
            &self.config.docker_binary,
            &[
                "inspect".to_owned(),
                "--type".to_owned(),
                "container".to_owned(),
                name.to_owned(),
            ],
            Duration::from_secs(15),
        )?;
        let output = require_success(output)?;
        let values: Vec<Value> = serde_json::from_str(&output.stdout)
            .map_err(|_| "Docker returned invalid container metadata".to_owned())?;
        let value = values
            .first()
            .filter(|_| values.len() == 1)
            .ok_or_else(|| "Docker returned an ambiguous container result".to_owned())?;
        if value.get("Name").and_then(Value::as_str) != Some(&format!("/{name}")) {
            return Err("Docker returned the wrong container".to_owned());
        }
        let policy = value
            .pointer("/HostConfig/RestartPolicy")
            .ok_or_else(|| "Docker omitted the container restart policy".to_owned())?;
        let policy_name = policy
            .get("Name")
            .and_then(Value::as_str)
            .filter(|value| value.len() <= 32)
            .ok_or_else(|| "Docker returned an invalid restart policy".to_owned())?;
        Ok(ContainerStatus {
            name: name.to_owned(),
            running: value
                .pointer("/State/Running")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            health: value
                .pointer("/State/Health/Status")
                .and_then(Value::as_str)
                .filter(|value| value.len() <= 32)
                .map(str::to_owned),
            restart_count: value
                .get("RestartCount")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            state_error: value
                .pointer("/State/Error")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(|value| value.chars().take(300).collect()),
            oom_killed: value
                .pointer("/State/OOMKilled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            restart_policy: RestartPolicy {
                name: policy_name.to_owned(),
                maximum_retry_count: policy
                    .get("MaximumRetryCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            },
        })
    }

    fn container_status_or_error(
        &self,
        name: &str,
        component: &str,
        errors: &mut Vec<Value>,
    ) -> Option<ContainerStatus> {
        match self.inspect_container(name) {
            Ok(status) => {
                if status.oom_killed {
                    push_error(errors, component, "oom_killed", "Container was OOM-killed");
                }
                if let Some(error) = &status.state_error {
                    push_error(errors, component, "container_error", error);
                }
                if status
                    .health
                    .as_deref()
                    .is_some_and(|health| health != "healthy")
                {
                    push_error(
                        errors,
                        component,
                        "container_health",
                        status.health.as_deref().unwrap_or("unknown"),
                    );
                }
                Some(status)
            }
            Err(error) => {
                push_error(errors, component, "container_unavailable", &error);
                None
            }
        }
    }

    fn resource_status(&self, errors: &mut Vec<Value>) -> Value {
        let args = vec![
            "stats".to_owned(),
            "--no-stream".to_owned(),
            "--format".to_owned(),
            "{{json .}}".to_owned(),
            self.config.dashboard_container.clone(),
            self.config.gateway_container.clone(),
        ];
        let containers = match self
            .runner
            .run(&self.config.docker_binary, &args, Duration::from_secs(20))
            .and_then(require_success)
        {
            Ok(output) => parse_docker_stats(
                &output.stdout,
                &self.config.dashboard_container,
                &self.config.gateway_container,
            )
            .unwrap_or_else(|error| {
                push_error(errors, "resources", "container_stats_invalid", &error);
                json!({})
            }),
            Err(error) => {
                push_error(errors, "resources", "container_stats_unavailable", &error);
                json!({})
            }
        };
        let broker = broker_process_resources().unwrap_or_else(|error| {
            push_error(errors, "resources", "broker_stats_unavailable", &error);
            json!({})
        });
        json!({
            "scope": "helix_only_excludes_game_servers",
            "containers": containers,
            "broker": broker
        })
    }

    fn docker_update_restart(&self, name: &str, policy: &str) -> Result<(), String> {
        let output = self.runner.run(
            &self.config.docker_binary,
            &[
                "update".to_owned(),
                "--restart".to_owned(),
                policy.to_owned(),
                name.to_owned(),
            ],
            Duration::from_secs(20),
        )?;
        require_success(output).map(|_| ())
    }

    fn rollback_restart_policies(
        &self,
        names: &[&str; 2],
        original: &[RestartPolicy; 2],
    ) -> Result<(), String> {
        let mut command_failures = [None, None];
        for index in (0..names.len()).rev() {
            let policy = original[index].docker_argument()?;
            if let Err(error) = self.docker_update_restart(names[index], &policy) {
                command_failures[index] = Some(error);
            }
        }
        let mut failures = Vec::new();
        for index in 0..names.len() {
            match self.inspect_container(names[index]) {
                Ok(container) if container.restart_policy == original[index] => {}
                Ok(_) => failures.push(command_failures[index].clone().unwrap_or_else(|| {
                    format!(
                        "Docker did not restore the original restart policy for {}",
                        names[index]
                    )
                })),
                Err(verification) => failures.push(command_failures[index].as_ref().map_or_else(
                    || format!("could not verify rollback for {}: {verification}", names[index]),
                    |command| {
                        format!(
                            "could not restore {}: {command}; verification also failed: {verification}",
                            names[index]
                        )
                    },
                )),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    fn recurring_record_path(&self) -> PathBuf {
        self.config.recurring_state_root.join(RECURRING_RECORD_NAME)
    }

    fn recurring_service_path(&self) -> Result<PathBuf, String> {
        recurring_unit_path(&self.config.systemd_unit_root, RECURRING_SERVICE_UNIT)
    }

    fn recurring_timer_path(&self) -> Result<PathBuf, String> {
        recurring_unit_path(&self.config.systemd_unit_root, RECURRING_TIMER_UNIT)
    }

    fn read_recurring_record_optional(&self) -> Result<Option<RecurringRebootRecord>, String> {
        let path = self.recurring_record_path();
        if !path.exists() {
            return Ok(None);
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "could not inspect the recurring reboot record".to_owned())?;
        if !metadata.file_type().is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.len() == 0
            || metadata.len() > MAX_POWER_RECORD_BYTES
        {
            return Err("the recurring reboot record is invalid".to_owned());
        }
        let record: RecurringRebootRecord = serde_json::from_slice(
            &fs::read(&path)
                .map_err(|_| "could not read the recurring reboot record".to_owned())?,
        )
        .map_err(|_| "the recurring reboot record is invalid".to_owned())?;
        validate_recurring_record(&record)?;
        Ok(Some(record))
    }

    fn write_recurring_record(&self, record: &RecurringRebootRecord) -> Result<(), String> {
        validate_recurring_record(record)?;
        let body = serde_json::to_vec_pretty(record)
            .map_err(|_| "could not encode the recurring reboot record".to_owned())?;
        write_private_file(&self.recurring_record_path(), &body)
    }

    fn verify_recurring_unit_files(&self, record: &RecurringRebootRecord) -> Result<(), String> {
        validate_recurring_record(record)?;
        let service = read_optional_regular_file(&self.recurring_service_path()?, 64 * 1024)?
            .ok_or_else(|| "the recurring reboot service unit is missing".to_owned())?;
        let timer = read_optional_regular_file(&self.recurring_timer_path()?, 64 * 1024)?
            .ok_or_else(|| "the recurring reboot timer unit is missing".to_owned())?;
        if sha256_hex(&service) != record.service_sha256
            || sha256_hex(&timer) != record.timer_sha256
            || !service.starts_with(RECURRING_UNIT_MARKER.as_bytes())
            || !timer.starts_with(RECURRING_UNIT_MARKER.as_bytes())
        {
            return Err(
                "recurring reboot unit files no longer match their protected Helix record; nothing was changed"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn systemctl(&self, args: &[&str]) -> Result<(), String> {
        let output = self.runner.run(
            &self.config.systemctl_binary,
            &args
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
            Duration::from_secs(20),
        )?;
        require_success(output).map(|_| ())
    }

    fn verify_recurring_timer_active(&self) -> Result<(), String> {
        if !self.unit_is_active(RECURRING_TIMER_UNIT)? {
            return Err("the recurring reboot timer is not active".to_owned());
        }
        let enabled = self.systemctl_state("is-enabled", RECURRING_TIMER_UNIT)?;
        if !matches!(enabled.as_str(), "enabled" | "enabled-runtime" | "linked") {
            return Err("the recurring reboot timer is not enabled for future boots".to_owned());
        }
        Ok(())
    }

    fn recurring_next_elapse(&self) -> Result<Option<u64>, String> {
        let output = self.runner.run(
            &self.config.systemctl_binary,
            &[
                "list-timers".to_owned(),
                RECURRING_TIMER_UNIT.to_owned(),
                "--all".to_owned(),
                "--no-pager".to_owned(),
                "--output=json".to_owned(),
            ],
            Duration::from_secs(5),
        )?;
        let output = require_success(output)?;
        let timers = serde_json::from_str::<Vec<Value>>(&output.stdout)
            .map_err(|_| "systemd returned invalid recurring reboot timer data".to_owned())?;
        let Some(timer) = timers.first().filter(|_| timers.len() == 1) else {
            return Ok(None);
        };
        if timer.get("unit").and_then(Value::as_str) != Some(RECURRING_TIMER_UNIT)
            || timer.get("activates").and_then(Value::as_str) != Some(RECURRING_SERVICE_UNIT)
        {
            return Err("systemd returned the wrong recurring reboot timer".to_owned());
        }
        let micros = timer
            .get("next")
            .and_then(Value::as_u64)
            .ok_or_else(|| "systemd returned an invalid next recurring reboot time".to_owned())?;
        Ok((micros > 0).then_some(micros / 1_000))
    }

    fn recurring_reboot_status(&self, errors: &mut Vec<Value>) -> Result<Value, String> {
        let record = match self.read_recurring_record_optional() {
            Ok(Some(record)) => record,
            Ok(None) => {
                if self.recurring_service_path()?.exists() || self.recurring_timer_path()?.exists()
                {
                    push_error(
                        errors,
                        "power",
                        "recurring_record_missing",
                        "Recurring reboot unit files exist without a protected Helix record",
                    );
                    return Ok(json!({
                        "state": "unavailable",
                        "reason": "unit_files_without_record"
                    }));
                }
                return Ok(json!({
                    "state": "none",
                    "timer_active": false,
                    "timer_enabled": false
                }));
            }
            Err(error) => {
                push_error(errors, "power", "recurring_record_invalid", &error);
                return Ok(json!({
                    "state": "unavailable",
                    "reason": "record_invalid"
                }));
            }
        };
        if let Err(error) = self.verify_recurring_unit_files(&record) {
            push_error(errors, "power", "recurring_units_invalid", &error);
            return Ok(json!({
                "state": "unavailable",
                "reason": "unit_verification_failed"
            }));
        }
        let timer_active = self.unit_is_active(RECURRING_TIMER_UNIT).unwrap_or(false);
        let timer_enabled = self
            .systemctl_state("is-enabled", RECURRING_TIMER_UNIT)
            .is_ok_and(|state| matches!(state.as_str(), "enabled" | "enabled-runtime" | "linked"));
        let next_at_unix_ms = self.recurring_next_elapse().ok().flatten();
        if !timer_active || !timer_enabled || next_at_unix_ms.is_none() {
            push_error(
                errors,
                "power",
                "recurring_timer_degraded",
                "The recurring reboot schedule exists, but systemd did not verify its active, enabled, and next-run state",
            );
        }
        Ok(json!({
            "state": if timer_active && timer_enabled && next_at_unix_ms.is_some() { "scheduled" } else { "degraded" },
            "schedule_id": record.schedule_id,
            "hostname": record.hostname,
            "weekdays": record.weekdays,
            "hour": record.hour,
            "minute": record.minute,
            "timezone": record.timezone,
            "calendar_expression": record.calendar_expression,
            "next_at_unix_ms": next_at_unix_ms,
            "timer_active": timer_active,
            "timer_enabled": timer_enabled,
            "missed_runs_catch_up": false,
            "execution_gate": "players_jobs_and_inventory_must_be_clear",
            "automatic_reboot_without_preflight": false,
            "created_at_unix_ms": record.created_at_unix_ms,
            "updated_at_unix_ms": record.updated_at_unix_ms
        }))
    }

    fn rollback_recurring_files(
        &self,
        previous: Option<&RecurringRebootRecord>,
        previous_service: Option<&[u8]>,
        previous_timer: Option<&[u8]>,
    ) -> Result<(), String> {
        match (previous, previous_service, previous_timer) {
            (Some(record), Some(service), Some(timer)) => {
                write_unit_file(&self.recurring_service_path()?, service)?;
                write_unit_file(&self.recurring_timer_path()?, timer)?;
                self.write_recurring_record(record)?;
                self.systemctl(&["daemon-reload"])?;
                self.systemctl(&["enable", "--now", RECURRING_TIMER_UNIT])?;
                self.verify_recurring_timer_active()
            }
            (None, _, _) => {
                let _ = self.systemctl(&["disable", "--now", RECURRING_TIMER_UNIT]);
                remove_optional_file_and_sync(&self.recurring_service_path()?)?;
                remove_optional_file_and_sync(&self.recurring_timer_path()?)?;
                remove_optional_file_and_sync(&self.recurring_record_path())?;
                self.systemctl(&["daemon-reload"])
            }
            _ => Err("the prior recurring reboot files were incomplete".to_owned()),
        }
    }

    fn power_record_path(&self, operation_id: &str) -> Result<PathBuf, String> {
        validate_operation_id(operation_id)?;
        Ok(self
            .config
            .power_state_root
            .join(format!("{operation_id}.json")))
    }

    fn active_power_record(&self) -> Result<Option<PowerOperationRecord>, String> {
        let records = self.power_records()?;
        for record in records {
            let service = reboot_service_unit(&record.operation_id)?;
            if self.unit_is_active(&format!("{service}.timer"))? || self.unit_is_active(&service)? {
                return Ok(Some(record));
            }
        }
        Ok(None)
    }

    fn power_records(&self) -> Result<Vec<PowerOperationRecord>, String> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.config.power_state_root)
            .map_err(|_| "could not read host power operation state".to_owned())?
        {
            let entry = entry.map_err(|_| "could not read a host power operation".to_owned())?;
            let path = entry.path();
            let Some(id) = path
                .file_name()
                .and_then(|value| value.to_str())
                .and_then(|value| value.strip_suffix(".json"))
            else {
                continue;
            };
            if validate_operation_id(id).is_err() {
                continue;
            }
            let record = read_power_record(&path)?;
            validate_power_record(&record, id)?;
            records.push(record);
            if records.len() > MAX_POWER_RECORDS {
                return Err("too many host power operation records exist".to_owned());
            }
        }
        Ok(records)
    }

    fn scheduled_reboot_status(&self, errors: &mut Vec<Value>) -> Result<Value, String> {
        let mut stale_records = 0_u64;
        for record in self.power_records()? {
            let service = reboot_service_unit(&record.operation_id)?;
            let timer = format!("{service}.timer");
            match (self.unit_is_active(&timer), self.unit_is_active(&service)) {
                (Ok(timer_active), Ok(service_active)) if timer_active || service_active => {
                    return Ok(json!({
                        "operation_id": record.operation_id,
                        "state": if service_active { "executing" } else { "scheduled" },
                        "scheduled_at_unix_ms": record.scheduled_at_unix_ms,
                        "execute_at_unix_ms": record.execute_at_unix_ms,
                        "delay_seconds": record.delay_seconds,
                        "cancellable": timer_active && !service_active
                    }));
                }
                (Ok(_), Ok(_)) => stale_records = stale_records.saturating_add(1),
                _ => push_error(
                    errors,
                    "power",
                    "timer_status_unavailable",
                    "Could not reconcile a recorded reboot timer",
                ),
            }
        }
        Ok(json!({
            "state": "none",
            "cancellable": false,
            "stale_records": stale_records,
            "reconciled": errors.iter().all(|error| error["component"] != "power")
        }))
    }
}

fn validate_config(config: &HostControlConfig) -> Result<(), String> {
    validate_config_shape(config)?;
    for (path, label) in [
        (&config.docker_binary, "Docker"),
        (&config.systemctl_binary, "systemctl"),
        (&config.systemd_run_binary, "systemd-run"),
        (&config.systemd_analyze_binary, "systemd-analyze"),
        (&config.broker_binary, "Helix broker helper"),
        (&config.timeout_binary, "timeout"),
    ] {
        let metadata = fs::metadata(path).ok();
        if !path.is_absolute()
            || metadata.as_ref().is_none_or(|metadata| {
                !metadata.is_file()
                    || metadata.permissions().mode() & 0o111 == 0
                    || metadata.uid() != 0
                    || metadata.mode() & 0o022 != 0
            })
        {
            return Err(format!("the configured {label} executable is unavailable"));
        }
    }
    let unit_root = fs::symlink_metadata(&config.systemd_unit_root)
        .map_err(|_| "the configured systemd unit directory is unavailable".to_owned())?;
    if !unit_root.file_type().is_dir()
        || unit_root.uid() != 0
        || unit_root.mode() & 0o022 != 0
        || fs::canonicalize(&config.systemd_unit_root)
            .map_or(true, |canonical| canonical != config.systemd_unit_root)
    {
        return Err("the configured systemd unit directory is unsafe".to_owned());
    }
    let broker_config = fs::symlink_metadata(&config.broker_config_path)
        .map_err(|_| "the configured Helix broker config is unavailable".to_owned())?;
    if !broker_config.file_type().is_file()
        || broker_config.uid() != 0
        || broker_config.mode() & 0o077 != 0
    {
        return Err("the configured Helix broker config is unsafe".to_owned());
    }
    Ok(())
}

fn validate_config_shape(config: &HostControlConfig) -> Result<(), String> {
    validate_container_name(&config.dashboard_container)?;
    validate_container_name(&config.gateway_container)?;
    if config.dashboard_container == config.gateway_container {
        return Err("dashboard and gateway container names must differ".to_owned());
    }
    validate_unit_name(&config.docker_unit)?;
    validate_unit_name(&config.broker_unit)?;
    if config.hook_services.len() > 32 {
        return Err("too many hook services are configured".to_owned());
    }
    let mut hook_ids = HashSet::with_capacity(config.hook_services.len());
    let mut hook_units = HashSet::with_capacity(config.hook_services.len());
    for hook in &config.hook_services {
        validate_hook_id(&hook.id)?;
        validate_unit_name(&hook.unit)?;
        if !hook_ids.insert(hook.id.as_str()) || !hook_units.insert(hook.unit.as_str()) {
            return Err("hook service identifiers and units must be unique".to_owned());
        }
    }
    if !config.power_state_root.is_absolute()
        || config.power_state_root == Path::new("/")
        || config.power_state_root.components().count() < 3
        || !config.recurring_state_root.is_absolute()
        || config.recurring_state_root == Path::new("/")
        || config.recurring_state_root.components().count() < 3
        || !config.systemd_unit_root.is_absolute()
        || config.systemd_unit_root == Path::new("/")
        || !config.broker_binary.is_absolute()
        || !config.broker_config_path.is_absolute()
        || !config.systemd_analyze_binary.is_absolute()
    {
        return Err("host control paths must be narrow absolute paths".to_owned());
    }
    for path in [&config.broker_binary, &config.broker_config_path] {
        let value = path.to_string_lossy();
        if value.chars().any(|character| {
            character.is_whitespace() || character.is_control() || matches!(character, '\\' | '"')
        }) {
            return Err(
                "Helix recurring reboot paths contain unsafe unit-file characters".to_owned(),
            );
        }
    }
    Ok(())
}

fn validate_hook_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || !id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
    {
        return Err("a configured hook identifier is invalid".to_owned());
    }
    Ok(())
}

fn validate_container_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("a configured Helix container name is invalid".to_owned());
    }
    Ok(())
}

fn validate_unit_name(name: &str) -> Result<(), String> {
    if !name.ends_with(".service")
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'@'))
    {
        return Err("a configured systemd unit name is invalid".to_owned());
    }
    Ok(())
}

fn validate_operation_id(id: &str) -> Result<(), String> {
    let parsed =
        Uuid::parse_str(id).map_err(|_| "host power operation ID is invalid".to_owned())?;
    if parsed.to_string() != id {
        return Err("host power operation ID is invalid".to_owned());
    }
    Ok(())
}

fn validate_recurring_spec(
    spec: &RecurringRebootSpec,
    hostname: &str,
    host_timezone: &str,
) -> Result<Vec<RebootWeekday>, String> {
    if spec.confirmation_hostname != hostname {
        return Err("hostname confirmation does not exactly match this host".to_owned());
    }
    if !spec.disruption_acknowledged {
        return Err("recurring reboot disruption must be acknowledged".to_owned());
    }
    if spec.hour > 23 || spec.minute > 59 {
        return Err("recurring reboot time is invalid".to_owned());
    }
    if spec.timezone != host_timezone {
        return Err(format!(
            "the recurring schedule must use this host's verified timezone: {host_timezone}"
        ));
    }
    validate_timezone(host_timezone)?;
    if spec.weekdays.is_empty() || spec.weekdays.len() > 7 {
        return Err("choose between one and seven weekdays".to_owned());
    }
    let mut weekdays = spec.weekdays.clone();
    weekdays.sort_by_key(|day| weekday_index(*day));
    weekdays.dedup();
    if weekdays.len() != spec.weekdays.len() {
        return Err("recurring reboot weekdays must be unique".to_owned());
    }
    Ok(weekdays)
}

fn validate_recurring_record(record: &RecurringRebootRecord) -> Result<(), String> {
    validate_operation_id(&record.schedule_id)?;
    validate_timezone(&record.timezone)?;
    if record.schema_version != 1
        || record.hostname.is_empty()
        || record.hostname.len() > 253
        || record.hostname.chars().any(char::is_control)
        || record.hour > 23
        || record.minute > 59
        || record.weekdays.is_empty()
        || record.weekdays.len() > 7
        || record.calendar_expression
            != recurring_calendar(
                &record.weekdays,
                record.hour,
                record.minute,
                &record.timezone,
            )
        || !valid_sha256(&record.service_sha256)
        || !valid_sha256(&record.timer_sha256)
        || record.created_at_unix_ms > record.updated_at_unix_ms
    {
        return Err("the recurring reboot record is invalid".to_owned());
    }
    let mut sorted = record.weekdays.clone();
    sorted.sort_by_key(|day| weekday_index(*day));
    sorted.dedup();
    if sorted != record.weekdays {
        return Err("the recurring reboot record is invalid".to_owned());
    }
    Ok(())
}

fn weekday_index(day: RebootWeekday) -> u8 {
    match day {
        RebootWeekday::Monday => 1,
        RebootWeekday::Tuesday => 2,
        RebootWeekday::Wednesday => 3,
        RebootWeekday::Thursday => 4,
        RebootWeekday::Friday => 5,
        RebootWeekday::Saturday => 6,
        RebootWeekday::Sunday => 7,
    }
}

fn weekday_abbreviation(day: RebootWeekday) -> &'static str {
    match day {
        RebootWeekday::Monday => "Mon",
        RebootWeekday::Tuesday => "Tue",
        RebootWeekday::Wednesday => "Wed",
        RebootWeekday::Thursday => "Thu",
        RebootWeekday::Friday => "Fri",
        RebootWeekday::Saturday => "Sat",
        RebootWeekday::Sunday => "Sun",
    }
}

fn recurring_calendar(weekdays: &[RebootWeekday], hour: u8, minute: u8, timezone: &str) -> String {
    let days = if weekdays.len() == 7 {
        String::new()
    } else {
        format!(
            "{} ",
            weekdays
                .iter()
                .map(|day| weekday_abbreviation(*day))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    format!("{days}*-*-* {hour:02}:{minute:02}:00 {timezone}")
}

fn validate_timezone(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || value.starts_with('/')
        || value.contains("..")
        || value.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '/' | '_' | '-' | '+' | '.'))
        })
    {
        return Err("the host timezone is invalid".to_owned());
    }
    Ok(())
}

fn read_host_timezone() -> Result<String, String> {
    if let Ok(body) = fs::read_to_string("/etc/timezone") {
        let timezone = body.trim().to_owned();
        if validate_timezone(&timezone).is_ok() {
            return Ok(timezone);
        }
    }
    if let Ok(target) = fs::read_link("/etc/localtime")
        && let Some(value) = target.to_str()
        && let Some(timezone) = value.split("/zoneinfo/").nth(1)
        && validate_timezone(timezone).is_ok()
    {
        return Ok(timezone.to_owned());
    }
    Err("Helix could not verify the host timezone".to_owned())
}

fn host_timezone() -> Value {
    read_host_timezone().map_or(Value::Null, Value::String)
}

fn recurring_service_unit(config: &HostControlConfig, schedule_id: &str) -> Result<String, String> {
    validate_operation_id(schedule_id)?;
    Ok(format!(
        "{RECURRING_UNIT_MARKER}\n[Unit]\nDescription=Helix recurring reboot safety gate\nRequires={}\nAfter={}\n\n[Service]\nType=oneshot\nExecStart={} --config {} --trigger-recurring-reboot {}\nNoNewPrivileges=yes\nPrivateTmp=yes\nPrivateDevices=yes\nProtectSystem=strict\nProtectHome=yes\nRestrictAddressFamilies=AF_UNIX\nLockPersonality=yes\nMemoryDenyWriteExecute=yes\nCapabilityBoundingSet=\n",
        config.broker_unit,
        config.broker_unit,
        config.broker_binary.display(),
        config.broker_config_path.display(),
        schedule_id
    ))
}

fn recurring_timer_unit(calendar_expression: &str) -> String {
    format!(
        "{RECURRING_UNIT_MARKER}\n[Unit]\nDescription=Helix recurring host reboot schedule\n\n[Timer]\nOnCalendar={calendar_expression}\nAccuracySec=1min\nRandomizedDelaySec=0\nPersistent=false\nUnit={RECURRING_SERVICE_UNIT}\n\n[Install]\nWantedBy=timers.target\n"
    )
}

fn recurring_unit_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !root.is_absolute()
        || !matches!(name, RECURRING_SERVICE_UNIT | RECURRING_TIMER_UNIT)
        || name.contains('/')
    {
        return Err("the recurring reboot unit path is invalid".to_owned());
    }
    Ok(root.join(name))
}

fn sha256_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(value);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_optional_regular_file(path: &Path, maximum: u64) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("could not inspect a recurring reboot file".to_owned()),
    };
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > maximum
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return Err("a recurring reboot file is unsafe".to_owned());
    }
    fs::read(path)
        .map(Some)
        .map_err(|_| "could not read a recurring reboot file".to_owned())
}

fn write_unit_file(path: &Path, body: &[u8]) -> Result<(), String> {
    write_atomic_file(path, body, 0o644, "recurring reboot unit")
}

fn write_private_file(path: &Path, body: &[u8]) -> Result<(), String> {
    write_atomic_file(path, body, 0o600, "recurring reboot state")
}

fn write_atomic_file(path: &Path, body: &[u8], mode: u32, label: &str) -> Result<(), String> {
    if body.is_empty() || body.len() > 64 * 1024 {
        return Err(format!("the {label} exceeds supported bounds"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("the {label} path is invalid"))?;
    let temporary = parent.join(format!(".helix-{}.partial", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|_| format!("could not stage the {label}"))?;
        file.write_all(body)
            .and_then(|()| file.sync_all())
            .map_err(|_| format!("could not persist the {label}"))?;
        fs::rename(&temporary, path).map_err(|_| format!("could not publish the {label}"))?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| format!("could not sync the {label} directory"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn remove_file_and_sync(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|_| "could not remove a recurring reboot file".to_owned())?;
    sync_parent(path, "recurring reboot file")
}

fn remove_optional_file_and_sync(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => sync_parent(path, "recurring reboot file"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("could not remove a recurring reboot file".to_owned()),
    }
}

fn sync_parent(path: &Path, label: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("the {label} path is invalid"))?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| format!("could not sync the {label} directory"))
}

fn prepare_private_directory(path: &Path, label: &str) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| format!("could not create {label}"))?;
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("could not inspect {label}"))?;
    if !metadata.file_type().is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(format!("the {label} directory is invalid"));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| format!("could not protect {label}"))
}

fn reboot_service_unit(operation_id: &str) -> Result<String, String> {
    validate_operation_id(operation_id)?;
    Ok(format!("helix-reboot-{}", operation_id.replace('-', "")))
}

fn container_json(container: ContainerStatus) -> Value {
    json!({
        "name": container.name,
        "running": container.running,
        "health": container.health,
        "restart_count": container.restart_count,
        "restart_policy": container.restart_policy.name,
        "restart_maximum_retry_count": container.restart_policy.maximum_retry_count,
        "oom_killed": container.oom_killed,
        "state_error": container.state_error
    })
}

fn parse_docker_stats(output: &str, dashboard: &str, gateway: &str) -> Result<Value, String> {
    let mut values = serde_json::Map::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)
            .map_err(|_| "Docker returned invalid resource statistics".to_owned())?;
        let name = value
            .get("Name")
            .and_then(Value::as_str)
            .filter(|name| *name == dashboard || *name == gateway)
            .ok_or_else(|| "Docker returned statistics for an unexpected container".to_owned())?;
        let role = if name == dashboard {
            "dashboard"
        } else {
            "gateway"
        };
        if values.contains_key(role) {
            return Err("Docker returned duplicate resource statistics".to_owned());
        }
        let cpu_percent = value
            .get("CPUPerc")
            .and_then(Value::as_str)
            .and_then(|value| value.trim_end_matches('%').parse::<f64>().ok());
        let memory_used_bytes = value
            .get("MemUsage")
            .and_then(Value::as_str)
            .and_then(|value| value.split_once('/').map(|(used, _)| used))
            .and_then(parse_human_bytes);
        let pids = value
            .get("PIDs")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u64>().ok());
        values.insert(
            role.to_owned(),
            json!({
                "cpu_percent": cpu_percent,
                "memory_used_bytes": memory_used_bytes,
                "pids": pids
            }),
        );
    }
    if values.len() != 2 || !values.contains_key("dashboard") || !values.contains_key("gateway") {
        return Err(
            "Docker omitted resource statistics for a configured Helix container".to_owned(),
        );
    }
    Ok(Value::Object(values))
}

fn broker_process_resources() -> Result<Value, String> {
    let content = fs::read_to_string("/proc/self/status")
        .map_err(|_| "could not read broker process status".to_owned())?;
    let mut rss_bytes = None;
    let mut peak_rss_bytes = None;
    let mut threads = None;
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            rss_bytes = parse_proc_kib(value);
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            peak_rss_bytes = parse_proc_kib(value);
        } else if let Some(value) = line.strip_prefix("Threads:") {
            threads = value.trim().parse::<u64>().ok();
        }
    }
    Ok(json!({
        "pid": std::process::id(),
        "rss_bytes": rss_bytes,
        "peak_rss_bytes": peak_rss_bytes,
        "threads": threads
    }))
}

fn parse_proc_kib(value: &str) -> Option<u64> {
    let kib = value.split_whitespace().next()?.parse::<u64>().ok()?;
    kib.checked_mul(1024)
}

pub(crate) fn parse_human_bytes(value: &str) -> Option<u64> {
    let value = value.trim();
    let split = value
        .bytes()
        .position(|byte| !byte.is_ascii_digit() && byte != b'.')?;
    let number = value[..split].parse::<f64>().ok()?;
    let multiplier = match value[split..].trim() {
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

pub(crate) fn require_success(output: CommandOutput) -> Result<CommandOutput, String> {
    if output.success {
        return Ok(output);
    }
    let message = first_line(&output.stderr)
        .or_else(|| first_line(&output.stdout))
        .unwrap_or("command failed");
    Err(message.chars().take(500).collect())
}

pub(crate) fn first_line(value: &str) -> Option<&str> {
    value.lines().map(str::trim).find(|line| !line.is_empty())
}

fn sanitize_state(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(64)
        .collect()
}

fn recognized_systemctl_state(verb: &str, state: &str) -> bool {
    match verb {
        "is-active" => matches!(
            state,
            "active"
                | "reloading"
                | "inactive"
                | "failed"
                | "activating"
                | "deactivating"
                | "maintenance"
                | "refreshing"
                | "unknown"
        ),
        "is-enabled" => matches!(
            state,
            "enabled"
                | "enabled-runtime"
                | "linked"
                | "linked-runtime"
                | "alias"
                | "masked"
                | "masked-runtime"
                | "static"
                | "indirect"
                | "disabled"
                | "generated"
                | "transient"
                | "not-found"
        ),
        _ => false,
    }
}

fn push_error(errors: &mut Vec<Value>, component: &str, code: &str, message: &str) {
    if errors.len() >= 32 {
        return;
    }
    errors.push(json!({
        "component": component,
        "code": code,
        "message": message.chars().take(500).collect::<String>()
    }));
}

fn current_hostname() -> Result<String, String> {
    let hostname = rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .into_owned();
    if hostname.is_empty()
        || hostname.len() > 253
        || hostname.chars().any(|character| character.is_control())
    {
        return Err("the current hostname is invalid".to_owned());
    }
    Ok(hostname)
}

fn write_power_record(path: &Path, record: &PowerOperationRecord) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(record)
        .map_err(|_| "could not encode the host power operation".to_owned())?;
    let temporary = path.with_extension(format!("partial.{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "could not stage the host power operation".to_owned())?;
        file.write_all(&body)
            .and_then(|()| file.sync_all())
            .map_err(|_| "could not persist the host power operation".to_owned())?;
        fs::rename(&temporary, path)
            .map_err(|_| "could not commit the host power operation".to_owned())?;
        sync_power_state_directory(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn remove_power_record_durable(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|_| "could not remove the host power operation".to_owned())?;
    sync_power_state_directory(path)
}

fn sync_power_state_directory(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "the host power operation path is invalid".to_owned())?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "could not sync host power operation state".to_owned())
}

fn read_power_record(path: &Path) -> Result<PowerOperationRecord, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "the host power operation does not exist".to_owned())?;
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_POWER_RECORD_BYTES
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err("the host power operation record is invalid".to_owned());
    }
    serde_json::from_slice(
        &fs::read(path).map_err(|_| "could not read the host power operation".to_owned())?,
    )
    .map_err(|_| "the host power operation record is invalid".to_owned())
}

fn validate_power_record(record: &PowerOperationRecord, operation_id: &str) -> Result<(), String> {
    let expected_execute_at = record
        .scheduled_at_unix_ms
        .saturating_add(u64::from(record.delay_seconds).saturating_mul(1_000));
    if record.schema_version != 1
        || record.operation_id != operation_id
        || record.hostname.is_empty()
        || record.hostname.len() > 253
        || record.hostname.chars().any(char::is_control)
        || record.delay_seconds < 10
        || record.delay_seconds > 300
        || record.execute_at_unix_ms != expected_execute_at
    {
        return Err("the host power operation record is invalid".to_owned());
    }
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

fn default_docker_binary() -> PathBuf {
    PathBuf::from("/usr/bin/docker")
}

fn default_dashboard_container() -> String {
    "server-dashboard".to_owned()
}

fn default_gateway_container() -> String {
    "server-dashboard-gateway".to_owned()
}

fn default_systemctl_binary() -> PathBuf {
    PathBuf::from("/usr/bin/systemctl")
}

fn default_systemd_run_binary() -> PathBuf {
    PathBuf::from("/usr/bin/systemd-run")
}

fn default_systemd_analyze_binary() -> PathBuf {
    PathBuf::from("/usr/bin/systemd-analyze")
}

fn default_timeout_binary() -> PathBuf {
    PathBuf::from("/usr/bin/timeout")
}

fn default_docker_unit() -> String {
    "docker.service".to_owned()
}

fn default_broker_unit() -> String {
    "helix-privd.service".to_owned()
}

fn default_power_state_root() -> PathBuf {
    PathBuf::from("/run/helix/power")
}

fn default_recurring_state_root() -> PathBuf {
    PathBuf::from("/var/lib/helix/power")
}

fn default_systemd_unit_root() -> PathBuf {
    PathBuf::from("/etc/systemd/system")
}

fn default_broker_binary() -> PathBuf {
    PathBuf::from("/usr/local/libexec/helix-privd")
}

fn default_broker_config_path() -> PathBuf {
    PathBuf::from("/etc/helix/privd.json")
}

fn default_hook_services() -> Vec<HookServiceConfig> {
    [
        ("plex", "plexmediaserver.service"),
        ("tailscale", "tailscaled.service"),
        ("pterodactyl", "wings.service"),
        ("jellyfin", "jellyfin.service"),
    ]
    .into_iter()
    .map(|(id, unit)| HookServiceConfig {
        id: id.to_owned(),
        unit: unit.to_owned(),
    })
    .collect()
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
        fn push(&self, output: CommandOutput) {
            self.outputs.lock().unwrap().push_back(Ok(output));
        }

        fn push_error(&self, message: &str) {
            self.outputs
                .lock()
                .unwrap()
                .push_back(Err(message.to_owned()));
        }

        fn calls(&self) -> Vec<(PathBuf, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl HostCommandRunner for MockRunner {
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

    fn success(stdout: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }

    fn inspect(name: &str, policy: &str) -> CommandOutput {
        success(&json!([{
            "Name": format!("/{name}"),
            "RestartCount": 0,
            "State": {"Running": true, "OOMKilled": false, "Error": "", "Health": {"Status": "healthy"}},
            "HostConfig": {"RestartPolicy": {"Name": policy, "MaximumRetryCount": 0}}
        }])
        .to_string())
    }

    fn config(root: PathBuf) -> HostControlConfig {
        HostControlConfig {
            dashboard_container: "server-dashboard".to_owned(),
            gateway_container: "server-dashboard-gateway".to_owned(),
            docker_binary: PathBuf::from("/usr/bin/docker"),
            systemctl_binary: PathBuf::from("/usr/bin/systemctl"),
            systemd_run_binary: PathBuf::from("/usr/bin/systemd-run"),
            systemd_analyze_binary: PathBuf::from("/usr/bin/systemd-analyze"),
            timeout_binary: PathBuf::from("/usr/bin/timeout"),
            docker_unit: "docker.service".to_owned(),
            broker_unit: "helix-privd.service".to_owned(),
            power_state_root: root.join("transient"),
            recurring_state_root: root.join("recurring"),
            systemd_unit_root: root.join("units"),
            broker_binary: PathBuf::from("/usr/local/libexec/helix-privd"),
            broker_config_path: PathBuf::from("/etc/helix/privd.json"),
            hook_services: default_hook_services(),
        }
    }

    #[test]
    fn start_on_boot_changes_only_the_two_configured_restart_policies() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(inspect("server-dashboard", "no"));
        runner.push(inspect("server-dashboard-gateway", "no"));
        runner.push(success("server-dashboard\n"));
        runner.push(success("server-dashboard-gateway\n"));
        runner.push(inspect("server-dashboard", "unless-stopped"));
        runner.push(inspect("server-dashboard-gateway", "unless-stopped"));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        let result = control.set_start_on_boot(true).unwrap();
        assert_eq!(result["enabled"], true);
        let calls = runner.calls();
        let updates = calls
            .iter()
            .filter(|(_, args)| args.first().is_some_and(|arg| arg == "update"))
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 2);
        assert_eq!(
            updates[0].1,
            ["update", "--restart", "unless-stopped", "server-dashboard"]
        );
        assert_eq!(
            updates[1].1,
            [
                "update",
                "--restart",
                "unless-stopped",
                "server-dashboard-gateway"
            ]
        );
        assert!(calls.iter().all(|(_, args)| {
            !args
                .iter()
                .any(|arg| matches!(arg.as_str(), "start" | "stop" | "restart"))
        }));
    }

    #[test]
    fn status_reads_only_exact_helix_services_containers_and_resources() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        for state in ["enabled\n", "active\n", "enabled\n", "active\n"] {
            runner.push(success(state));
        }
        runner.push(inspect("server-dashboard", "unless-stopped"));
        runner.push(inspect("server-dashboard-gateway", "unless-stopped"));
        runner.push(success(
            "{\"Name\":\"server-dashboard\",\"CPUPerc\":\"1.5%\",\"MemUsage\":\"64MiB / 1GiB\",\"PIDs\":\"8\"}\n{\"Name\":\"server-dashboard-gateway\",\"CPUPerc\":\"0.2%\",\"MemUsage\":\"8MiB / 1GiB\",\"PIDs\":\"3\"}\n",
        ));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        let status = control.status().unwrap();
        assert_eq!(status["availability"], "ready");
        assert_eq!(status["services"]["docker"]["active"], true);
        assert_eq!(status["services"]["helix_privd"]["enabled"], true);
        assert_eq!(status["start_on_boot"]["enabled"], true);
        assert_eq!(
            status["resources"]["containers"]["dashboard"]["memory_used_bytes"],
            64 * 1024 * 1024
        );
        let calls = runner.calls();
        assert!(calls.iter().all(|(_, args)| {
            !args.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "update" | "start" | "stop" | "restart" | "reboot"
                )
            })
        }));
        let stats = calls
            .iter()
            .find(|(_, args)| args.first().is_some_and(|arg| arg == "stats"))
            .expect("Docker stats call");
        assert_eq!(stats.1.len(), 6);
        assert_eq!(stats.1[4], "server-dashboard");
        assert_eq!(stats.1[5], "server-dashboard-gateway");
    }

    #[test]
    fn hook_inventory_is_read_only_and_reports_missing_services_truthfully() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(success("not-found\n"));
        runner.push(success("unknown\n"));
        let mut settings = config(temporary.path().join("power"));
        settings.hook_services = vec![HookServiceConfig {
            id: "tailscale".to_owned(),
            unit: "tailscaled.service".to_owned(),
        }];
        let control = HostControl::with_runner(settings, runner.clone()).unwrap();

        let inventory = control.hook_inventory().unwrap();

        assert_eq!(inventory["hooks"][0]["id"], "tailscale");
        assert_eq!(inventory["hooks"][0]["installed"], false);
        assert_eq!(inventory["hooks"][0]["actions"], json!([]));
        assert!(runner.calls().iter().all(|(_, args)| {
            matches!(
                args.first().map(String::as_str),
                Some("is-enabled" | "is-active")
            )
        }));
    }

    #[test]
    fn hook_service_action_uses_only_the_configured_exact_unit_and_verifies_it() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(success("enabled\n"));
        runner.push(success("active\n"));
        runner.push(success(""));
        runner.push(success("enabled\n"));
        runner.push(success("active\n"));
        let mut settings = config(temporary.path().join("power"));
        settings.hook_services = vec![HookServiceConfig {
            id: "plex".to_owned(),
            unit: "plexmediaserver.service".to_owned(),
        }];
        let control = HostControl::with_runner(settings, runner.clone()).unwrap();

        let result = control
            .manage_hook_service("plex", HookServiceAction::Restart)
            .unwrap();

        assert_eq!(result["verified"], true);
        assert!(runner.calls().iter().any(|(program, args)| {
            program == Path::new("/usr/bin/systemctl")
                && args == &["restart", "plexmediaserver.service"]
        }));
        assert!(runner.calls().iter().all(|(_, args)| {
            !args
                .iter()
                .any(|arg| arg.contains(';') || arg.contains("../"))
        }));
    }

    #[test]
    fn unknown_or_malformed_hook_ids_never_run_commands() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();

        assert!(
            control
                .manage_hook_service("plex; reboot", HookServiceAction::Restart)
                .is_err()
        );
        assert!(
            control
                .manage_hook_service("unknown", HookServiceAction::Restart)
                .is_err()
        );
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn failed_second_policy_update_rolls_back_the_first_exactly() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(inspect("server-dashboard", "on-failure"));
        runner.push(inspect("server-dashboard-gateway", "no"));
        runner.push(success("server-dashboard\n"));
        runner.push(CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "update rejected".to_owned(),
        });
        runner.push(success("server-dashboard-gateway\n"));
        runner.push(success("server-dashboard\n"));
        runner.push(inspect("server-dashboard", "on-failure"));
        runner.push(inspect("server-dashboard-gateway", "no"));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        assert!(control.set_start_on_boot(true).is_err());
        let calls = runner.calls();
        assert!(calls.iter().any(|(_, args)| {
            args == &["update", "--restart", "on-failure", "server-dashboard"]
        }));
    }

    #[test]
    fn unsupported_original_restart_policy_is_rejected_before_mutation() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(inspect("server-dashboard", "unexpected"));
        runner.push(inspect("server-dashboard-gateway", "no"));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        assert!(control.set_start_on_boot(true).is_err());
        assert!(
            runner
                .calls()
                .iter()
                .all(|(_, args)| args.first().is_some_and(|arg| arg == "inspect"))
        );
    }

    #[test]
    fn failed_policy_verification_rolls_back_every_changed_target() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(inspect("server-dashboard", "no"));
        runner.push(inspect("server-dashboard-gateway", "no"));
        runner.push(success("server-dashboard\n"));
        runner.push(success("server-dashboard-gateway\n"));
        runner.push_error("inspect unavailable");
        runner.push(success("server-dashboard-gateway\n"));
        runner.push(success("server-dashboard\n"));
        runner.push(inspect("server-dashboard", "no"));
        runner.push(inspect("server-dashboard-gateway", "no"));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        assert!(
            control
                .set_start_on_boot(true)
                .unwrap_err()
                .contains("original policies were restored")
        );
        let updates = runner
            .calls()
            .into_iter()
            .filter(|(_, args)| args.first().is_some_and(|arg| arg == "update"))
            .map(|(_, args)| args)
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 4);
        assert_eq!(
            updates[2],
            ["update", "--restart", "no", "server-dashboard-gateway"]
        );
        assert_eq!(
            updates[3],
            ["update", "--restart", "no", "server-dashboard"]
        );
    }

    #[test]
    fn ambiguous_first_policy_update_rolls_back_and_verifies_both_targets() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(inspect("server-dashboard", "no"));
        runner.push(inspect("server-dashboard-gateway", "no"));
        runner.push(CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "client timed out after Docker accepted the request".to_owned(),
        });
        runner.push(success("server-dashboard-gateway\n"));
        runner.push(success("server-dashboard\n"));
        runner.push(inspect("server-dashboard", "no"));
        runner.push(inspect("server-dashboard-gateway", "no"));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();

        assert!(
            control
                .set_start_on_boot(true)
                .unwrap_err()
                .contains("was not changed")
        );
        let updates = runner
            .calls()
            .into_iter()
            .filter(|(_, args)| args.first().is_some_and(|arg| arg == "update"))
            .map(|(_, args)| args)
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 3);
        assert_eq!(
            updates[1],
            ["update", "--restart", "no", "server-dashboard-gateway"]
        );
        assert_eq!(
            updates[2],
            ["update", "--restart", "no", "server-dashboard"]
        );
    }

    #[test]
    fn reboot_schedule_uses_a_transient_timer_and_never_a_shell() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(success(""));
        runner.push(success("active\n"));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        let hostname = current_hostname().unwrap();
        let result = control.schedule_reboot(&hostname, 30, true).unwrap();
        assert_eq!(result["state"], "scheduled");
        let calls = runner.calls();
        let schedule = &calls[0];
        assert_eq!(schedule.0, PathBuf::from("/usr/bin/systemd-run"));
        assert!(schedule.1.iter().any(|arg| arg == "--on-active=30s"));
        assert!(schedule.1.iter().any(|arg| arg == "/usr/bin/systemctl"));
        assert_eq!(schedule.1.last().map(String::as_str), Some("reboot"));
        assert!(schedule.1.iter().all(|arg| arg != "sh" && arg != "bash"));
    }

    #[test]
    fn reboot_validation_rejects_without_running_any_command() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        assert!(control.schedule_reboot("wrong-host", 30, true).is_err());
        assert!(
            control
                .schedule_reboot(&current_hostname().unwrap(), 9, true)
                .is_err()
        );
        assert!(
            control
                .schedule_reboot(&current_hostname().unwrap(), 301, true)
                .is_err()
        );
        assert!(
            control
                .schedule_reboot(&current_hostname().unwrap(), 30, false)
                .is_err()
        );
        assert!(control.cancel_reboot("../../reboot").is_err());
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn reboot_timer_is_cancelled_by_exact_opaque_operation_id() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(success(""));
        runner.push(success("active\n"));
        runner.push(success("inactive\n"));
        runner.push(success("active\n"));
        runner.push(success(""));
        runner.push(success("inactive\n"));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        let operation_id = control
            .schedule_reboot(&current_hostname().unwrap(), 30, true)
            .unwrap()["operation_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let cancelled = control.cancel_reboot(&operation_id).unwrap();
        assert_eq!(cancelled["state"], "cancelled");
        assert!(!control.power_record_path(&operation_id).unwrap().exists());
        let timer = format!("{}.timer", reboot_service_unit(&operation_id).unwrap());
        assert!(runner.calls().iter().any(|(program, args)| {
            program == Path::new("/usr/bin/systemctl")
                && args == &["stop".to_owned(), timer.clone()]
        }));
    }

    #[test]
    fn ambiguous_schedule_failure_attempts_fail_safe_timer_cleanup() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "systemd-run failed".to_owned(),
        });
        runner.push(success(""));
        runner.push(success("inactive\n"));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        assert!(
            control
                .schedule_reboot(&current_hostname().unwrap(), 30, true)
                .unwrap_err()
                .contains("could not schedule")
        );
        assert!(runner.calls().iter().any(|(program, args)| {
            program == Path::new("/usr/bin/systemctl")
                && args.first().is_some_and(|arg| arg == "stop")
                && args.get(1).is_some_and(|unit| {
                    unit.starts_with("helix-reboot-") && unit.ends_with(".timer")
                })
        }));
        assert_eq!(
            fs::read_dir(&control.config.power_state_root)
                .unwrap()
                .count(),
            0
        );
    }

    fn recurring_spec() -> RecurringRebootSpec {
        RecurringRebootSpec {
            weekdays: vec![
                RebootWeekday::Monday,
                RebootWeekday::Wednesday,
                RebootWeekday::Friday,
            ],
            hour: 5,
            minute: 30,
            timezone: read_host_timezone().unwrap(),
            confirmation_hostname: current_hostname().unwrap(),
            disruption_acknowledged: true,
        }
    }

    fn recurring_timer_listing() -> CommandOutput {
        success(
            &json!([{
                "next": 1_800_050_000_000_000_u64,
                "left": 1_u64,
                "last": 0_u64,
                "passed": 0_u64,
                "unit": RECURRING_TIMER_UNIT,
                "activates": RECURRING_SERVICE_UNIT
            }])
            .to_string(),
        )
    }

    #[test]
    fn recurring_reboot_uses_verified_units_and_the_broker_safety_gate() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push(success("calendar valid\n"));
        runner.push(success(""));
        runner.push(success(""));
        runner.push(success("active\n"));
        runner.push(success("enabled\n"));
        runner.push(recurring_timer_listing());
        runner.push(success("active\n"));
        runner.push(success("enabled\n"));
        runner.push(success(""));
        runner.push(success("inactive\n"));
        runner.push(success(""));
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();

        let scheduled = control.set_recurring_reboot(recurring_spec()).unwrap();
        assert_eq!(scheduled["state"], "scheduled");
        assert_eq!(scheduled["next_at_unix_ms"], 1_800_050_000_000_u64);
        assert_eq!(scheduled["automatic_reboot_without_preflight"], false);
        let schedule_id = scheduled["schedule_id"].as_str().unwrap().to_owned();
        assert_eq!(
            control.verify_recurring_trigger(&schedule_id).unwrap(),
            current_hostname().unwrap()
        );

        let service = fs::read_to_string(control.recurring_service_path().unwrap()).unwrap();
        let timer = fs::read_to_string(control.recurring_timer_path().unwrap()).unwrap();
        assert!(service.contains("--trigger-recurring-reboot"));
        assert!(service.contains("RestrictAddressFamilies=AF_UNIX"));
        assert!(!service.contains("systemctl reboot"));
        assert!(timer.contains("Persistent=false"));
        assert!(timer.contains("Mon,Wed,Fri *-*-* 05:30:00"));
        assert_eq!(
            fs::metadata(control.recurring_record_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let removed = control
            .delete_recurring_reboot(&current_hostname().unwrap())
            .unwrap();
        assert_eq!(removed["state"], "removed");
        assert!(!control.recurring_record_path().exists());
        assert!(!control.recurring_service_path().unwrap().exists());
        assert!(!control.recurring_timer_path().unwrap().exists());
        assert!(
            runner
                .calls()
                .iter()
                .all(|(_, args)| args.iter().all(|argument| argument != "reboot"))
        );
    }

    #[test]
    fn recurring_reboot_validation_rejects_before_writing_or_running_commands() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        let control =
            HostControl::with_runner(config(temporary.path().join("power")), runner.clone())
                .unwrap();
        let mut invalid = recurring_spec();
        invalid.timezone = "Etc/UTC".to_owned();
        invalid.weekdays = Vec::new();

        assert!(control.set_recurring_reboot(invalid).is_err());
        assert!(runner.calls().is_empty());
        assert!(!control.recurring_record_path().exists());
        assert!(!control.recurring_service_path().unwrap().exists());
        assert!(!control.recurring_timer_path().unwrap().exists());
    }

    #[test]
    fn byte_parsers_are_bounded_and_unit_aware() {
        assert_eq!(parse_human_bytes("512MiB"), Some(512 * 1024 * 1024));
        assert_eq!(parse_proc_kib("  1024 kB"), Some(1024 * 1024));
        assert_eq!(parse_human_bytes("unknown"), None);
        assert!(parse_docker_stats("", "dashboard", "gateway").is_err());
        assert!(
            parse_docker_stats(
                "{\"Name\":\"dashboard\",\"CPUPerc\":\"0%\",\"MemUsage\":\"1MiB / 2MiB\",\"PIDs\":\"1\"}\n{\"Name\":\"dashboard\",\"CPUPerc\":\"0%\",\"MemUsage\":\"1MiB / 2MiB\",\"PIDs\":\"1\"}",
                "dashboard",
                "gateway"
            )
            .is_err()
        );
        assert!(recognized_systemctl_state("is-active", "inactive"));
        assert!(!recognized_systemctl_state(
            "is-active",
            "Failedtoconnecttobus"
        ));
    }
}
