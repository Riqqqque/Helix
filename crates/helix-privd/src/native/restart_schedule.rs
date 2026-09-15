use super::*;
use fs2::FileExt as _;
use helix_privd::ServerRestartSchedule;

const WARNING_MS: u64 = 300_000;
const LATE_MS: u64 = 30_000;

struct ScheduleLock(File);

impl Drop for ScheduleLock {
    fn drop(&mut self) {
        // A concurrently forked child can briefly retain the open file description.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[derive(Clone, Copy)]
struct RestartProfile {
    player_warnings: bool,
    flush_save: bool,
    method: &'static str,
    runtime: Option<&'static str>,
}

fn restart_profile(manifest: &InstanceManifest) -> RestartProfile {
    // Keep this exhaustive: a new game must choose a shutdown policy at compile time.
    match manifest.kind {
        GameKind::Minecraft => match manifest.software {
            MinecraftSoftware::Pumpkin => RestartProfile {
                player_warnings: true,
                flush_save: false,
                method: "Pumpkin graceful shutdown and world save",
                runtime: None,
            },
            MinecraftSoftware::Custom
            | MinecraftSoftware::Vanilla
            | MinecraftSoftware::Paper
            | MinecraftSoftware::Purpur
            | MinecraftSoftware::Folia
            | MinecraftSoftware::Leaves
            | MinecraftSoftware::Fabric
            | MinecraftSoftware::NeoForge
            | MinecraftSoftware::Forge
            | MinecraftSoftware::Quilt
            | MinecraftSoftware::Pufferfish => RestartProfile {
                player_warnings: true,
                flush_save: !manifest.is_pumpkin(),
                method: "Confirmed world save and graceful JVM shutdown",
                runtime: None,
            },
        },
        GameKind::Valheim => RestartProfile {
            player_warnings: false,
            flush_save: false,
            method: "Valheim Ctrl+C shutdown and world save",
            runtime: Some(valheim::RUNTIME_IMAGE),
        },
        GameKind::Terraria => RestartProfile {
            player_warnings: false,
            flush_save: false,
            method: "Terraria console exit with world save",
            runtime: Some(terraria::RUNTIME_IMAGE),
        },
        GameKind::VRising => RestartProfile {
            player_warnings: false,
            flush_save: false,
            method: "V Rising Windows console Ctrl+C shutdown",
            runtime: Some(vrising::RUNTIME_IMAGE),
        },
    }
}

fn save_confirmed(response: &str) -> bool {
    let response = response.to_ascii_lowercase();
    response.contains("saved the game")
        && !["failed", "error", "not saved", "unable"]
            .iter()
            .any(|word| response.contains(word))
}

fn clean_shutdown(state: &Value) -> bool {
    // A JVM that completes its SIGTERM shutdown hooks can report 128 + SIGTERM.
    state["Running"] == false
        && state["OOMKilled"] == false
        && matches!(state["ExitCode"].as_u64(), Some(0 | 143))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Schedule {
    spec: Option<ServerRestartSchedule>,
    revision: String,
    next_at_unix_ms: u64,
    state: String,
    boot: String,
    warned: u8,
    last_result: String,
    last_at_unix_ms: u64,
}

fn validate(spec: &ServerRestartSchedule, now: u64) -> Result<(), String> {
    if ![6, 12, 24, 48, 168].contains(&spec.interval_hours) {
        return Err("Choose a restart interval of 6, 12, 24, 48, or 168 hours".into());
    }
    if spec.first_at_unix_ms < now.saturating_add(WARNING_MS + 60_000)
        || spec.first_at_unix_ms > now.saturating_add(366 * 86_400_000)
    {
        return Err("The first restart must be at least 6 minutes away and within one year".into());
    }
    Ok(())
}

fn finish(record: &mut Schedule, now: u64, result: &str) {
    record.last_result = result.into();
    record.last_at_unix_ms = now;
    record.state = if record.spec.is_some() {
        "scheduled"
    } else {
        "disabled"
    }
    .into();
    record.boot.clear();
    record.warned = 0;
    if let Some(spec) = &record.spec {
        let period = u64::from(spec.interval_hours) * 3_600_000;
        if period > 0 && record.next_at_unix_ms <= now.saturating_add(WARNING_MS) {
            let jumps = (now.saturating_add(WARNING_MS) - record.next_at_unix_ms) / period + 1;
            record.next_at_unix_ms = record
                .next_at_unix_ms
                .saturating_add(jumps.saturating_mul(period));
        }
    }
}

impl NativeManager {
    fn target_profile(&self, id: &str) -> Result<(RestartProfile, String), String> {
        if id.starts_with("amp:") {
            validate_id(id.trim_start_matches("amp:"))?;
            self.amp.as_ref().ok_or("AMP is not configured")?;
            return Ok((
                RestartProfile {
                    player_warnings: false,
                    flush_save: false,
                    method: "AMP application Stop / Start with verified state; AMP controls shutdown behavior",
                    runtime: None,
                },
                String::new(),
            ));
        }
        let manifest = self.load_manifest(native_id(id))?;
        Ok((restart_profile(&manifest), manifest.runtime_image))
    }

    fn scheduled_ids(&self) -> Vec<String> {
        fs::read_dir(self.state_root.join("restart-schedules"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .take(1024)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let stem = name.strip_suffix(".json")?;
                let (prefix, id) = stem
                    .strip_prefix("amp-")
                    .map_or(("", stem), |id| ("amp:", id));
                validate_id(id).ok()?;
                Some(format!("{prefix}{id}"))
            })
            .collect()
    }

    fn target_boot(&self, id: &str) -> Result<String, String> {
        if id.starts_with("amp:") {
            return self
                .amp
                .as_ref()
                .ok_or("AMP is not configured")?
                .scheduled_boot(id)
                .map(|boot| boot.to_string());
        }
        self.schedule_boot(&self.load_manifest(native_id(id))?)
    }

    fn schedule_path(&self, id: &str) -> Result<PathBuf, String> {
        if let Some(id) = id.strip_prefix("amp:") {
            validate_id(id)?;
            return Ok(self
                .state_root
                .join("restart-schedules")
                .join(format!("amp-{id}.json")));
        }
        // Manifest lookup validates the opaque id and rejects removed servers.
        let manifest = self.load_manifest(native_id(id))?;
        Ok(self
            .state_root
            .join("restart-schedules")
            .join(format!("{}.json", manifest.id)))
    }

    fn with_schedule<T>(
        &self,
        id: &str,
        action: impl FnOnce(&mut Schedule) -> Result<T, String>,
    ) -> Result<T, String> {
        let path = self.schedule_path(id)?;
        let parent = path.parent().ok_or("Missing schedule directory")?;
        fs::create_dir_all(parent).map_err(|_| "Schedule storage unavailable")?;
        if fs::symlink_metadata(parent)
            .map_err(|_| "Schedule storage unavailable")?
            .file_type()
            .is_symlink()
        {
            return Err("Unsafe schedule directory".into());
        }
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect schedules")?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(path.with_extension("lock"))
            .map_err(|_| "Schedule lock unavailable")?;
        lock.try_lock_exclusive()
            .map_err(|_| "Schedule is busy; try again")?;
        let _lock = ScheduleLock(lock);
        let mut record = if path
            .try_exists()
            .map_err(|_| "Schedule storage unavailable")?
        {
            serde_json::from_str::<Schedule>(&read_small_regular_file(
                &path,
                16_384,
                "restart schedule",
            )?)
            .map_err(|_| "Invalid saved schedule; automatic restart is blocked")?
        } else {
            Schedule {
                state: "disabled".into(),
                ..Schedule::default()
            }
        };
        if let Some(spec) = &record.spec {
            if ![6, 12, 24, 48, 168].contains(&spec.interval_hours) {
                return Err("Invalid saved interval; automatic restart is blocked".into());
            }
        }
        let before = serde_json::to_string(&record).map_err(|_| "Cannot encode schedule")?;
        let result = action(&mut record)?;
        let after = serde_json::to_string(&record).map_err(|_| "Cannot encode schedule")?;
        if before != after {
            write_private_text(&path, &after)?;
        }
        Ok(result)
    }

    pub fn restart_schedule_status(&self, id: &str) -> Value {
        let profile = self.target_profile(id);
        self.with_schedule(id, |record| {
            let (profile, image) = profile?;
            Ok(json!({
                "enabled": record.spec.is_some(), "state": record.state,
                "interval_hours": record.spec.as_ref().map(|spec| spec.interval_hours),
                "next_at_unix_ms": record.spec.as_ref().map(|_| record.next_at_unix_ms),
                "last_result": record.last_result, "last_at_unix_ms": record.last_at_unix_ms,
                "player_warnings": profile.player_warnings, "shutdown_method": profile.method,
                "runtime_update_required": profile.runtime.is_some_and(|required| required != image),
            }))
        })
        .unwrap_or_else(
            |error| json!({"enabled": false, "state": "unavailable", "last_result": error}),
        )
    }

    pub fn set_restart_schedule(
        &self,
        id: &str,
        spec: Option<ServerRestartSchedule>,
    ) -> Result<Value, String> {
        let (profile, runtime_image) = self.target_profile(id)?;
        let now = now_unix_ms();
        if let Some(spec) = &spec {
            validate(spec, now)?;
            if id.starts_with("amp:") {
                self.amp
                    .as_ref()
                    .ok_or("AMP is not configured")?
                    .scheduled_target(id)?;
            }
            if !profile.player_warnings && !spec.allow_unwarned_restart {
                return Err(
                    "Acknowledge that this server cannot deliver in-game restart warnings".into(),
                );
            }
            if profile
                .runtime
                .is_some_and(|required| required != runtime_image)
            {
                return Err(
                    "This server needs the updated graceful-shutdown runtime before scheduling"
                        .into(),
                );
            }
        }
        let was_warning = self.with_schedule(id, |record| {
            if record.state == "restarting" {
                return Err("A restart is executing; wait for its result".into());
            }
            let warning = record.state == "warning";
            record.revision = Uuid::new_v4().to_string();
            record.next_at_unix_ms = spec.as_ref().map_or(0, |spec| spec.first_at_unix_ms);
            record.spec = spec;
            finish(
                record,
                now,
                if record.spec.is_some() {
                    "Schedule saved"
                } else {
                    "Schedule disabled"
                },
            );
            Ok(warning)
        })?;
        if was_warning {
            let _ = self.server_console(id, "say [Helix] Scheduled restart cancelled.");
        }
        Ok(self.restart_schedule_status(id))
    }

    fn schedule_boot(&self, manifest: &InstanceManifest) -> Result<String, String> {
        let output = self.docker(
            [
                "inspect",
                "--format",
                "{{json .}}",
                manifest.container_name.as_str(),
            ],
            15,
        )?;
        let value: Value =
            serde_json::from_str(&output).map_err(|_| "Cannot verify server runtime")?;
        if value["State"]["Running"] != true
            || value["State"]["Restarting"] == true
            || value["State"]["Paused"] == true
        {
            return Err("Server is stopped, paused, or restarting".into());
        }
        if manifest.uses_ready_marker()
            && !self
                .instance_path(&manifest.id)?
                .join(READY_MARKER)
                .is_file()
        {
            return Err("Server startup has not completed".into());
        }
        let started = value["State"]["StartedAt"]
            .as_str()
            .ok_or("Missing boot identity")?;
        let id = value["Id"].as_str().ok_or("Missing container identity")?;
        Ok(format!("{id}:{started}"))
    }

    pub fn recover_restart_schedules(&self) {
        for id in self.scheduled_ids() {
            let _ = self.with_schedule(&id, |record| {
                if matches!(record.state.as_str(), "warning" | "restarting") {
                    // Do not replay a claimed restart after a broker crash or update.
                    let due = record.next_at_unix_ms;
                    finish(
                        record,
                        now_unix_ms().max(due),
                        "Interrupted by broker restart; not retried",
                    );
                }
                Ok(())
            });
        }
    }

    pub fn fail_restart_schedule(&self, id: &str, message: &str) {
        if let Err(error) = self.with_schedule(id, |record| {
            finish(record, now_unix_ms().max(record.next_at_unix_ms), message);
            Ok(())
        }) {
            eprintln!("Cannot record scheduled restart outcome: {error}");
        }
        if message.starts_with("Skipped:") {
            let _ = self.server_console(
                id,
                "say [Helix] Scheduled restart skipped. The server will stay online.",
            );
        }
    }

    pub fn restart_schedule_tick(&self) -> Vec<(String, String)> {
        let mut ready = Vec::new();
        for id in self.scheduled_ids() {
            let now = now_unix_ms();
            let result = self.with_schedule(&id, |record| {
                if record.spec.is_none()
                    || record.state == "restarting"
                    || now.saturating_add(WARNING_MS) < record.next_at_unix_ms
                {
                    return Ok(None);
                }
                let (profile, _) = self.target_profile(&id)?;
                if !profile.player_warnings
                    && !record
                        .spec
                        .as_ref()
                        .is_some_and(|spec| spec.allow_unwarned_restart)
                {
                    finish(
                        record,
                        now.max(record.next_at_unix_ms),
                        "Skipped: consent for an unwarned restart is missing",
                    );
                    return Ok(None);
                }
                let due = record.next_at_unix_ms;
                if now > due.saturating_add(LATE_MS) {
                    finish(record, now, "Skipped: restart window was missed");
                    return Ok(None);
                }
                let boot = match self.target_boot(&id) {
                    Ok(boot) => boot,
                    Err(_) => {
                        finish(record, now.max(due), "Skipped: server was not running");
                        return Ok(None);
                    }
                };
                if record.state == "scheduled" {
                    if now > due.saturating_sub(WARNING_MS).saturating_add(LATE_MS) {
                        finish(
                            record,
                            now.max(due),
                            "Skipped: full player warning was unavailable",
                        );
                        return Ok(None);
                    }
                    record.boot = boot.clone();
                    record.state = "warning".into();
                }
                let same_boot = if id.starts_with("amp:") {
                    record
                        .boot
                        .parse::<u64>()
                        .ok()
                        .zip(boot.parse::<u64>().ok())
                        .is_some_and(|(old, current)| old.abs_diff(current) <= 5)
                } else {
                    record.boot == boot
                };
                if !same_boot {
                    finish(
                        record,
                        now.max(due),
                        "Skipped: server restarted during countdown",
                    );
                    return Ok(None);
                }
                let remaining = due.saturating_sub(now);
                let (stage, command) = if remaining > 60_000 {
                    (
                        1,
                        "say [Helix] Scheduled restart in 5 minutes. Your progress will be saved.",
                    )
                } else if remaining > 10_000 {
                    (2, "say [Helix] Scheduled restart in 1 minute.")
                } else if remaining > 0 {
                    (3, "say [Helix] Scheduled restart in 10 seconds.")
                } else {
                    (4, "say [Helix] Saving world and restarting now.")
                };
                if stage > record.warned {
                    if stage != record.warned + 1
                        || (profile.player_warnings && self.server_console(&id, command).is_err())
                    {
                        finish(
                            record,
                            now.max(due),
                            "Skipped: player warning could not be delivered",
                        );
                        return Ok(None);
                    }
                    record.warned = stage;
                }
                if stage == 4 {
                    record.state = "restarting".into();
                    return Ok(Some(record.revision.clone()));
                }
                Ok(None)
            });
            match result {
                Ok(Some(revision)) => ready.push((id.clone(), revision)),
                Ok(None) => {}
                Err(error) => eprintln!("Scheduled restart check blocked for {id}: {error}"),
            }
        }
        ready
    }

    pub fn execute_scheduled_restart(&self, id: &str, revision: &str) -> Result<Value, String> {
        let result: Result<Value, String> = (|| {
            if id.starts_with("amp:") {
                let boot = self.with_schedule(id, |record| {
                    if record.revision != revision
                        || record.state != "restarting"
                        || !record
                            .spec
                            .as_ref()
                            .is_some_and(|spec| spec.allow_unwarned_restart)
                    {
                        return Err("Scheduled AMP restart was cancelled or lacks consent".into());
                    }
                    record
                        .boot
                        .parse::<u64>()
                        .map_err(|_| "Invalid AMP runtime identity".into())
                })?;
                return self
                    .amp
                    .as_ref()
                    .ok_or("AMP is not configured")?
                    .scheduled_restart(id, boot);
            }
            let manifest = self.load_manifest(native_id(id))?;
            let _operation = self.begin_instance_operation(&manifest.id, "scheduled restart")?;
            let expected_boot = self.with_schedule(id, |record| {
                if record.revision != revision || record.state != "restarting" {
                    return Err("Scheduled restart was cancelled or replaced".into());
                }
                Ok(record.boot.clone())
            })?;
            if self.schedule_boot(&manifest)? != expected_boot {
                return Err("Server runtime changed; restart cancelled".into());
            }
            let profile = restart_profile(&manifest);
            if profile
                .runtime
                .is_some_and(|required| required != manifest.runtime_image)
            {
                return Err("Graceful-shutdown runtime changed; restart cancelled".into());
            }
            if profile.flush_save {
                let saved = rcon_command_timed(
                    manifest.rcon_port,
                    &manifest.rcon_password,
                    "save-all flush",
                    Duration::from_secs(3),
                    Duration::from_secs(120),
                )?;
                if !save_confirmed(&saved) {
                    return Err("World save was not confirmed; restart cancelled".into());
                }
            }
            let path = self
                .instance_path(&manifest.id)?
                .join(manifest.settings_name());
            let settings = if manifest.is_minecraft() {
                Some(read_small_regular_file(
                    &path,
                    MAX_PROPERTIES_BYTES,
                    "server settings",
                )?)
            } else {
                None
            };
            // Docker waits indefinitely rather than escalating to SIGKILL. Our client timeout
            // stops waiting, not the game; a hung shutdown requires operator attention.
            self.docker(
                [
                    "stop",
                    "--time",
                    "-1",
                    "--signal",
                    "SIGTERM",
                    manifest.container_name.as_str(),
                ],
                180,
            )?;
            let state = self.docker(
                [
                    "inspect",
                    "--format",
                    "{{json .State}}",
                    manifest.container_name.as_str(),
                ],
                15,
            )?;
            let state: Value =
                serde_json::from_str(&state).map_err(|_| "Cannot verify clean shutdown")?;
            if !clean_shutdown(&state) {
                return Err("Shutdown was not clean; automatic startup blocked".into());
            }
            if let Some(settings) = settings {
                preserve_settings_after_stop(&path, &settings, manifest.run_uid)?;
            }
            let expected = self.minecraft_settings_snapshot(&manifest)?;
            self.clear_ready_marker(&manifest)?;
            self.docker(["start", manifest.container_name.as_str()], 90)?;
            self.wait_until_ready(&manifest, self.ready_timeout(&manifest), |_| {})?;
            self.verify_minecraft_settings(&manifest, expected)?;
            Ok(json!({"online": true, "scheduled": true}))
        })();
        let outcome = match &result {
            Ok(_) => "Restart completed; server is online".to_owned(),
            Err(error) => format!(
                "Restart failed: {}. Check the server before retrying.",
                error.chars().take(400).collect::<String>()
            ),
        };
        self.fail_restart_schedule(id, &outcome);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "f2210f81-6b2d-4f4f-badc-c9cf2f80b7ca";

    fn fixture() -> (tempfile::TempDir, NativeManager) {
        let root = tempfile::tempdir_in("/dev/shm").unwrap();
        let manager = NativeManager::new(NativeConfig {
            state_root: root.path().join("state"),
            instance_root: root.path().join("instances"),
            backup_root: root.path().join("backups"),
            docker_binary: PathBuf::from("/bin/false"),
            console_history_max_bytes: default_console_history_max_bytes(),
            console_history_files: default_console_history_files(),
            backup_trash_retention_days: 30,
            custom_artifact_roots: Vec::new(),
        })
        .unwrap();
        let manifest = serde_json::from_value(json!({
            "schema_version": 1, "id": ID, "name": "Test", "instance_name": "test",
            "container_name": format!("helix-game-{ID}"), "software": "paper", "minecraft_version": "1.21.8",
            "build": "1", "java_version": 21,
            "runtime_image": "eclipse-temurin@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifact_url": "https://example.invalid/server.jar", "artifact_sha256": "a".repeat(64),
            "memory_mb": 4096, "max_players": 20, "game_port": 25565, "rcon_port": 30000,
            "rcon_password": "test", "start_on_boot": true, "run_uid": 20000, "created_at_unix_ms": 1
        })).unwrap();
        write_manifest(&manager.manifest_path(ID).unwrap(), &manifest).unwrap();
        (root, manager)
    }

    #[test]
    fn schedule_lock_releases_even_when_a_child_retains_the_descriptor() {
        let file = tempfile::NamedTempFile::new_in("/dev/shm").unwrap();
        let lock = file.reopen().unwrap();
        lock.try_lock_exclusive().unwrap();
        let inherited = lock.try_clone().unwrap();
        let guard = ScheduleLock(lock);
        let competing = file.reopen().unwrap();
        assert!(competing.try_lock_exclusive().is_err());
        drop(guard);
        competing.try_lock_exclusive().unwrap();
        drop(inherited);
    }

    fn enable(manager: &NativeManager) {
        manager
            .set_restart_schedule(
                ID,
                Some(ServerRestartSchedule {
                    first_at_unix_ms: now_unix_ms() + 3_600_000,
                    interval_hours: 24,
                    allow_unwarned_restart: false,
                }),
            )
            .unwrap();
    }

    #[test]
    fn every_native_game_and_minecraft_flavor_has_an_explicit_policy() {
        let (_root, manager) = fixture();
        let mut manifest = manager.load_manifest(ID).unwrap();
        for software in [
            MinecraftSoftware::Pumpkin,
            MinecraftSoftware::Custom,
            MinecraftSoftware::Vanilla,
            MinecraftSoftware::Paper,
            MinecraftSoftware::Purpur,
            MinecraftSoftware::Folia,
            MinecraftSoftware::Leaves,
            MinecraftSoftware::Fabric,
            MinecraftSoftware::NeoForge,
            MinecraftSoftware::Forge,
            MinecraftSoftware::Quilt,
            MinecraftSoftware::Pufferfish,
        ] {
            manifest.software = software;
            let profile = restart_profile(&manifest);
            assert!(profile.player_warnings);
            assert_eq!(profile.flush_save, software != MinecraftSoftware::Pumpkin);
        }
        for (kind, image) in [
            (GameKind::Valheim, valheim::RUNTIME_IMAGE),
            (GameKind::Terraria, terraria::RUNTIME_IMAGE),
            (GameKind::VRising, vrising::RUNTIME_IMAGE),
        ] {
            manifest.kind = kind;
            manifest.runtime_image = image.into();
            write_manifest(&manager.manifest_path(ID).unwrap(), &manifest).unwrap();
            let spec = ServerRestartSchedule {
                first_at_unix_ms: now_unix_ms() + 3_600_000,
                interval_hours: 24,
                allow_unwarned_restart: false,
            };
            assert!(
                manager
                    .set_restart_schedule(ID, Some(spec.clone()))
                    .is_err()
            );
            let result = manager
                .set_restart_schedule(
                    ID,
                    Some(ServerRestartSchedule {
                        allow_unwarned_restart: true,
                        ..spec
                    }),
                )
                .unwrap();
            assert_eq!(result["enabled"], true, "{kind:?}: {result}");
            assert_eq!(result["player_warnings"], false);
            assert_eq!(result["runtime_update_required"], false);
            manager.set_restart_schedule(ID, None).unwrap();
        }
    }

    #[test]
    fn old_game_runtimes_cannot_be_scheduled_with_unsafe_signal_forwarding() {
        let (_root, manager) = fixture();
        let mut manifest = manager.load_manifest(ID).unwrap();
        manifest.kind = GameKind::Valheim;
        manifest.runtime_image = "helix-valheim-runtime:1".into();
        write_manifest(&manager.manifest_path(ID).unwrap(), &manifest).unwrap();
        assert_eq!(
            manager.restart_schedule_status(ID)["runtime_update_required"],
            true
        );
        assert!(
            manager
                .set_restart_schedule(
                    ID,
                    Some(ServerRestartSchedule {
                        first_at_unix_ms: now_unix_ms() + 3_600_000,
                        interval_hours: 24,
                        allow_unwarned_restart: true
                    })
                )
                .is_err()
        );
        manager.set_restart_schedule(ID, None).unwrap();
    }

    #[test]
    fn amp_and_native_schedule_ids_do_not_collide_and_recover_without_live_amp() {
        let (_root, manager) = fixture();
        let amp_id = format!("amp:{ID}");
        assert_ne!(
            manager.schedule_path(ID).unwrap(),
            manager.schedule_path(&amp_id).unwrap()
        );
        manager
            .with_schedule(&amp_id, |record| {
                record.state = "restarting".into();
                Ok(())
            })
            .unwrap();
        manager.recover_restart_schedules();
        manager
            .with_schedule(&amp_id, |record| {
                assert_eq!(record.state, "disabled");
                Ok(())
            })
            .unwrap();
        assert!(manager.schedule_path("amp:../../escape").is_err());
    }

    #[test]
    fn disk_schedule_persists_and_disabling_preserves_outcome() {
        let (_root, manager) = fixture();
        assert_eq!(manager.restart_schedule_status(ID)["enabled"], false);
        enable(&manager);
        let status = manager.restart_schedule_status(ID);
        assert_eq!(status["enabled"], true, "{status}");
        let path = manager.schedule_path(ID).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        manager.set_restart_schedule(ID, None).unwrap();
        assert_eq!(manager.restart_schedule_status(ID)["state"], "disabled");
        assert!(manager.restart_schedule_tick().is_empty());
    }

    #[test]
    fn stopped_unverifiable_and_missed_servers_never_queue() {
        let (_root, manager) = fixture();
        enable(&manager);
        for offset in [0, 600_000] {
            manager
                .with_schedule(ID, |record| {
                    record.next_at_unix_ms = now_unix_ms() - offset;
                    Ok(())
                })
                .unwrap();
            assert!(manager.restart_schedule_tick().is_empty());
            let status = manager.restart_schedule_status(ID);
            assert!(
                status["last_result"]
                    .as_str()
                    .unwrap()
                    .starts_with("Skipped:")
            );
            assert!(status["next_at_unix_ms"].as_u64().unwrap() > now_unix_ms());
        }
    }

    #[test]
    fn crash_recovery_does_not_replay_claimed_runs() {
        let (_root, manager) = fixture();
        enable(&manager);
        for state in ["warning", "restarting"] {
            manager
                .with_schedule(ID, |record| {
                    record.state = state.into();
                    Ok(())
                })
                .unwrap();
            manager.recover_restart_schedules();
            let status = manager.restart_schedule_status(ID);
            assert_eq!(status["state"], "scheduled");
            assert_eq!(
                status["last_result"],
                "Interrupted by broker restart; not retried"
            );
            assert!(manager.restart_schedule_tick().is_empty());
        }
    }

    #[test]
    fn competing_operations_and_stale_claims_cannot_restart() {
        let (_root, manager) = fixture();
        enable(&manager);
        let revision = manager
            .with_schedule(ID, |record| {
                record.state = "restarting".into();
                Ok(record.revision.clone())
            })
            .unwrap();
        let operation = manager.begin_instance_operation(ID, "backup").unwrap();
        assert!(
            manager
                .execute_scheduled_restart(ID, &revision)
                .unwrap_err()
                .contains("progress")
        );
        drop(operation);
        assert!(
            manager
                .execute_scheduled_restart(ID, "stale-claim")
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(manager.restart_schedule_tick().is_empty());
    }

    #[test]
    fn corrupt_records_fail_closed_and_executing_runs_cannot_be_replaced() {
        let (_root, manager) = fixture();
        enable(&manager);
        manager
            .with_schedule(ID, |record| {
                record.state = "restarting".into();
                Ok(())
            })
            .unwrap();
        assert!(manager.set_restart_schedule(ID, None).is_err());
        fs::write(manager.schedule_path(ID).unwrap(), "broken").unwrap();
        assert_eq!(manager.restart_schedule_status(ID)["state"], "unavailable");
        assert!(manager.restart_schedule_tick().is_empty());
    }

    #[test]
    fn save_requires_positive_confirmation_not_just_the_word_saved() {
        assert!(save_confirmed(
            "Saving the game (this may take a moment!)\nSaved the game"
        ));
        for failure in [
            "",
            "not saved",
            "Error: saved the game",
            "Unknown command",
            "Unable to save",
        ] {
            assert!(!save_confirmed(failure));
        }
    }
    #[test]
    fn shutdown_rejects_kills_oom_missing_status_and_running_processes() {
        for code in [0, 143] {
            assert!(clean_shutdown(
                &json!({"Running": false, "OOMKilled": false, "ExitCode": code})
            ));
        }
        for value in [
            json!({}),
            json!({"Running": false, "OOMKilled": false, "ExitCode": 137}),
            json!({"Running": false, "OOMKilled": true, "ExitCode": 0}),
            json!({"Running": true, "OOMKilled": false, "ExitCode": 0}),
        ] {
            assert!(!clean_shutdown(&value));
        }
    }

    #[test]
    fn validates_bounded_intervals_and_full_warning_time() {
        let mut spec = ServerRestartSchedule {
            first_at_unix_ms: 1_000_000,
            interval_hours: 24,
            allow_unwarned_restart: false,
        };
        assert!(validate(&spec, 0).is_ok());
        assert!(validate(&spec, 900_000).is_err());
        spec.interval_hours = 1;
        assert!(validate(&spec, 0).is_err());
        spec.interval_hours = 24;
        spec.first_at_unix_ms = u64::MAX;
        assert!(validate(&spec, 0).is_err());
    }
    #[test]
    fn missed_runs_advance_without_replay_or_cadence_drift() {
        let mut record = Schedule {
            spec: Some(ServerRestartSchedule {
                first_at_unix_ms: 1_000_000,
                interval_hours: 6,
                allow_unwarned_restart: false,
            }),
            next_at_unix_ms: 1_000_000,
            state: "restarting".into(),
            warned: 4,
            ..Schedule::default()
        };
        finish(&mut record, 50_000_000, "missed");
        assert_eq!(record.next_at_unix_ms, 65_800_000);
        assert_eq!(record.state, "scheduled");
        assert_eq!(record.warned, 0);
        assert_eq!(record.last_result, "missed");
    }
    #[test]
    fn disabled_schedule_does_not_get_rescheduled() {
        let mut record = Schedule::default();
        finish(&mut record, 100, "disabled");
        assert_eq!(record.state, "disabled");
        assert_eq!(record.next_at_unix_ms, 0);
    }
}
