use crate::bounded_command::run_bounded_command;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write as _},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const MAX_COMMAND_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPOSITORY_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookInstallerConfig {
    #[serde(default = "default_os_release_path")]
    pub os_release_path: PathBuf,
    #[serde(default = "default_curl_binary")]
    pub curl_binary: PathBuf,
    #[serde(default = "default_gpg_binary")]
    pub gpg_binary: PathBuf,
    #[serde(default = "default_apt_get_binary")]
    pub apt_get_binary: PathBuf,
    #[serde(default = "default_dpkg_binary")]
    pub dpkg_binary: PathBuf,
    #[serde(default = "default_systemctl_binary")]
    pub systemctl_binary: PathBuf,
    #[serde(default = "default_docker_binary")]
    pub docker_binary: PathBuf,
    #[serde(default = "default_timeout_binary")]
    pub timeout_binary: PathBuf,
    #[serde(default = "default_apt_sources_root")]
    pub apt_sources_root: PathBuf,
    #[serde(default = "default_apt_keyrings_root")]
    pub apt_keyrings_root: PathBuf,
    #[serde(default = "default_share_keyrings_root")]
    pub share_keyrings_root: PathBuf,
    #[serde(default = "default_staging_root")]
    pub staging_root: PathBuf,
}

impl Default for HookInstallerConfig {
    fn default() -> Self {
        Self {
            os_release_path: default_os_release_path(),
            curl_binary: default_curl_binary(),
            gpg_binary: default_gpg_binary(),
            apt_get_binary: default_apt_get_binary(),
            dpkg_binary: default_dpkg_binary(),
            systemctl_binary: default_systemctl_binary(),
            docker_binary: default_docker_binary(),
            timeout_binary: default_timeout_binary(),
            apt_sources_root: default_apt_sources_root(),
            apt_keyrings_root: default_apt_keyrings_root(),
            share_keyrings_root: default_share_keyrings_root(),
            staging_root: default_staging_root(),
        }
    }
}

pub struct HookInstaller {
    config: HookInstallerConfig,
    runner: Arc<dyn HookInstallCommandRunner>,
    mutation: Mutex<()>,
}

#[derive(Clone, Debug)]
struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

trait HookInstallCommandRunner: Send + Sync {
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

impl HookInstallCommandRunner for ProcessRunner {
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
            &[("LC_ALL", "C"), ("DEBIAN_FRONTEND", "noninteractive")],
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

#[derive(Clone, Debug)]
struct HostPlatform {
    id: String,
    name: String,
    codename: String,
    architecture: String,
}

#[derive(Clone, Debug)]
struct SavedFile {
    path: PathBuf,
    body: Option<Vec<u8>>,
    mode: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryRole {
    PrivateStaging,
    AptWrite,
}

impl DirectoryRole {
    fn intended_mode(self) -> u32 {
        match self {
            Self::PrivateStaging => 0o700,
            Self::AptWrite => 0o755,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DirectoryInspection {
    Missing,
    Safe { mode: u32 },
    NeedsModeFix { mode: u32 },
    Blocked { reason: String },
}

impl HookInstaller {
    pub fn new(config: HookInstallerConfig) -> Result<Self, String> {
        validate_config(&config)?;
        Ok(Self {
            runner: Arc::new(ProcessRunner {
                timeout_binary: config.timeout_binary.clone(),
            }),
            config,
            mutation: Mutex::new(()),
        })
    }

    pub fn preflight(&self, hook_id: &str) -> Result<Value, String> {
        validate_hook_id(hook_id)?;
        let platform = self.platform();
        let (mode, official_docs, changes, next_steps) = match hook_id {
            "tailscale" => (
                "one_click",
                "https://tailscale.com/docs/install/linux",
                vec![
                    "Add Tailscale's official, signed APT repository",
                    "Install the tailscale package",
                    "Enable and start tailscaled.service",
                ],
                vec![
                    "Open the Tailscale sign-in link from the host terminal",
                    "Approve this machine in the intended tailnet",
                ],
            ),
            "jellyfin" => (
                "one_click",
                "https://jellyfin.org/docs/general/installation/linux/",
                vec![
                    "Add Jellyfin's official, signed APT repository",
                    "Install the jellyfin package and its dependencies",
                    "Enable and start jellyfin.service",
                ],
                vec![
                    "Open Jellyfin Web on port 8096",
                    "Complete Jellyfin's first-run owner and library setup",
                ],
            ),
            "pterodactyl" => (
                "guided",
                "https://pterodactyl.io/wings/1.0/installing.html",
                vec![
                    "Verify Linux, architecture, Docker, and systemd prerequisites",
                    "Keep the panel-generated node configuration under Pterodactyl's control",
                ],
                vec![
                    "Create the node in the Pterodactyl Panel",
                    "Copy its generated config.yml into /etc/pterodactyl",
                    "Install and validate Wings using the official release instructions",
                ],
            ),
            _ => return Err("this hook does not have a Helix installer".to_owned()),
        };

        let mut checks = Vec::new();
        let mut blockers = Vec::new();
        match &platform {
            Ok(platform) => {
                let apt_supported = matches!(platform.id.as_str(), "ubuntu" | "debian")
                    && valid_codename(&platform.codename);
                let os_detail = if platform.codename.is_empty() {
                    platform.name.clone()
                } else {
                    format!("{} · {}", platform.name, platform.codename)
                };
                checks.push(check(
                    "operating_system",
                    "Supported Linux release",
                    if hook_id == "pterodactyl" || apt_supported {
                        "pass"
                    } else {
                        "block"
                    },
                    &os_detail,
                ));
                if matches!(hook_id, "tailscale" | "jellyfin") && !apt_supported {
                    blockers.push("One-click Tailscale and Jellyfin installs currently require a Debian-family APT release (Debian, Ubuntu, or a derivative with UBUNTU_CODENAME or a Debian codename).".to_owned());
                }
                let architecture_supported = match hook_id {
                    "jellyfin" => {
                        matches!(platform.architecture.as_str(), "amd64" | "arm64" | "armhf")
                    }
                    _ => matches!(platform.architecture.as_str(), "amd64" | "arm64"),
                };
                checks.push(check(
                    "architecture",
                    "Supported architecture",
                    if architecture_supported {
                        "pass"
                    } else {
                        "block"
                    },
                    &platform.architecture,
                ));
                if !architecture_supported {
                    blockers.push(
                        "This architecture is not in Helix's tested installer allowlist."
                            .to_owned(),
                    );
                }
            }
            Err(error) => {
                checks.push(check(
                    "operating_system",
                    "Supported Linux release",
                    "block",
                    error,
                ));
                blockers.push(
                    "Helix could not verify the host distribution and architecture.".to_owned(),
                );
            }
        }

        let mut required_tools = vec![
            ("curl", "HTTPS download tool", &self.config.curl_binary),
            (
                "systemd",
                "systemd service manager",
                &self.config.systemctl_binary,
            ),
            (
                "timeout",
                "Bounded command runner",
                &self.config.timeout_binary,
            ),
        ];
        if matches!(hook_id, "tailscale" | "jellyfin") {
            required_tools.extend([
                ("apt", "APT package manager", &self.config.apt_get_binary),
                ("dpkg", "Debian package database", &self.config.dpkg_binary),
            ]);
        }
        for (id, label, path) in required_tools {
            let available = executable(path);
            checks.push(check(
                id,
                label,
                if available { "pass" } else { "block" },
                if available { "Available" } else { "Missing" },
            ));
            if !available {
                blockers.push(
                    match id {
                        "curl" => "curl is required for verified HTTPS downloads.",
                        "apt" => "apt-get is required for package installation.",
                        "dpkg" => "dpkg is required for Debian package installs.",
                        "timeout" => "timeout is required to enforce command deadlines.",
                        _ => "systemctl is required to verify the installed service.",
                    }
                    .to_owned(),
                );
            }
        }
        if hook_id == "jellyfin" {
            let available = executable(&self.config.gpg_binary);
            checks.push(check(
                "gpg",
                "Repository key verifier",
                if available { "pass" } else { "block" },
                if available { "Available" } else { "Missing" },
            ));
            if !available {
                blockers.push(
                    "GnuPG is required to convert Jellyfin's official signing key.".to_owned(),
                );
            }
        }
        if hook_id == "pterodactyl" {
            let available = executable(&self.config.docker_binary);
            checks.push(check(
                "docker",
                "Docker execution backend",
                if available { "pass" } else { "block" },
                if available { "Available" } else { "Missing" },
            ));
            if !available {
                blockers.push(
                    "Wings requires Docker before its node configuration can be validated."
                        .to_owned(),
                );
            }
        }
        if matches!(hook_id, "tailscale" | "jellyfin") {
            push_write_directory_check(
                &mut checks,
                &mut blockers,
                "write_staging",
                "Private installer workspace",
                &self.config.staging_root,
                DirectoryRole::PrivateStaging,
            );
            push_write_directory_check(
                &mut checks,
                &mut blockers,
                "write_apt_sources",
                "APT sources directory",
                &self.config.apt_sources_root,
                DirectoryRole::AptWrite,
            );
            let keyring_root = if hook_id == "jellyfin" {
                &self.config.apt_keyrings_root
            } else {
                &self.config.share_keyrings_root
            };
            push_write_directory_check(
                &mut checks,
                &mut blockers,
                "write_keyring",
                "Repository keyring directory",
                keyring_root,
                DirectoryRole::AptWrite,
            );
        }

        let install_available = mode == "one_click" && blockers.is_empty();
        Ok(json!({
            "schema_version": 1,
            "hook_id": hook_id,
            "mode": mode,
            "install_available": install_available,
            "status": if install_available { "ready" } else if mode == "guided" && blockers.is_empty() { "needs_input" } else { "blocked" },
            "platform": platform.ok().map(|platform| json!({
                "id": platform.id,
                "name": platform.name,
                "codename": platform.codename,
                "architecture": platform.architecture,
            })),
            "checks": checks,
            "changes": changes,
            "next_steps": next_steps,
            "writes": planned_writes(hook_id, &self.config),
            "blockers": blockers,
            "official_docs": official_docs,
            "automatic_account_creation": false,
            "secrets_persisted_by_helix": false,
            "collected_at_unix_ms": now_unix_ms(),
        }))
    }

    pub fn install(
        &self,
        hook_id: &str,
        confirmation: &str,
        repository_change_acknowledged: bool,
        progress: &dyn Fn(&str, u8),
    ) -> Result<Value, String> {
        validate_hook_id(hook_id)?;
        if confirmation != hook_id {
            return Err(format!(
                "type {hook_id} exactly to confirm this installation"
            ));
        }
        if !repository_change_acknowledged {
            return Err(
                "confirm the official repository and package changes before installing".to_owned(),
            );
        }
        let plan = self.preflight(hook_id)?;
        if plan["install_available"] != true {
            return Err("this hook is not ready for a one-click install on this host".to_owned());
        }
        let _mutation = self
            .mutation
            .try_lock()
            .map_err(|_| "another hook or package installation is already running".to_owned())?;
        let platform = self.platform()?;
        match hook_id {
            "tailscale" => self.install_tailscale(&platform, progress),
            "jellyfin" => self.install_jellyfin(&platform, progress),
            _ => Err(
                "this hook requires owner configuration and cannot be installed automatically"
                    .to_owned(),
            ),
        }
    }

    fn install_tailscale(
        &self,
        platform: &HostPlatform,
        progress: &dyn Fn(&str, u8),
    ) -> Result<Value, String> {
        let key_url = format!(
            "https://pkgs.tailscale.com/stable/{}/{}.noarmor.gpg",
            platform.id, platform.codename
        );
        let list_url = format!(
            "https://pkgs.tailscale.com/stable/{}/{}.tailscale-keyring.list",
            platform.id, platform.codename
        );
        let mut staging: Option<PathBuf> = None;
        let result = (|| {
            progress("Preparing a private installer workspace", 16);
            let workspace = self.private_staging_directory()?;
            let key_download = workspace.join("tailscale-keyring.gpg");
            let list_download = workspace.join("tailscale.list");
            staging = Some(workspace);
            progress("Downloading Tailscale's official repository files", 20);
            self.download(&key_url, &key_download)?;
            self.download(&list_url, &list_download)?;
            progress("Validating the signed Tailscale repository", 34);
            let key = read_bounded(&key_download, "Tailscale repository key")?;
            if key.len() < 512 {
                return Err("Tailscale returned an invalid repository key".to_owned());
            }
            let list = read_bounded(&list_download, "Tailscale repository definition")?;
            validate_tailscale_repository(&list, platform)?;
            ensure_root_directory(&self.config.share_keyrings_root, DirectoryRole::AptWrite)?;
            ensure_root_directory(&self.config.apt_sources_root, DirectoryRole::AptWrite)?;
            let key_path = self
                .config
                .share_keyrings_root
                .join("tailscale-archive-keyring.gpg");
            let list_path = self.config.apt_sources_root.join("tailscale.list");
            let first = self.replace_file(&key_path, &key, 0o644)?;
            let second = match self.replace_file(&list_path, &list, 0o644) {
                Ok(saved) => saved,
                Err(error) => return Err(combine_rollback(error, restore_files(&[first]))),
            };
            let files = [first, second];
            progress("Installing the allowlisted tailscale package", 52);
            if let Err(error) = self.apt_install("tailscale") {
                let rollback = restore_files(&files);
                return Err(combine_rollback(error, rollback));
            }
            progress("Enabling and verifying tailscaled.service", 88);
            self.enable_service("tailscaled.service")?;
            progress("Tailscale service verified", 96);
            Ok(json!({
                "hook_id": "tailscale",
                "installed": true,
                "service": "tailscaled.service",
                "service_active": true,
                "account_connected": false,
                "next_action": "Run tailscale up in Terminal and open the one-time sign-in URL.",
                "panel_url": "https://login.tailscale.com/admin/machines",
                "repository": "https://pkgs.tailscale.com/stable/",
                "automatic_reboot": false,
                "completed_at_unix_ms": now_unix_ms(),
            }))
        })();
        if let Some(path) = staging {
            let _ = fs::remove_dir_all(path);
        }
        result
    }

    fn install_jellyfin(
        &self,
        platform: &HostPlatform,
        progress: &dyn Fn(&str, u8),
    ) -> Result<Value, String> {
        let mut staging: Option<PathBuf> = None;
        let result = (|| {
            progress("Preparing a private installer workspace", 16);
            let workspace = self.private_staging_directory()?;
            let key_download = workspace.join("jellyfin.asc");
            let keyring = workspace.join("jellyfin.gpg");
            staging = Some(workspace);
            progress("Downloading Jellyfin's official repository key", 20);
            self.download(
                "https://repo.jellyfin.org/jellyfin_team.gpg.key",
                &key_download,
            )?;
            progress("Validating the Jellyfin repository key", 30);
            let output = self.runner.run(
                &self.config.gpg_binary,
                &[
                    "--batch".to_owned(),
                    "--yes".to_owned(),
                    "--dearmor".to_owned(),
                    "--output".to_owned(),
                    keyring.to_string_lossy().into_owned(),
                    key_download.to_string_lossy().into_owned(),
                ],
                Duration::from_secs(30),
            )?;
            require_success(output, "Jellyfin repository key conversion")?;
            let key = read_bounded(&keyring, "Jellyfin repository key")?;
            if key.len() < 512 {
                return Err("Jellyfin returned an invalid repository key".to_owned());
            }
            let source = format!(
                "Types: deb\nURIs: https://repo.jellyfin.org/{}\nSuites: {}\nComponents: main\nArchitectures: {}\nSigned-By: /etc/apt/keyrings/jellyfin.gpg\n",
                platform.id, platform.codename, platform.architecture
            );
            validate_jellyfin_repository(&source, platform)?;
            ensure_root_directory(&self.config.apt_keyrings_root, DirectoryRole::AptWrite)?;
            ensure_root_directory(&self.config.apt_sources_root, DirectoryRole::AptWrite)?;
            let key_path = self.config.apt_keyrings_root.join("jellyfin.gpg");
            let source_path = self.config.apt_sources_root.join("jellyfin.sources");
            let first = self.replace_file(&key_path, &key, 0o644)?;
            let second = match self.replace_file(&source_path, source.as_bytes(), 0o644) {
                Ok(saved) => saved,
                Err(error) => return Err(combine_rollback(error, restore_files(&[first]))),
            };
            let files = [first, second];
            progress("Installing the allowlisted jellyfin package", 52);
            if let Err(error) = self.apt_install("jellyfin") {
                let rollback = restore_files(&files);
                return Err(combine_rollback(error, rollback));
            }
            progress("Enabling and verifying jellyfin.service", 88);
            self.enable_service("jellyfin.service")?;
            progress("Jellyfin service verified", 96);
            Ok(json!({
                "hook_id": "jellyfin",
                "installed": true,
                "service": "jellyfin.service",
                "service_active": true,
                "next_action": "Open Jellyfin Web and complete its first-run setup.",
                "panel_port": 8096,
                "repository": format!("https://repo.jellyfin.org/{}", platform.id),
                "automatic_reboot": false,
                "completed_at_unix_ms": now_unix_ms(),
            }))
        })();
        if let Some(path) = staging {
            let _ = fs::remove_dir_all(path);
        }
        result
    }

    fn platform(&self) -> Result<HostPlatform, String> {
        let body = fs::read_to_string(&self.config.os_release_path)
            .map_err(|_| "could not read /etc/os-release".to_owned())?;
        if body.len() > 64 * 1024 {
            return Err("the host operating-system metadata is invalid".to_owned());
        }
        let values = parse_os_release(&body)?;
        let architecture = self.host_architecture()?;
        let name = values
            .get("PRETTY_NAME")
            .cloned()
            .unwrap_or_else(|| "Linux".to_owned());
        if let Some((id, codename)) = apt_repo_identity(&values) {
            Ok(HostPlatform {
                id,
                name,
                codename,
                architecture,
            })
        } else {
            Ok(HostPlatform {
                id: required_platform_value(&values, "ID")?.to_ascii_lowercase(),
                name,
                codename: values
                    .get("VERSION_CODENAME")
                    .cloned()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                architecture,
            })
        }
    }

    fn host_architecture(&self) -> Result<String, String> {
        if executable(&self.config.dpkg_binary) {
            match self.runner.run(
                &self.config.dpkg_binary,
                &["--print-architecture".to_owned()],
                Duration::from_secs(10),
            ) {
                Ok(output) if output.success => {
                    let architecture = output.stdout.trim().to_owned();
                    if valid_token(&architecture, 24) {
                        return Ok(architecture);
                    }
                }
                _ => {}
            }
        }
        let architecture = debian_architecture_name();
        if !valid_token(&architecture, 24) {
            return Err("the host architecture is invalid".to_owned());
        }
        Ok(architecture)
    }

    fn private_staging_directory(&self) -> Result<PathBuf, String> {
        if self.config.staging_root.parent() == Some(Path::new("/run/helix")) {
            ensure_root_directory(Path::new("/run/helix"), DirectoryRole::AptWrite)?;
        }
        ensure_root_directory(&self.config.staging_root, DirectoryRole::PrivateStaging)?;
        let path = self.config.staging_root.join(Uuid::new_v4().to_string());
        fs::create_dir(&path).map_err(|error| {
            format!(
                "Helix could not create a private installer workspace at {}: {}.",
                path.display(),
                io_detail(&error)
            )
        })?;
        set_directory_mode_nofollow(&path, 0o700)?;
        Ok(path)
    }

    fn download(&self, url: &str, destination: &Path) -> Result<(), String> {
        if !url.starts_with("https://")
            || !(url.starts_with("https://pkgs.tailscale.com/")
                || url.starts_with("https://repo.jellyfin.org/"))
        {
            return Err("the hook installer refused an untrusted download URL".to_owned());
        }
        let output = self.runner.run(
            &self.config.curl_binary,
            &[
                "--fail".to_owned(),
                "--silent".to_owned(),
                "--show-error".to_owned(),
                "--proto".to_owned(),
                "=https".to_owned(),
                "--tlsv1.2".to_owned(),
                "--connect-timeout".to_owned(),
                "10".to_owned(),
                "--max-time".to_owned(),
                "60".to_owned(),
                "--max-filesize".to_owned(),
                MAX_REPOSITORY_FILE_BYTES.to_string(),
                "--output".to_owned(),
                destination.to_string_lossy().into_owned(),
                url.to_owned(),
            ],
            Duration::from_secs(75),
        )?;
        require_success(output, "official repository download")?;
        let metadata = fs::symlink_metadata(destination)
            .map_err(|_| "the official repository download is unavailable".to_owned())?;
        if !metadata.file_type().is_file()
            || metadata.len() == 0
            || metadata.len() > MAX_REPOSITORY_FILE_BYTES
        {
            return Err("the official repository download is invalid".to_owned());
        }
        Ok(())
    }

    fn apt_install(&self, package: &str) -> Result<(), String> {
        if !matches!(package, "tailscale" | "jellyfin") {
            return Err("the hook package is not allowlisted".to_owned());
        }
        let common = [
            "-o".to_owned(),
            "DPkg::Lock::Timeout=60".to_owned(),
            "-o".to_owned(),
            "Acquire::Retries=3".to_owned(),
            "-o".to_owned(),
            "Dpkg::Use-Pty=0".to_owned(),
        ];
        let mut update = common.to_vec();
        update.push("update".to_owned());
        require_success(
            self.runner.run(
                &self.config.apt_get_binary,
                &update,
                Duration::from_secs(5 * 60),
            )?,
            "APT package-list refresh",
        )?;
        let mut install = common.to_vec();
        install.extend([
            "-y".to_owned(),
            "-o".to_owned(),
            "Dpkg::Options::=--force-confold".to_owned(),
            "install".to_owned(),
            package.to_owned(),
        ]);
        require_success(
            self.runner.run(
                &self.config.apt_get_binary,
                &install,
                Duration::from_secs(20 * 60),
            )?,
            "APT hook installation",
        )?;
        Ok(())
    }

    fn enable_service(&self, unit: &str) -> Result<(), String> {
        if !matches!(unit, "tailscaled.service" | "jellyfin.service") {
            return Err("the hook service is not allowlisted".to_owned());
        }
        require_success(
            self.runner.run(
                &self.config.systemctl_binary,
                &["enable".to_owned(), "--now".to_owned(), unit.to_owned()],
                Duration::from_secs(90),
            )?,
            "hook service activation",
        )?;
        let active = require_success(
            self.runner.run(
                &self.config.systemctl_binary,
                &["is-active".to_owned(), unit.to_owned()],
                Duration::from_secs(15),
            )?,
            "hook service verification",
        )?;
        if active.trim() != "active" {
            return Err("the installed hook service did not reach the active state".to_owned());
        }
        Ok(())
    }

    fn replace_file(&self, path: &Path, body: &[u8], mode: u32) -> Result<SavedFile, String> {
        if body.is_empty() || body.len() > MAX_REPOSITORY_FILE_BYTES as usize {
            return Err("the repository file is outside the safety limit".to_owned());
        }
        let parent = path
            .parent()
            .ok_or_else(|| "the repository file has no parent directory".to_owned())?;
        require_safe_write_directory(parent)?;
        let previous = match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file()
                    || metadata.len() > MAX_REPOSITORY_FILE_BYTES
                    || metadata.uid() != 0
                {
                    return Err(format!(
                        "Helix refused to replace {}. It must be a regular file owned by root and under 1 MiB.",
                        path.display()
                    ));
                }
                Some(fs::read(path).map_err(|_| {
                    format!(
                        "could not back up the existing repository file {}",
                        path.display()
                    )
                })?)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(_) => {
                return Err(format!(
                    "could not inspect the existing repository file {}",
                    path.display()
                ));
            }
        };
        let previous_mode = fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .unwrap_or(mode);
        write_atomic(path, body, mode)?;
        Ok(SavedFile {
            path: path.to_owned(),
            body: previous,
            mode: previous_mode,
        })
    }
}

fn validate_hook_id(value: &str) -> Result<(), String> {
    if matches!(value, "tailscale" | "jellyfin" | "pterodactyl") {
        Ok(())
    } else {
        Err("this hook does not have a Helix installer".to_owned())
    }
}

fn like_has(like: &str, token: &str) -> bool {
    like.split_whitespace().any(|candidate| candidate == token)
}

fn apt_repo_identity(values: &HashMap<String, String>) -> Option<(String, String)> {
    let id = values.get("ID")?.to_ascii_lowercase();
    if !valid_token(&id, 64) {
        return None;
    }
    let like = values
        .get("ID_LIKE")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let ubuntu_codename = values
        .get("UBUNTU_CODENAME")
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| valid_codename(value));
    let version_codename = values
        .get("VERSION_CODENAME")
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| valid_codename(value));
    let debian_codename = values
        .get("DEBIAN_CODENAME")
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| valid_codename(value))
        .or(version_codename.clone());

    if id == "ubuntu" {
        return ubuntu_codename
            .or(version_codename)
            .map(|codename| (id, codename));
    }
    if id == "debian" {
        return debian_codename.map(|codename| (id, codename));
    }
    if (matches!(
        id.as_str(),
        "pop" | "elementary" | "zorin" | "neon" | "linuxlite" | "peppermint" | "bodhi"
    ) || like_has(&like, "ubuntu"))
        && let Some(codename) = ubuntu_codename
    {
        return Some(("ubuntu".to_owned(), codename));
    }
    if matches!(
        id.as_str(),
        "raspbian"
            | "raspberrypi"
            | "kali"
            | "devuan"
            | "linuxmint"
            | "deepin"
            | "uos"
            | "mx"
            | "parrot"
            | "pureos"
            | "trisquel"
    ) || like_has(&like, "debian")
    {
        return debian_codename.map(|codename| ("debian".to_owned(), codename));
    }
    None
}

fn debian_architecture_name() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "amd64".to_owned(),
        "aarch64" => "arm64".to_owned(),
        "arm" => "armhf".to_owned(),
        other => other.to_owned(),
    }
}

fn parse_os_release(body: &str) -> Result<HashMap<String, String>, String> {
    let mut values = HashMap::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        if !key
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
        {
            continue;
        }
        let value = if raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2 {
            raw[1..raw.len() - 1]
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
        } else {
            raw.to_owned()
        };
        if value.len() <= 512 && !value.chars().any(char::is_control) {
            values.insert(key.to_owned(), value);
        }
    }
    if values.is_empty() {
        return Err("the host operating-system metadata is invalid".to_owned());
    }
    Ok(values)
}

fn required_platform_value(values: &HashMap<String, String>, key: &str) -> Result<String, String> {
    values
        .get(key)
        .filter(|value| valid_token(value, 64))
        .cloned()
        .ok_or_else(|| format!("the host operating-system metadata omitted {key}"))
}

fn valid_token(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_codename(value: &str) -> bool {
    valid_token(value, 32)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_tailscale_repository(body: &[u8], platform: &HostPlatform) -> Result<(), String> {
    let text = std::str::from_utf8(body)
        .map_err(|_| "Tailscale returned an invalid repository definition".to_owned())?;
    let expected = format!(
        "deb [signed-by=/usr/share/keyrings/tailscale-archive-keyring.gpg] https://pkgs.tailscale.com/stable/{} {} main",
        platform.id, platform.codename
    );
    let entries = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    if entries != [expected.as_str()] {
        return Err("Tailscale returned an unexpected repository definition".to_owned());
    }
    Ok(())
}

fn validate_jellyfin_repository(body: &str, platform: &HostPlatform) -> Result<(), String> {
    let expected = format!(
        "Types: deb\nURIs: https://repo.jellyfin.org/{}\nSuites: {}\nComponents: main\nArchitectures: {}\nSigned-By: /etc/apt/keyrings/jellyfin.gpg\n",
        platform.id, platform.codename, platform.architecture
    );
    if body != expected {
        return Err("Jellyfin repository definition validation failed".to_owned());
    }
    Ok(())
}

fn check(id: &str, label: &str, status: &str, detail: &str) -> Value {
    json!({ "id": id, "label": label, "status": status, "detail": detail })
}

fn planned_writes(hook_id: &str, config: &HookInstallerConfig) -> Vec<Value> {
    match hook_id {
        "tailscale" => vec![
            json!({
                "path": config.staging_root.display().to_string(),
                "kind": "staging",
            }),
            json!({
                "path": config.share_keyrings_root.join("tailscale-archive-keyring.gpg").display().to_string(),
                "kind": "keyring",
            }),
            json!({
                "path": config.apt_sources_root.join("tailscale.list").display().to_string(),
                "kind": "source",
            }),
        ],
        "jellyfin" => vec![
            json!({
                "path": config.staging_root.display().to_string(),
                "kind": "staging",
            }),
            json!({
                "path": config.apt_keyrings_root.join("jellyfin.gpg").display().to_string(),
                "kind": "keyring",
            }),
            json!({
                "path": config.apt_sources_root.join("jellyfin.sources").display().to_string(),
                "kind": "source",
            }),
        ],
        _ => Vec::new(),
    }
}

fn push_write_directory_check(
    checks: &mut Vec<Value>,
    blockers: &mut Vec<String>,
    id: &str,
    label: &str,
    path: &Path,
    role: DirectoryRole,
) {
    match inspect_write_directory(path, role) {
        DirectoryInspection::Missing => checks.push(check(
            id,
            label,
            "warning",
            &format!(
                "Missing. Helix will create {} as root, mode {:04o}, before downloading anything.",
                path.display(),
                role.intended_mode() & 0o777
            ),
        )),
        DirectoryInspection::Safe { mode } => checks.push(check(
            id,
            label,
            "pass",
            &format!(
                "{} · mode {:04o} · owned by root",
                path.display(),
                mode & 0o777
            ),
        )),
        DirectoryInspection::NeedsModeFix { mode } => checks.push(check(
            id,
            label,
            "warning",
            &mode_fix_detail(path, mode, role),
        )),
        DirectoryInspection::Blocked { reason } => {
            checks.push(check(id, label, "block", &reason));
            blockers.push(reason);
        }
    }
}

fn mode_fix_detail(path: &Path, mode: u32, role: DirectoryRole) -> String {
    let visible = mode & 0o777;
    match role {
        DirectoryRole::PrivateStaging => format!(
            "{} is mode {:04o}. Helix will set it to 0700 before downloading repository files so those files stay private. helix-privd uses umask 0007, which can leave a newly created directory group-writable until that chmod.",
            path.display(),
            visible
        ),
        DirectoryRole::AptWrite => format!(
            "{} is mode {:04o} (group or world writable). Helix will set it to 0755 before adding the signed repository file, so another user cannot drop a fake source there.",
            path.display(),
            visible
        ),
    }
}

fn inspect_write_directory(path: &Path, role: DirectoryRole) -> DirectoryInspection {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => DirectoryInspection::Missing,
        Err(_) => DirectoryInspection::Blocked {
            reason: format!(
                "Helix could not inspect {}. Confirm it is a real directory Helix may read.",
                path.display()
            ),
        },
        Ok(metadata) => classify_directory(path, &metadata, role),
    }
}

fn classify_directory(
    path: &Path,
    metadata: &fs::Metadata,
    role: DirectoryRole,
) -> DirectoryInspection {
    if metadata.file_type().is_symlink() {
        return DirectoryInspection::Blocked {
            reason: format!(
                "{} is a symlink. Helix will not write repository files through a redirected directory because another process could retarget it. Recreate it as a real directory owned by root, mode 0755.",
                path.display()
            ),
        };
    }
    if !metadata.file_type().is_dir() {
        return DirectoryInspection::Blocked {
            reason: format!(
                "{} exists but is not a directory. Move or rename that file, then let Helix create a real directory owned by root.",
                path.display()
            ),
        };
    }
    let mode = metadata.permissions().mode() & 0o7777;
    if metadata.uid() != 0 {
        return DirectoryInspection::Blocked {
            reason: format!(
                "{} is owned by uid {}, not root. Helix will not put a repository file in a directory it does not own as root. Change the owner to root, or recreate the directory.",
                path.display(),
                metadata.uid()
            ),
        };
    }
    match role {
        DirectoryRole::PrivateStaging if mode & 0o777 != 0o700 => {
            DirectoryInspection::NeedsModeFix { mode }
        }
        DirectoryRole::AptWrite if mode & 0o022 != 0 => DirectoryInspection::NeedsModeFix { mode },
        _ => DirectoryInspection::Safe { mode },
    }
}

fn ensure_root_directory(path: &Path, role: DirectoryRole) -> Result<(), String> {
    match inspect_write_directory(path, role) {
        DirectoryInspection::Missing => {
            fs::create_dir_all(path).map_err(|error| {
                format!(
                    "Helix could not create {} ({}). Check that helix-privd may write that path and that nothing is blocking it.",
                    path.display(),
                    io_detail(&error)
                )
            })?;
        }
        DirectoryInspection::Blocked { reason } => return Err(reason),
        DirectoryInspection::Safe { .. } | DirectoryInspection::NeedsModeFix { .. } => {}
    }
    match inspect_write_directory(path, role) {
        DirectoryInspection::Safe { .. } => Ok(()),
        DirectoryInspection::NeedsModeFix { .. } | DirectoryInspection::Missing => {
            let intended = role.intended_mode();
            set_directory_mode_nofollow(path, intended)?;
            require_safe_write_directory(path)
        }
        DirectoryInspection::Blocked { reason } => Err(reason),
    }
}

fn require_safe_write_directory(path: &Path) -> Result<(), String> {
    match inspect_write_directory(path, DirectoryRole::AptWrite) {
        DirectoryInspection::Safe { .. } => Ok(()),
        DirectoryInspection::Missing => Err(format!(
            "{} is missing. Helix expected a real directory owned by root.",
            path.display()
        )),
        DirectoryInspection::NeedsModeFix { mode } => Err(format!(
            "{} is mode {:04o} (group or world writable). Helix refuses that so another user cannot drop a fake repository file. It should be 0755 or 0700, owned by root.",
            path.display(),
            mode & 0o777
        )),
        DirectoryInspection::Blocked { reason } => Err(reason),
    }
}

fn set_directory_mode_nofollow(path: &Path, mode: u32) -> Result<(), String> {
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| {
        format!(
            "Helix could not open {} as a real directory. If it is a symlink, recreate it as a real directory owned by root.",
            path.display()
        )
    })?;
    let stat = rustix::fs::fstat(&descriptor).map_err(|_| {
        format!(
            "Helix could not inspect {} after opening it.",
            path.display()
        )
    })?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_dir() {
        return Err(format!("{} is not a directory.", path.display()));
    }
    if stat.st_uid != 0 {
        return Err(format!(
            "{} is owned by uid {}, not root. Helix will not change mode on a directory it does not own as root.",
            path.display(),
            stat.st_uid
        ));
    }
    rustix::fs::fchmod(&descriptor, rustix::fs::Mode::from_raw_mode(mode)).map_err(|_| {
        format!(
            "Helix could not set {} to mode {:04o}. Check that helix-privd is running as root and that the path is not immutable.",
            path.display(),
            mode & 0o777
        )
    })?;
    Ok(())
}

fn io_detail(error: &std::io::Error) -> String {
    error.to_string().chars().take(200).collect()
}

fn executable(path: &Path) -> bool {
    path.is_absolute()
        && fs::metadata(path).is_ok_and(|metadata| {
            metadata.is_file()
                && metadata.uid() == 0
                && metadata.permissions().mode() & 0o111 != 0
                && metadata.permissions().mode() & 0o022 == 0
        })
}

fn require_success(output: CommandOutput, label: &str) -> Result<String, String> {
    if output.success {
        return Ok(output.stdout);
    }
    let detail = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    Err(if detail.is_empty() {
        format!("{label} failed")
    } else {
        format!(
            "{label} failed: {}",
            detail.chars().take(2_048).collect::<String>()
        )
    })
}

fn read_bounded(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("{label} is unavailable"))?;
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_REPOSITORY_FILE_BYTES
    {
        return Err(format!("{label} is invalid"));
    }
    fs::read(path).map_err(|_| format!("could not read {label}"))
}

fn write_atomic(path: &Path, body: &[u8], mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "the target file has no parent".to_owned())?;
    let temporary = parent.join(format!(".helix-hook-{}.partial", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|_| "could not stage the repository file".to_owned())?;
        file.write_all(body)
            .and_then(|()| file.sync_all())
            .map_err(|_| "could not persist the repository file".to_owned())?;
        fs::rename(&temporary, path)
            .map_err(|_| "could not commit the repository file".to_owned())?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "could not sync the repository directory".to_owned())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn restore_files(files: &[SavedFile]) -> Result<(), String> {
    let mut errors = Vec::new();
    for saved in files.iter().rev() {
        let result = match &saved.body {
            Some(body) => write_atomic(&saved.path, body, saved.mode),
            None => fs::remove_file(&saved.path)
                .map_err(|_| "could not remove a new repository file".to_owned()),
        };
        if let Err(error) = result {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn combine_rollback(error: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => format!("{error}; the previous repository files were restored"),
        Err(rollback) => format!("{error}; repository rollback also failed: {rollback}"),
    }
}

fn validate_config(config: &HookInstallerConfig) -> Result<(), String> {
    for path in [
        &config.os_release_path,
        &config.curl_binary,
        &config.gpg_binary,
        &config.apt_get_binary,
        &config.dpkg_binary,
        &config.systemctl_binary,
        &config.docker_binary,
        &config.timeout_binary,
        &config.apt_sources_root,
        &config.apt_keyrings_root,
        &config.share_keyrings_root,
        &config.staging_root,
    ] {
        if !path.is_absolute() {
            return Err("hook installer paths must be absolute".to_owned());
        }
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

fn default_os_release_path() -> PathBuf {
    PathBuf::from("/etc/os-release")
}
fn default_curl_binary() -> PathBuf {
    PathBuf::from("/usr/bin/curl")
}
fn default_gpg_binary() -> PathBuf {
    PathBuf::from("/usr/bin/gpg")
}
fn default_apt_get_binary() -> PathBuf {
    PathBuf::from("/usr/bin/apt-get")
}
fn default_dpkg_binary() -> PathBuf {
    PathBuf::from("/usr/bin/dpkg")
}
fn default_systemctl_binary() -> PathBuf {
    PathBuf::from("/usr/bin/systemctl")
}
fn default_docker_binary() -> PathBuf {
    PathBuf::from("/usr/bin/docker")
}
fn default_timeout_binary() -> PathBuf {
    PathBuf::from("/usr/bin/timeout")
}
fn default_apt_sources_root() -> PathBuf {
    PathBuf::from("/etc/apt/sources.list.d")
}
fn default_apt_keyrings_root() -> PathBuf {
    PathBuf::from("/etc/apt/keyrings")
}
fn default_share_keyrings_root() -> PathBuf {
    PathBuf::from("/usr/share/keyrings")
}
fn default_staging_root() -> PathBuf {
    PathBuf::from("/run/helix/hook-installs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_parser_accepts_normal_linux_metadata_only() {
        let values = parse_os_release(
            "ID=ubuntu\nPRETTY_NAME=\"Ubuntu 24.04.3 LTS\"\nVERSION_CODENAME=noble\n",
        )
        .unwrap();
        assert_eq!(values["ID"], "ubuntu");
        assert_eq!(values["VERSION_CODENAME"], "noble");
        assert!(!values.contains_key("bad-key"));
    }

    #[test]
    fn mint_and_pop_map_to_ubuntu_apt_identity() {
        let mint = parse_os_release(
            "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\nPRETTY_NAME=\"Linux Mint 22\"\nVERSION_CODENAME=wilma\nUBUNTU_CODENAME=noble\n",
        )
        .unwrap();
        assert_eq!(
            apt_repo_identity(&mint),
            Some(("ubuntu".to_owned(), "noble".to_owned()))
        );
        let pop = parse_os_release(
            "ID=pop\nID_LIKE=\"ubuntu debian\"\nPRETTY_NAME=\"Pop!_OS 24.04 LTS\"\nVERSION_CODENAME=noble\nUBUNTU_CODENAME=noble\n",
        )
        .unwrap();
        assert_eq!(
            apt_repo_identity(&pop),
            Some(("ubuntu".to_owned(), "noble".to_owned()))
        );
    }

    #[test]
    fn fedora_has_no_apt_repo_identity() {
        let fedora = parse_os_release(
            "ID=fedora\nID_LIKE=\"rhel centos fedora\"\nPRETTY_NAME=\"Fedora Linux 42\"\nVERSION_ID=42\n",
        )
        .unwrap();
        assert_eq!(apt_repo_identity(&fedora), None);
        let bazzite = parse_os_release(
            "ID=bazzite\nID_LIKE=fedora\nPRETTY_NAME=\"Bazzite\"\nVERSION_ID=42\n",
        )
        .unwrap();
        assert_eq!(apt_repo_identity(&bazzite), None);
    }

    #[test]
    fn debian_derivatives_map_to_debian_apt_identity() {
        let lmde = parse_os_release(
            "ID=linuxmint\nID_LIKE=debian\nPRETTY_NAME=\"LMDE 6\"\nVERSION_CODENAME=faye\nDEBIAN_CODENAME=bookworm\n",
        )
        .unwrap();
        assert_eq!(
            apt_repo_identity(&lmde),
            Some(("debian".to_owned(), "bookworm".to_owned()))
        );
        let kali = parse_os_release(
            "ID=kali\nID_LIKE=debian\nPRETTY_NAME=\"Kali GNU/Linux Rolling\"\nVERSION_CODENAME=kali-rolling\n",
        )
        .unwrap();
        assert_eq!(
            apt_repo_identity(&kali),
            Some(("debian".to_owned(), "kali-rolling".to_owned()))
        );
        let deepin = parse_os_release(
            "ID=deepin\nID_LIKE=debian\nPRETTY_NAME=\"Deepin 23\"\nVERSION_CODENAME=beige\n",
        )
        .unwrap();
        assert_eq!(
            apt_repo_identity(&deepin),
            Some(("debian".to_owned(), "beige".to_owned()))
        );
    }

    #[test]
    fn debian_architecture_name_uses_the_compiled_cpu() {
        let architecture = debian_architecture_name();
        assert!(
            matches!(
                architecture.as_str(),
                "amd64" | "arm64" | "armhf" | "x86" | "riscv64"
            ) || valid_token(&architecture, 24)
        );
    }

    #[test]
    fn tailscale_repository_must_match_the_detected_release_exactly() {
        let platform = HostPlatform {
            id: "ubuntu".to_owned(),
            name: "Ubuntu".to_owned(),
            codename: "noble".to_owned(),
            architecture: "amd64".to_owned(),
        };
        let valid = b"# Tailscale packages for ubuntu noble\ndeb [signed-by=/usr/share/keyrings/tailscale-archive-keyring.gpg] https://pkgs.tailscale.com/stable/ubuntu noble main\n";
        assert!(validate_tailscale_repository(valid, &platform).is_ok());
        assert!(
            validate_tailscale_repository(b"deb https://example.test stable main\n", &platform)
                .is_err()
        );
    }

    #[test]
    fn only_explicit_hook_installers_are_accepted() {
        for hook in ["tailscale", "jellyfin", "pterodactyl"] {
            assert!(validate_hook_id(hook).is_ok());
        }
        for hook in ["plex", "amp", "tailscale; reboot", "../jellyfin"] {
            assert!(validate_hook_id(hook).is_err());
        }
    }

    #[test]
    fn inspect_explains_missing_symlink_and_non_directory_write_paths() {
        let temporary = tempfile::tempdir().unwrap();
        let missing = temporary.path().join("missing");
        assert!(matches!(
            inspect_write_directory(&missing, DirectoryRole::PrivateStaging),
            DirectoryInspection::Missing
        ));

        let file = temporary.path().join("not-a-dir");
        fs::write(&file, b"x").unwrap();
        match inspect_write_directory(&file, DirectoryRole::AptWrite) {
            DirectoryInspection::Blocked { reason } => {
                assert!(reason.contains(file.to_str().unwrap()));
                assert!(reason.contains("not a directory"));
            }
            other => panic!("{other:?}"),
        }

        let real = temporary.path().join("real");
        let link = temporary.path().join("link");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        match inspect_write_directory(&link, DirectoryRole::AptWrite) {
            DirectoryInspection::Blocked { reason } => {
                assert!(reason.contains(link.to_str().unwrap()));
                assert!(reason.contains("symlink"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ensure_repairs_group_writable_staging_when_running_as_root() {
        let temporary = tempfile::tempdir().unwrap();
        let staging = temporary.path().join("hook-installs");
        fs::create_dir(&staging).unwrap();
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o770)).unwrap();
        let owned_by_root = fs::symlink_metadata(&staging).unwrap().uid() == 0;
        if owned_by_root {
            ensure_root_directory(&staging, DirectoryRole::PrivateStaging).unwrap();
            let mode = fs::symlink_metadata(&staging).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        } else {
            let error = ensure_root_directory(&staging, DirectoryRole::PrivateStaging).unwrap_err();
            assert!(error.contains(staging.to_str().unwrap()));
            assert!(error.contains("owned by uid"));
            assert!(!error.contains("the repository directory is not a root-owned real directory"));
        }
    }

    #[test]
    fn ensure_creates_missing_apt_write_directory_when_running_as_root() {
        let temporary = tempfile::tempdir().unwrap();
        let sources = temporary.path().join("sources.list.d");
        let owned_by_root = fs::symlink_metadata(temporary.path()).unwrap().uid() == 0;
        if owned_by_root {
            ensure_root_directory(&sources, DirectoryRole::AptWrite).unwrap();
            assert!(sources.is_dir());
            let mode = fs::symlink_metadata(&sources).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        } else {
            let error = ensure_root_directory(&sources, DirectoryRole::AptWrite).unwrap_err();
            assert!(error.contains(sources.to_str().unwrap()));
        }
    }

    #[test]
    fn tailscale_and_jellyfin_preflight_list_exact_write_paths() {
        let temporary = tempfile::tempdir().unwrap();
        let config = HookInstallerConfig {
            staging_root: temporary.path().join("hook-installs"),
            apt_sources_root: temporary.path().join("sources.list.d"),
            apt_keyrings_root: temporary.path().join("keyrings"),
            share_keyrings_root: temporary.path().join("share-keyrings"),
            ..HookInstallerConfig::default()
        };
        let installer = HookInstaller::new(config).unwrap();

        let tailscale = installer.preflight("tailscale").unwrap();
        let tailscale_writes = tailscale["writes"].as_array().expect("tailscale writes");
        assert_eq!(tailscale_writes.len(), 3);
        assert_eq!(tailscale_writes[0]["kind"], "staging");
        assert!(
            tailscale_writes[0]["path"]
                .as_str()
                .unwrap()
                .ends_with("hook-installs")
        );
        assert_eq!(tailscale_writes[1]["kind"], "keyring");
        assert!(
            tailscale_writes[1]["path"]
                .as_str()
                .unwrap()
                .ends_with("tailscale-archive-keyring.gpg")
        );
        assert_eq!(tailscale_writes[2]["kind"], "source");
        assert!(
            tailscale_writes[2]["path"]
                .as_str()
                .unwrap()
                .ends_with("tailscale.list")
        );
        let staging = tailscale["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["id"] == "write_staging")
            .expect("staging check");
        assert_eq!(staging["status"], "warning");
        assert!(
            staging["detail"]
                .as_str()
                .unwrap()
                .contains("hook-installs")
        );

        let jellyfin = installer.preflight("jellyfin").unwrap();
        let jellyfin_writes = jellyfin["writes"].as_array().expect("jellyfin writes");
        assert_eq!(jellyfin_writes.len(), 3);
        assert!(
            jellyfin_writes[1]["path"]
                .as_str()
                .unwrap()
                .ends_with("jellyfin.gpg")
        );
        assert!(
            jellyfin_writes[2]["path"]
                .as_str()
                .unwrap()
                .ends_with("jellyfin.sources")
        );

        let guided = installer.preflight("pterodactyl").unwrap();
        assert_eq!(guided["writes"].as_array().unwrap().len(), 0);
        assert!(
            guided["checks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|check| check["id"] != "write_staging")
        );
    }
}
