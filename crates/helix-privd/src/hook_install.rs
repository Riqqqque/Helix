use crate::bounded_command::run_bounded_command;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write as _,
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
                    blockers.push("One-click Tailscale and Jellyfin installs currently require a Debian-family APT release (Debian, Ubuntu, or a derivative with UBUNTU_CODENAME or a Debian codename).");
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
                    blockers
                        .push("This architecture is not in Helix's tested installer allowlist.");
                }
            }
            Err(error) => {
                checks.push(check(
                    "operating_system",
                    "Supported Linux release",
                    "block",
                    error,
                ));
                blockers.push("Helix could not verify the host distribution and architecture.");
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
                blockers.push(match id {
                    "curl" => "curl is required for verified HTTPS downloads.",
                    "apt" => "apt-get is required for package installation.",
                    "dpkg" => "dpkg is required for Debian package installs.",
                    "timeout" => "timeout is required to enforce command deadlines.",
                    _ => "systemctl is required to verify the installed service.",
                });
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
                blockers.push("GnuPG is required to convert Jellyfin's official signing key.");
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
                blockers
                    .push("Wings requires Docker before its node configuration can be validated.");
            }
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
        let staging = self.private_staging_directory()?;
        let key_download = staging.join("tailscale-keyring.gpg");
        let list_download = staging.join("tailscale.list");
        let key_url = format!(
            "https://pkgs.tailscale.com/stable/{}/{}.noarmor.gpg",
            platform.id, platform.codename
        );
        let list_url = format!(
            "https://pkgs.tailscale.com/stable/{}/{}.tailscale-keyring.list",
            platform.id, platform.codename
        );
        let result = (|| {
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
            fs::create_dir_all(&self.config.share_keyrings_root)
                .map_err(|_| "could not create the shared APT keyring directory".to_owned())?;
            fs::create_dir_all(&self.config.apt_sources_root)
                .map_err(|_| "could not create the APT source directory".to_owned())?;
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
        let _ = fs::remove_dir_all(staging);
        result
    }

    fn install_jellyfin(
        &self,
        platform: &HostPlatform,
        progress: &dyn Fn(&str, u8),
    ) -> Result<Value, String> {
        let staging = self.private_staging_directory()?;
        let key_download = staging.join("jellyfin.asc");
        let keyring = staging.join("jellyfin.gpg");
        let result = (|| {
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
            fs::create_dir_all(&self.config.apt_keyrings_root)
                .map_err(|_| "could not create the APT keyring directory".to_owned())?;
            fs::create_dir_all(&self.config.apt_sources_root)
                .map_err(|_| "could not create the APT source directory".to_owned())?;
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
        let _ = fs::remove_dir_all(staging);
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
        fs::create_dir_all(&self.config.staging_root)
            .map_err(|_| "could not create the hook installer staging directory".to_owned())?;
        require_root_directory(&self.config.staging_root)?;
        fs::set_permissions(&self.config.staging_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the hook installer staging directory".to_owned())?;
        let path = self.config.staging_root.join(Uuid::new_v4().to_string());
        fs::create_dir(&path)
            .map_err(|_| "could not create a private hook installer workspace".to_owned())?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the hook installer workspace".to_owned())?;
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
        require_root_directory(parent)?;
        let previous = match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file()
                    || metadata.len() > MAX_REPOSITORY_FILE_BYTES
                    || metadata.uid() != 0
                {
                    return Err("Helix refused to replace an unsafe repository file".to_owned());
                }
                Some(
                    fs::read(path)
                        .map_err(|_| "could not back up the existing repository file".to_owned())?,
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("could not inspect the existing repository file".to_owned()),
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

fn require_root_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "the repository directory is unavailable".to_owned())?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != 0
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err("the repository directory is not a root-owned real directory".to_owned());
    }
    Ok(())
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
}
