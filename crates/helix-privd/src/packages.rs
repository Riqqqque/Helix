use crate::bounded_command::run_bounded_command;
use helix_privd::PackageUpdateCandidate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_COMMAND_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PACKAGES: usize = 5_000;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_ERRORS: usize = 16;
const MAX_REBOOT_PACKAGE_MARKER_BYTES: u64 = 256 * 1024;
const MAX_PACKAGE_UPDATES: usize = 512;
const PACKAGE_UPDATE_DISK_HEADROOM_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageConfig {
    #[serde(default = "default_dpkg_query_binary")]
    pub dpkg_query_binary: PathBuf,
    #[serde(default = "default_apt_cache_binary")]
    pub apt_cache_binary: PathBuf,
    #[serde(default = "default_apt_get_binary")]
    pub apt_get_binary: PathBuf,
    #[serde(default = "default_apt_mark_binary")]
    pub apt_mark_binary: PathBuf,
    #[serde(default = "default_timeout_binary")]
    pub timeout_binary: PathBuf,
    #[serde(default = "default_apt_lists_root")]
    pub apt_lists_root: PathBuf,
    #[serde(default = "default_apt_archives_root")]
    pub apt_archives_root: PathBuf,
    #[serde(default = "default_reboot_required_path")]
    pub reboot_required_path: PathBuf,
    #[serde(default = "default_reboot_packages_path")]
    pub reboot_packages_path: PathBuf,
}

impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            dpkg_query_binary: default_dpkg_query_binary(),
            apt_cache_binary: default_apt_cache_binary(),
            apt_get_binary: default_apt_get_binary(),
            apt_mark_binary: default_apt_mark_binary(),
            timeout_binary: default_timeout_binary(),
            apt_lists_root: default_apt_lists_root(),
            apt_archives_root: default_apt_archives_root(),
            reboot_required_path: default_reboot_required_path(),
            reboot_packages_path: default_reboot_packages_path(),
        }
    }
}

pub struct PackageManager {
    config: PackageConfig,
    runner: Arc<dyn PackageCommandRunner>,
    assume_binaries_available: bool,
    mutation: Mutex<()>,
}

#[derive(Clone, Debug)]
struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

trait PackageCommandRunner: Send + Sync {
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

impl PackageCommandRunner for ProcessRunner {
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

#[derive(Clone, Debug, Serialize)]
struct PackageRecord {
    name: String,
    installed_version: String,
    candidate_version: Option<String>,
    upgrade_available: bool,
    held: Option<bool>,
    download_size_bytes: Option<u64>,
    installed_size_bytes: Option<u64>,
    source_package: Option<String>,
    candidate_origin: Option<String>,
    category: Option<String>,
    description: String,
    security_update: Option<bool>,
    restart_hint: String,
    restart_impact_known: bool,
}

#[derive(Clone, Debug)]
struct CandidateMetadata {
    version: String,
    download_size_bytes: Option<u64>,
    installed_size_bytes: Option<u64>,
    source_package: Option<String>,
    category: Option<String>,
    description: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct SimulationSummary {
    available: bool,
    upgrade_candidates: usize,
    new_packages: usize,
    removals: usize,
    held_back: usize,
    #[serde(skip_serializing)]
    candidates: HashMap<String, SimulatedCandidate>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct SimulatedCandidate {
    version: String,
    origin: Option<String>,
    security: bool,
}

impl PackageManager {
    pub fn new(config: PackageConfig) -> Result<Self, String> {
        validate_config(&config)?;
        Ok(Self {
            runner: Arc::new(ProcessRunner {
                timeout_binary: config.timeout_binary.clone(),
            }),
            config,
            assume_binaries_available: false,
            mutation: Mutex::new(()),
        })
    }

    #[cfg(test)]
    fn with_runner(
        config: PackageConfig,
        runner: Arc<dyn PackageCommandRunner>,
    ) -> Result<Self, String> {
        validate_config(&config)?;
        Ok(Self {
            config,
            runner,
            assume_binaries_available: true,
            mutation: Mutex::new(()),
        })
    }

    pub fn inventory(&self) -> Result<Value, String> {
        let collected_at_unix_ms = now_unix_ms();
        let cache_refreshed_at_unix_ms = newest_mtime_ms(&self.config.apt_lists_root);
        let mut errors = Vec::new();
        let dpkg_available = self.binary_available(&self.config.dpkg_query_binary);
        let apt_cache_available = self.binary_available(&self.config.apt_cache_binary);
        let apt_get_available = self.binary_available(&self.config.apt_get_binary);
        let apt_mark_available = self.binary_available(&self.config.apt_mark_binary);

        let mut packages = if dpkg_available {
            match self.query_installed_packages() {
                Ok(packages) => packages,
                Err(error) => {
                    push_error(&mut errors, "dpkg_query", &error);
                    Vec::new()
                }
            }
        } else {
            push_error(
                &mut errors,
                "dpkg_query",
                "the configured dpkg-query binary is unavailable",
            );
            Vec::new()
        };
        let installed_total = packages.len();
        let truncated = installed_total >= MAX_PACKAGES;

        let held = if apt_mark_available {
            match self.query_held_packages() {
                Ok(held) => Some(held),
                Err(error) => {
                    push_error(&mut errors, "apt_mark", &error);
                    None
                }
            }
        } else {
            push_error(
                &mut errors,
                "apt_mark",
                "the configured apt-mark binary is unavailable",
            );
            None
        };

        let simulation = if apt_get_available {
            match self.simulate_upgrade() {
                Ok(simulation) => simulation,
                Err(error) => SimulationSummary {
                    error: Some(error),
                    ..SimulationSummary::default()
                },
            }
        } else {
            SimulationSummary {
                error: Some("the configured apt-get binary is unavailable".to_owned()),
                ..SimulationSummary::default()
            }
        };
        if let Some(error) = &simulation.error {
            push_error(&mut errors, "apt_simulation", error);
        }

        if apt_cache_available && !packages.is_empty() {
            match self.query_candidate_metadata(&packages) {
                Ok(metadata) => enrich_candidates(&mut packages, &metadata),
                Err(error) => push_error(&mut errors, "apt_cache", &error),
            }
        } else if !apt_cache_available {
            push_error(
                &mut errors,
                "apt_cache",
                "the configured apt-cache binary is unavailable",
            );
        }

        let reboot_packages = read_package_set(&self.config.reboot_packages_path);
        let reboot_required = self.config.reboot_required_path.is_file();
        for package in &mut packages {
            package.held = held.as_ref().map(|held| {
                held.contains(&package.name)
                    || held.contains(package.name.split(':').next().unwrap_or(&package.name))
            });
            if let Some(candidate) = simulation.candidates.get(&package.name).or_else(|| {
                simulation
                    .candidates
                    .get(package.name.split(':').next().unwrap_or(&package.name))
            }) {
                package.candidate_version = Some(candidate.version.clone());
                package.candidate_origin = candidate.origin.clone();
                package.security_update = Some(candidate.security);
            }
            package.upgrade_available = package
                .candidate_version
                .as_ref()
                .is_some_and(|candidate| candidate != &package.installed_version);
            let base = package.name.split(':').next().unwrap_or(&package.name);
            if reboot_packages.contains(&package.name) || reboot_packages.contains(base) {
                package.restart_hint = "host_reboot_requested".to_owned();
                package.restart_impact_known = true;
            }
        }
        packages.sort_by(|left, right| {
            right
                .upgrade_available
                .cmp(&left.upgrade_available)
                .then_with(|| left.name.cmp(&right.name))
        });

        let upgrade_count = packages
            .iter()
            .filter(|package| package.upgrade_available)
            .count();
        let security_count = packages
            .iter()
            .filter(|package| package.security_update == Some(true))
            .count();
        let apply_available = dpkg_available
            && apt_cache_available
            && apt_get_available
            && apt_mark_available
            && simulation.available
            && !packages.is_empty();
        let (apply_reason_code, apply_reason) = if apply_available {
            (
                "selected_exact_candidates_supported",
                "Helix can apply explicitly selected, exact APT candidates in one serialized background job. It rechecks installed and candidate versions, held packages, free download space, a no-removal simulation, and the final installed versions. Existing configuration files are preserved and Linux is never rebooted automatically.",
            )
        } else {
            (
                "package_safety_evidence_unavailable",
                "Selected updates remain unavailable until dpkg-query, apt-cache, apt-get, apt-mark, and a current read-only simulation all return usable evidence.",
            )
        };
        Ok(json!({
            "schema_version": 1,
            "availability": if packages.is_empty() { "unavailable" } else if errors.is_empty() { "ready" } else { "degraded" },
            "collected_at_unix_ms": collected_at_unix_ms,
            "apt_cache_refreshed_at_unix_ms": cache_refreshed_at_unix_ms,
            "apt_cache_refresh_performed": false,
            "inventory": {
                "installed_total": installed_total,
                "upgrade_available_total": upgrade_count,
                "security_update_total": security_count,
                "truncated": truncated,
                "packages": packages
            },
            "simulation": {
                "available": simulation.available,
                "upgrade_candidates": simulation.upgrade_candidates,
                "new_packages": simulation.new_packages,
                "removals": simulation.removals,
                "held_back": simulation.held_back,
                "error": simulation.error,
                "state_can_change_after_simulation": true,
                "mutated_package_state": false
            },
            "host_restart": {
                "reboot_required_marker_present": reboot_required,
                "packages": reboot_packages,
                "automatic_reboot": false
            },
            "upgrade_apply": {
                "available": apply_available,
                "reason_code": apply_reason_code,
                "reason": apply_reason,
                "would_require_explicit_package_candidates": true,
                "would_require_disruption_acknowledgement": true,
                "required_capability": "system.packages.write",
                "rollback_claimed": false,
                "automatic_reboot": false,
                "apt_or_dpkg_mutation_available": apt_get_available,
                "package_lists_refresh_available": apt_get_available,
                "conffile_policy": "preserve_existing",
                "new_packages_allowed": false,
                "package_removals_allowed": false
            },
            "helix_self_update": {
                "available": false,
                "reason_code": "verified_release_pipeline_not_implemented",
                "reason": "Helix self-update remains unavailable until releases are signed and digest-pinned and the updater has staging, configuration/data backup, health verification, and automatic rollback.",
                "git_pull_used": false
            },
            "tools": {
                "dpkg_query": dpkg_available,
                "apt_cache": apt_cache_available,
                "apt_get": apt_get_available,
                "apt_mark": apt_mark_available
            },
            "errors": errors
        }))
    }

    pub fn refresh_lists(&self) -> Result<Value, String> {
        if !self.binary_available(&self.config.apt_get_binary) {
            return Err("the configured apt-get binary is unavailable".to_owned());
        }
        let _mutation = self
            .mutation
            .try_lock()
            .map_err(|_| "another package operation is already running".to_owned())?;
        let started_at_unix_ms = now_unix_ms();
        let output = self.runner.run(
            &self.config.apt_get_binary,
            &[
                "-o".to_owned(),
                "DPkg::Lock::Timeout=30".to_owned(),
                "-o".to_owned(),
                "Acquire::Retries=3".to_owned(),
                "-o".to_owned(),
                "Dpkg::Use-Pty=0".to_owned(),
                "update".to_owned(),
            ],
            Duration::from_secs(5 * 60),
        )?;
        require_success(&output)?;
        Ok(json!({
            "refreshed": true,
            "started_at_unix_ms": started_at_unix_ms,
            "completed_at_unix_ms": now_unix_ms(),
            "apt_cache_refreshed_at_unix_ms": newest_mtime_ms(&self.config.apt_lists_root),
            "package_state_mutated": false,
            "automatic_reboot": false,
            "note": "APT package lists were refreshed. No package was installed, removed, or upgraded."
        }))
    }

    pub fn apply_updates(
        &self,
        requested: &[PackageUpdateCandidate],
        confirmation: &str,
        disruption_acknowledged: bool,
    ) -> Result<Value, String> {
        validate_update_request(requested, confirmation, disruption_acknowledged)?;
        if ![
            &self.config.dpkg_query_binary,
            &self.config.apt_cache_binary,
            &self.config.apt_get_binary,
            &self.config.apt_mark_binary,
        ]
        .iter()
        .all(|path| self.binary_available(path))
        {
            return Err("the required dpkg and APT tools are unavailable".to_owned());
        }
        let _mutation = self
            .mutation
            .try_lock()
            .map_err(|_| "another package operation is already running".to_owned())?;
        let started_at_unix_ms = now_unix_ms();

        let installed = self.query_installed_packages()?;
        let installed_by_name = installed
            .iter()
            .map(|package| (package.name.as_str(), package))
            .collect::<HashMap<_, _>>();
        let held = self.query_held_packages()?;
        let mut selected_records = Vec::with_capacity(requested.len());
        for request in requested {
            let package = installed_by_name
                .get(request.name.as_str())
                .copied()
                .or_else(|| {
                    installed_by_name
                        .get(request.name.split(':').next().unwrap_or(&request.name))
                        .copied()
                })
                .ok_or_else(|| {
                    format!(
                        "{} is no longer reported as an installed package; refresh the list and choose updates again",
                        request.name
                    )
                })?;
            if package.installed_version != request.installed_version {
                return Err(format!(
                    "{} changed from version {} to {}; refresh the list before applying updates",
                    request.name, request.installed_version, package.installed_version
                ));
            }
            let base = request.name.split(':').next().unwrap_or(&request.name);
            if held.contains(&request.name) || held.contains(base) {
                return Err(format!(
                    "{} is held by APT and Helix will not override that hold",
                    request.name
                ));
            }
            selected_records.push(package.clone());
        }

        let candidates = self.query_candidate_metadata(&selected_records)?;
        let mut download_bytes = 0_u64;
        for request in requested {
            let base = request.name.split(':').next().unwrap_or(&request.name);
            let candidate = candidates
                .get(&request.name)
                .or_else(|| candidates.get(base))
                .ok_or_else(|| {
                    format!(
                        "APT no longer reports a candidate for {}; refresh the package list",
                        request.name
                    )
                })?;
            if candidate.version != request.candidate_version
                || request.candidate_version == request.installed_version
            {
                return Err(format!(
                    "the selected candidate for {} changed; refresh the list before applying updates",
                    request.name
                ));
            }
            download_bytes = download_bytes
                .checked_add(candidate.download_size_bytes.ok_or_else(|| {
                    format!(
                        "APT did not report the download size for {}; Helix cannot prove the disk-space gate",
                        request.name
                    )
                })?)
                .ok_or_else(|| "the selected package sizes exceed supported bounds".to_owned())?;
        }
        let available_bytes = fs2::available_space(&self.config.apt_archives_root)
            .map_err(|_| "could not measure free space for APT downloads".to_owned())?;
        let required_bytes = download_bytes
            .checked_add(PACKAGE_UPDATE_DISK_HEADROOM_BYTES)
            .ok_or_else(|| "the selected package sizes exceed supported bounds".to_owned())?;
        if available_bytes < required_bytes {
            return Err(format!(
                "selected updates need {required_bytes} free bytes including Helix's 512 MiB safety headroom, but the APT filesystem reports only {available_bytes} bytes available"
            ));
        }

        let simulation_args = selected_update_args(requested, true);
        let simulation_output = self.runner.run(
            &self.config.apt_get_binary,
            &simulation_args,
            Duration::from_secs(2 * 60),
        )?;
        require_success(&simulation_output)?;
        let simulation = parse_apt_simulation(&simulation_output.stdout);
        verify_selected_simulation(requested, &simulation)?;

        let apply_output = self.runner.run(
            &self.config.apt_get_binary,
            &selected_update_args(requested, false),
            Duration::from_secs(30 * 60),
        )?;
        require_success(&apply_output)?;

        let after = self.query_installed_packages()?;
        let after_by_name = after
            .iter()
            .map(|package| (package.name.as_str(), package.installed_version.as_str()))
            .collect::<HashMap<_, _>>();
        for request in requested {
            let installed_version =
                after_by_name
                    .get(request.name.as_str())
                    .copied()
                    .or_else(|| {
                        after_by_name
                            .get(request.name.split(':').next().unwrap_or(&request.name))
                            .copied()
                    });
            if installed_version != Some(request.candidate_version.as_str()) {
                return Err(format!(
                    "APT exited successfully, but {} did not verify at the selected candidate version",
                    request.name
                ));
            }
        }
        let reboot_packages = read_package_set(&self.config.reboot_packages_path);
        Ok(json!({
            "updated": requested.iter().map(|package| json!({
                "name": package.name,
                "from": package.installed_version,
                "to": package.candidate_version
            })).collect::<Vec<_>>(),
            "started_at_unix_ms": started_at_unix_ms,
            "completed_at_unix_ms": now_unix_ms(),
            "download_size_bytes": download_bytes,
            "free_space_before_bytes": available_bytes,
            "reboot_required": self.config.reboot_required_path.is_file(),
            "reboot_required_packages": reboot_packages,
            "configuration_policy": "preserve_existing",
            "rollback_claimed": false,
            "automatic_reboot": false,
            "note": "Every selected version verified after APT completed. Package maintainer scripts can restart affected services; Linux was not rebooted."
        }))
    }

    fn query_installed_packages(&self) -> Result<Vec<PackageRecord>, String> {
        let format = "${binary:Package}\t${Version}\t${Installed-Size}\t${source:Package}\t${Section}\t${db:Status-Abbrev}\t${binary:Synopsis}\n";
        let output = self.runner.run(
            &self.config.dpkg_query_binary,
            &["-W".to_owned(), "-f".to_owned(), format.to_owned()],
            Duration::from_secs(5),
        )?;
        require_success(&output)?;
        Ok(parse_dpkg_query(&output.stdout))
    }

    fn query_held_packages(&self) -> Result<HashSet<String>, String> {
        let output = self.runner.run(
            &self.config.apt_mark_binary,
            &["showhold".to_owned()],
            Duration::from_secs(3),
        )?;
        require_success(&output)?;
        Ok(output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|name| valid_package_name(name))
            .take(MAX_PACKAGES)
            .map(str::to_owned)
            .collect())
    }

    fn simulate_upgrade(&self) -> Result<SimulationSummary, String> {
        let output = self.runner.run(
            &self.config.apt_get_binary,
            &[
                "--simulate".to_owned(),
                "-o".to_owned(),
                "Dpkg::Options::=--force-confold".to_owned(),
                "upgrade".to_owned(),
            ],
            Duration::from_secs(8),
        )?;
        require_success(&output)?;
        Ok(parse_apt_simulation(&output.stdout))
    }

    fn query_candidate_metadata(
        &self,
        installed: &[PackageRecord],
    ) -> Result<HashMap<String, CandidateMetadata>, String> {
        let mut args = vec!["--no-all-versions".to_owned(), "show".to_owned()];
        args.extend(
            installed
                .iter()
                .map(|package| package.name.clone())
                .filter(|name| valid_package_name(name))
                .take(MAX_PACKAGES),
        );
        let output =
            self.runner
                .run(&self.config.apt_cache_binary, &args, Duration::from_secs(8))?;
        require_success(&output)?;
        Ok(parse_apt_cache_show(&output.stdout))
    }

    fn binary_available(&self, path: &Path) -> bool {
        self.assume_binaries_available || path.is_file()
    }
}

fn validate_update_request(
    requested: &[PackageUpdateCandidate],
    confirmation: &str,
    disruption_acknowledged: bool,
) -> Result<(), String> {
    if requested.is_empty() || requested.len() > MAX_PACKAGE_UPDATES {
        return Err(format!(
            "select between 1 and {MAX_PACKAGE_UPDATES} package updates"
        ));
    }
    let expected = format!(
        "APPLY {} UPDATE{}",
        requested.len(),
        if requested.len() == 1 { "" } else { "S" }
    );
    if confirmation != expected {
        return Err(format!("type {expected} to confirm the selected updates"));
    }
    if !disruption_acknowledged {
        return Err(
            "package update disruption must be acknowledged before applying updates".to_owned(),
        );
    }
    let mut names = HashSet::with_capacity(requested.len());
    for package in requested {
        if !valid_package_name(&package.name)
            || !valid_package_version(&package.installed_version)
            || !valid_package_version(&package.candidate_version)
            || !names.insert(package.name.as_str())
        {
            return Err("the selected package update list is invalid".to_owned());
        }
    }
    Ok(())
}

fn selected_update_args(requested: &[PackageUpdateCandidate], simulate: bool) -> Vec<String> {
    let mut args = Vec::with_capacity(22 + requested.len());
    if simulate {
        args.push("--simulate".to_owned());
    } else {
        args.push("--assume-yes".to_owned());
    }
    args.extend([
        "--no-remove".to_owned(),
        "--only-upgrade".to_owned(),
        "-o".to_owned(),
        "DPkg::Lock::Timeout=30".to_owned(),
        "-o".to_owned(),
        "Dpkg::Options::=--force-confold".to_owned(),
        "-o".to_owned(),
        "Dpkg::Use-Pty=0".to_owned(),
        "-o".to_owned(),
        "APT::Get::AllowUnauthenticated=false".to_owned(),
        "-o".to_owned(),
        "APT::Get::allow-downgrades=false".to_owned(),
        "-o".to_owned(),
        "APT::Get::allow-remove-essential=false".to_owned(),
        "-o".to_owned(),
        "APT::Get::allow-change-held-packages=false".to_owned(),
        "install".to_owned(),
    ]);
    let mut exact = requested
        .iter()
        .map(|package| format!("{}={}", package.name, package.candidate_version))
        .collect::<Vec<_>>();
    exact.sort();
    args.extend(exact);
    args
}

fn verify_selected_simulation(
    requested: &[PackageUpdateCandidate],
    simulation: &SimulationSummary,
) -> Result<(), String> {
    if !simulation.available {
        return Err("APT could not simulate the selected updates".to_owned());
    }
    if simulation.removals != 0 || simulation.new_packages != 0 {
        return Err(format!(
            "the selected update simulation proposed {} removal(s) and {} new package(s); Helix did not apply it",
            simulation.removals, simulation.new_packages
        ));
    }
    for request in requested {
        let candidate = simulation.candidates.get(&request.name).or_else(|| {
            simulation
                .candidates
                .get(request.name.split(':').next().unwrap_or(&request.name))
        });
        if candidate.map(|candidate| candidate.version.as_str())
            != Some(request.candidate_version.as_str())
        {
            return Err(format!(
                "APT's final simulation did not contain the exact selected version of {}; nothing was applied",
                request.name
            ));
        }
    }
    let requested_names = requested
        .iter()
        .flat_map(|package| {
            [
                package.name.as_str(),
                package.name.split(':').next().unwrap_or(&package.name),
            ]
        })
        .collect::<HashSet<_>>();
    if simulation
        .candidates
        .keys()
        .any(|name| !requested_names.contains(name.as_str()))
    {
        return Err(
            "APT's final simulation included an unselected package; nothing was applied".to_owned(),
        );
    }
    Ok(())
}

fn parse_dpkg_query(output: &str) -> Vec<PackageRecord> {
    output
        .lines()
        .filter_map(|line| {
            let fields = line.splitn(7, '\t').collect::<Vec<_>>();
            if fields.len() != 7
                || !valid_package_name(fields[0])
                || fields[1].is_empty()
                || !fields[5].starts_with("ii ")
            {
                return None;
            }
            Some(PackageRecord {
                name: fields[0].to_owned(),
                installed_version: sanitize_text(fields[1], 200),
                candidate_version: None,
                upgrade_available: false,
                held: None,
                download_size_bytes: None,
                installed_size_bytes: fields[2]
                    .parse::<u64>()
                    .ok()
                    .map(|kib| kib.saturating_mul(1_024)),
                source_package: nonempty_sanitized(fields[3], 200),
                candidate_origin: None,
                category: nonempty_sanitized(fields[4], 120),
                description: sanitize_text(fields[6], MAX_DESCRIPTION_CHARS),
                security_update: None,
                restart_hint: "unknown".to_owned(),
                restart_impact_known: false,
            })
        })
        .take(MAX_PACKAGES)
        .collect()
}

fn parse_apt_cache_show(output: &str) -> HashMap<String, CandidateMetadata> {
    output
        .split("\n\n")
        .filter_map(parse_candidate_stanza)
        .take(MAX_PACKAGES)
        .flat_map(|(name, architecture, metadata)| {
            let qualified = architecture.map(|architecture| format!("{name}:{architecture}"));
            std::iter::once((name, metadata.clone())).chain(qualified.map(|name| (name, metadata)))
        })
        .collect()
}

fn parse_candidate_stanza(stanza: &str) -> Option<(String, Option<String>, CandidateMetadata)> {
    let mut fields = HashMap::new();
    for line in stanza.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if !line.starts_with(' ') {
            fields.entry(key.trim()).or_insert(value.trim());
        }
    }
    let name = fields.get("Package").copied()?.to_owned();
    if !valid_package_name(&name) {
        return None;
    }
    let version = sanitize_text(fields.get("Version").copied()?, 200);
    let source_package = fields
        .get("Source")
        .copied()
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| nonempty_sanitized(value, 200));
    let metadata = CandidateMetadata {
        version,
        download_size_bytes: fields.get("Size").and_then(|value| value.parse().ok()),
        installed_size_bytes: fields
            .get("Installed-Size")
            .and_then(|value| value.parse::<u64>().ok())
            .map(|kib| kib.saturating_mul(1_024)),
        source_package,
        category: fields
            .get("Section")
            .and_then(|value| nonempty_sanitized(value, 120)),
        description: fields
            .get("Description")
            .and_then(|value| nonempty_sanitized(value, MAX_DESCRIPTION_CHARS)),
    };
    let architecture = fields
        .get("Architecture")
        .and_then(|value| nonempty_sanitized(value, 40));
    Some((name, architecture, metadata))
}

fn enrich_candidates(
    installed: &mut [PackageRecord],
    candidates: &HashMap<String, CandidateMetadata>,
) {
    for package in installed {
        let base = package.name.split(':').next().unwrap_or(&package.name);
        let Some(candidate) = candidates
            .get(&package.name)
            .or_else(|| candidates.get(base))
        else {
            continue;
        };
        package.candidate_version = Some(candidate.version.clone());
        package.download_size_bytes = candidate.download_size_bytes;
        package.installed_size_bytes = candidate
            .installed_size_bytes
            .or(package.installed_size_bytes);
        package.source_package = candidate
            .source_package
            .clone()
            .or_else(|| package.source_package.clone());
        package.category = candidate
            .category
            .clone()
            .or_else(|| package.category.clone());
        if let Some(description) = &candidate.description {
            package.description = description.clone();
        }
    }
}

fn parse_apt_simulation(output: &str) -> SimulationSummary {
    let mut summary = SimulationSummary {
        available: true,
        ..SimulationSummary::default()
    };
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Inst ") {
            let Some(name) = rest.split_whitespace().next() else {
                continue;
            };
            if !valid_package_name(name) {
                continue;
            }
            let candidate = rest.find('(').map(|start| &rest[start + 1..]);
            let version = candidate
                .and_then(|candidate| candidate.split_whitespace().next())
                .map(|value| sanitize_text(value, 200));
            if let Some(version) = version {
                let origin = candidate.and_then(|candidate| {
                    candidate
                        .strip_prefix(&version)
                        .map(|value| value.trim().trim_end_matches(')'))
                        .and_then(|value| nonempty_sanitized(value, 300))
                });
                summary.candidates.insert(
                    name.to_owned(),
                    SimulatedCandidate {
                        version,
                        origin,
                        security: rest.to_ascii_lowercase().contains("security"),
                    },
                );
            }
        } else if line.starts_with("Remv ") {
            summary.removals = summary.removals.saturating_add(1);
        } else if line.contains(" newly installed,") && line.contains(" not upgraded.") {
            let words = line.split_whitespace().collect::<Vec<_>>();
            if let Some(index) = words.iter().position(|word| *word == "newly") {
                summary.new_packages = index
                    .checked_sub(1)
                    .and_then(|index| words.get(index))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(summary.new_packages);
            }
            if let Some(index) = words.iter().position(|word| *word == "to") {
                summary.removals = index
                    .checked_sub(1)
                    .and_then(|index| words.get(index))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(summary.removals);
            }
            if let Some(index) = words.iter().position(|word| *word == "not") {
                summary.held_back = index
                    .checked_sub(1)
                    .and_then(|index| words.get(index))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(summary.held_back);
            }
        }
    }
    summary.upgrade_candidates = summary.candidates.len();
    summary
}

fn read_package_set(path: &Path) -> HashSet<String> {
    let body = (|| {
        let descriptor = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
        )
        .ok()?;
        let file = File::from(descriptor);
        let metadata = file.metadata().ok()?;
        if !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > MAX_REBOOT_PACKAGE_MARKER_BYTES
        {
            return None;
        }
        let mut body = String::with_capacity(usize::try_from(metadata.len()).ok()?);
        file.take(MAX_REBOOT_PACKAGE_MARKER_BYTES.saturating_add(1))
            .read_to_string(&mut body)
            .ok()?;
        (u64::try_from(body.len()).ok()? <= MAX_REBOOT_PACKAGE_MARKER_BYTES).then_some(body)
    })();
    body.into_iter()
        .flat_map(|body| {
            body.lines()
                .map(str::trim)
                .filter(|name| valid_package_name(name))
                .take(MAX_PACKAGES)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn newest_mtime_ms(path: &Path) -> Option<u64> {
    let entries = fs::read_dir(path).ok()?;
    entries
        .flatten()
        .take(20_000)
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
        .and_then(system_time_ms)
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

fn valid_package_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.' | b':' | b'~')
        })
}

fn valid_package_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value.is_ascii()
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.' | b':' | b'~')
        })
}

fn validate_config(config: &PackageConfig) -> Result<(), String> {
    if [
        &config.dpkg_query_binary,
        &config.apt_cache_binary,
        &config.apt_get_binary,
        &config.apt_mark_binary,
        &config.timeout_binary,
        &config.apt_lists_root,
        &config.apt_archives_root,
        &config.reboot_required_path,
        &config.reboot_packages_path,
    ]
    .iter()
    .any(|path| !path.is_absolute())
    {
        return Err("package inventory paths must be absolute".to_owned());
    }
    Ok(())
}

fn require_success(output: &CommandOutput) -> Result<(), String> {
    if output.success {
        Ok(())
    } else {
        let message = if output.stderr.trim().is_empty() {
            output.stdout.trim()
        } else {
            output.stderr.trim()
        };
        Err(if message.is_empty() {
            "the command failed without details".to_owned()
        } else {
            sanitize_text(message, 500)
        })
    }
}

fn push_error(errors: &mut Vec<Value>, component: &str, message: &str) {
    if errors.len() < MAX_ERRORS {
        errors.push(json!({
            "component": component,
            "message": sanitize_text(message, 500)
        }));
    }
}

fn nonempty_sanitized(value: &str, max_chars: usize) -> Option<String> {
    let value = sanitize_text(value, max_chars);
    (!value.is_empty()).then_some(value)
}

fn sanitize_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
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

fn default_dpkg_query_binary() -> PathBuf {
    PathBuf::from("/usr/bin/dpkg-query")
}

fn default_apt_cache_binary() -> PathBuf {
    PathBuf::from("/usr/bin/apt-cache")
}

fn default_apt_get_binary() -> PathBuf {
    PathBuf::from("/usr/bin/apt-get")
}

fn default_apt_mark_binary() -> PathBuf {
    PathBuf::from("/usr/bin/apt-mark")
}

fn default_timeout_binary() -> PathBuf {
    PathBuf::from("/usr/bin/timeout")
}

fn default_apt_lists_root() -> PathBuf {
    PathBuf::from("/var/lib/apt/lists")
}

fn default_apt_archives_root() -> PathBuf {
    PathBuf::from("/var/cache/apt/archives")
}

fn default_reboot_required_path() -> PathBuf {
    PathBuf::from("/var/run/reboot-required")
}

fn default_reboot_packages_path() -> PathBuf {
    PathBuf::from("/var/run/reboot-required.pkgs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    #[derive(Default)]
    struct MockRunner {
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
        outputs: Mutex<VecDeque<CommandOutput>>,
    }

    impl MockRunner {
        fn push(&self, stdout: &str) {
            self.outputs.lock().unwrap().push_back(CommandOutput {
                success: true,
                stdout: stdout.to_owned(),
                stderr: String::new(),
            });
        }
    }

    impl PackageCommandRunner for MockRunner {
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
                .ok_or_else(|| "unexpected command".to_owned())
        }
    }

    #[test]
    fn dpkg_parser_keeps_only_installed_packages_and_converts_kib() {
        let packages = parse_dpkg_query(
            "bash\t5.2\t2048\tbash\tshells\tii \tGNU Bourne Again SHell\nmissing\t1\t10\tmissing\tmisc\tun \tnot installed\n",
        );
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "bash");
        assert_eq!(packages[0].installed_size_bytes, Some(2 * 1024 * 1024));
    }

    #[test]
    fn apt_cache_parser_returns_candidate_metadata_and_bounded_description() {
        let metadata = parse_apt_cache_show(
            "Package: bash\nArchitecture: amd64\nVersion: 5.2.15-2\nInstalled-Size: 7164\nSize: 1490048\nSource: bash (5.2.15-2)\nSection: shells\nDescription: GNU Bourne Again SHell\n\n",
        );
        let bash = metadata.get("bash").unwrap();
        assert_eq!(bash.version, "5.2.15-2");
        assert_eq!(bash.download_size_bytes, Some(1_490_048));
        assert_eq!(bash.source_package.as_deref(), Some("bash"));
    }

    #[test]
    fn apt_simulation_extracts_candidates_without_claiming_apply() {
        let summary = parse_apt_simulation(
            "Inst openssl [3.0.1] (3.0.2 Ubuntu:22.04/jammy-security [amd64])\nInst bash [5.1] (5.2 Ubuntu:22.04/jammy-updates [amd64])\nConf openssl (3.0.2 Ubuntu:22.04/jammy-security [amd64])\n",
        );
        assert!(summary.available);
        assert_eq!(summary.upgrade_candidates, 2);
        assert!(summary.candidates["openssl"].security);
        assert!(!summary.candidates["bash"].security);
    }

    #[test]
    fn reboot_package_marker_is_bounded_and_does_not_follow_symlinks() {
        let temporary = tempfile::tempdir().unwrap();
        let valid = temporary.path().join("valid.pkgs");
        fs::write(&valid, "linux-image-generic\nlibc6\ninvalid package\n").unwrap();
        let packages = read_package_set(&valid);
        assert_eq!(packages.len(), 2);
        assert!(packages.contains("linux-image-generic"));

        let oversized = temporary.path().join("oversized.pkgs");
        fs::write(
            &oversized,
            vec![b'a'; usize::try_from(MAX_REBOOT_PACKAGE_MARKER_BYTES).unwrap() + 1],
        )
        .unwrap();
        assert!(read_package_set(&oversized).is_empty());

        #[cfg(unix)]
        {
            let linked = temporary.path().join("linked.pkgs");
            std::os::unix::fs::symlink(&valid, &linked).unwrap();
            assert!(read_package_set(&linked).is_empty());
        }
    }

    #[test]
    fn inventory_runs_only_read_commands_and_reports_exact_apply_readiness() {
        let temporary = tempfile::tempdir().unwrap();
        let lists = temporary.path().join("lists");
        fs::create_dir(&lists).unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push("bash\t5.1\t1024\tbash\tshells\tii \tShell\n");
        runner.push("");
        runner.push("Inst bash [5.1] (5.2 Ubuntu:stable-updates [amd64])\n");
        runner.push("Package: bash\nArchitecture: amd64\nVersion: 5.2\nInstalled-Size: 1100\nSize: 900000\nSource: bash\nSection: shells\nDescription: Shell\n\n");
        let manager = PackageManager::with_runner(
            PackageConfig {
                apt_lists_root: lists,
                reboot_required_path: temporary.path().join("reboot-required"),
                reboot_packages_path: temporary.path().join("reboot-required.pkgs"),
                ..PackageConfig::default()
            },
            runner.clone(),
        )
        .unwrap();
        let inventory = manager.inventory().unwrap();
        assert_eq!(inventory["inventory"]["upgrade_available_total"], 1);
        assert_eq!(inventory["upgrade_apply"]["available"], true);
        assert_eq!(
            inventory["upgrade_apply"]["package_removals_allowed"],
            false
        );
        assert_eq!(inventory["helix_self_update"]["git_pull_used"], false);
        let calls = runner.calls.lock().unwrap();
        assert!(calls.iter().all(|(_, args)| {
            !args.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "update" | "install" | "remove" | "dist-upgrade" | "full-upgrade"
                )
            })
        }));
        assert!(
            calls
                .iter()
                .any(|(_, args)| args.iter().any(|arg| arg == "--simulate"))
        );
    }

    #[test]
    fn selected_apply_revalidates_exact_versions_and_never_allows_removals() {
        let temporary = tempfile::tempdir().unwrap();
        let archives = temporary.path().join("archives");
        fs::create_dir(&archives).unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push("bash\t5.1\t1024\tbash\tshells\tii \tShell\n");
        runner.push("");
        runner.push("Package: bash\nArchitecture: amd64\nVersion: 5.2\nInstalled-Size: 1100\nSize: 900000\nSource: bash\nSection: shells\nDescription: Shell\n\n");
        runner.push("Inst bash [5.1] (5.2 Ubuntu:stable-updates [amd64])\n0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.\n");
        runner.push("updated\n");
        runner.push("bash\t5.2\t1100\tbash\tshells\tii \tShell\n");
        let manager = PackageManager::with_runner(
            PackageConfig {
                apt_archives_root: archives,
                reboot_required_path: temporary.path().join("reboot-required"),
                reboot_packages_path: temporary.path().join("reboot-required.pkgs"),
                ..PackageConfig::default()
            },
            runner.clone(),
        )
        .unwrap();
        let selected = [PackageUpdateCandidate {
            name: "bash".to_owned(),
            installed_version: "5.1".to_owned(),
            candidate_version: "5.2".to_owned(),
        }];
        let result = manager
            .apply_updates(&selected, "APPLY 1 UPDATE", true)
            .unwrap();
        assert_eq!(result["updated"][0]["to"], "5.2");
        assert_eq!(result["automatic_reboot"], false);
        assert_eq!(result["rollback_claimed"], false);
        let calls = runner.calls.lock().unwrap();
        let apply = calls
            .iter()
            .find(|(_, args)| args.iter().any(|arg| arg == "--assume-yes"))
            .expect("actual APT apply call");
        assert!(apply.1.iter().any(|arg| arg == "--no-remove"));
        assert!(apply.1.iter().any(|arg| arg == "--only-upgrade"));
        assert!(apply.1.iter().any(|arg| arg == "bash=5.2"));
        assert!(
            apply
                .1
                .iter()
                .any(|arg| arg == "Dpkg::Options::=--force-confold")
        );
        assert!(!apply.1.iter().any(|arg| arg == "sh" || arg == "bash -c"));
    }

    #[test]
    fn candidate_drift_rejects_before_any_package_mutation() {
        let temporary = tempfile::tempdir().unwrap();
        let archives = temporary.path().join("archives");
        fs::create_dir(&archives).unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push("bash\t5.1\t1024\tbash\tshells\tii \tShell\n");
        runner.push("");
        runner.push("Package: bash\nArchitecture: amd64\nVersion: 5.3\nInstalled-Size: 1100\nSize: 900000\nSource: bash\nSection: shells\nDescription: Shell\n\n");
        let manager = PackageManager::with_runner(
            PackageConfig {
                apt_archives_root: archives,
                ..PackageConfig::default()
            },
            runner.clone(),
        )
        .unwrap();
        let error = manager
            .apply_updates(
                &[PackageUpdateCandidate {
                    name: "bash".to_owned(),
                    installed_version: "5.1".to_owned(),
                    candidate_version: "5.2".to_owned(),
                }],
                "APPLY 1 UPDATE",
                true,
            )
            .unwrap_err();
        assert!(error.contains("candidate"));
        assert!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|(_, args)| !args.iter().any(|arg| arg == "--assume-yes"))
        );
    }

    #[test]
    fn package_list_refresh_cannot_install_or_reboot() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::default());
        runner.push("Hit:1 stable InRelease\n");
        let manager = PackageManager::with_runner(
            PackageConfig {
                apt_lists_root: temporary.path().to_owned(),
                ..PackageConfig::default()
            },
            runner.clone(),
        )
        .unwrap();
        let result = manager.refresh_lists().unwrap();
        assert_eq!(result["refreshed"], true);
        assert_eq!(result["package_state_mutated"], false);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.last().map(String::as_str), Some("update"));
        assert!(
            calls[0].1.iter().all(|arg| {
                !matches!(arg.as_str(), "install" | "upgrade" | "reboot" | "remove")
            })
        );
    }
}
