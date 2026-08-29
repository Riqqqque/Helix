use crate::bounded_command::run_bounded_command;
use crate::host::HostControlConfig;
use crate::native::prepare_docker_cli_home;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const USER_AGENT: &str = concat!(
    "Helix/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Riqqqque/Helix)"
);
const REQUIRED_CONFIRMATION: &str = "UPDATE HELIX";
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_COMMAND_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SHA256SUMS_BYTES: u64 = 64 * 1024;
const MAX_RELEASE_NOTES_CHARS: usize = 4_000;
const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const DISK_HEADROOM_BYTES: u64 = 1024 * 1024 * 1024;
const DOCKER_BUILD_TIMEOUT: Duration = Duration::from_secs(3_600);
const DOCKER_UP_TIMEOUT: Duration = Duration::from_secs(300);
const CURL_API_TIMEOUT: Duration = Duration::from_secs(30);
const CURL_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const GITHUB_API_HOST: &str = "api.github.com";
const ROLLBACK_DASHBOARD_TAG: &str = "helix-rollback";
const PENDING_SCHEMA: u32 = 1;

pub const FINALIZE_UNIT: &str = "helix-finalize-update.service";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelixUpdateConfig {
    #[serde(default = "default_github_owner")]
    pub github_owner: String,
    #[serde(default = "default_github_repo")]
    pub github_repo: String,
    #[serde(default)]
    pub compose_project_directory: Option<PathBuf>,
    #[serde(default = "default_update_state_root")]
    pub state_root: PathBuf,
    #[serde(default = "default_curl_binary")]
    pub curl_binary: PathBuf,
    #[serde(default = "default_tar_binary")]
    pub tar_binary: PathBuf,
    #[serde(default = "default_timeout_binary")]
    pub timeout_binary: PathBuf,
    #[serde(default = "default_privd_install_path")]
    pub privd_install_path: PathBuf,
    #[serde(default = "default_terminald_install_path")]
    pub terminald_install_path: PathBuf,
    #[serde(default = "default_finalize_unit")]
    pub finalize_unit: String,
}

impl Default for HelixUpdateConfig {
    fn default() -> Self {
        Self {
            github_owner: default_github_owner(),
            github_repo: default_github_repo(),
            compose_project_directory: None,
            state_root: default_update_state_root(),
            curl_binary: default_curl_binary(),
            tar_binary: default_tar_binary(),
            timeout_binary: default_timeout_binary(),
            privd_install_path: default_privd_install_path(),
            terminald_install_path: default_terminald_install_path(),
            finalize_unit: default_finalize_unit(),
        }
    }
}

pub struct HelixUpdateManager {
    config: HelixUpdateConfig,
    host: HostControlConfig,
    docker_state_root: PathBuf,
    mutation: Mutex<()>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReleasePlan {
    tag: String,
    version: String,
    name: String,
    notes: String,
    html_url: String,
    published_at: Option<String>,
    source_name: String,
    source_url: String,
    checksums_url: String,
    source_sha256: String,
    source_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PendingUpdate {
    schema_version: u32,
    target_version: String,
    target_tag: String,
    image_tag: String,
    source_revision: String,
    compose_file: PathBuf,
    env_file: PathBuf,
    project_name: String,
    privd_new: PathBuf,
    terminald_new: PathBuf,
    privd_backup: PathBuf,
    terminald_backup: PathBuf,
    dashboard_previous_id: String,
    gateway_previous_id: String,
    unrelated_container_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Clone, Debug)]
struct ComposeContext {
    project_name: String,
    env_file: PathBuf,
}

impl HelixUpdateManager {
    pub fn new(
        config: HelixUpdateConfig,
        host: HostControlConfig,
        native_state_root: Option<PathBuf>,
    ) -> Result<Self, String> {
        validate_github_identity(&config.github_owner, &config.github_repo)?;
        if config.finalize_unit != FINALIZE_UNIT {
            return Err("the Helix finalize unit name is not the shipped unit".to_owned());
        }
        Ok(Self {
            docker_state_root: native_state_root.unwrap_or_else(|| PathBuf::from("/var/lib/helix")),
            config,
            host,
            mutation: Mutex::new(()),
        })
    }

    pub fn status(&self, refresh: bool) -> Value {
        match self.status_inner(refresh) {
            Ok(value) => value,
            Err(error) => unavailable(
                "github_release_check_failed",
                format!("Helix could not check GitHub for a newer release: {error}"),
            ),
        }
    }

    pub fn apply(
        &self,
        target_tag: &str,
        confirmation: &str,
        disruption_acknowledged: bool,
        progress: &dyn Fn(&str, u8),
    ) -> Result<Value, String> {
        if confirmation != REQUIRED_CONFIRMATION {
            return Err(format!(
                "type {REQUIRED_CONFIRMATION} to apply a Helix release"
            ));
        }
        if !disruption_acknowledged {
            return Err(
                "acknowledge that the Helix dashboard will restart before applying a release"
                    .to_owned(),
            );
        }
        let _guard = self
            .mutation
            .try_lock()
            .map_err(|_| "another Helix update is already running".to_owned())?;
        progress("Checking the GitHub release", 5);
        let plan = self.load_plan(true)?;
        if plan.tag != target_tag {
            return Err(format!(
                "GitHub latest is {}, not {target_tag}; check again before applying",
                plan.tag
            ));
        }
        if !version_is_newer(&plan.version, env!("CARGO_PKG_VERSION"))? {
            return Err(format!(
                "this host is already on Helix {} or newer",
                env!("CARGO_PKG_VERSION")
            ));
        }
        let compose = self.compose_context()?;
        self.require_tools()?;
        prepare_private_directory(&self.config.state_root, "Helix update state")?;
        require_disk_headroom(&self.config.state_root, DISK_HEADROOM_BYTES)?;
        let staging = self.config.state_root.join("staging");
        let pending_dir = self.config.state_root.join("pending");
        let rollback_dir = self.config.state_root.join("rollback");
        reset_directory(&staging)?;
        reset_directory(&pending_dir)?;
        prepare_private_directory(&rollback_dir, "Helix update rollback")?;

        progress("Downloading the digest-pinned source archive", 15);
        let sums_path = staging.join("SHA256SUMS");
        self.download_https(
            &plan.checksums_url,
            &sums_path,
            MAX_SHA256SUMS_BYTES,
            CURL_API_TIMEOUT,
        )?;
        let listed = parse_sha256sums(
            &fs::read_to_string(&sums_path).map_err(|_| "could not read SHA256SUMS".to_owned())?,
            &plan.source_name,
        )?;
        if !plan.source_sha256.is_empty() && listed != plan.source_sha256 {
            return Err("SHA256SUMS does not match the GitHub asset digest".to_owned());
        }
        let archive = staging.join(&plan.source_name);
        self.download_https(
            &plan.source_url,
            &archive,
            MAX_SOURCE_BYTES,
            CURL_DOWNLOAD_TIMEOUT,
        )?;
        let actual = file_sha256(&archive)?;
        if !actual.eq_ignore_ascii_case(&listed) {
            let _ = fs::remove_file(&archive);
            return Err("the Helix source archive failed its SHA-256 digest".to_owned());
        }
        progress("Unpacking the verified source", 25);
        let listing = self.run_program(
            &self.config.tar_binary,
            &["-tzf".to_owned(), archive.to_string_lossy().into_owned()],
            Duration::from_secs(60),
            &[],
        )?;
        require_safe_archive_paths(&listing, &plan.version)?;
        self.run_program(
            &self.config.tar_binary,
            &[
                "--no-same-owner".to_owned(),
                "--no-same-permissions".to_owned(),
                "--no-absolute-filenames".to_owned(),
                "-C".to_owned(),
                staging.to_string_lossy().into_owned(),
                "-xzf".to_owned(),
                archive.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(120),
            &[],
        )?;
        let source_root = staging.join(format!("helix-{}", plan.version));
        let compose_file = source_root.join("compose.yaml");
        if !compose_file.is_file() || !source_root.join("Dockerfile").is_file() {
            return Err("the verified archive is missing compose.yaml or Dockerfile".to_owned());
        }

        progress("Recording a Helix-only rollback snapshot", 35);
        let dashboard_previous_id = self.container_id(&self.host.dashboard_container)?;
        let gateway_previous_id = self.container_id(&self.host.gateway_container)?;
        let unrelated_container_ids = self.unrelated_container_ids()?;
        self.docker(
            &[
                "tag".to_owned(),
                dashboard_previous_id.clone(),
                format!("helix-dashboard:{ROLLBACK_DASHBOARD_TAG}"),
            ],
            Duration::from_secs(30),
        )?;
        self.docker(
            &[
                "tag".to_owned(),
                gateway_previous_id.clone(),
                format!("helix-dashboard-gateway:{ROLLBACK_DASHBOARD_TAG}"),
            ],
            Duration::from_secs(30),
        )?;
        self.backup_file(
            &self.config.privd_install_path,
            &rollback_dir.join("helix-privd"),
        )?;
        if self.config.terminald_install_path.is_file() {
            self.backup_file(
                &self.config.terminald_install_path,
                &rollback_dir.join("helix-terminald"),
            )?;
        }
        let _ = self.backup_state_snapshot();

        progress("Building Helix dashboard, gateway, and broker images", 45);
        let image_tag = plan.version.clone();
        let source_revision = plan.tag.clone();
        self.compose(
            &compose_file,
            &compose,
            &image_tag,
            &source_revision,
            &[
                "build".to_owned(),
                "dashboard".to_owned(),
                "gateway".to_owned(),
            ],
            DOCKER_BUILD_TIMEOUT,
        )?;
        self.docker_build_target(
            &source_root,
            "privd",
            &format!("helix-privd-artifact:{image_tag}"),
            &source_revision,
        )?;
        self.docker_build_target(
            &source_root,
            "terminald",
            &format!("helix-terminald-artifact:{image_tag}"),
            &source_revision,
        )?;
        progress("Extracting the new broker binaries", 80);
        let privd_new = pending_dir.join("helix-privd");
        let terminald_new = pending_dir.join("helix-terminald");
        self.extract_image_file(
            &format!("helix-privd-artifact:{image_tag}"),
            "/helix-privd",
            &privd_new,
        )?;
        self.extract_image_file(
            &format!("helix-terminald-artifact:{image_tag}"),
            "/helix-terminald",
            &terminald_new,
        )?;
        let unit_source = source_root.join("deploy/helix-finalize-update.service");
        if unit_source.is_file() {
            copy_regular_file(
                &unit_source,
                &pending_dir.join("helix-finalize-update.service"),
            )?;
        }
        let pending = PendingUpdate {
            schema_version: PENDING_SCHEMA,
            target_version: plan.version.clone(),
            target_tag: plan.tag.clone(),
            image_tag: image_tag.clone(),
            source_revision,
            compose_file,
            env_file: compose.env_file.clone(),
            project_name: compose.project_name.clone(),
            privd_new,
            terminald_new,
            privd_backup: rollback_dir.join("helix-privd"),
            terminald_backup: rollback_dir.join("helix-terminald"),
            dashboard_previous_id,
            gateway_previous_id,
            unrelated_container_ids,
        };
        write_json_private(&self.pending_path(), &pending)?;
        progress("Restarting Helix (games stay running)", 90);
        self.install_finalize_unit(&pending_dir.join("helix-finalize-update.service"))?;
        self.run_program(
            &self.host.systemctl_binary,
            &[
                "start".to_owned(),
                "--no-block".to_owned(),
                self.config.finalize_unit.clone(),
            ],
            Duration::from_secs(30),
            &[],
        )?;
        Ok(json!({
            "applied": true,
            "restarting": true,
            "git_pull_used": false,
            "target_version": plan.version,
            "target_tag": plan.tag,
            "note": "Helix staged a digest-pinned GitHub release and is restarting the dashboard, gateway, and broker only. Refresh after it comes back. Game containers, AMP, and Plex are not replaced."
        }))
    }

    pub fn finalize_pending(&self) -> Result<Value, String> {
        let pending: PendingUpdate = read_json_private(&self.pending_path())?;
        if pending.schema_version != PENDING_SCHEMA {
            return Err("the staged Helix update uses an unsupported pending schema".to_owned());
        }
        thread::sleep(Duration::from_secs(2));
        let result = self.finalize_inner(&pending);
        match &result {
            Ok(value) => {
                let _ = write_json_private(&self.config.state_root.join("last-result.json"), value);
                let _ = fs::remove_file(self.pending_path());
            }
            Err(error) => {
                let rollback = self.rollback(&pending);
                let _ = write_json_private(
                    &self.config.state_root.join("last-result.json"),
                    &json!({
                        "ok": false,
                        "error": error,
                        "rollback": match &rollback {
                            Ok(_) => Value::from("restored previous Helix images and binaries"),
                            Err(rollback_error) => Value::from(rollback_error.as_str()),
                        }
                    }),
                );
                if let Err(rollback_error) = rollback {
                    return Err(format!(
                        "{error}; Helix also could not restore the previous release: {rollback_error}"
                    ));
                }
                return Err(format!(
                    "{error}; Helix restored the previous dashboard, gateway, and broker"
                ));
            }
        }
        result
    }

    fn status_inner(&self, refresh: bool) -> Result<Value, String> {
        let current = env!("CARGO_PKG_VERSION");
        let compose = self.compose_context().ok();
        let plan = match self.load_plan(refresh) {
            Ok(plan) => plan,
            Err(error) => {
                return Ok(json!({
                    "available": false,
                    "reason_code": "github_release_unavailable",
                    "reason": format!("This host is Helix {current}. GitHub could not be checked yet: {error}"),
                    "git_pull_used": false,
                    "current_version": current,
                    "latest_version": Value::Null,
                    "latest_tag": Value::Null,
                    "release_url": Value::Null,
                    "release_notes": Value::Null,
                    "update_available": false,
                    "compose_detected": compose.is_some(),
                    "required_confirmation": REQUIRED_CONFIRMATION,
                    "rollback_claimed": true,
                    "automatic_reboot": false
                }));
            }
        };
        let update_available = version_is_newer(&plan.version, current)?;
        let compose_detected = compose.is_some();
        let (available, reason_code, reason) = if !compose_detected {
            (
                false,
                "compose_project_not_detected",
                "Helix can see a GitHub release, but this host has no detected dashboard Compose project, so it will not replace binaries or containers from here.".to_owned(),
            )
        } else if !update_available {
            (
                false,
                "already_current",
                format!(
                    "This host is already on Helix {current}, the latest digest-pinned GitHub release."
                ),
            )
        } else {
            (
                true,
                "github_release_ready",
                format!(
                    "Helix {} is on GitHub. Apply downloads the SHA-256-pinned source, rebuilds only the dashboard and gateway, replaces helix-privd and helix-terminald, health-checks, and rolls those back on failure. Game containers stay running.",
                    plan.version
                ),
            )
        };
        Ok(json!({
            "available": available,
            "reason_code": reason_code,
            "reason": reason,
            "git_pull_used": false,
            "current_version": current,
            "latest_version": plan.version,
            "latest_tag": plan.tag,
            "release_url": plan.html_url,
            "release_notes": plan.notes,
            "release_name": plan.name,
            "published_at": plan.published_at,
            "update_available": update_available,
            "compose_detected": compose_detected,
            "required_confirmation": REQUIRED_CONFIRMATION,
            "rollback_claimed": true,
            "automatic_reboot": false
        }))
    }

    fn load_plan(&self, refresh: bool) -> Result<ReleasePlan, String> {
        let cache_path = self.config.state_root.join("check-cache.json");
        if !refresh && let Ok(cached) = read_cached_plan(&cache_path) {
            return Ok(cached);
        }
        prepare_private_directory(&self.config.state_root, "Helix update state")?;
        let url = format!(
            "https://{GITHUB_API_HOST}/repos/{}/{}/releases/latest",
            self.config.github_owner, self.config.github_repo
        );
        let body = self.download_to_string(&url, MAX_METADATA_BYTES, CURL_API_TIMEOUT)?;
        let release: GithubRelease = serde_json::from_str(&body)
            .map_err(|_| "GitHub returned an invalid latest-release document".to_owned())?;
        let mut plan = plan_from_github_release(&release)?;
        if plan.source_sha256.is_empty() {
            let sums = self.download_to_string(
                &plan.checksums_url,
                MAX_SHA256SUMS_BYTES,
                CURL_API_TIMEOUT,
            )?;
            plan.source_sha256 = parse_sha256sums(&sums, &plan.source_name)?;
        }
        let _ = write_json_private(&cache_path, &plan);
        Ok(plan)
    }

    fn compose_context(&self) -> Result<ComposeContext, String> {
        if let Some(directory) = &self.config.compose_project_directory {
            let env_file = directory.join(".env");
            if !directory.is_dir() {
                return Err(
                    "the configured Compose project directory is not a directory".to_owned(),
                );
            }
            if !env_file.is_file() {
                return Err("the configured Compose project directory has no .env file".to_owned());
            }
            return Ok(ComposeContext {
                project_name: directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| is_compose_project_name(name))
                    .unwrap_or("server-dashboard")
                    .to_owned(),
                env_file,
            });
        }
        let labels = self.container_labels(&self.host.dashboard_container)?;
        let working_dir = labels
            .get("com.docker.compose.project.working_dir")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| "the dashboard container has no Compose working directory".to_owned())?;
        let project_name = labels
            .get("com.docker.compose.project")
            .and_then(Value::as_str)
            .filter(|name| is_compose_project_name(name))
            .ok_or_else(|| "the dashboard container has no Compose project name".to_owned())?
            .to_owned();
        let env_file = working_dir.join(".env");
        if !env_file.is_file() {
            return Err("the Compose working directory has no .env file".to_owned());
        }
        Ok(ComposeContext {
            project_name,
            env_file,
        })
    }

    fn finalize_inner(&self, pending: &PendingUpdate) -> Result<Value, String> {
        if !pending.privd_new.is_file() {
            return Err("the staged helix-privd binary is missing".to_owned());
        }
        install_executable(&pending.privd_new, &self.config.privd_install_path)?;
        if pending.terminald_new.is_file() {
            install_executable(&pending.terminald_new, &self.config.terminald_install_path)?;
        }
        self.restart_helix_units()?;
        let compose = ComposeContext {
            project_name: pending.project_name.clone(),
            env_file: pending.env_file.clone(),
        };
        self.compose(
            &pending.compose_file,
            &compose,
            &pending.image_tag,
            &pending.source_revision,
            &[
                "up".to_owned(),
                "-d".to_owned(),
                "--wait".to_owned(),
                "--no-deps".to_owned(),
                "dashboard".to_owned(),
                "gateway".to_owned(),
            ],
            DOCKER_UP_TIMEOUT,
        )?;
        self.verify_unrelated_containers(&pending.unrelated_container_ids)?;
        self.health_check()?;
        Ok(json!({
            "ok": true,
            "version": pending.target_version,
            "tag": pending.target_tag,
            "git_pull_used": false
        }))
    }

    fn rollback(&self, pending: &PendingUpdate) -> Result<(), String> {
        if pending.privd_backup.is_file() {
            install_executable(&pending.privd_backup, &self.config.privd_install_path)?;
        }
        if pending.terminald_backup.is_file() {
            install_executable(
                &pending.terminald_backup,
                &self.config.terminald_install_path,
            )?;
        }
        let _ = self.docker(
            &[
                "tag".to_owned(),
                pending.dashboard_previous_id.clone(),
                format!("helix-dashboard:{ROLLBACK_DASHBOARD_TAG}"),
            ],
            Duration::from_secs(30),
        );
        let _ = self.docker(
            &[
                "tag".to_owned(),
                pending.gateway_previous_id.clone(),
                format!("helix-dashboard-gateway:{ROLLBACK_DASHBOARD_TAG}"),
            ],
            Duration::from_secs(30),
        );
        if pending.compose_file.is_file() && pending.env_file.is_file() {
            let compose = ComposeContext {
                project_name: pending.project_name.clone(),
                env_file: pending.env_file.clone(),
            };
            self.compose(
                &pending.compose_file,
                &compose,
                ROLLBACK_DASHBOARD_TAG,
                "rollback",
                &[
                    "up".to_owned(),
                    "-d".to_owned(),
                    "--wait".to_owned(),
                    "--no-deps".to_owned(),
                    "dashboard".to_owned(),
                    "gateway".to_owned(),
                ],
                DOCKER_UP_TIMEOUT,
            )?;
        }
        self.restart_helix_units()?;
        Ok(())
    }

    fn restart_helix_units(&self) -> Result<(), String> {
        let mut units = vec!["helix-privd.service".to_owned()];
        units.extend(self.active_terminald_units()?);
        let mut args = vec!["restart".to_owned()];
        args.extend(units);
        self.run_program(
            &self.host.systemctl_binary,
            &args,
            Duration::from_secs(60),
            &[],
        )?;
        self.run_program(
            &self.host.systemctl_binary,
            &["is-active".to_owned(), "helix-privd.service".to_owned()],
            Duration::from_secs(15),
            &[],
        )?;
        Ok(())
    }

    fn active_terminald_units(&self) -> Result<Vec<String>, String> {
        let output = self.run_program(
            &self.host.systemctl_binary,
            &[
                "list-units".to_owned(),
                "--type=service".to_owned(),
                "--state=active".to_owned(),
                "--no-legend".to_owned(),
                "--no-pager".to_owned(),
                "helix-terminald@*.service".to_owned(),
            ],
            Duration::from_secs(15),
            &[],
        )?;
        Ok(output
            .lines()
            .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
            .filter(|unit| unit.starts_with("helix-terminald@") && unit.ends_with(".service"))
            .collect())
    }

    fn health_check(&self) -> Result<(), String> {
        self.docker(
            &[
                "exec".to_owned(),
                self.host.dashboard_container.clone(),
                "/app/bin/helixctl".to_owned(),
                "--config".to_owned(),
                "/app/config/helix.toml".to_owned(),
                "ready".to_owned(),
                "--timeout-seconds".to_owned(),
                "20".to_owned(),
            ],
            Duration::from_secs(40),
        )?;
        Ok(())
    }

    fn backup_state_snapshot(&self) -> Result<(), String> {
        let destination = format!("/var/lib/helix-backups/helix-update-{}.db", now_unix_ms());
        match self.docker(
            &[
                "exec".to_owned(),
                self.host.dashboard_container.clone(),
                "/app/bin/helixctl".to_owned(),
                "--config".to_owned(),
                "/app/config/helix.toml".to_owned(),
                "backup-state".to_owned(),
                destination,
            ],
            Duration::from_secs(60),
        ) {
            Ok(_) => Ok(()),
            Err(error) => {
                tracing_log(&format!(
                    "Helix update continues without a dashboard state snapshot: {error}"
                ));
                Ok(())
            }
        }
    }

    fn require_tools(&self) -> Result<(), String> {
        for (path, name) in [
            (&self.config.curl_binary, "curl"),
            (&self.config.tar_binary, "tar"),
            (&self.config.timeout_binary, "timeout"),
            (&self.host.docker_binary, "docker"),
            (&self.host.systemctl_binary, "systemctl"),
        ] {
            if !path.is_file() {
                return Err(format!("the {name} tool is unavailable"));
            }
        }
        Ok(())
    }

    fn install_finalize_unit(&self, staged: &Path) -> Result<(), String> {
        let destination = Path::new("/etc/systemd/system").join(&self.config.finalize_unit);
        if staged.is_file() {
            copy_regular_file(staged, &destination)?;
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o644))
                .map_err(|_| "could not protect the Helix finalize unit".to_owned())?;
            self.run_program(
                &self.host.systemctl_binary,
                &["daemon-reload".to_owned()],
                Duration::from_secs(30),
                &[],
            )?;
        } else if !destination.is_file() {
            return Err("helix-finalize-update.service is not installed on this host".to_owned());
        }
        Ok(())
    }

    fn backup_file(&self, source: &Path, destination: &Path) -> Result<(), String> {
        if !source.is_file() {
            return Err(format!(
                "{} is not a regular file to snapshot",
                source.display()
            ));
        }
        copy_regular_file(source, destination)
    }

    fn extract_image_file(
        &self,
        image: &str,
        source: &str,
        destination: &Path,
    ) -> Result<(), String> {
        let created = self.docker(
            &["create".to_owned(), image.to_owned()],
            Duration::from_secs(60),
        )?;
        let container = created.trim();
        if container.is_empty() {
            return Err("Docker did not return a temporary container id".to_owned());
        }
        let copy = self.docker(
            &[
                "cp".to_owned(),
                format!("{container}:{source}"),
                destination.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(60),
        );
        let _ = self.docker(
            &["rm".to_owned(), "-f".to_owned(), container.to_owned()],
            Duration::from_secs(30),
        );
        copy?;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755))
            .map_err(|_| "could not protect the extracted Helix binary".to_owned())?;
        if !destination.is_file() {
            return Err("the extracted Helix binary is missing".to_owned());
        }
        Ok(())
    }

    fn docker_build_target(
        &self,
        source_root: &Path,
        target: &str,
        image: &str,
        revision: &str,
    ) -> Result<(), String> {
        self.docker(
            &[
                "build".to_owned(),
                "--target".to_owned(),
                target.to_owned(),
                "-t".to_owned(),
                image.to_owned(),
                "--build-arg".to_owned(),
                format!("HELIX_SOURCE_REVISION={revision}"),
                source_root.to_string_lossy().into_owned(),
            ],
            DOCKER_BUILD_TIMEOUT,
        )?;
        Ok(())
    }

    fn compose(
        &self,
        compose_file: &Path,
        compose: &ComposeContext,
        image_tag: &str,
        revision: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<String, String> {
        let mut command = vec![
            "compose".to_owned(),
            "--env-file".to_owned(),
            compose.env_file.to_string_lossy().into_owned(),
            "--project-name".to_owned(),
            compose.project_name.clone(),
            "-f".to_owned(),
            compose_file.to_string_lossy().into_owned(),
        ];
        command.extend_from_slice(args);
        let home = prepare_docker_cli_home(&self.docker_state_root)?;
        let home_s = home.to_string_lossy().into_owned();
        let cache_s = home.join("cache").to_string_lossy().into_owned();
        let buildx_s = home.join("buildx").to_string_lossy().into_owned();
        let tmp_s = home.join("tmp").to_string_lossy().into_owned();
        self.run_program(
            &self.host.docker_binary,
            &command,
            timeout,
            &[
                ("HELIX_IMAGE_TAG", image_tag),
                ("HELIX_SOURCE_REVISION", revision),
                ("HOME", home_s.as_str()),
                ("DOCKER_CONFIG", home_s.as_str()),
                ("XDG_CACHE_HOME", cache_s.as_str()),
                ("BUILDX_CONFIG", buildx_s.as_str()),
                ("TMPDIR", tmp_s.as_str()),
            ],
        )
    }

    fn docker(&self, args: &[String], timeout: Duration) -> Result<String, String> {
        let home = prepare_docker_cli_home(&self.docker_state_root)?;
        let home_s = home.to_string_lossy().into_owned();
        self.run_program(
            &self.host.docker_binary,
            args,
            timeout,
            &[
                ("HOME", home_s.as_str()),
                ("DOCKER_CONFIG", home_s.as_str()),
            ],
        )
    }

    fn container_id(&self, name: &str) -> Result<String, String> {
        let id = self
            .docker(
                &[
                    "inspect".to_owned(),
                    "-f".to_owned(),
                    "{{.Id}}".to_owned(),
                    name.to_owned(),
                ],
                Duration::from_secs(20),
            )?
            .trim()
            .to_owned();
        if id.len() < 12 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{name} did not return a Docker id"));
        }
        Ok(id)
    }

    fn container_labels(&self, name: &str) -> Result<Value, String> {
        let raw = self.docker(
            &[
                "inspect".to_owned(),
                "-f".to_owned(),
                "{{json .Config.Labels}}".to_owned(),
                name.to_owned(),
            ],
            Duration::from_secs(20),
        )?;
        serde_json::from_str(raw.trim())
            .map_err(|_| format!("{name} did not return Compose labels"))
    }

    fn unrelated_container_ids(&self) -> Result<Vec<String>, String> {
        let dashboard = self.container_id(&self.host.dashboard_container)?;
        let gateway = self.container_id(&self.host.gateway_container)?;
        let listed = self.docker(
            &["ps".to_owned(), "-q".to_owned(), "--no-trunc".to_owned()],
            Duration::from_secs(20),
        )?;
        Ok(listed
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty() && *id != dashboard && *id != gateway)
            .map(str::to_owned)
            .collect())
    }

    fn verify_unrelated_containers(&self, expected: &[String]) -> Result<(), String> {
        let current = self.unrelated_container_ids()?;
        for id in expected {
            if !current.iter().any(|current_id| current_id == id) {
                return Err(
                    "an unrelated container changed during the Helix update; Helix will restore the previous release".to_owned(),
                );
            }
        }
        Ok(())
    }

    fn download_https(
        &self,
        url: &str,
        destination: &Path,
        maximum_bytes: u64,
        timeout: Duration,
    ) -> Result<(), String> {
        require_github_https(url)?;
        let parent = destination
            .parent()
            .ok_or_else(|| "download path has no parent".to_owned())?;
        let temporary = parent.join(format!(".helix-download-{}.partial", now_unix_ms()));
        let result = self.run_program(
            &self.config.curl_binary,
            &[
                "--fail".to_owned(),
                "--silent".to_owned(),
                "--show-error".to_owned(),
                "--location".to_owned(),
                "--max-redirs".to_owned(),
                "5".to_owned(),
                "--proto".to_owned(),
                "=https".to_owned(),
                "--tlsv1.2".to_owned(),
                "--connect-timeout".to_owned(),
                "10".to_owned(),
                "--max-time".to_owned(),
                timeout.as_secs().to_string(),
                "--max-filesize".to_owned(),
                maximum_bytes.to_string(),
                "--header".to_owned(),
                format!("User-Agent: {USER_AGENT}"),
                "--header".to_owned(),
                "Accept: application/vnd.github+json, application/octet-stream".to_owned(),
                "--output".to_owned(),
                temporary.to_string_lossy().into_owned(),
                url.to_owned(),
            ],
            timeout.saturating_add(Duration::from_secs(15)),
            &[],
        );
        match result {
            Ok(_) => {
                let metadata = fs::metadata(&temporary)
                    .map_err(|_| "the downloaded Helix file is missing".to_owned())?;
                if metadata.len() == 0 || metadata.len() > maximum_bytes {
                    let _ = fs::remove_file(&temporary);
                    return Err("the downloaded Helix file is outside the size limit".to_owned());
                }
                fs::rename(&temporary, destination)
                    .map_err(|_| "could not commit the Helix download".to_owned())
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(error)
            }
        }
    }

    fn download_to_string(
        &self,
        url: &str,
        maximum_bytes: u64,
        timeout: Duration,
    ) -> Result<String, String> {
        let path = self
            .config
            .state_root
            .join(format!("download-{}.partial", now_unix_ms()));
        self.download_https(url, &path, maximum_bytes, timeout)?;
        let bytes =
            fs::read(&path).map_err(|_| "could not read the GitHub release document".to_owned())?;
        let _ = fs::remove_file(&path);
        String::from_utf8(bytes)
            .map_err(|_| "GitHub returned a non-UTF-8 release document".to_owned())
    }

    fn run_program(
        &self,
        program: &Path,
        args: &[String],
        timeout: Duration,
        environment: &[(&str, &str)],
    ) -> Result<String, String> {
        let output = run_bounded_command(
            &self.config.timeout_binary,
            program,
            args,
            timeout,
            environment,
            MAX_COMMAND_OUTPUT_BYTES,
        )?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = stderr.trim();
            if !detail.is_empty() {
                return Err(detail.chars().take(500).collect());
            }
            let fallback = stdout.trim();
            if !fallback.is_empty() {
                return Err(fallback.chars().take(500).collect());
            }
            return Err(format!("{} failed", program.display()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn pending_path(&self) -> PathBuf {
        self.config.state_root.join("pending.json")
    }
}

fn unavailable(reason_code: &str, reason: String) -> Value {
    json!({
        "available": false,
        "reason_code": reason_code,
        "reason": reason,
        "git_pull_used": false,
        "current_version": env!("CARGO_PKG_VERSION"),
        "latest_version": Value::Null,
        "latest_tag": Value::Null,
        "release_url": Value::Null,
        "release_notes": Value::Null,
        "update_available": false,
        "compose_detected": false,
        "required_confirmation": REQUIRED_CONFIRMATION,
        "rollback_claimed": true,
        "automatic_reboot": false
    })
}

fn plan_from_github_release(release: &GithubRelease) -> Result<ReleasePlan, String> {
    if release.draft || release.prerelease {
        return Err("the latest GitHub release is a draft or prerelease".to_owned());
    }
    let version = parse_release_tag(&release.tag_name)?;
    let source_name = format!("helix-source-{version}.tar.gz");
    let source = release
        .assets
        .iter()
        .find(|asset| asset.name == source_name)
        .ok_or_else(|| {
            format!(
                "GitHub release {} is missing {source_name}",
                release.tag_name
            )
        })?;
    require_github_https(&source.browser_download_url)?;
    let checksums = release
        .assets
        .iter()
        .find(|asset| asset.name == "SHA256SUMS")
        .ok_or_else(|| format!("GitHub release {} is missing SHA256SUMS", release.tag_name))?;
    require_github_https(&checksums.browser_download_url)?;
    if source.size == 0 || source.size > MAX_SOURCE_BYTES {
        return Err("the Helix source archive is outside the size limit".to_owned());
    }
    Ok(ReleasePlan {
        tag: release.tag_name.clone(),
        version,
        name: release
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| release.tag_name.clone()),
        notes: truncate_notes(release.body.as_deref().unwrap_or_default()),
        html_url: release.html_url.clone(),
        published_at: release.published_at.clone(),
        source_name,
        source_url: source.browser_download_url.clone(),
        checksums_url: checksums.browser_download_url.clone(),
        source_sha256: github_asset_sha256(source)?.unwrap_or_default(),
        source_bytes: source.size,
    })
}

fn github_asset_sha256(asset: &GithubAsset) -> Result<Option<String>, String> {
    let Some(digest) = asset.digest.as_deref() else {
        return Ok(None);
    };
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| "GitHub asset digest is not SHA-256".to_owned())?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("GitHub asset digest is not a SHA-256 hex string".to_owned());
    }
    Ok(Some(hex.to_ascii_lowercase()))
}

fn parse_sha256sums(body: &str, file_name: &str) -> Result<String, String> {
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hash, name) = line
            .split_once("  ")
            .or_else(|| line.split_once(" *"))
            .ok_or_else(|| "SHA256SUMS has an invalid line".to_owned())?;
        if name == file_name {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("SHA256SUMS has an invalid hash".to_owned());
            }
            return Ok(hash.to_ascii_lowercase());
        }
    }
    Err(format!("SHA256SUMS does not list {file_name}"))
}

pub(crate) fn parse_release_tag(tag: &str) -> Result<String, String> {
    let version = tag
        .strip_prefix('v')
        .ok_or_else(|| "Helix release tags must look like v1.0.0".to_owned())?;
    if version.contains('-') || version.contains('+') {
        return Err("Helix will not apply a prerelease GitHub tag".to_owned());
    }
    let parsed = semver::Version::parse(version)
        .map_err(|_| "Helix release tags must be SemVer vMAJOR.MINOR.PATCH".to_owned())?;
    if parsed.pre.is_empty() && parsed.build.is_empty() {
        Ok(parsed.to_string())
    } else {
        Err("Helix will not apply a prerelease GitHub tag".to_owned())
    }
}

fn version_is_newer(candidate: &str, current: &str) -> Result<bool, String> {
    let candidate = semver::Version::parse(candidate)
        .map_err(|_| "the GitHub release version is not valid SemVer".to_owned())?;
    let current = semver::Version::parse(current)
        .map_err(|_| "the compiled Helix version is not valid SemVer".to_owned())?;
    Ok(candidate > current)
}

fn require_safe_archive_paths(listing: &str, version: &str) -> Result<(), String> {
    let prefix = format!("helix-{version}/");
    let mut count = 0_usize;
    for line in listing.lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err("the Helix source archive lists too many paths".to_owned());
        }
        if path.contains('\0')
            || path.contains('\\')
            || path.contains("..")
            || path.starts_with('/')
            || path.starts_with("./")
        {
            return Err("the Helix source archive contains an unsafe path".to_owned());
        }
        if path != prefix.trim_end_matches('/') && !path.starts_with(&prefix) {
            return Err(
                "the Helix source archive uses an unexpected top-level directory".to_owned(),
            );
        }
        if !path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b'+' | b'@')
        }) {
            return Err("the Helix source archive contains an unsupported path".to_owned());
        }
    }
    if count == 0 {
        return Err("the Helix source archive is empty".to_owned());
    }
    Ok(())
}

fn require_github_https(url: &str) -> Result<(), String> {
    let remainder = url
        .strip_prefix("https://")
        .ok_or_else(|| "Helix only downloads GitHub releases over HTTPS".to_owned())?;
    let host = remainder.split('/').next().unwrap_or_default();
    if host.contains(['@', ':'])
        || ![
            "api.github.com",
            "github.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com",
        ]
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
    {
        return Err("the release download is not from GitHub".to_owned());
    }
    Ok(())
}

fn validate_github_identity(owner: &str, repo: &str) -> Result<(), String> {
    if !is_github_owner(owner) || !is_github_repo(repo) {
        return Err("the configured GitHub repository identity is invalid".to_owned());
    }
    Ok(())
}

fn is_github_owner(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=39).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

fn is_github_repo(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=100).contains(&bytes.len())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b'-'))
}

fn is_compose_project_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
}

fn truncate_notes(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= MAX_RELEASE_NOTES_CHARS {
        return trimmed.to_owned();
    }
    trimmed.chars().take(MAX_RELEASE_NOTES_CHARS).collect()
}

fn prepare_private_directory(path: &Path, label: &str) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| format!("could not create the {label} directory"))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| format!("could not inspect the {label} directory"))?;
    if !metadata.file_type().is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(format!(
            "the {label} directory is not a root-owned real directory"
        ));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| format!("could not protect the {label} directory"))?;
    Ok(())
}

fn reset_directory(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|_| "could not clear the Helix update staging directory".to_owned())?;
    }
    prepare_private_directory(path, "Helix update staging")
}

fn require_disk_headroom(path: &Path, bytes: u64) -> Result<(), String> {
    let stats = rustix::fs::statvfs(path)
        .map_err(|_| "could not measure free space for a Helix update".to_owned())?;
    let available = stats.f_bavail.saturating_mul(stats.f_frsize);
    if available < bytes {
        return Err(
            "the Helix update staging filesystem does not have a full gigabyte free".to_owned(),
        );
    }
    Ok(())
}

fn copy_regular_file(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|_| format!("could not inspect {}", source.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!("{} is not a regular file", source.display()));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "could not create the Helix update destination".to_owned())?;
    }
    let mut input =
        File::open(source).map_err(|_| format!("could not read {}", source.display()))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(destination)
        .map_err(|_| format!("could not write {}", destination.display()))?;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| format!("could not copy {}", source.display()))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|_| format!("could not copy {}", source.display()))?;
    }
    output
        .sync_all()
        .map_err(|_| format!("could not persist {}", destination.display()))?;
    Ok(())
}

fn install_executable(source: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "install path has no parent".to_owned())?;
    let temporary = parent.join(format!(
        ".{}.new",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("helix-bin")
    ));
    copy_regular_file(source, &temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
        .map_err(|_| "could not protect the new Helix binary".to_owned())?;
    fs::rename(&temporary, destination)
        .map_err(|_| format!("could not install {}", destination.display()))?;
    Ok(())
}

fn write_json_private<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| "could not encode Helix update state".to_owned())?;
    if let Some(parent) = path.parent() {
        prepare_private_directory(parent, "Helix update state")?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| format!("could not write {}", path.display()))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| format!("could not persist {}", path.display()))?;
    Ok(())
}

fn read_json_private<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|_| "the staged Helix update is missing".to_owned())?;
    serde_json::from_slice(&bytes).map_err(|_| "the staged Helix update is not valid".to_owned())
}

fn read_cached_plan(path: &Path) -> Result<ReleasePlan, String> {
    let metadata = fs::metadata(path).map_err(|_| "no cached GitHub release".to_owned())?;
    let age = metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .unwrap_or(CACHE_TTL);
    if age > CACHE_TTL {
        return Err("cached GitHub release is stale".to_owned());
    }
    read_json_private(path)
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "could not hash the Helix download".to_owned())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "could not hash the Helix download".to_owned())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn tracing_log(message: &str) {
    eprintln!("{message}");
}

fn default_github_owner() -> String {
    "Riqqqque".to_owned()
}

fn default_github_repo() -> String {
    "Helix".to_owned()
}

fn default_update_state_root() -> PathBuf {
    PathBuf::from("/var/lib/helix/updates")
}

fn default_curl_binary() -> PathBuf {
    PathBuf::from("/usr/bin/curl")
}

fn default_tar_binary() -> PathBuf {
    PathBuf::from("/usr/bin/tar")
}

fn default_timeout_binary() -> PathBuf {
    PathBuf::from("/usr/bin/timeout")
}

fn default_privd_install_path() -> PathBuf {
    PathBuf::from("/usr/local/libexec/helix-privd")
}

fn default_terminald_install_path() -> PathBuf {
    PathBuf::from("/usr/local/libexec/helix-terminald")
}

fn default_finalize_unit() -> String {
    FINALIZE_UNIT.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_tags_reject_prerelease_and_non_semver() {
        assert_eq!(parse_release_tag("v1.0.0").unwrap(), "1.0.0");
        assert!(parse_release_tag("1.0.0").is_err());
        assert!(parse_release_tag("v1.0.0-alpha.1").is_err());
        assert!(parse_release_tag("v1.0.0+build").is_err());
        assert!(parse_release_tag("latest").is_err());
    }

    #[test]
    fn newer_release_comparison_is_semver() {
        assert!(version_is_newer("1.0.1", "1.0.0").unwrap());
        assert!(!version_is_newer("1.0.0", "1.0.0").unwrap());
        assert!(!version_is_newer("1.0.0", "1.0.1").unwrap());
    }

    #[test]
    fn sha256sums_parser_accepts_gnu_format() {
        let body = "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd  helix-source-1.0.1.tar.gz\n";
        assert_eq!(
            parse_sha256sums(body, "helix-source-1.0.1.tar.gz").unwrap(),
            "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
        );
        assert!(parse_sha256sums(body, "missing.tar.gz").is_err());
    }

    #[test]
    fn archive_listing_requires_version_prefix_and_rejects_parent_paths() {
        assert!(
            require_safe_archive_paths("helix-1.0.0/\nhelix-1.0.0/Cargo.toml\n", "1.0.0").is_ok()
        );
        assert!(require_safe_archive_paths("helix-1.0.0/../etc/passwd\n", "1.0.0").is_err());
        assert!(require_safe_archive_paths("/etc/passwd\n", "1.0.0").is_err());
        assert!(require_safe_archive_paths("other/Cargo.toml\n", "1.0.0").is_err());
    }

    #[test]
    fn github_urls_are_https_and_host_limited() {
        assert!(
            require_github_https("https://api.github.com/repos/Riqqqque/Helix/releases/latest")
                .is_ok()
        );
        assert!(require_github_https(
            "https://github.com/Riqqqque/Helix/releases/download/v1.0.0/helix-source-1.0.0.tar.gz"
        )
        .is_ok());
        assert!(require_github_https("https://evil.example/helix.tar.gz").is_err());
        assert!(require_github_https("http://github.com/Riqqqque/Helix").is_err());
    }

    #[test]
    fn github_release_plan_requires_source_asset_and_stable_tag() {
        let release = GithubRelease {
            tag_name: "v1.0.1".to_owned(),
            name: Some("Helix 1.0.1".to_owned()),
            body: Some("Fixes update apply.".to_owned()),
            html_url: "https://github.com/Riqqqque/Helix/releases/tag/v1.0.1".to_owned(),
            draft: false,
            prerelease: false,
            published_at: Some("2026-08-29T00:00:00Z".to_owned()),
            assets: vec![
                GithubAsset {
                    name: "helix-source-1.0.1.tar.gz".to_owned(),
                    size: 12,
                    browser_download_url:
                        "https://github.com/Riqqqque/Helix/releases/download/v1.0.1/helix-source-1.0.1.tar.gz"
                            .to_owned(),
                    digest: Some(
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_owned(),
                    ),
                },
                GithubAsset {
                    name: "SHA256SUMS".to_owned(),
                    size: 80,
                    browser_download_url:
                        "https://github.com/Riqqqque/Helix/releases/download/v1.0.1/SHA256SUMS"
                            .to_owned(),
                    digest: None,
                },
            ],
        };
        let plan = plan_from_github_release(&release).unwrap();
        assert_eq!(plan.version, "1.0.1");
        assert_eq!(
            plan.source_sha256,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        let mut draft = release;
        draft.prerelease = true;
        assert!(plan_from_github_release(&draft).is_err());
    }
}
