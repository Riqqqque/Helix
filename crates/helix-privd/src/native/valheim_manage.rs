use super::*;
use helix_privd::valheim_config::{ValheimRequest, ValheimSettings};

impl NativeManager {
    pub fn valheim_manage<F>(
        &self,
        id: &str,
        request: &ValheimRequest,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        let manifest = self.load_manifest(native_id(id))?;
        if !manifest.is_valheim() {
            return Err("This action requires a Helix Valheim server".into());
        }
        let _operation = self.begin_instance_operation(&manifest.id, "Valheim management")?;
        let root = self.instance_path(&manifest.id)?;
        let path = root.join("valheim.json");
        let original = if path.exists() {
            read_small_regular_file(&path, 64 * 1024, "Valheim settings")?
        } else {
            serde_json::to_string(&ValheimSettings::default()).map_err(|e| e.to_string())?
        };
        match request {
            ValheimRequest::Status => {
                let settings: ValheimSettings = serde_json::from_str(&original)
                    .map_err(|_| "Invalid valheim.json; correct it in Files or restore a backup")?;
                let inventory = root.join(".helix-valheim/mods.json");
                let mods: Value = if inventory.exists() {
                    let read = self.api_files.execute(
                        &manifest.id,
                        &root,
                        manifest.run_uid,
                        helix_privd::ServerFileRequest::Read {
                            path: ".helix-valheim/mods.json".into(),
                        },
                    )?;
                    serde_json::from_str(
                        read["content"]
                            .as_str()
                            .ok_or("Invalid mod inventory content")?,
                    )
                    .map_err(|_| "Invalid mod inventory; restore a backup before changing mods")?
                } else {
                    json!({"packages": []})
                };
                Ok(
                    json!({"settings": settings, "expected_revision": settings_revision(&original),
                    "mods": mods["packages"], "running": self.runtime_running_checked(&manifest)?,
                    "runtime_current": manifest.runtime_image == valheim::RUNTIME_IMAGE}),
                )
            }
            ValheimRequest::SaveSettings {
                expected_revision,
                settings,
            } => {
                settings.validate(&manifest.name, false)?;
                if *expected_revision != settings_revision(&original) {
                    return Err("Valheim settings changed elsewhere; reload before saving".into());
                }
                if self.runtime_running_checked(&manifest)? {
                    return Err("Stop Valheim before changing launch settings; this protects the world and prevents the game overwriting your changes".into());
                }
                let updated = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
                write_managed_file(
                    &path.with_extension(format!("{}.bak", now_unix_ms())),
                    original.as_bytes(),
                    0o600,
                    0,
                    0,
                )?;
                write_managed_file(&path, updated.as_bytes(), 0o660, 0, manifest.run_uid)?;
                Ok(
                    json!({"saved": true, "expected_revision": settings_revision(&updated), "settings": settings}),
                )
            }
            _ => {
                if request.changes_files() && self.runtime_running_checked(&manifest)? {
                    return Err("Stop Valheim first. Helix then backs up before changing mods or server software; start it again when the job finishes".into());
                }
                progress("Preparing the Valheim tools", 5);
                self.ensure_valheim_runtime_image(&mut progress)?;
                let backup = if request.changes_files() {
                    progress("Backing up the server and checking the archive", 10);
                    Some(backup_id_from_path(&self.backup(&manifest)?))
                } else {
                    None
                };
                progress(
                    if request.changes_files() {
                        "Resolving packages and applying the requested change"
                    } else {
                        "Checking Thunderstore releases"
                    },
                    35,
                );
                let encoded = serde_json::to_string(request).map_err(|e| e.to_string())?;
                let args = vec![
                    "run".into(),
                    "--rm".into(),
                    "--name".into(),
                    format!("helix-valheim-tools-{}", manifest.id),
                    "--user".into(),
                    format!("{}:{}", manifest.run_uid, manifest.run_uid),
                    "--cap-drop=ALL".into(),
                    "--security-opt=no-new-privileges:true".into(),
                    "--memory=1536m".into(),
                    "--memory-swap=1536m".into(),
                    "--cpus=2".into(),
                    "--pids-limit=256".into(),
                    "--env=HOME=/data".into(),
                    "--mount".into(),
                    format!("type=bind,src={},dst=/data", root.display()),
                    valheim::RUNTIME_IMAGE.into(),
                    encoded,
                ];
                // No host shell, Docker socket, elevated capabilities, or unrelated mounts.
                let result = self.docker_owned(&args, 25 * 60);
                let output = match result {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = self.docker(
                            [
                                "stop",
                                "--time",
                                "10",
                                &format!("helix-valheim-tools-{}", manifest.id),
                            ],
                            20,
                        );
                        return Err(format!(
                            "{error}. Server remains stopped; safety backup: {}",
                            backup
                                .as_deref()
                                .unwrap_or("not needed for this read-only check")
                        ));
                    }
                };
                let result = output
                    .lines()
                    .find_map(|line| line.strip_prefix("HELIX_RESULT="))
                    .ok_or(
                        "Valheim tools returned no result; inspect the job error before retrying",
                    )?;
                let mut value: Value =
                    serde_json::from_str(result).map_err(|_| "Invalid Valheim tools response")?;
                if let Some(backup) = backup {
                    value["backup_id"] = json!(backup);
                }
                progress(
                    if request.changes_files() {
                        "Complete; start the server when you are ready"
                    } else {
                        "Release check complete"
                    },
                    100,
                );
                Ok(value)
            }
        }
    }
}
