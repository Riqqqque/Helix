use super::*;
use helix_privd::NativeRuntimeChangeSpec;

fn validate_version_change(current: &str, target: &str) -> Result<(), String> {
    if current == target {
        return Ok(());
    }
    let parse = |value: &str| -> Option<Vec<u32>> {
        let mut parts = value
            .split('.')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        if !(2..=3).contains(&parts.len()) {
            return None;
        }
        parts.resize(3, 0);
        Some(parts)
    };
    match (parse(current), parse(target)) {
        (Some(old), Some(new)) if new >= old => Ok(()),
        _ => Err("Downgrades and snapshot migrations need a separate server or a matching full backup. Existing worlds cannot safely be downgraded.".to_owned()),
    }
}

impl NativeManager {
    fn runtime_running_checked(&self, manifest: &InstanceManifest) -> Result<bool, String> {
        let Some((managed, id)) = self.exact_container_identity(&manifest.container_name)? else {
            return Ok(false);
        };
        if managed != "true" || id != manifest.id {
            return Err("The container identity does not match this Helix server; no runtime files were changed.".to_owned());
        }
        let state = self.docker(
            [
                "inspect",
                "--format",
                "{{.State.Running}}",
                &manifest.container_name,
            ],
            20,
        )?;
        match state.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(
                "Docker returned an unknown running state; runtime files cannot safely be changed."
                    .to_owned(),
            ),
        }
    }

    fn stop_runtime_for_files(&self, manifest: &InstanceManifest) -> Result<(), String> {
        if self.runtime_running_checked(manifest)? {
            self.docker(["stop", "--time", "45", &manifest.container_name], 75)?;
        }
        if self.runtime_running_checked(manifest)? {
            return Err(
                "The server is still running; its runtime files were left alone.".to_owned(),
            );
        }
        Ok(())
    }

    pub fn change_runtime<F>(
        &self,
        id: &str,
        spec: &NativeRuntimeChangeSpec,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        let id = id.strip_prefix("helix:").unwrap_or(id);
        let _operation = self.begin_instance_operation(id, "runtime change")?;
        let manifest = self.load_manifest(id)?;
        if spec.confirmation_name != manifest.name
            || spec.expected_version != manifest.minecraft_version
            || spec.expected_build != manifest.build
        {
            return Err(
                "Server details changed. Refresh this server and confirm the runtime change again."
                    .to_owned(),
            );
        }
        if manifest.uses_ready_marker() || manifest.software == MinecraftSoftware::Custom {
            return Err("This runtime uses its own update process; custom JARs must be replaced through Files after a backup.".to_owned());
        }
        if spec.version.is_some() && manifest.modpack.is_some() {
            return Err("Update this modpack from its pack update controls. Its Minecraft and loader versions must stay together; exact runtime repair is available here.".to_owned());
        }
        progress("Resolving the selected runtime", 5);
        let artifact = if let Some(version) = &spec.version {
            if version.is_empty() || version.len() > 128 {
                return Err("Choose a published version".to_owned());
            }
            self.resolve_artifact(manifest.software, version)?
        } else {
            Artifact {
                software: manifest.software,
                version: manifest.minecraft_version.clone(),
                build: manifest.build.clone(),
                java_version: manifest.java_version,
                url: manifest.artifact_url.clone(),
                local_source: None,
                expected_hash: Some(ExpectedHash {
                    algorithm: HashAlgorithm::Sha256,
                    value: manifest.artifact_sha256.clone(),
                }),
                install_server: manifest.unix_args.is_some(),
            }
        };
        validate_version_change(&manifest.minecraft_version, &artifact.version)?;
        self.activate_runtime(&manifest, artifact, &mut progress)
    }

    pub(super) fn activate_runtime<F>(
        &self,
        manifest: &InstanceManifest,
        artifact: Artifact,
        progress: &mut F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        validate_version_change(&manifest.minecraft_version, &artifact.version)?;
        let stage = self
            .instance_root
            .join(".staging")
            .join(format!("runtime-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&stage).map_err(|e| format!("Could not stage runtime: {e}"))?;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o750))
            .map_err(|e| e.to_string())?;
        let result = (|| {
            progress("Downloading and verifying runtime files", 15);
            let digest =
                self.download_artifact(&artifact, &stage.join(manifest.artifact_name()))?;
            let mut updated = manifest.clone();
            updated.minecraft_version = artifact.version.clone();
            updated.build = artifact.build.clone();
            updated.java_version = artifact.java_version;
            updated.runtime_image = if manifest.is_pumpkin() {
                self.pumpkin_runtime()?
            } else {
                self.resolve_runtime_image(artifact.java_version)?
            };
            updated.artifact_url = artifact.url.clone();
            updated.artifact_sha256 = digest;
            if artifact.install_server {
                progress("Installing loader libraries in staging", 35);
                updated.unix_args = Some(self.run_loader_installer(
                    &artifact,
                    &stage,
                    manifest.run_uid,
                    &updated.runtime_image,
                )?);
            }
            let data = self.instance_path(&manifest.id)?;
            let running = self.runtime_running_checked(manifest)?;
            progress("Stopping this server and making a full safety backup", 55);
            self.stop_runtime_for_files(manifest)?;
            let backup = match self.archive_data(manifest) {
                Ok(path) => path,
                Err(error) => {
                    let restart = self.restart_if_previously_running(manifest, running);
                    return Err(format!(
                        "Safety backup failed; runtime was not changed: {error}{}",
                        restart
                            .err()
                            .map(|e| format!("; restart failed: {e}"))
                            .unwrap_or_default()
                    ));
                }
            };
            let activation = (|| {
                progress("Activating the verified runtime", 75);
                if artifact.install_server {
                    let libraries = data.join("libraries");
                    if libraries.try_exists().map_err(|e| e.to_string())? {
                        fs::rename(&libraries, stage.join("previous-libraries"))
                            .map_err(|e| format!("Could not preserve old libraries: {e}"))?;
                    }
                    fs::rename(stage.join("libraries"), &libraries)
                        .map_err(|e| format!("Could not activate loader libraries: {e}"))?;
                }
                fs::rename(
                    stage.join(manifest.artifact_name()),
                    data.join(manifest.artifact_name()),
                )
                .map_err(|e| format!("Could not activate runtime: {e}"))?;
                self.protect_instance_artifacts(&data, manifest.run_uid)?;
                write_manifest(&self.manifest_path(&manifest.id)?, &updated)?;
                self.republish_minecraft_container(&updated, &data, running)?;
                if running {
                    progress("Checking server startup; rollback is available", 90);
                    self.wait_until_ready(&updated, self.ready_timeout(&updated), |_| {})?;
                }
                Ok::<(), String>(())
            })();
            if let Err(error) = activation {
                progress("Restoring the complete safety backup", 95);
                let rollback = (|| {
                    self.stop_runtime_for_files(&updated)?;
                    self.restore_modpack_safety_backup(manifest, &data, &backup)?;
                    write_manifest(&self.manifest_path(&manifest.id)?, manifest)?;
                    self.republish_minecraft_container(manifest, &data, running)?;
                    Ok::<(), String>(())
                })();
                return Err(match rollback {
                    Ok(()) => format!(
                        "Runtime activation failed: {error}. The full safety backup was restored, including worlds and settings."
                    ),
                    Err(restore) => format!(
                        "Runtime activation failed: {error}. Recovery needs attention: {restore}. Keep safety backup {} and the failed-data recovery directory.",
                        backup_id_from_path(&backup)
                    ),
                });
            }
            Ok(
                json!({"backup_id": backup_id_from_path(&backup), "version": updated.minecraft_version, "build": updated.build, "server_was_running": running, "runtime_validation_performed": running, "restart_required": !running}),
            )
        })();
        let _ = fs::remove_dir_all(&stage);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_upgrades_and_exact_repairs_are_allowed() {
        for (old, new) in [
            ("1.21.1", "1.21.1"),
            ("1.21.1", "1.21.10"),
            ("1.21", "26.2"),
            ("snapshot", "snapshot"),
        ] {
            assert!(validate_version_change(old, new).is_ok());
        }
    }
    #[test]
    fn downgrades_and_unordered_versions_need_backup_migration() {
        for (old, new) in [
            ("26.2", "1.21.1"),
            ("1.21.10", "1.21.1"),
            ("snapshot", "1.21"),
            ("1.21", "latest"),
        ] {
            assert!(validate_version_change(old, new).is_err());
        }
    }
}
