use super::{
    FORGECDN_DOWNLOAD_HOSTS, InstalledModpack, InstanceManifest, MANIFEST_VERSION,
    MAX_METADATA_BYTES, MAX_MODPACK_LOCK_BYTES, MAX_SERVER_JAR_BYTES, MODPACK_LOCK_FILE,
    ManagedModpackFile, MinecraftCreateSpec, MinecraftModpackCreateSpec, MinecraftSoftware,
    ModpackLock, NativeManager, allocate_rcon_port, allocate_run_uid, file_sha256, instance_name,
    marketplace::{curseforge_icon_proxy_url, modrinth_icon_proxy_url},
    now_unix_ms, require_https_host, run_program, server_properties, sync_directory,
    validate_create_spec, write_manifest, write_new_file,
};
use helix_privd::mrpack::{
    MrpackLimits, extract_overrides, inspect_mrpack, json_u64, prepare_download_path,
    require_exact_https_host, validate_relative_path, verify_download, verify_sha512,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write as _},
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive};

const MODRINTH_API_HOST: &str = "api.modrinth.com";
const MODRINTH_CDN_HOST: &str = "cdn.modrinth.com";
const MAX_SEARCH_QUERY_BYTES: usize = 120;
const MAX_SEARCH_OFFSET: u32 = 10_000;
const MAX_SEARCH_LIMIT: u8 = 50;
const MAX_PROJECT_BODY_CHARS: usize = 128 * 1024;
const MAX_VERSIONS_RETURNED: usize = 200;
const INSTALL_DEADLINE: Duration = Duration::from_secs(45 * 60);
const DISK_HEADROOM_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_CURSEFORGE_SERVER_PACK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_CURSEFORGE_SERVER_PACK_UNPACKED_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const MAX_CURSEFORGE_SERVER_PACK_ENTRIES: usize = 8_192;
const MAX_CURSEFORGE_COMPRESSION_RATIO: u64 = 250;

#[derive(Clone)]
struct ResolvedVersion {
    project_id: String,
    project_slug: String,
    project_title: String,
    version_id: String,
    version_name: String,
    version_number: String,
    game_versions: Vec<String>,
    file_url: String,
    file_name: String,
    file_size: u64,
    file_sha512: String,
}

#[derive(Clone)]
struct ModpackUpdateCandidate {
    version_id: String,
    version_name: String,
    version_number: String,
}

struct PreparedModpackUpdate {
    manifest: InstanceManifest,
    lock: ModpackLock,
}

struct ModpackActivation {
    rollback_root: PathBuf,
    moved_old_files: Vec<String>,
    activated_new_files: Vec<String>,
    old_lock_moved: bool,
}

impl NativeManager {
    pub fn minecraft_modpack_search(
        &self,
        query: &str,
        offset: u32,
        limit: u8,
        provider: helix_privd::ModpackProvider,
    ) -> Result<Value, String> {
        match provider {
            helix_privd::ModpackProvider::Curseforge => {
                return self.curseforge_modpack_search(query, offset, limit);
            }
            helix_privd::ModpackProvider::Modrinth => {}
        }
        validate_search(query, offset, limit)?;
        let facets = serde_json::to_string(&json!([["project_type:modpack"]]))
            .map_err(|_| "could not encode the Modrinth search filter".to_owned())?;
        let url = format!(
            "https://api.modrinth.com/v2/search?query={}&facets={}&index=relevance&offset={offset}&limit={limit}",
            percent_encode(query.trim()),
            percent_encode(&facets)
        );
        let response = self.fetch_modrinth_json(&url)?;
        let hits = response
            .get("hits")
            .and_then(Value::as_array)
            .ok_or_else(|| "Modrinth returned invalid modpack search results".to_owned())?;
        let results = hits
            .iter()
            .take(usize::from(limit))
            .map(sanitize_search_hit)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "schema_version": 1,
            "query": clean_text(query.trim(), MAX_SEARCH_QUERY_BYTES),
            "offset": offset,
            "limit": limit,
            "total_hits": response.get("total_hits").and_then(Value::as_u64).unwrap_or_else(|| u64::try_from(results.len()).unwrap_or(u64::MAX)),
            "results": results,
            "installation_scope": {
                "loaders": ["fabric", "forge", "neoforge", "quilt"],
                "stable_releases_only": true,
                "server_capable_only": true,
            },
            "source": "Modrinth",
            "provider": "modrinth",
            "collected_at_unix_ms": now_unix_ms(),
        }))
    }

    fn curseforge_modpack_search(
        &self,
        query: &str,
        offset: u32,
        limit: u8,
    ) -> Result<Value, String> {
        validate_search(query, offset, limit)?;
        let path = format!(
            "mods/search?gameId=432&classId=4471&index={offset}&pageSize={limit}&sortField=2&sortOrder=desc&searchFilter={}",
            percent_encode(query.trim())
        );
        let response = self.curseforge_v1(&path)?;
        let hits = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| "CurseForge returned an unexpected catalog shape".to_owned())?;
        let results = hits
            .iter()
            .take(usize::from(limit))
            .map(sanitize_curseforge_search_hit)
            .collect::<Result<Vec<_>, _>>()?;
        let total = response
            .pointer("/pagination/totalCount")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| u64::try_from(results.len()).unwrap_or(u64::MAX));
        Ok(json!({
            "schema_version": 1,
            "query": clean_text(query.trim(), MAX_SEARCH_QUERY_BYTES),
            "offset": offset,
            "limit": limit,
            "total_hits": total,
            "results": results,
            "installation_scope": {
                "loaders": ["forge", "neoforge", "fabric", "quilt"],
                "server_pack_preferred": true,
                "official_api_key": true,
            },
            "source": "CurseForge",
            "provider": "curseforge",
            "collected_at_unix_ms": now_unix_ms(),
        }))
    }

    fn curseforge_modpack_project(&self, project_id: &str) -> Result<Value, String> {
        let project = self.curseforge_v1(&format!("mods/{project_id}"))?;
        let project = project.get("data").cloned().unwrap_or(project);
        let slug = required_curseforge_text(&project, "slug", 128)
            .or_else(|_| required_curseforge_text(&project, "name", 128))?;
        let files_response = self.curseforge_v1(&format!("mods/{project_id}/files?pageSize=50"))?;
        let total_files = files_response
            .pointer("/pagination/totalCount")
            .and_then(Value::as_u64);
        let files = files_response
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut versions = Vec::new();
        for file in files.iter().take(MAX_VERSIONS_RETURNED) {
            versions.push(sanitize_curseforge_modpack_version(file)?);
        }
        let compatible_count = versions
            .iter()
            .filter(|version| version["installable"].as_bool() == Some(true))
            .count();
        Ok(json!({
            "schema_version": 1,
            "project": {
                "id": project_id,
                "slug": slug,
                "title": optional_text(&project, "name", 256).unwrap_or_else(|| slug.clone()),
                "description": optional_text(&project, "summary", 2_048),
                "body": optional_text_chars(&project, "summary", MAX_PROJECT_BODY_CHARS),
                "author": project.pointer("/authors/0/name").and_then(Value::as_str).map(|value| clean_text(value, 128)),
                "downloads": project.get("downloadCount").and_then(Value::as_u64).unwrap_or(0),
                "followers": 0,
                "server_side": "optional",
                "client_side": "unknown",
                "loaders": curseforge_loaders(&project),
                "web_url": format!("https://www.curseforge.com/minecraft/modpacks/{}", percent_encode(&slug)),
                "icon_url": curseforge_icon_proxy_url(project.pointer("/logo/url").and_then(Value::as_str)),
            },
            "versions": versions,
            "compatible_version_count": compatible_count,
            "version_results_truncated": total_files.is_some_and(|total| total > u64::try_from(files.len()).unwrap_or(u64::MAX)),
            "source": "CurseForge",
            "provider": "curseforge",
            "collected_at_unix_ms": now_unix_ms(),
        }))
    }

    pub fn minecraft_modpack_project(
        &self,
        project_id: &str,
        provider: helix_privd::ModpackProvider,
    ) -> Result<Value, String> {
        if matches!(provider, helix_privd::ModpackProvider::Curseforge) {
            if project_id.is_empty() || !project_id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("CurseForge project ids must be numeric".to_owned());
            }
            return self.curseforge_modpack_project(project_id);
        }
        validate_modrinth_id(project_id, "project")?;
        let project =
            self.fetch_modrinth_json(&format!("https://api.modrinth.com/v2/project/{project_id}"))?;
        let resolved_project_id = required_text(&project, "id", 64)?;
        if resolved_project_id != project_id {
            return Err("Modrinth returned a different modpack project".to_owned());
        }
        if required_text(&project, "project_type", 32)? != "modpack" {
            return Err("the selected Modrinth project is not a modpack".to_owned());
        }
        let slug = required_text(&project, "slug", 128)?;
        validate_slug(&slug)?;
        let server_side = required_text(&project, "server_side", 32)?;
        let versions = self.fetch_modrinth_json(&format!(
            "https://api.modrinth.com/v2/project/{project_id}/version?include_changelog=false"
        ))?;
        let versions = versions
            .as_array()
            .ok_or_else(|| "Modrinth returned invalid modpack versions".to_owned())?;
        let mut sanitized = Vec::new();
        for version in versions.iter().take(MAX_VERSIONS_RETURNED) {
            sanitized.push(sanitize_modpack_version(version, project_id, &server_side)?);
        }
        let compatible_count = sanitized
            .iter()
            .filter(|version| version["installable"].as_bool() == Some(true))
            .count();
        let loaders = bounded_string_array(project.get("loaders"), 32, 32);
        Ok(json!({
            "schema_version": 1,
            "project": {
                "id": resolved_project_id,
                "slug": slug,
                "title": required_text(&project, "title", 256)?,
                "description": optional_text(&project, "description", 2_048),
                "body": optional_text_chars(&project, "body", MAX_PROJECT_BODY_CHARS),
                "author": optional_text(&project, "author", 128),
                "downloads": project.get("downloads").and_then(Value::as_u64).unwrap_or(0),
                "followers": project.get("followers").and_then(Value::as_u64).unwrap_or(0),
                "server_side": server_side,
                "client_side": optional_text(&project, "client_side", 32),
                "loaders": loaders,
                "web_url": format!(
                    "https://modrinth.com/modpack/{}",
                    percent_encode(&slug)
                ),
                "icon_url": modrinth_icon_proxy_url(project.get("icon_url").and_then(Value::as_str)),
            },
            "versions": sanitized,
            "compatible_version_count": compatible_count,
            "version_results_truncated": versions.len() > MAX_VERSIONS_RETURNED,
            "installation_scope": {
                "loader": "fabric",
                "stable_releases_only": true,
                "modrinth_declared_sha512_required": true,
                "server_optional_files": "excluded",
                "client_only_files": "excluded",
                "exact_exclusion_counts": "reported_after_archive_validation",
                "full_pack_parity": false,
            },
            "body_format": "plain_text",
            "source": "Modrinth",
            "provider": "modrinth",
            "collected_at_unix_ms": now_unix_ms(),
        }))
    }

    pub fn create_minecraft_modpack<F>(
        &self,
        request: &MinecraftModpackCreateSpec,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        if matches!(request.provider, helix_privd::ModpackProvider::Curseforge) {
            return self.create_curseforge_modpack(request, progress);
        }
        validate_modrinth_id(&request.project_id, "project")?;
        validate_modrinth_id(&request.version_id, "version")?;
        let base_spec = MinecraftCreateSpec {
            pumpkin_bedrock_port: None,
            name: request.name.clone(),
            software: MinecraftSoftware::Fabric,
            version: "latest".to_owned(),
            memory_mb: request.memory_mb,
            cpu_millis: request.cpu_millis,
            max_players: request.max_players,
            game_port: request.game_port,
            network_exposure: request.network_exposure,
            start_on_boot: request.start_on_boot,
            eula_accepted: request.eula_accepted,
            custom_jar: None,
        };
        validate_create_spec(&base_spec)?;
        let _operation = self.begin_creation_operation()?;
        let deadline = Instant::now() + INSTALL_DEADLINE;

        progress("Checking ports, names, and storage", 4);
        let manifests = self.load_manifests()?;
        if manifests
            .iter()
            .any(|manifest| manifest.name.eq_ignore_ascii_case(request.name.trim()))
        {
            return Err("a Helix server with that name already exists".to_owned());
        }
        let (game_port, allocated_automatically) = self.resolve_game_port(
            helix_privd::GameKind::Minecraft,
            request.game_port,
            &manifests,
        )?;
        let mut base_spec = base_spec;
        base_spec.game_port = Some(game_port);
        let rcon_port = allocate_rcon_port(&manifests, &self.amp_occupied_ports())?;
        let id = Uuid::new_v4().to_string();
        let instance_name = instance_name(request.name.trim(), &id);
        let container_name = modpack_container_name(&id);
        let run_uid = allocate_run_uid(&id, &manifests)?;
        let data_path = self.instance_path(&id)?;
        let manifest_path = self.manifest_path(&id)?;
        let staging_path = self
            .instance_root
            .join(format!(".helix-modpack-staging-{id}"));
        let archive_path = staging_path.join(".helix-source.mrpack");
        let mut activated = false;

        let result = (|| -> Result<Value, String> {
            ensure_before(deadline)?;
            progress("Resolving the exact Modrinth project and release", 8);
            let resolved =
                self.resolve_modpack_version(&request.project_id, &request.version_id)?;

            fs::create_dir(&staging_path).map_err(|_| {
                "could not create the isolated modpack staging directory".to_owned()
            })?;
            fs::set_permissions(&staging_path, fs::Permissions::from_mode(0o700))
                .map_err(|_| "could not protect the modpack staging directory".to_owned())?;
            ensure_disk_space(
                &self.instance_root,
                resolved
                    .file_size
                    .saturating_add(MAX_SERVER_JAR_BYTES)
                    .saturating_add(DISK_HEADROOM_BYTES),
            )?;

            progress("Downloading the Modrinth-hosted .mrpack", 14);
            require_exact_https_host(&resolved.file_url, MODRINTH_CDN_HOST)?;
            let limits = MrpackLimits::default();
            self.curl_no_redirect(
                &resolved.file_url,
                &archive_path,
                limits.maximum_archive_bytes,
                remaining_download_seconds(deadline)?,
            )?;
            let archive_metadata = fs::symlink_metadata(&archive_path)
                .map_err(|_| "the downloaded modpack archive is unavailable".to_owned())?;
            if !archive_metadata.file_type().is_file()
                || archive_metadata.len() == 0
                || archive_metadata.len() > limits.maximum_archive_bytes
            {
                return Err(
                    "the downloaded modpack archive is outside Helix size limits".to_owned(),
                );
            }
            verify_sha512(&archive_path, &resolved.file_sha512, "the modpack archive")?;

            progress(
                "Validating paths, hashes, loader pins, and safety bounds",
                22,
            );
            let plan = inspect_mrpack(&archive_path, &limits, deadline)?;
            if !resolved
                .game_versions
                .iter()
                .any(|version| version == &plan.minecraft_version)
            {
                return Err(format!(
                    "the .mrpack pins Minecraft {}, but the selected Modrinth version does not declare it",
                    plan.minecraft_version
                ));
            }
            let required_bytes = plan
                .required_staging_bytes()
                .saturating_add(MAX_SERVER_JAR_BYTES)
                .saturating_add(DISK_HEADROOM_BYTES);
            ensure_disk_space(&self.instance_root, required_bytes)?;
            let artifact = match plan.loader {
                "fabric" => self
                    .resolve_pinned_fabric(&plan.minecraft_version, &plan.fabric_loader_version)?,
                "quilt" => {
                    self.resolve_pinned_quilt(&plan.minecraft_version, &plan.fabric_loader_version)?
                }
                "forge" => {
                    self.resolve_pinned_forge(&plan.minecraft_version, &plan.fabric_loader_version)?
                }
                "neoforge" => self.resolve_pinned_neoforge(
                    &plan.minecraft_version,
                    &plan.fabric_loader_version,
                )?,
                other => {
                    return Err(format!(
                        "the modpack loader {other} is not installable in this Helix release"
                    ));
                }
            };
            if artifact.java_version < 17 || artifact.java_version > 25 {
                return Err(format!(
                    "Minecraft {} requires Java {}, which this Helix release does not manage yet",
                    artifact.version, artifact.java_version
                ));
            }

            progress("Applying common and server-only overrides", 30);
            extract_overrides(&archive_path, &staging_path, &plan, &limits, deadline)?;

            let file_count = plan.files.len();
            for (index, file) in plan.files.iter().enumerate() {
                ensure_before(deadline)?;
                let output = prepare_download_path(&staging_path, &file.path, &limits)?;
                let scaled_progress = index
                    .saturating_mul(23)
                    .checked_div(file_count)
                    .unwrap_or(23);
                let file_progress = 32 + u8::try_from(scaled_progress).unwrap_or(23);
                progress(
                    "Downloading and verifying declared server files",
                    file_progress,
                );
                self.curl_no_redirect(
                    &file.url,
                    &output,
                    limits.maximum_file_bytes,
                    remaining_download_seconds(deadline)?,
                )?;
                verify_download(&output, file)?;
            }

            progress("Pinning the exact Minecraft server runtime", 58);
            let jar_path = staging_path.join("server.jar");
            let artifact_sha256 = self.download_artifact(&artifact, &jar_path)?;
            write_new_file(&staging_path.join("eula.txt"), b"eula=true\n", 0o640)?;
            let rcon_password = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            let server_spec = MinecraftCreateSpec {
                pumpkin_bedrock_port: None,
                version: plan.minecraft_version.clone(),
                ..base_spec.clone()
            };
            write_new_file(
                &staging_path.join("server.properties"),
                server_properties(&server_spec, game_port, rcon_port, &rcon_password).as_bytes(),
                0o640,
            )?;
            fs::remove_file(&archive_path).map_err(|_| {
                "could not remove the validated source archive from staging".to_owned()
            })?;

            progress("Committing the complete server atomically", 68);
            fs::rename(&staging_path, &data_path)
                .map_err(|_| "could not atomically activate the staged modpack".to_owned())?;
            activated = true;
            let runtime_image = self.resolve_runtime_image(artifact.java_version)?;
            let unix_args = if artifact.install_server {
                progress("Running the official loader installer", 72);
                Some(self.run_loader_installer(&artifact, &data_path, run_uid, &runtime_image)?)
            } else {
                None
            };
            let installed_modpack = InstalledModpack {
                schema_version: 1,
                provider: helix_privd::ModpackProvider::Modrinth,
                project_id: resolved.project_id.clone(),
                project_title: resolved.project_title.clone(),
                version_id: resolved.version_id.clone(),
                version_name: resolved.version_name.clone(),
                version_number: resolved.version_number.clone(),
                minecraft_version: plan.minecraft_version.clone(),
                loader: plan.loader.to_owned(),
                loader_version: plan.fabric_loader_version.clone(),
            };
            let lock = build_modpack_lock(&data_path, &installed_modpack)?;
            write_modpack_lock(&data_path, &lock)?;
            let manifest = InstanceManifest {
                schema_version: MANIFEST_VERSION,
                id: id.clone(),
                name: request.name.trim().to_owned(),
                instance_name: instance_name.clone(),
                container_name: container_name.clone(),
                software: artifact.software,
                minecraft_version: artifact.version,
                build: artifact.build,
                java_version: artifact.java_version,
                runtime_image,
                artifact_url: artifact.url,
                artifact_sha256,
                memory_mb: request.memory_mb,
                cpu_millis: request.cpu_millis,
                max_players: request.max_players,
                game_port,
                rcon_port,
                rcon_password,
                start_on_boot: request.start_on_boot,
                run_uid,
                created_at_unix_ms: now_unix_ms(),
                kind: helix_privd::GameKind::Minecraft,
                query_port: 0,
                unix_args,
                backup_keep_count: 0,
                backup_keep_days: 0,
                modpack: Some(installed_modpack),
            };
            write_manifest(&manifest_path, &manifest)?;
            self.chown_instance(&data_path, run_uid)?;
            self.protect_instance_artifacts(&data_path, run_uid)?;

            progress("Creating the isolated Helix workload", 75);
            self.create_validation_container(&manifest, &data_path)?;
            progress("Starting the modpack server", 82);
            self.docker(["start", manifest.container_name.as_str()], 90)?;
            self.wait_for_minecraft(
                &manifest,
                remaining_duration(deadline, Duration::from_secs(10 * 60))?,
                |elapsed| {
                    let percent = 82_u64.saturating_add((elapsed / 40).min(16));
                    progress(
                        "Generating the world and waiting for the modpack",
                        u8::try_from(percent).unwrap_or(98),
                    );
                },
            )?;
            self.finalize_container_restart_policy(&manifest)?;
            self.ensure_console_archiver(&manifest)?;
            progress("Online", 100);
            Ok(json!({
                "schema_version": 1,
                "instance_id": format!("helix:{id}"),
                "instance_name": instance_name,
                "game_port": game_port,
                "port_allocated_automatically": allocated_automatically,
                "manager": "helix",
                "execution_backend": "docker",
                "modpack": {
                    "project_id": resolved.project_id,
                    "project_slug": resolved.project_slug,
                    "project_title": resolved.project_title,
                    "version_id": resolved.version_id,
                    "version_name": resolved.version_name,
                    "version_number": resolved.version_number,
                    "source_filename": resolved.file_name,
                    "minecraft_version": plan.minecraft_version,
                    "loader": plan.loader,
                    "loader_version": plan.fabric_loader_version,
                    "fabric_loader_version": plan.fabric_loader_version,
                    "provider": "modrinth",
                    "installed_server_files": plan.files.len(),
                    "excluded_server_optional_files": plan.skipped_optional_files,
                    "excluded_client_only_files": plan.skipped_client_only_files,
                    "server_safe_subset": true,
                    "full_pack_parity": false,
                },
                "modrinth_declared_sha512_verified": true,
                "declared_file_hashes_verified": ["sha1", "sha512"],
            }))
        })();

        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                progress("Rolling back the incomplete modpack install", 99);
                let cleanup = if activated {
                    self.rollback_modpack_creation(&id, &container_name, &data_path, &manifest_path)
                } else {
                    remove_staging_directory(&self.instance_root, &staging_path)
                };
                Err(match cleanup {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; cleanup also failed: {cleanup}"),
                })
            }
        }
    }

    fn create_curseforge_modpack<F>(
        &self,
        request: &MinecraftModpackCreateSpec,
        mut progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str, u8),
    {
        let project_id = request.project_id.trim();
        let file_id = request.version_id.trim();
        if !project_id.bytes().all(|byte| byte.is_ascii_digit())
            || !file_id.bytes().all(|byte| byte.is_ascii_digit())
            || project_id.is_empty()
            || file_id.is_empty()
        {
            return Err("CurseForge installs need the numeric project and file ids".to_owned());
        }
        let base_spec = MinecraftCreateSpec {
            pumpkin_bedrock_port: None,
            name: request.name.clone(),
            software: MinecraftSoftware::Fabric,
            version: "latest".to_owned(),
            memory_mb: request.memory_mb,
            cpu_millis: request.cpu_millis,
            max_players: request.max_players,
            game_port: request.game_port,
            network_exposure: request.network_exposure,
            start_on_boot: request.start_on_boot,
            eula_accepted: request.eula_accepted,
            custom_jar: None,
        };
        validate_create_spec(&base_spec)?;
        let _operation = self.begin_creation_operation()?;
        let deadline = Instant::now() + INSTALL_DEADLINE;
        progress("Checking ports, names, and storage", 4);
        let manifests = self.load_manifests()?;
        if manifests
            .iter()
            .any(|manifest| manifest.name.eq_ignore_ascii_case(request.name.trim()))
        {
            return Err("a Helix server with that name already exists".to_owned());
        }
        let (game_port, allocated_automatically) = self.resolve_game_port(
            helix_privd::GameKind::Minecraft,
            request.game_port,
            &manifests,
        )?;
        let mut base_spec = base_spec;
        base_spec.game_port = Some(game_port);
        let rcon_port = allocate_rcon_port(&manifests, &self.amp_occupied_ports())?;
        let id = Uuid::new_v4().to_string();
        let instance_name = instance_name(request.name.trim(), &id);
        let container_name = modpack_container_name(&id);
        let run_uid = allocate_run_uid(&id, &manifests)?;
        let data_path = self.instance_path(&id)?;
        let manifest_path = self.manifest_path(&id)?;
        let staging_path = self
            .instance_root
            .join(format!(".helix-modpack-staging-{id}"));
        let archive_path = staging_path.join(".helix-source.zip");
        let mut activated = false;

        let result = (|| -> Result<Value, String> {
            ensure_before(deadline)?;
            progress("Resolving the CurseForge file", 8);
            let file = self.curseforge_v1(&format!("mods/{project_id}/files/{file_id}"))?;
            let file = file.get("data").cloned().unwrap_or(file);
            let resolved_project_id = file
                .get("modId")
                .and_then(Value::as_u64)
                .map(|value| value.to_string())
                .ok_or_else(|| "CurseForge returned a pack file without a project id".to_owned())?;
            if resolved_project_id != project_id
                || file.get("id").and_then(json_u64) != file_id.parse::<u64>().ok()
            {
                return Err("CurseForge returned a pack file from a different project".to_owned());
            }
            if file.get("isAvailable").and_then(Value::as_bool) == Some(false) {
                return Err("the selected CurseForge pack file is no longer available".to_owned());
            }
            let file_name = required_curseforge_text(&file, "fileName", 256)?;
            if !file_name.to_ascii_lowercase().ends_with(".zip")
                || file_name.contains(['/', '\\', ':'])
                || Path::new(&file_name)
                    .file_name()
                    .and_then(|value| value.to_str())
                    != Some(file_name.as_str())
            {
                return Err("the selected CurseForge pack filename is unsafe".to_owned());
            }
            let file_size = file.get("fileLength").and_then(json_u64).unwrap_or(0);
            if file_size == 0 || file_size > MAX_SERVER_JAR_BYTES {
                return Err("the CurseForge file size is outside Helix safety limits".to_owned());
            }
            progress("Resolving the publisher's CurseForge server pack", 11);
            let server_pack = self.resolve_curseforge_server_pack(project_id, &file)?;
            fs::create_dir(&staging_path).map_err(|_| {
                "could not create the isolated modpack staging directory".to_owned()
            })?;
            fs::set_permissions(&staging_path, fs::Permissions::from_mode(0o700))
                .map_err(|_| "could not protect the modpack staging directory".to_owned())?;
            let compressed_install_bytes = server_pack
                .2
                .saturating_add(file_size)
                .saturating_add(MAX_SERVER_JAR_BYTES)
                .saturating_add(DISK_HEADROOM_BYTES);
            ensure_disk_space(&self.instance_root, compressed_install_bytes)?;
            let url = self.resolve_curseforge_download_url(project_id, &file)?;
            progress("Downloading the CurseForge pack zip", 16);
            require_forgecdn_host(&url)?;
            self.curl_no_redirect(
                &url,
                &archive_path,
                MAX_SERVER_JAR_BYTES,
                remaining_download_seconds(deadline)?,
            )?;
            self.verify_curseforge_download(&archive_path, &file, file_size, "pack archive")?;
            progress("Reading the CurseForge manifest", 24);
            let pack = read_curseforge_manifest(&archive_path)?;
            let (
                installed_server_files,
                excluded_non_jar_files,
                excluded_launch_files,
                server_pack_used,
                server_pack_filename,
            ) = {
                let (server_pack_meta, server_pack_name, server_pack_size) = server_pack;
                let server_pack_path = staging_path.join(".helix-server-pack.zip");
                let server_pack_url =
                    self.resolve_curseforge_download_url(project_id, &server_pack_meta)?;
                require_forgecdn_host(&server_pack_url)?;
                progress("Downloading the publisher's CurseForge server pack", 28);
                self.curl_no_redirect(
                    &server_pack_url,
                    &server_pack_path,
                    MAX_CURSEFORGE_SERVER_PACK_BYTES,
                    remaining_download_seconds(deadline)?,
                )?;
                self.verify_curseforge_download(
                    &server_pack_path,
                    &server_pack_meta,
                    server_pack_size,
                    "server pack archive",
                )?;
                progress("Validating and extracting the CurseForge server pack", 43);
                let extraction =
                    inspect_curseforge_server_pack(&server_pack_path, &staging_path, deadline)?;
                ensure_disk_space(
                    &self.instance_root,
                    extraction
                        .unpacked_bytes
                        .saturating_add(MAX_SERVER_JAR_BYTES)
                        .saturating_add(DISK_HEADROOM_BYTES),
                )?;
                extract_curseforge_server_pack(
                    &server_pack_path,
                    &staging_path,
                    &extraction,
                    deadline,
                )?;
                fs::remove_file(&server_pack_path).map_err(|_| {
                    "could not remove the verified CurseForge server-pack archive".to_owned()
                })?;
                (
                    extraction.mod_jar_count,
                    0,
                    extraction.skipped_helix_owned_files,
                    true,
                    Some(server_pack_name),
                )
            };
            let artifact = match pack.loader.as_str() {
                "fabric" => {
                    self.resolve_pinned_fabric(&pack.minecraft_version, &pack.loader_version)?
                }
                "quilt" => {
                    self.resolve_pinned_quilt(&pack.minecraft_version, &pack.loader_version)?
                }
                "forge" => {
                    self.resolve_pinned_forge(&pack.minecraft_version, &pack.loader_version)?
                }
                "neoforge" => {
                    self.resolve_pinned_neoforge(&pack.minecraft_version, &pack.loader_version)?
                }
                other => {
                    return Err(format!(
                        "this CurseForge pack uses {other}, which Helix cannot install yet"
                    ));
                }
            };
            progress("Pinning the Minecraft server runtime", 58);
            let jar_path = staging_path.join("server.jar");
            let artifact_sha256 = self.download_artifact(&artifact, &jar_path)?;
            write_new_file(&staging_path.join("eula.txt"), b"eula=true\n", 0o640)?;
            let rcon_password = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            let server_spec = MinecraftCreateSpec {
                pumpkin_bedrock_port: None,
                version: pack.minecraft_version.clone(),
                software: artifact.software,
                ..base_spec.clone()
            };
            write_new_file(
                &staging_path.join("server.properties"),
                server_properties(&server_spec, game_port, rcon_port, &rcon_password).as_bytes(),
                0o640,
            )?;
            let _ = fs::remove_file(&archive_path);
            progress("Committing the complete server atomically", 68);
            fs::rename(&staging_path, &data_path)
                .map_err(|_| "could not atomically activate the staged modpack".to_owned())?;
            activated = true;
            let runtime_image = self.resolve_runtime_image(artifact.java_version)?;
            let unix_args = if artifact.install_server {
                progress("Running the official loader installer", 72);
                Some(self.run_loader_installer(&artifact, &data_path, run_uid, &runtime_image)?)
            } else {
                None
            };
            let installed_modpack = InstalledModpack {
                schema_version: 1,
                provider: helix_privd::ModpackProvider::Curseforge,
                project_id: project_id.to_owned(),
                project_title: pack.name.clone(),
                version_id: file_id.to_owned(),
                version_name: file
                    .get("displayName")
                    .and_then(Value::as_str)
                    .map(|value| clean_text(value, 256))
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| pack.version.clone()),
                version_number: pack.version.clone(),
                minecraft_version: pack.minecraft_version.clone(),
                loader: pack.loader.clone(),
                loader_version: pack.loader_version.clone(),
            };
            let lock = build_modpack_lock(&data_path, &installed_modpack)?;
            write_modpack_lock(&data_path, &lock)?;
            let manifest = InstanceManifest {
                schema_version: MANIFEST_VERSION,
                id: id.clone(),
                name: request.name.trim().to_owned(),
                instance_name: instance_name.clone(),
                container_name: container_name.clone(),
                software: artifact.software,
                minecraft_version: artifact.version,
                build: artifact.build,
                java_version: artifact.java_version,
                runtime_image,
                artifact_url: artifact.url,
                artifact_sha256,
                memory_mb: request.memory_mb,
                cpu_millis: request.cpu_millis,
                max_players: request.max_players,
                game_port,
                rcon_port,
                rcon_password,
                start_on_boot: request.start_on_boot,
                run_uid,
                created_at_unix_ms: now_unix_ms(),
                kind: helix_privd::GameKind::Minecraft,
                query_port: 0,
                unix_args,
                backup_keep_count: 0,
                backup_keep_days: 0,
                modpack: Some(installed_modpack),
            };
            write_manifest(&manifest_path, &manifest)?;
            self.chown_instance(&data_path, run_uid)?;
            self.protect_instance_artifacts(&data_path, run_uid)?;
            progress("Creating the isolated Helix workload", 75);
            self.create_validation_container(&manifest, &data_path)?;
            progress("Starting the modpack server", 82);
            self.docker(["start", manifest.container_name.as_str()], 90)?;
            self.wait_for_minecraft(
                &manifest,
                remaining_duration(deadline, Duration::from_secs(20 * 60))?,
                |elapsed| {
                    let percent = 82_u64.saturating_add((elapsed / 40).min(16));
                    progress(
                        "Generating the world and waiting for the modpack",
                        u8::try_from(percent).unwrap_or(98),
                    );
                },
            )?;
            self.finalize_container_restart_policy(&manifest)?;
            self.ensure_console_archiver(&manifest)?;
            progress("Online", 100);
            Ok(json!({
                "schema_version": 1,
                "instance_id": format!("helix:{id}"),
                "instance_name": instance_name,
                "game_port": game_port,
                "port_allocated_automatically": allocated_automatically,
                "manager": "helix",
                "execution_backend": "docker",
                "modpack": {
                    "project_id": project_id,
                    "project_title": pack.name,
                    "version_id": file_id,
                    "version_number": pack.version,
                    "source_filename": file_name,
                    "minecraft_version": pack.minecraft_version,
                    "loader": pack.loader,
                    "loader_version": pack.loader_version,
                    "provider": "curseforge",
                    "installed_server_files": installed_server_files,
                    "excluded_non_jar_files": excluded_non_jar_files,
                    "excluded_launch_files": excluded_launch_files,
                    "server_pack_used": server_pack_used,
                    "server_pack_filename": server_pack_filename,
                    "full_pack_parity": false,
                }
            }))
        })();

        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                progress("Rolling back the incomplete modpack install", 99);
                let cleanup = if activated {
                    self.rollback_modpack_creation(&id, &container_name, &data_path, &manifest_path)
                } else {
                    remove_staging_directory(&self.instance_root, &staging_path)
                };
                Err(match cleanup {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; cleanup also failed: {cleanup}"),
                })
            }
        }
    }

    pub(super) fn update_minecraft_modpack(
        &self,
        manifest: &InstanceManifest,
    ) -> Result<Value, String> {
        let installed = manifest
            .modpack
            .as_ref()
            .ok_or_else(|| "this server is not tracked as a modpack".to_owned())?;
        let Some(candidate) = self.latest_modpack_update(installed)? else {
            return Ok(json!({
                "updated": false,
                "already_current": true,
                "provider": installed.provider,
                "project_title": installed.project_title,
                "version_id": installed.version_id,
                "version_number": installed.version_number,
                "backup_created": false
            }));
        };

        let deadline = Instant::now() + INSTALL_DEADLINE;
        let data_path = self.instance_path(&manifest.id)?;
        let staging_path = self.instance_root.join(format!(
            ".helix-modpack-update-{}-{}",
            manifest.id,
            Uuid::new_v4().simple()
        ));
        fs::create_dir(&staging_path)
            .map_err(|_| "could not create the isolated modpack update workspace".to_owned())?;
        fs::set_permissions(&staging_path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the modpack update workspace".to_owned())?;

        let prepared = match self.prepare_modpack_update(
            manifest,
            installed,
            &candidate.version_id,
            &staging_path,
            deadline,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                let cleanup = remove_modpack_update_workspace(&self.instance_root, &staging_path);
                return Err(match cleanup {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; update staging cleanup failed: {cleanup}"),
                });
            }
        };

        let old_lock = match read_modpack_lock(&data_path) {
            Ok(lock) => lock,
            Err(error) if error == "this modpack predates safe update tracking" => {
                let bootstrap_path = self.instance_root.join(format!(
                    ".helix-modpack-update-{}-bootstrap-{}",
                    manifest.id,
                    Uuid::new_v4().simple()
                ));
                fs::create_dir(&bootstrap_path).map_err(|_| {
                    "could not create the legacy modpack tracking workspace".to_owned()
                })?;
                fs::set_permissions(&bootstrap_path, fs::Permissions::from_mode(0o700)).map_err(
                    |_| "could not protect the legacy modpack tracking workspace".to_owned(),
                )?;
                let rebuilt = self
                    .prepare_modpack_update(
                        manifest,
                        installed,
                        &installed.version_id,
                        &bootstrap_path,
                        deadline,
                    )
                    .map(|prepared| prepared.lock);
                let cleanup = remove_modpack_update_workspace(&self.instance_root, &bootstrap_path);
                let lock = rebuilt.map_err(|rebuild| {
                    format!(
                        "the original modpack inventory could not be rebuilt safely: {rebuild}{}",
                        cleanup
                            .err()
                            .map(|cleanup| format!("; cleanup also failed: {cleanup}"))
                            .unwrap_or_default()
                    )
                })?;
                write_modpack_lock(&data_path, &lock)?;
                self.protect_instance_artifacts(&data_path, manifest.run_uid)?;
                lock
            }
            Err(error) => return Err(error),
        };
        validate_lock_identity(&old_lock, installed)?;
        validate_lock_identity(
            &prepared.lock,
            prepared
                .manifest
                .modpack
                .as_ref()
                .ok_or_else(|| "the prepared update lost its modpack identity".to_owned())?,
        )?;

        let was_running = self.container_running(&manifest.container_name);
        if was_running {
            self.docker(
                ["stop", "--time", "45", manifest.container_name.as_str()],
                75,
            )?;
        }
        let preflight =
            preflight_modpack_activation(&data_path, &staging_path, &old_lock, &prepared.lock);
        let preserved = match preflight {
            Ok(preserved) => preserved,
            Err(error) => {
                let restart = self.restart_if_previously_running(manifest, was_running);
                let _ = remove_modpack_update_workspace(&self.instance_root, &staging_path);
                return Err(format!(
                    "the modpack update was not started because local files need attention: {error}{}",
                    restart
                        .err()
                        .map(|restart| format!(
                            "; the original server failed to restart: {restart}"
                        ))
                        .unwrap_or_default()
                ));
            }
        };
        let safety_backup = match self.archive_data(manifest) {
            Ok(path) => path,
            Err(error) => {
                let restart = self.restart_if_previously_running(manifest, was_running);
                let _ = remove_modpack_update_workspace(&self.instance_root, &staging_path);
                return Err(format!(
                    "the modpack update safety backup failed, so no update files were changed: {error}{}",
                    restart
                        .err()
                        .map(|restart| format!(
                            "; the original server failed to restart: {restart}"
                        ))
                        .unwrap_or_default()
                ));
            }
        };
        let backup_id = super::backup_id_from_path(&safety_backup);
        self.stop_console_archiver(&manifest.id);
        let mut activation: Option<ModpackActivation> = None;
        let update_result = (|| {
            self.docker(["rm", manifest.container_name.as_str()], 60)?;
            let activated = activate_modpack_files(
                &self.instance_root,
                &data_path,
                &staging_path,
                &old_lock,
                &prepared.lock,
                &preserved,
            )?;
            activation = Some(activated);
            write_manifest(&self.manifest_path(&manifest.id)?, &prepared.manifest)?;
            self.chown_instance(&data_path, prepared.manifest.run_uid)?;
            self.protect_instance_artifacts(&data_path, prepared.manifest.run_uid)?;
            self.create_validation_container(&prepared.manifest, &data_path)?;
            self.docker(["start", prepared.manifest.container_name.as_str()], 90)?;
            self.wait_for_minecraft(&prepared.manifest, Duration::from_secs(20 * 60), |_| {})?;
            self.finalize_container_restart_policy(&prepared.manifest)?;
            if !was_running {
                self.docker(
                    [
                        "stop",
                        "--time",
                        "45",
                        prepared.manifest.container_name.as_str(),
                    ],
                    75,
                )?;
            }
            Ok::<(), String>(())
        })();

        if let Err(error) = update_result {
            let _ = self.docker(
                ["rm", "--force", prepared.manifest.container_name.as_str()],
                60,
            );
            let mut failed_update_recovery = None;
            let rollback_error = activation.as_ref().and_then(|activation| {
                match self.restore_modpack_safety_backup(
                    manifest,
                    &data_path,
                    &safety_backup,
                ) {
                    Ok(recovery) => {
                        failed_update_recovery = Some(recovery);
                        None
                    }
                    Err(full_restore) => {
                        let managed_restore = rollback_modpack_files(&data_path, activation);
                        Some(match managed_restore {
                            Ok(()) => format!(
                                "the full data rollback needs attention: {full_restore}; pack-managed files were restored, and safety backup {backup_id} remains available"
                            ),
                            Err(managed_restore) => format!(
                                "the full data rollback needs attention: {full_restore}; pack-file rollback also failed: {managed_restore}"
                            ),
                        })
                    }
                }
            });
            let _ = write_manifest(&self.manifest_path(&manifest.id)?, manifest);
            let recreate = self.create_container(manifest, &data_path);
            let restart = recreate.and_then(|()| {
                if was_running {
                    self.docker(["start", manifest.container_name.as_str()], 90)?;
                    self.wait_for_minecraft(manifest, Duration::from_secs(20 * 60), |_| {})?;
                }
                Ok(())
            });
            let archiver = self.ensure_console_archiver(manifest);
            let _ = remove_modpack_update_workspace(&self.instance_root, &staging_path);
            let rollback_summary = if activation.is_none() {
                "no update files remained active"
            } else if rollback_error.is_none() {
                "Helix restored the complete pre-update backup"
            } else {
                "the automatic rollback needs attention"
            };
            return Err(format!(
                "the modpack update failed validation; {rollback_summary}: {error}{}{}{}{}; safety backup {backup_id} remains available",
                rollback_error
                    .map(|rollback| format!("; file rollback needs attention: {rollback}"))
                    .unwrap_or_default(),
                restart
                    .err()
                    .map(|restart| format!("; the previous server failed to restart: {restart}"))
                    .unwrap_or_default(),
                archiver
                    .err()
                    .map(|archiver| format!("; console archiving failed to resume: {archiver}"))
                    .unwrap_or_default(),
                failed_update_recovery
                    .map(|recovery| format!(
                        "; failed update files were retained in {}",
                        recovery.display()
                    ))
                    .unwrap_or_default()
            ));
        }

        if let Some(activation) = activation {
            remove_modpack_update_workspace(&self.instance_root, &activation.rollback_root)?;
        }
        remove_modpack_update_workspace(&self.instance_root, &staging_path)?;
        let retention = self.enforce_backup_retention(manifest, &backup_id);
        self.ensure_console_archiver(&prepared.manifest)?;
        Ok(json!({
            "updated": true,
            "already_current": false,
            "provider": installed.provider,
            "project_title": installed.project_title,
            "previous_version_id": installed.version_id,
            "previous_version_number": installed.version_number,
            "version_id": candidate.version_id,
            "version_name": candidate.version_name,
            "version_number": candidate.version_number,
            "backup_created": true,
            "backup_id": backup_id,
            "restore_available": true,
            "server_was_running": was_running,
            "runtime_validation_performed": true,
            "previous_state_restored": !was_running,
            "preserved_local_files": preserved.len(),
            "retention_trashed": retention.as_ref().ok().cloned().unwrap_or_default(),
            "retention_error": retention.err()
        }))
    }

    fn restore_modpack_safety_backup(
        &self,
        manifest: &InstanceManifest,
        data_path: &Path,
        archive: &Path,
    ) -> Result<PathBuf, String> {
        let backup_root = self.backup_path(&manifest.id)?;
        if archive.parent() != Some(backup_root.as_path()) {
            return Err("the modpack safety backup path is outside this server".to_owned());
        }
        let metadata = fs::symlink_metadata(archive)
            .map_err(|_| "the modpack safety backup is unavailable".to_owned())?;
        if !metadata.file_type().is_file() || metadata.len() == 0 {
            return Err("the modpack safety backup is invalid".to_owned());
        }
        let data_metadata = fs::symlink_metadata(data_path)
            .map_err(|_| "the updated server data is unavailable for rollback".to_owned())?;
        if data_metadata.file_type().is_symlink() || !data_metadata.file_type().is_dir() {
            return Err("the updated server data is not a real directory".to_owned());
        }
        let failed_root = self.instance_root.join(".failed");
        let recovery = failed_root.join(format!(
            "modpack-update-{}-{}",
            manifest.id,
            Uuid::new_v4().simple()
        ));
        fs::rename(data_path, &recovery)
            .map_err(|_| "could not stage the failed update for full rollback".to_owned())?;
        if let Err(error) = sync_directory(&self.instance_root) {
            fs::rename(&recovery, data_path).map_err(|_| {
                format!(
                    "{error}; the failed update could not be put back from {}",
                    recovery.display()
                )
            })?;
            return Err(error);
        }

        let restore = (|| {
            fs::create_dir(data_path)
                .map_err(|_| "could not create the full rollback directory".to_owned())?;
            fs::set_permissions(data_path, fs::Permissions::from_mode(0o750))
                .map_err(|_| "could not protect the full rollback directory".to_owned())?;
            run_program(
                Path::new("/usr/bin/tar"),
                &[
                    "--extract".to_owned(),
                    "--gzip".to_owned(),
                    "--file".to_owned(),
                    archive.to_string_lossy().into_owned(),
                    "--directory".to_owned(),
                    data_path.to_string_lossy().into_owned(),
                    "--no-same-owner".to_owned(),
                    "--no-same-permissions".to_owned(),
                ],
                30 * 60,
            )?;
            self.chown_instance(data_path, manifest.run_uid)?;
            self.protect_instance_artifacts(data_path, manifest.run_uid)?;
            sync_directory(data_path)?;
            Ok::<(), String>(())
        })();
        if let Err(error) = restore {
            let partial = failed_root.join(format!(
                "modpack-update-partial-{}-{}",
                manifest.id,
                Uuid::new_v4().simple()
            ));
            let partial_move = if data_path.exists() {
                fs::rename(data_path, &partial).map_err(|_| {
                    "could not move the partial full rollback into recovery".to_owned()
                })
            } else {
                Ok(())
            };
            if let Err(partial_move) = partial_move {
                return Err(format!(
                    "{error}; {partial_move}; the pre-rollback data remains at {}",
                    recovery.display()
                ));
            }
            fs::rename(&recovery, data_path).map_err(|_| {
                format!(
                    "{error}; the updated data could not be put back from {}",
                    recovery.display()
                )
            })?;
            sync_directory(&self.instance_root)?;
            return Err(error);
        }
        Ok(recovery)
    }

    fn latest_modpack_update(
        &self,
        installed: &InstalledModpack,
    ) -> Result<Option<ModpackUpdateCandidate>, String> {
        let detail = self.minecraft_modpack_project(&installed.project_id, installed.provider)?;
        let versions = detail
            .get("versions")
            .and_then(Value::as_array)
            .ok_or_else(|| "the modpack catalog returned no release list".to_owned())?;
        let installed_date = versions
            .iter()
            .find(|version| {
                version.get("id").and_then(Value::as_str) == Some(&installed.version_id)
            })
            .and_then(|version| version.get("date_published").and_then(Value::as_str))
            .and_then(catalog_utc_second)
            .map(Ok)
            .unwrap_or_else(|| self.installed_modpack_catalog_date(installed))?;
        let mut candidate: Option<(String, ModpackUpdateCandidate)> = None;
        for version in versions {
            let id = required_text(version, "id", 64)?;
            let same_game = bounded_string_array(version.get("game_versions"), 64, 64)
                .iter()
                .any(|value| value == &installed.minecraft_version);
            let same_loader = bounded_string_array(version.get("loaders"), 32, 32)
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&installed.loader));
            let published = version
                .get("date_published")
                .and_then(Value::as_str)
                .and_then(catalog_utc_second);
            if version.get("installable").and_then(Value::as_bool) == Some(true)
                && same_game
                && same_loader
                && published
                    .as_ref()
                    .is_some_and(|date| date > &installed_date)
            {
                let date = published.expect("checked above");
                if candidate
                    .as_ref()
                    .is_none_or(|(current, _)| date > *current)
                {
                    candidate = Some((
                        date,
                        ModpackUpdateCandidate {
                            version_id: id,
                            version_name: required_text(version, "name", 256)?,
                            version_number: required_text(version, "version_number", 128)?,
                        },
                    ));
                }
            }
        }
        Ok(candidate.map(|(_, candidate)| candidate))
    }

    fn installed_modpack_catalog_date(
        &self,
        installed: &InstalledModpack,
    ) -> Result<String, String> {
        let raw = match installed.provider {
            helix_privd::ModpackProvider::Modrinth => {
                validate_modrinth_id(&installed.version_id, "version")?;
                let version = self.fetch_modrinth_json(&format!(
                    "https://api.modrinth.com/v2/version/{}",
                    installed.version_id
                ))?;
                if version.get("project_id").and_then(Value::as_str) != Some(&installed.project_id)
                {
                    return Err(
                        "Modrinth returned the installed release from a different project"
                            .to_owned(),
                    );
                }
                required_text(&version, "date_published", 64)?
            }
            helix_privd::ModpackProvider::Curseforge => {
                let version = self.curseforge_v1(&format!(
                    "mods/{}/files/{}",
                    installed.project_id, installed.version_id
                ))?;
                let version = version.get("data").cloned().unwrap_or(version);
                if version
                    .get("modId")
                    .and_then(Value::as_u64)
                    .map(|id| id.to_string())
                    != Some(installed.project_id.clone())
                {
                    return Err(
                        "CurseForge returned the installed release from a different project"
                            .to_owned(),
                    );
                }
                required_curseforge_text(&version, "fileDate", 64)?
            }
        };
        catalog_utc_second(&raw).ok_or_else(|| {
            "the provider returned an invalid installed release timestamp; no update was applied"
                .to_owned()
        })
    }

    fn prepare_modpack_update(
        &self,
        manifest: &InstanceManifest,
        installed: &InstalledModpack,
        version_id: &str,
        staging_path: &Path,
        deadline: Instant,
    ) -> Result<PreparedModpackUpdate, String> {
        match installed.provider {
            helix_privd::ModpackProvider::Modrinth => self.prepare_modrinth_update(
                manifest,
                installed,
                version_id,
                staging_path,
                deadline,
            ),
            helix_privd::ModpackProvider::Curseforge => self.prepare_curseforge_update(
                manifest,
                installed,
                version_id,
                staging_path,
                deadline,
            ),
        }
    }

    fn prepare_modrinth_update(
        &self,
        manifest: &InstanceManifest,
        installed: &InstalledModpack,
        version_id: &str,
        staging_path: &Path,
        deadline: Instant,
    ) -> Result<PreparedModpackUpdate, String> {
        validate_modrinth_id(&installed.project_id, "project")?;
        validate_modrinth_id(version_id, "version")?;
        let resolved = self.resolve_modpack_version(&installed.project_id, version_id)?;
        let archive_path = staging_path.join(".helix-source.mrpack");
        let limits = MrpackLimits::default();
        ensure_disk_space(
            &self.instance_root,
            resolved
                .file_size
                .saturating_add(MAX_SERVER_JAR_BYTES)
                .saturating_add(DISK_HEADROOM_BYTES),
        )?;
        require_exact_https_host(&resolved.file_url, MODRINTH_CDN_HOST)?;
        self.curl_no_redirect(
            &resolved.file_url,
            &archive_path,
            limits.maximum_archive_bytes,
            remaining_download_seconds(deadline)?,
        )?;
        let archive_metadata = fs::symlink_metadata(&archive_path)
            .map_err(|_| "the downloaded modpack archive is unavailable".to_owned())?;
        if !archive_metadata.file_type().is_file()
            || archive_metadata.len() == 0
            || archive_metadata.len() > limits.maximum_archive_bytes
        {
            return Err("the downloaded modpack archive is outside Helix size limits".to_owned());
        }
        verify_sha512(&archive_path, &resolved.file_sha512, "the modpack archive")?;
        let plan = inspect_mrpack(&archive_path, &limits, deadline)?;
        require_compatible_modpack_line(installed, &plan.minecraft_version, plan.loader)?;
        if !resolved
            .game_versions
            .iter()
            .any(|version| version == &plan.minecraft_version)
        {
            return Err(format!(
                "the .mrpack pins Minecraft {}, but the selected Modrinth version does not declare it",
                plan.minecraft_version
            ));
        }
        ensure_disk_space(
            &self.instance_root,
            plan.required_staging_bytes()
                .saturating_add(MAX_SERVER_JAR_BYTES)
                .saturating_add(DISK_HEADROOM_BYTES),
        )?;
        extract_overrides(&archive_path, staging_path, &plan, &limits, deadline)?;
        for file in &plan.files {
            ensure_before(deadline)?;
            let output = prepare_download_path(staging_path, &file.path, &limits)?;
            self.curl_no_redirect(
                &file.url,
                &output,
                limits.maximum_file_bytes,
                remaining_download_seconds(deadline)?,
            )?;
            verify_download(&output, file)?;
        }
        let artifact = self.resolve_modpack_loader(
            &plan.minecraft_version,
            plan.loader,
            &plan.fabric_loader_version,
        )?;
        let artifact_sha256 =
            self.download_artifact(&artifact, &staging_path.join("server.jar"))?;
        fs::remove_file(&archive_path)
            .map_err(|_| "could not remove the verified Modrinth source archive".to_owned())?;
        let runtime_image = self.resolve_runtime_image(artifact.java_version)?;
        let unix_args = if artifact.install_server {
            Some(self.run_loader_installer(
                &artifact,
                staging_path,
                manifest.run_uid,
                &runtime_image,
            )?)
        } else {
            None
        };
        let metadata = InstalledModpack {
            schema_version: 1,
            provider: helix_privd::ModpackProvider::Modrinth,
            project_id: resolved.project_id,
            project_title: resolved.project_title,
            version_id: resolved.version_id,
            version_name: resolved.version_name,
            version_number: resolved.version_number,
            minecraft_version: plan.minecraft_version.clone(),
            loader: plan.loader.to_owned(),
            loader_version: plan.fabric_loader_version,
        };
        let mut updated = manifest.clone();
        updated.software = artifact.software;
        updated.minecraft_version = artifact.version;
        updated.build = artifact.build;
        updated.java_version = artifact.java_version;
        updated.runtime_image = runtime_image;
        updated.artifact_url = artifact.url;
        updated.artifact_sha256 = artifact_sha256;
        updated.unix_args = unix_args;
        updated.modpack = Some(metadata.clone());
        let lock = build_modpack_lock(staging_path, &metadata)?;
        write_modpack_lock(staging_path, &lock)?;
        Ok(PreparedModpackUpdate {
            manifest: updated,
            lock,
        })
    }

    fn prepare_curseforge_update(
        &self,
        manifest: &InstanceManifest,
        installed: &InstalledModpack,
        version_id: &str,
        staging_path: &Path,
        deadline: Instant,
    ) -> Result<PreparedModpackUpdate, String> {
        let project_id = installed.project_id.as_str();
        if project_id.is_empty()
            || version_id.is_empty()
            || !project_id.bytes().all(|byte| byte.is_ascii_digit())
            || !version_id.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("CurseForge updates need numeric project and file ids".to_owned());
        }
        let file = self.curseforge_v1(&format!("mods/{project_id}/files/{version_id}"))?;
        let file = file.get("data").cloned().unwrap_or(file);
        if file
            .get("modId")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
            != Some(project_id.to_owned())
        {
            return Err("CurseForge returned an update from a different project".to_owned());
        }
        if file.get("isAvailable").and_then(Value::as_bool) == Some(false) {
            return Err(
                "the selected CurseForge modpack release is no longer available".to_owned(),
            );
        }
        safe_curseforge_zip_name(&file, "fileName", "modpack")?;
        let file_size = file.get("fileLength").and_then(json_u64).unwrap_or(0);
        if file_size == 0 || file_size > MAX_SERVER_JAR_BYTES {
            return Err("the CurseForge modpack size is outside Helix safety limits".to_owned());
        }
        let archive_path = staging_path.join(".helix-source.zip");
        ensure_disk_space(
            &self.instance_root,
            file_size
                .saturating_add(MAX_CURSEFORGE_SERVER_PACK_BYTES)
                .saturating_add(MAX_SERVER_JAR_BYTES)
                .saturating_add(DISK_HEADROOM_BYTES),
        )?;
        let url = self.resolve_curseforge_download_url(project_id, &file)?;
        require_forgecdn_host(&url)?;
        self.curl_no_redirect(
            &url,
            &archive_path,
            MAX_SERVER_JAR_BYTES,
            remaining_download_seconds(deadline)?,
        )?;
        self.verify_curseforge_download(&archive_path, &file, file_size, "modpack archive")?;
        let pack = read_curseforge_manifest(&archive_path)?;
        require_compatible_modpack_line(installed, &pack.minecraft_version, &pack.loader)?;

        {
            let (server_pack, _, server_pack_size) =
                self.resolve_curseforge_server_pack(project_id, &file)?;
            let server_pack_path = staging_path.join(".helix-server-pack.zip");
            let server_pack_url = self.resolve_curseforge_download_url(project_id, &server_pack)?;
            require_forgecdn_host(&server_pack_url)?;
            self.curl_no_redirect(
                &server_pack_url,
                &server_pack_path,
                MAX_CURSEFORGE_SERVER_PACK_BYTES,
                remaining_download_seconds(deadline)?,
            )?;
            self.verify_curseforge_download(
                &server_pack_path,
                &server_pack,
                server_pack_size,
                "server pack archive",
            )?;
            let extraction =
                inspect_curseforge_server_pack(&server_pack_path, staging_path, deadline)?;
            ensure_disk_space(
                &self.instance_root,
                extraction
                    .unpacked_bytes
                    .saturating_add(MAX_SERVER_JAR_BYTES)
                    .saturating_add(DISK_HEADROOM_BYTES),
            )?;
            extract_curseforge_server_pack(&server_pack_path, staging_path, &extraction, deadline)?;
            fs::remove_file(&server_pack_path)
                .map_err(|_| "could not remove the verified CurseForge server pack".to_owned())?;
        }
        let artifact = self.resolve_modpack_loader(
            &pack.minecraft_version,
            &pack.loader,
            &pack.loader_version,
        )?;
        let artifact_sha256 =
            self.download_artifact(&artifact, &staging_path.join("server.jar"))?;
        fs::remove_file(&archive_path)
            .map_err(|_| "could not remove the verified CurseForge source archive".to_owned())?;
        let runtime_image = self.resolve_runtime_image(artifact.java_version)?;
        let unix_args = if artifact.install_server {
            Some(self.run_loader_installer(
                &artifact,
                staging_path,
                manifest.run_uid,
                &runtime_image,
            )?)
        } else {
            None
        };
        let version_name = file
            .get("displayName")
            .and_then(Value::as_str)
            .map(|value| clean_text(value, 256))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| pack.version.clone());
        let metadata = InstalledModpack {
            schema_version: 1,
            provider: helix_privd::ModpackProvider::Curseforge,
            project_id: project_id.to_owned(),
            project_title: if pack.name == "CurseForge pack" {
                installed.project_title.clone()
            } else {
                pack.name
            },
            version_id: version_id.to_owned(),
            version_name,
            version_number: pack.version,
            minecraft_version: pack.minecraft_version,
            loader: pack.loader,
            loader_version: pack.loader_version,
        };
        let mut updated = manifest.clone();
        updated.software = artifact.software;
        updated.minecraft_version = artifact.version;
        updated.build = artifact.build;
        updated.java_version = artifact.java_version;
        updated.runtime_image = runtime_image;
        updated.artifact_url = artifact.url;
        updated.artifact_sha256 = artifact_sha256;
        updated.unix_args = unix_args;
        updated.modpack = Some(metadata.clone());
        let lock = build_modpack_lock(staging_path, &metadata)?;
        write_modpack_lock(staging_path, &lock)?;
        Ok(PreparedModpackUpdate {
            manifest: updated,
            lock,
        })
    }

    fn resolve_curseforge_server_pack(
        &self,
        project_id: &str,
        file: &Value,
    ) -> Result<(Value, String, u64), String> {
        let (server_pack_id, alternate) = curseforge_server_pack_link(file)?;
        let metadata = self.curseforge_v1(&format!("mods/{project_id}/files/{server_pack_id}"))?;
        let metadata = metadata.get("data").cloned().unwrap_or(metadata);
        validate_curseforge_server_pack_link(
            project_id,
            file,
            &metadata,
            server_pack_id,
            alternate,
        )?;
        let name = safe_curseforge_zip_name(&metadata, "fileName", "server pack")?;
        let size = metadata.get("fileLength").and_then(json_u64).unwrap_or(0);
        if size == 0 || size > MAX_CURSEFORGE_SERVER_PACK_BYTES {
            return Err("the CurseForge server pack is outside Helix safety limits".to_owned());
        }
        Ok((metadata, name, size))
    }

    fn resolve_modpack_loader(
        &self,
        minecraft_version: &str,
        loader: &str,
        loader_version: &str,
    ) -> Result<super::Artifact, String> {
        let artifact = match loader {
            "fabric" => self.resolve_pinned_fabric(minecraft_version, loader_version)?,
            "quilt" => self.resolve_pinned_quilt(minecraft_version, loader_version)?,
            "forge" => self.resolve_pinned_forge(minecraft_version, loader_version)?,
            "neoforge" => self.resolve_pinned_neoforge(minecraft_version, loader_version)?,
            other => {
                return Err(format!(
                    "this update uses {other}, which Helix cannot install safely"
                ));
            }
        };
        if !(17..=25).contains(&artifact.java_version) {
            return Err(format!(
                "Minecraft {} requires Java {}, which this Helix release does not manage yet",
                artifact.version, artifact.java_version
            ));
        }
        Ok(artifact)
    }

    fn verify_curseforge_download(
        &self,
        path: &Path,
        metadata: &Value,
        expected_size: u64,
        label: &str,
    ) -> Result<(), String> {
        let actual_size = fs::metadata(path)
            .map_err(|_| format!("could not inspect the downloaded CurseForge {label}"))?
            .len();
        if actual_size != expected_size {
            return Err(format!(
                "the downloaded CurseForge {label} size did not match its catalog metadata"
            ));
        }
        let expected_sha1 = metadata
            .get("hashes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|entry| {
                let algorithm = entry.get("algo").and_then(Value::as_u64)?;
                let digest = entry.get("value").and_then(Value::as_str)?;
                (algorithm == 1 && valid_hex(digest, 40)).then(|| digest.to_ascii_lowercase())
            })
            .ok_or_else(|| format!("the CurseForge {label} omitted a valid SHA-1 checksum"))?;
        if !self.file_sha1(path)?.eq_ignore_ascii_case(&expected_sha1) {
            return Err(format!(
                "the downloaded CurseForge {label} failed its catalog SHA-1 check"
            ));
        }
        Ok(())
    }

    fn resolve_modpack_version(
        &self,
        project_id: &str,
        version_id: &str,
    ) -> Result<ResolvedVersion, String> {
        let project =
            self.fetch_modrinth_json(&format!("https://api.modrinth.com/v2/project/{project_id}"))?;
        if required_text(&project, "id", 64)? != project_id
            || required_text(&project, "project_type", 32)? != "modpack"
        {
            return Err("the selected Modrinth project is not the requested modpack".to_owned());
        }
        let server_side = required_text(&project, "server_side", 32)?;
        if !matches!(server_side.as_str(), "required" | "optional") {
            return Err(
                "the selected Modrinth modpack does not support dedicated servers".to_owned(),
            );
        }
        let version =
            self.fetch_modrinth_json(&format!("https://api.modrinth.com/v2/version/{version_id}"))?;
        if required_text(&version, "id", 64)? != version_id
            || required_text(&version, "project_id", 64)? != project_id
        {
            return Err(
                "the selected Modrinth version does not belong to the requested project".to_owned(),
            );
        }
        ensure_installable_version(&version, &server_side)?;
        let file = select_mrpack_file(&version)?;
        let slug = required_text(&project, "slug", 128)?;
        validate_slug(&slug)?;
        Ok(ResolvedVersion {
            project_id: project_id.to_owned(),
            project_slug: slug,
            project_title: required_text(&project, "title", 256)?,
            version_id: version_id.to_owned(),
            version_name: required_text(&version, "name", 256)?,
            version_number: required_text(&version, "version_number", 128)?,
            game_versions: bounded_string_array(version.get("game_versions"), 64, 64),
            file_url: file.url,
            file_name: file.name,
            file_size: file.size,
            file_sha512: file.sha512,
        })
    }

    fn fetch_modrinth_json(&self, url: &str) -> Result<Value, String> {
        require_exact_https_host(url, MODRINTH_API_HOST)?;
        let cache = self.state_root.join("metadata");
        fs::create_dir_all(&cache).map_err(|_| "could not create the metadata cache".to_owned())?;
        let path = cache.join(format!("modrinth-{}.json", Uuid::new_v4()));
        let result = (|| {
            self.curl_no_redirect(url, &path, MAX_METADATA_BYTES, 30)?;
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| "downloaded Modrinth metadata is unavailable".to_owned())?;
            if !metadata.file_type().is_file()
                || metadata.len() == 0
                || metadata.len() > MAX_METADATA_BYTES
            {
                return Err("downloaded Modrinth metadata is outside the size limit".to_owned());
            }
            serde_json::from_slice(
                &fs::read(&path)
                    .map_err(|_| "could not read downloaded Modrinth metadata".to_owned())?,
            )
            .map_err(|_| "Modrinth returned invalid metadata".to_owned())
        })();
        let _ = fs::remove_file(path);
        result
    }

    fn rollback_modpack_creation(
        &self,
        id: &str,
        container_name: &str,
        data_path: &Path,
        manifest_path: &Path,
    ) -> Result<(), String> {
        if container_name != modpack_container_name(id) {
            return Err("refused to remove an unexpected modpack container".to_owned());
        }
        // Docker may have committed this uniquely named container even when
        // `docker create` itself timed out, so removal is always attempted.
        if self.docker(["rm", "--force", container_name], 60).is_err() {
            let exact_filter = format!("name=^/{container_name}$");
            let remaining = self.docker(
                [
                    "ps",
                    "--all",
                    "--filter",
                    exact_filter.as_str(),
                    "--format",
                    "{{.Names}}",
                ],
                30,
            )?;
            if remaining.lines().any(|name| name.trim() == container_name) {
                return Err(
                    "could not remove the incomplete modpack container; instance files were preserved"
                        .to_owned(),
                );
            }
        }
        match fs::remove_file(manifest_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(
                    "could not remove the incomplete modpack manifest; instance files were preserved"
                        .to_owned(),
                );
            }
        }
        remove_activated_instance(&self.instance_root, data_path, id)
    }
}

struct ModpackFile {
    name: String,
    url: String,
    size: u64,
    sha512: String,
}

fn sanitize_search_hit(hit: &Value) -> Result<Value, String> {
    let project_id = required_text(hit, "project_id", 64)?;
    validate_modrinth_id(&project_id, "project")?;
    let slug = required_text(hit, "slug", 128)?;
    validate_slug(&slug)?;
    let categories = bounded_string_array(hit.get("categories"), 64, 64);
    let display_categories = bounded_string_array(hit.get("display_categories"), 64, 64);
    let mut loaders = categories.clone();
    loaders.extend(display_categories.iter().cloned());
    loaders.sort();
    loaders.dedup();
    let server_side = optional_text(hit, "server_side", 32).unwrap_or_else(|| "unknown".into());
    let (status, reason) = loader_preview_status(&loaders, &server_side);
    Ok(json!({
        "project_id": project_id,
        "slug": slug,
        "title": required_text(hit, "title", 256)?,
        "description": optional_text(hit, "description", 2_048),
        "author": optional_text(hit, "author", 128),
        "downloads": hit.get("downloads").and_then(Value::as_u64).unwrap_or(0),
        "follows": hit.get("follows").and_then(Value::as_u64).unwrap_or(0),
        "latest_version": optional_text(hit, "latest_version", 64),
        "minecraft_versions": bounded_string_array(hit.get("versions"), 64, 64),
        "loaders": loaders,
        "server_side": server_side,
        "compatibility_status": status,
        "compatibility_reason": reason,
        "requires_version_check": status.ends_with("_candidate"),
        "web_url": format!(
            "https://modrinth.com/modpack/{}",
            percent_encode(&slug)
        ),
        "icon_url": modrinth_icon_proxy_url(hit.get("icon_url").and_then(Value::as_str)),
    }))
}

fn sanitize_curseforge_search_hit(hit: &Value) -> Result<Value, String> {
    let project_id = hit
        .get("id")
        .and_then(Value::as_u64)
        .map(|id| id.to_string())
        .ok_or_else(|| "CurseForge returned a modpack without an id".to_owned())?;
    let slug = required_curseforge_text(hit, "slug", 128)
        .or_else(|_| required_curseforge_text(hit, "name", 128))?;
    let loaders = curseforge_loaders(hit);
    let (status, reason) = if loaders.is_empty() {
        (
            "unverified",
            "Open releases to check the exact Minecraft version and loader".to_owned(),
        )
    } else {
        loader_preview_status(&loaders, "optional")
    };
    let versions = curseforge_game_versions(hit);
    let author = hit
        .get("authors")
        .and_then(Value::as_array)
        .and_then(|authors| authors.first())
        .and_then(|author| author.get("name").or_else(|| author.get("username")))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let icon = hit
        .pointer("/logo/url")
        .or_else(|| hit.pointer("/avatarUrl"))
        .or_else(|| hit.get("thumbnailUrl"))
        .and_then(Value::as_str);
    Ok(json!({
        "project_id": project_id,
        "slug": slug,
        "title": optional_text(hit, "name", 256).or_else(|| optional_text(hit, "title", 256)).unwrap_or_else(|| slug.clone()),
        "description": optional_text(hit, "summary", 2_048).or_else(|| optional_text(hit, "description", 2_048)),
        "author": author,
        "downloads": hit.get("downloadCount").or_else(|| hit.get("downloads")).and_then(Value::as_u64).unwrap_or(0),
        "follows": 0,
        "latest_version": versions.first().cloned().map(Value::from).unwrap_or(Value::Null),
        "minecraft_versions": versions,
        "loaders": loaders,
        "server_side": "optional",
        "compatibility_status": status,
        "compatibility_reason": reason,
        "requires_version_check": true,
        "provider": "curseforge",
        "web_url": format!(
            "https://www.curseforge.com/minecraft/modpacks/{}",
            percent_encode(&slug)
        ),
        "icon_url": curseforge_icon_proxy_url(icon),
    }))
}

fn curseforge_game_versions(hit: &Value) -> Vec<String> {
    let mut versions = Vec::new();
    if let Some(entries) = hit.get("latestFilesIndexes").and_then(Value::as_array) {
        for entry in entries {
            if let Some(items) = entry.get("gameVersions").and_then(Value::as_array) {
                for item in items {
                    let Some(text) = item.as_str() else {
                        continue;
                    };
                    if text
                        .chars()
                        .next()
                        .is_some_and(|character| character.is_ascii_digit())
                    {
                        versions.push(text.chars().take(32).collect());
                    }
                }
            }
        }
    }
    versions.sort();
    versions.dedup();
    versions.truncate(64);
    versions
}

fn curseforge_loaders(hit: &Value) -> Vec<String> {
    let mut loaders = Vec::new();
    if let Some(entries) = hit.get("latestFilesIndexes").and_then(Value::as_array) {
        for entry in entries {
            if let Some(versions) = entry.get("gameVersions").and_then(Value::as_array) {
                for version in versions {
                    if let Some(text) = version.as_str() {
                        let lower = text.to_ascii_lowercase();
                        if matches!(lower.as_str(), "forge" | "neoforge" | "fabric" | "quilt") {
                            loaders.push(lower);
                        }
                    }
                }
            }
        }
    }
    loaders.sort();
    loaders.dedup();
    loaders
}

fn sanitize_curseforge_modpack_version(file: &Value) -> Result<Value, String> {
    let id = file
        .get("id")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .ok_or_else(|| "CurseForge returned a modpack file without an id".to_owned())?;
    let game_versions = bounded_string_array(file.get("gameVersions"), 64, 64);
    let mut loaders = game_versions
        .iter()
        .filter(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "forge" | "neoforge" | "fabric" | "quilt"
            )
        })
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    loaders.sort();
    loaders.dedup();

    let (loader_status, loader_reason) = loader_preview_status(&loaders, "optional");
    let release_type = file.get("releaseType").and_then(Value::as_u64).unwrap_or(0);
    let version_type = match release_type {
        1 => "release",
        2 => "beta",
        _ => "alpha",
    };
    let available = file.get("isAvailable").and_then(Value::as_bool) != Some(false);
    let filename = optional_text(file, "fileName", 256).filter(|name| {
        name.to_ascii_lowercase().ends_with(".zip")
            && !name.contains(['/', '\\', ':'])
            && Path::new(name).file_name().and_then(|value| value.to_str()) == Some(name)
    });
    let size = file
        .get("fileLength")
        .and_then(json_u64)
        .filter(|size| (1..=MAX_SERVER_JAR_BYTES).contains(size));
    let archive = filename.as_ref().zip(size).map(|(filename, size)| {
        json!({
            "filename": filename,
            "size": size,
            "modrinth_declared_sha512_available": false,
        })
    });
    let installable = loader_status.ends_with("_candidate")
        && release_type == 1
        && available
        && archive.is_some()
        && curseforge_server_pack_link(file).is_ok();
    let reason = if !loader_status.ends_with("_candidate") {
        loader_reason
    } else if release_type != 1 {
        "Only stable CurseForge releases can be installed".to_owned()
    } else if !available {
        "This CurseForge file is not available for download".to_owned()
    } else if filename.is_none() {
        "This release does not provide a safe CurseForge pack ZIP".to_owned()
    } else if size.is_none() {
        "This CurseForge pack is empty or exceeds the install size limit".to_owned()
    } else if let Err(reason) = curseforge_server_pack_link(file) {
        reason
    } else {
        let loader = loaders
            .first()
            .map(|value| {
                let mut chars = value.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_else(|| "supported loader".to_owned())
            })
            .unwrap_or_else(|| "supported loader".to_owned());
        format!("Stable {loader} CurseForge pack")
    };
    let fallback_name = filename
        .clone()
        .unwrap_or_else(|| format!("CurseForge file {id}"));

    Ok(json!({
        "id": id,
        "name": optional_text(file, "displayName", 256).unwrap_or_else(|| fallback_name.clone()),
        "version_number": fallback_name,
        "version_type": version_type,
        "status": if available { "available" } else { "unavailable" },
        "date_published": optional_text(file, "fileDate", 64),
        "downloads": file.get("downloadCount").and_then(json_u64).unwrap_or(0),
        "game_versions": game_versions,
        "loaders": loaders,
        "installable": installable,
        "compatibility_reason": reason,
        "mrpack_file": archive,
    }))
}

fn sanitize_modpack_version(
    version: &Value,
    project_id: &str,
    server_side: &str,
) -> Result<Value, String> {
    let id = required_text(version, "id", 64)?;
    validate_modrinth_id(&id, "version")?;
    if required_text(version, "project_id", 64)? != project_id {
        return Err("Modrinth returned a version from a different project".to_owned());
    }
    let compatibility =
        ensure_installable_version(version, server_side).and_then(|()| select_mrpack_file(version));
    let (installable, reason, file) = match compatibility {
        Ok(file) => (true, "Stable Fabric server pack".to_owned(), Some(file)),
        Err(reason) => (false, reason, None),
    };
    Ok(json!({
        "id": id,
        "name": required_text(version, "name", 256)?,
        "version_number": required_text(version, "version_number", 128)?,
        "version_type": required_text(version, "version_type", 32)?,
        "status": optional_text(version, "status", 32),
        "date_published": optional_text(version, "date_published", 64),
        "downloads": version.get("downloads").and_then(Value::as_u64).unwrap_or(0),
        "game_versions": bounded_string_array(version.get("game_versions"), 64, 64),
        "loaders": bounded_string_array(version.get("loaders"), 32, 32),
        "installable": installable,
        "compatibility_reason": reason,
        "mrpack_file": file.map(|file| json!({
            "filename": file.name,
            "size": file.size,
            "modrinth_declared_sha512_available": true,
        })),
    }))
}

fn ensure_installable_version(version: &Value, server_side: &str) -> Result<(), String> {
    if !matches!(server_side, "required" | "optional") {
        return Err("This pack does not support dedicated servers".to_owned());
    }
    if version.get("version_type").and_then(Value::as_str) != Some("release") {
        return Err("Only stable release versions can be installed".to_owned());
    }
    if version.get("status").and_then(Value::as_str) != Some("listed") {
        return Err("Only listed Modrinth releases can be installed".to_owned());
    }
    let loaders = bounded_string_array(version.get("loaders"), 32, 32);
    if !loaders
        .iter()
        .any(|loader| matches!(loader.as_str(), "fabric" | "forge" | "neoforge" | "quilt"))
    {
        let (_, reason) = loader_preview_status(&loaders, server_side);
        return Err(reason);
    }
    if bounded_string_array(version.get("game_versions"), 64, 64).is_empty() {
        return Err("This version does not declare a Minecraft version".to_owned());
    }
    Ok(())
}

fn select_mrpack_file(version: &Value) -> Result<ModpackFile, String> {
    let files = version
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "This version has no downloadable files".to_owned())?;
    let mrpacks = files
        .iter()
        .filter(|file| {
            file.get("filename")
                .and_then(Value::as_str)
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".mrpack"))
        })
        .collect::<Vec<_>>();
    let primary = mrpacks
        .iter()
        .copied()
        .filter(|file| file.get("primary").and_then(Value::as_bool) == Some(true))
        .collect::<Vec<_>>();
    let selected = match (primary.as_slice(), mrpacks.as_slice()) {
        ([selected], _) | ([], [selected]) => *selected,
        _ => {
            return Err("This version has no unambiguous primary .mrpack file".to_owned());
        }
    };
    let name = required_text(selected, "filename", 180)?;
    if name.contains(['/', '\\']) || !name.to_ascii_lowercase().ends_with(".mrpack") {
        return Err("The selected version has an invalid .mrpack filename".to_owned());
    }
    let url = required_text(selected, "url", 4_096)?;
    require_exact_https_host(&url, MODRINTH_CDN_HOST)?;
    let limits = MrpackLimits::default();
    let size = selected
        .get("size")
        .and_then(json_u64)
        .filter(|size| *size > 0 && *size <= limits.maximum_archive_bytes)
        .ok_or_else(|| "The .mrpack archive size is outside Helix safety limits".to_owned())?;
    let sha512 = selected
        .pointer("/hashes/sha512")
        .and_then(Value::as_str)
        .filter(|hash| valid_hex(hash, 128))
        .ok_or_else(|| {
            "The .mrpack file has no valid Modrinth-declared SHA-512 checksum".to_owned()
        })?
        .to_ascii_lowercase();
    Ok(ModpackFile {
        name,
        url,
        size,
        sha512,
    })
}

fn loader_preview_status(loaders: &[String], server_side: &str) -> (&'static str, String) {
    if server_side == "unsupported" {
        return (
            "incompatible",
            "This pack does not support dedicated servers".to_owned(),
        );
    }
    if loaders.iter().any(|loader| loader == "fabric") {
        return (
            "fabric_candidate",
            "Fabric is lifecycle-ready; choose a stable server-capable release to continue"
                .to_owned(),
        );
    }
    if loaders.iter().any(|loader| loader == "neoforge") {
        return (
            "neoforge_candidate",
            "NeoForge is lifecycle-ready; choose a stable server-capable release to continue"
                .to_owned(),
        );
    }
    if loaders.iter().any(|loader| loader == "forge") {
        return (
            "forge_candidate",
            "Forge is lifecycle-ready; choose a stable server-capable release to continue"
                .to_owned(),
        );
    }
    if loaders.iter().any(|loader| loader == "quilt") {
        return (
            "quilt_candidate",
            "Quilt is lifecycle-ready; choose a stable server-capable release to continue"
                .to_owned(),
        );
    }
    (
        "incompatible",
        "No lifecycle-ready server loader was declared".to_owned(),
    )
}

fn validate_search(query: &str, offset: u32, limit: u8) -> Result<(), String> {
    if query.len() > MAX_SEARCH_QUERY_BYTES || query.chars().any(char::is_control) {
        return Err("the modpack search query is invalid".to_owned());
    }
    if offset > MAX_SEARCH_OFFSET || !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Err("the modpack search page is outside the supported range".to_owned());
    }
    Ok(())
}

fn validate_modrinth_id(value: &str, label: &str) -> Result<(), String> {
    if value.len() < 3
        || value.len() > 64
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(format!("the Modrinth {label} ID is invalid"));
    }
    Ok(())
}

fn validate_slug(value: &str) -> Result<(), String> {
    if !(3..=64).contains(&value.len())
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'!'
                        | b'@'
                        | b'$'
                        | b'('
                        | b')'
                        | b'`'
                        | b'.'
                        | b'+'
                        | b','
                        | b'"'
                        | b'-'
                        | b'\''
                )
        })
    {
        return Err("Modrinth returned an invalid project slug".to_owned());
    }
    Ok(())
}

fn ensure_disk_space(path: &Path, required: u64) -> Result<(), String> {
    let available = fs2::available_space(path)
        .map_err(|_| "could not verify free space for the modpack install".to_owned())?;
    if available < required {
        return Err(format!(
            "the modpack needs at least {required} free bytes including Helix's 2 GiB safety headroom; only {available} bytes are available"
        ));
    }
    Ok(())
}

fn remaining_download_seconds(deadline: Instant) -> Result<u64, String> {
    ensure_before(deadline)?;
    Ok(deadline
        .saturating_duration_since(Instant::now())
        .as_secs()
        .clamp(1, 10 * 60))
}

fn remaining_duration(deadline: Instant, maximum: Duration) -> Result<Duration, String> {
    ensure_before(deadline)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    Ok(remaining.min(maximum))
}

fn ensure_before(deadline: Instant) -> Result<(), String> {
    if Instant::now() > deadline {
        Err("the modpack install exceeded its 45-minute safety limit".to_owned())
    } else {
        Ok(())
    }
}

fn modpack_container_name(id: &str) -> String {
    format!("helix-game-{id}")
}

fn remove_staging_directory(root: &Path, staging: &Path) -> Result<(), String> {
    if staging.parent() != Some(root)
        || !staging
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".helix-modpack-staging-"))
    {
        return Err("refused to remove an untrusted modpack staging path".to_owned());
    }
    match fs::symlink_metadata(staging) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(staging)
                .map_err(|_| "could not remove the incomplete modpack staging directory".to_owned())
        }
        Ok(_) => Err("the modpack staging path changed during cleanup".to_owned()),
        Err(_) => Err("could not inspect the modpack staging path during cleanup".to_owned()),
    }
}

fn remove_activated_instance(root: &Path, instance: &Path, id: &str) -> Result<(), String> {
    if instance.parent() != Some(root)
        || instance.file_name().and_then(|name| name.to_str()) != Some(id)
        || Uuid::parse_str(id).is_err()
    {
        return Err("refused to remove an untrusted modpack instance path".to_owned());
    }
    match fs::symlink_metadata(instance) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(instance)
                .map_err(|_| "could not remove the incomplete modpack instance".to_owned())
        }
        Ok(_) => Err("the modpack instance path changed during rollback".to_owned()),
        Err(_) => Err("could not inspect the modpack instance path during rollback".to_owned()),
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn required_text(value: &Value, field: &str, maximum: usize) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= maximum)
        .map(|text| clean_text(text, maximum))
        .ok_or_else(|| format!("the Modrinth field {field} was missing or invalid"))
}

fn required_curseforge_text(value: &Value, field: &str, maximum: usize) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= maximum)
        .map(|text| clean_text(text, maximum))
        .ok_or_else(|| format!("the CurseForge field {field} was missing or invalid"))
}

fn optional_text(value: &Value, field: &str, maximum: usize) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|text| clean_text(text, maximum))
}

fn optional_text_chars(value: &Value, field: &str, maximum: usize) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(|text| {
        text.chars()
            .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
            .take(maximum)
            .collect()
    })
}

fn clean_text(value: &str, maximum_bytes: usize) -> String {
    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= maximum_bytes)
        .last()
        .unwrap_or(0);
    let end = if value.len() <= maximum_bytes {
        value.len()
    } else {
        end
    };
    value[..end]
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .collect()
}

fn bounded_string_array(
    value: Option<&Value>,
    maximum_items: usize,
    maximum_length: usize,
) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(maximum_items)
        .map(|value| clean_text(value, maximum_length))
        .collect()
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct CurseforgePack {
    name: String,
    version: String,
    minecraft_version: String,
    loader: String,
    loader_version: String,
}

fn require_forgecdn_host(url: &str) -> Result<(), String> {
    require_https_host(url, FORGECDN_DOWNLOAD_HOSTS)
}

fn curseforge_server_pack_link(file: &Value) -> Result<(u64, bool), String> {
    if let Some(id) = file
        .get("serverPackFileId")
        .and_then(json_u64)
        .filter(|id| *id > 0)
    {
        return Ok((id, false));
    }
    // Some publishers attach the server ZIP as an additional file rather than marking it a server pack.
    if let Some(id) = file
        .get("alternateFileId")
        .and_then(json_u64)
        .filter(|id| *id > 0)
    {
        return Ok((id, true));
    }
    Err("This CurseForge release has no linked server pack. Choose a release with publisher-provided server files; Helix will not install the client mod list on a dedicated server.".to_owned())
}

fn validate_curseforge_server_pack_link(
    project_id: &str,
    client: &Value,
    server: &Value,
    expected_id: u64,
    alternate: bool,
) -> Result<(), String> {
    let client_id = client
        .get("id")
        .and_then(json_u64)
        .filter(|id| *id > 0)
        .ok_or_else(|| "CurseForge returned a pack without a release id".to_owned())?;
    if server.get("modId").and_then(json_u64) != project_id.parse::<u64>().ok()
        || server.get("id").and_then(json_u64) != Some(expected_id)
        || client_id == expected_id
    {
        return Err(
            "CurseForge returned a server pack from a different project or release".to_owned(),
        );
    }
    let parent = server
        .get("parentProjectFileId")
        .and_then(json_u64)
        .filter(|id| *id > 0);
    if parent.is_some_and(|id| id != client_id) || (alternate && parent != Some(client_id)) {
        return Err(
            "The CurseForge additional file does not belong to the selected release".to_owned(),
        );
    }
    if server.get("isAvailable").and_then(Value::as_bool) == Some(false) {
        return Err("The publisher's CurseForge server pack is no longer available".to_owned());
    }
    Ok(())
}

fn safe_curseforge_zip_name(value: &Value, field: &str, label: &str) -> Result<String, String> {
    let name = required_curseforge_text(value, field, 256)?;
    if !name.to_ascii_lowercase().ends_with(".zip")
        || name.contains(['/', '\\', ':'])
        || Path::new(&name)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(name.as_str())
    {
        return Err(format!("the CurseForge {label} filename is unsafe"));
    }
    Ok(name)
}

fn require_compatible_modpack_line(
    installed: &InstalledModpack,
    minecraft_version: &str,
    loader: &str,
) -> Result<(), String> {
    if minecraft_version != installed.minecraft_version {
        return Err(format!(
            "this release changes Minecraft from {} to {}; use a new server for cross-version upgrades",
            installed.minecraft_version, minecraft_version
        ));
    }
    if !loader.eq_ignore_ascii_case(&installed.loader) {
        return Err(format!(
            "this release changes the loader from {} to {loader}; use a new server for loader migrations",
            installed.loader
        ));
    }
    Ok(())
}

fn catalog_utc_second(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.last() != Some(&b'Z')
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || bytes
            .iter()
            .take(19)
            .enumerate()
            .any(|(index, byte)| !matches!(index, 4 | 7 | 10 | 13 | 16) && !byte.is_ascii_digit())
    {
        return None;
    }
    Some(value[..19].to_owned())
}

fn read_curseforge_manifest(archive_path: &Path) -> Result<CurseforgePack, String> {
    let file = File::open(archive_path)
        .map_err(|_| "could not open the CurseForge pack archive".to_owned())?;
    let mut archive =
        ZipArchive::new(file).map_err(|_| "the CurseForge pack is not a valid ZIP".to_owned())?;
    let mut manifest = archive.by_name("manifest.json").map_err(|_| {
        "this CurseForge zip has no manifest.json; Helix needs a standard pack or server pack"
            .to_owned()
    })?;
    let mut bytes = Vec::new();
    manifest
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read the CurseForge manifest".to_owned())?;
    drop(manifest);
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "the CurseForge manifest is not valid JSON".to_owned())?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(|value| clean_text(value, 256))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "CurseForge pack".to_owned());
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .map(|value| clean_text(value, 128))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "selected release".to_owned());
    let minecraft_version = value
        .pointer("/minecraft/version")
        .and_then(Value::as_str)
        .ok_or_else(|| "the CurseForge manifest does not pin a Minecraft version".to_owned())?
        .to_owned();
    let loader_id = value
        .pointer("/minecraft/modLoaders")
        .and_then(Value::as_array)
        .and_then(|loaders| {
            loaders.iter().find_map(|entry| {
                let primary = entry.get("primary").and_then(Value::as_bool) != Some(false);
                primary
                    .then(|| entry.get("id").and_then(Value::as_str))
                    .flatten()
            })
        })
        .ok_or_else(|| "the CurseForge manifest does not pin a server loader".to_owned())?;
    let (loader, loader_version) = split_curseforge_loader(loader_id)?;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "the CurseForge manifest has no file list".to_owned())?;
    if files.len() > 4_000 {
        return Err("the CurseForge pack lists too many files for a safe install".to_owned());
    }
    Ok(CurseforgePack {
        name,
        version,
        minecraft_version,
        loader,
        loader_version,
    })
}

fn split_curseforge_loader(loader_id: &str) -> Result<(String, String), String> {
    let (loader, version) = loader_id
        .split_once('-')
        .ok_or_else(|| "the CurseForge loader pin is invalid".to_owned())?;
    let loader = match loader {
        "forge" | "neoforge" | "fabric" | "quilt" => loader.to_owned(),
        "fabric-loader" => "fabric".to_owned(),
        "quilt-loader" => "quilt".to_owned(),
        other => {
            return Err(format!(
                "this CurseForge pack uses {other}, which Helix cannot install yet"
            ));
        }
    };
    Ok((loader, version.to_owned()))
}

struct CurseforgeServerPackPlan {
    entries: Vec<CurseforgeServerPackEntry>,
    unpacked_bytes: u64,
    mod_jar_count: usize,
    skipped_helix_owned_files: usize,
}

struct CurseforgeServerPackEntry {
    archive_index: usize,
    relative_path: String,
    size: u64,
}

fn inspect_curseforge_server_pack(
    archive_path: &Path,
    destination: &Path,
    deadline: Instant,
) -> Result<CurseforgeServerPackPlan, String> {
    ensure_before(deadline)?;
    let metadata = fs::symlink_metadata(destination)
        .map_err(|_| "the CurseForge server-pack staging directory is unavailable".to_owned())?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("the CurseForge server-pack staging directory is unsafe".to_owned());
    }
    let file = File::open(archive_path)
        .map_err(|_| "could not open the CurseForge server-pack archive".to_owned())?;
    let mut archive = ZipArchive::new(file)
        .map_err(|_| "the CurseForge server pack is not a valid ZIP".to_owned())?;
    if archive.is_empty() || archive.len() > MAX_CURSEFORGE_SERVER_PACK_ENTRIES {
        return Err(format!(
            "the CurseForge server pack must contain 1 to {MAX_CURSEFORGE_SERVER_PACK_ENTRIES} entries"
        ));
    }
    let limits = MrpackLimits::default();
    let mut entries = Vec::new();
    let mut unpacked_bytes = 0_u64;
    let mut mod_jar_count = 0_usize;
    let mut skipped_helix_owned_files = 0_usize;
    let mut file_keys = HashSet::new();
    let mut directory_keys = HashSet::new();

    for archive_index in 0..archive.len() {
        ensure_before(deadline)?;
        let entry = archive
            .by_index(archive_index)
            .map_err(|_| "could not inspect a CurseForge server-pack entry".to_owned())?;
        validate_curseforge_server_pack_entry(&entry)?;
        let raw_name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| "the CurseForge server pack contains a non-UTF-8 path".to_owned())?;
        let relative_path = raw_name.trim_end_matches('/');
        if relative_path.is_empty() {
            continue;
        }
        validate_relative_path(relative_path, &limits)?;
        let key = relative_path.to_ascii_lowercase();
        if entry.is_dir() {
            if file_keys.contains(&key) {
                return Err(format!(
                    "the CurseForge server pack uses {relative_path} as both a file and directory"
                ));
            }
            directory_keys.insert(key);
            continue;
        }
        if curseforge_server_pack_path_is_helix_owned(&key) {
            skipped_helix_owned_files = skipped_helix_owned_files.saturating_add(1);
            continue;
        }
        if file_keys.contains(&key) || directory_keys.contains(&key) {
            return Err(format!(
                "the CurseForge server pack repeats the path {relative_path}"
            ));
        }
        let mut parent = String::new();
        let segments = key.split('/').collect::<Vec<_>>();
        for segment in segments.iter().take(segments.len().saturating_sub(1)) {
            if !parent.is_empty() {
                parent.push('/');
            }
            parent.push_str(segment);
            if file_keys.contains(&parent) {
                return Err(format!(
                    "the CurseForge server pack places {relative_path} below a file"
                ));
            }
            directory_keys.insert(parent.clone());
        }
        let size = entry.size();
        if size > MAX_SERVER_JAR_BYTES {
            return Err(format!(
                "the CurseForge server-pack file {relative_path} exceeds the per-file safety limit"
            ));
        }
        if size > 0
            && (entry.compressed_size() == 0
                || size.div_ceil(entry.compressed_size()) > MAX_CURSEFORGE_COMPRESSION_RATIO)
        {
            return Err(format!(
                "the CurseForge server-pack file {relative_path} exceeds the compression safety limit"
            ));
        }
        unpacked_bytes = unpacked_bytes
            .checked_add(size)
            .ok_or_else(|| "the CurseForge server-pack size overflowed".to_owned())?;
        if unpacked_bytes > MAX_CURSEFORGE_SERVER_PACK_UNPACKED_BYTES {
            return Err(
                "the CurseForge server pack exceeds the 10 GiB unpacked safety limit".to_owned(),
            );
        }
        if key.starts_with("mods/") && key.ends_with(".jar") {
            mod_jar_count = mod_jar_count.saturating_add(1);
        }
        file_keys.insert(key);
        entries.push(CurseforgeServerPackEntry {
            archive_index,
            relative_path: relative_path.to_owned(),
            size,
        });
    }
    if mod_jar_count == 0 {
        return Err("the CurseForge server pack contains no server mod JARs".to_owned());
    }
    Ok(CurseforgeServerPackPlan {
        entries,
        unpacked_bytes,
        mod_jar_count,
        skipped_helix_owned_files,
    })
}

fn extract_curseforge_server_pack(
    archive_path: &Path,
    destination: &Path,
    plan: &CurseforgeServerPackPlan,
    deadline: Instant,
) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|_| "could not reopen the CurseForge server-pack archive".to_owned())?;
    let mut archive = ZipArchive::new(file)
        .map_err(|_| "the CurseForge server pack could not be reopened".to_owned())?;
    for planned in &plan.entries {
        ensure_before(deadline)?;
        let mut entry = archive
            .by_index(planned.archive_index)
            .map_err(|_| "the CurseForge server pack changed during extraction".to_owned())?;
        validate_curseforge_server_pack_entry(&entry)?;
        let raw_name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| "the CurseForge server pack changed during extraction".to_owned())?;
        if raw_name != planned.relative_path || entry.size() != planned.size {
            return Err("the CurseForge server pack changed during extraction".to_owned());
        }
        let output = destination.join(Path::new(&planned.relative_path));
        let parent = output
            .parent()
            .ok_or_else(|| "the CurseForge server pack produced an invalid path".to_owned())?;
        create_curseforge_server_pack_directories(destination, parent)?;
        let mut output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&output)
            .map_err(|_| {
                format!(
                    "could not create the staged CurseForge server-pack file {}",
                    planned.relative_path
                )
            })?;
        let mut written = 0_u64;
        let mut buffer = [0_u8; 128 * 1024];
        loop {
            ensure_before(deadline)?;
            let read = entry.read(&mut buffer).map_err(|_| {
                format!(
                    "could not extract the CurseForge server-pack file {}",
                    planned.relative_path
                )
            })?;
            if read == 0 {
                break;
            }
            written = written
                .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
                .ok_or_else(|| {
                    "the CurseForge server-pack extraction size overflowed".to_owned()
                })?;
            if written > planned.size {
                return Err(format!(
                    "the CurseForge server-pack file {} exceeded its declared size",
                    planned.relative_path
                ));
            }
            output_file.write_all(&buffer[..read]).map_err(|_| {
                format!(
                    "could not write the CurseForge server-pack file {}",
                    planned.relative_path
                )
            })?;
        }
        if written != planned.size {
            return Err(format!(
                "the CurseForge server-pack file {} did not match its declared size",
                planned.relative_path
            ));
        }
        output_file.sync_data().map_err(|_| {
            format!(
                "could not persist the CurseForge server-pack file {}",
                planned.relative_path
            )
        })?;
    }
    Ok(())
}

fn validate_curseforge_server_pack_entry<R: Read + ?Sized>(
    entry: &zip::read::ZipFile<'_, R>,
) -> Result<(), String> {
    if entry.encrypted() {
        return Err("encrypted CurseForge server-pack entries are not supported".to_owned());
    }
    if entry.is_symlink() {
        return Err("the CurseForge server pack contains a symbolic link".to_owned());
    }
    if !entry.is_file() && !entry.is_dir() {
        return Err("the CurseForge server pack contains a special file".to_owned());
    }
    if let Some(mode) = entry.unix_mode() {
        let kind = mode & 0o170000;
        if kind != 0 && kind != 0o100000 && kind != 0o040000 {
            return Err("the CurseForge server pack contains a special file".to_owned());
        }
    }
    if !matches!(
        entry.compression(),
        CompressionMethod::Stored | CompressionMethod::Deflated
    ) {
        return Err("the CurseForge server pack uses unsupported compression".to_owned());
    }
    Ok(())
}

fn curseforge_server_pack_path_is_helix_owned(path: &str) -> bool {
    let root = path.split('/').next().unwrap_or_default();
    matches!(root, "server.jar" | "eula.txt" | "server.properties") || root.starts_with(".helix")
}

fn create_curseforge_server_pack_directories(root: &Path, parent: &Path) -> Result<(), String> {
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| "a CurseForge server-pack output escaped staging".to_owned())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err(
                    "a CurseForge server-pack output parent is not a real directory".to_owned(),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|_| {
                    "could not create a CurseForge server-pack directory".to_owned()
                })?;
            }
            Err(_) => {
                return Err(
                    "could not inspect a CurseForge server-pack output directory".to_owned(),
                );
            }
        }
    }
    Ok(())
}

fn build_modpack_lock(root: &Path, metadata: &InstalledModpack) -> Result<ModpackLock, String> {
    let mut managed_files = Vec::new();
    collect_managed_modpack_files(root, root, &mut managed_files)?;
    managed_files.sort_by(|left, right| left.path.cmp(&right.path));
    if managed_files.is_empty() || managed_files.len() > MAX_CURSEFORGE_SERVER_PACK_ENTRIES {
        return Err("the prepared modpack produced an invalid managed-file inventory".to_owned());
    }
    Ok(ModpackLock {
        schema_version: 1,
        provider: metadata.provider,
        project_id: metadata.project_id.clone(),
        version_id: metadata.version_id.clone(),
        managed_files,
    })
}

fn collect_managed_modpack_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<ManagedModpackFile>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|_| "could not inventory the prepared modpack".to_owned())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "could not inventory the prepared modpack".to_owned())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if output.len() >= MAX_CURSEFORGE_SERVER_PACK_ENTRIES {
            return Err("the prepared modpack contains too many managed files".to_owned());
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "could not inspect a prepared modpack file".to_owned())?;
        if metadata.file_type().is_symlink() {
            return Err("the prepared modpack contains a symbolic link".to_owned());
        }
        if metadata.file_type().is_dir() {
            collect_managed_modpack_files(root, &path, output)?;
            continue;
        }
        if !metadata.file_type().is_file() {
            return Err("the prepared modpack contains a special file".to_owned());
        }
        let relative = path
            .strip_prefix(root)
            .ok()
            .and_then(Path::to_str)
            .map(|value| value.replace('\\', "/"))
            .ok_or_else(|| "the prepared modpack contains an invalid path".to_owned())?;
        if modpack_inventory_path_is_mutable(&relative) {
            continue;
        }
        validate_relative_path(&relative, &MrpackLimits::default())?;
        output.push(ManagedModpackFile {
            path: relative,
            sha256: file_sha256(&path)?,
        });
    }
    Ok(())
}

fn modpack_inventory_path_is_mutable(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let root = lower.split('/').next().unwrap_or_default();
    matches!(
        root,
        MODPACK_LOCK_FILE
            | "server.properties"
            | "eula.txt"
            | "server-icon.png"
            | "ops.json"
            | "whitelist.json"
            | "banned-ips.json"
            | "banned-players.json"
            | "usercache.json"
    ) || root.starts_with("world")
        || matches!(root, "logs" | "crash-reports" | "backups")
}

fn write_modpack_lock(root: &Path, lock: &ModpackLock) -> Result<(), String> {
    let body = serde_json::to_vec(lock)
        .map_err(|_| "could not encode the modpack update inventory".to_owned())?;
    if body.is_empty() || u64::try_from(body.len()).unwrap_or(u64::MAX) > MAX_MODPACK_LOCK_BYTES {
        return Err("the modpack update inventory exceeds Helix limits".to_owned());
    }
    write_new_file(&root.join(MODPACK_LOCK_FILE), &body, 0o440)
        .map_err(|_| "could not persist the modpack update inventory".to_owned())
}

fn read_modpack_lock(root: &Path) -> Result<ModpackLock, String> {
    let path = root.join(MODPACK_LOCK_FILE);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| "this modpack predates safe update tracking".to_owned())?;
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_MODPACK_LOCK_BYTES
    {
        return Err("the saved modpack update inventory is invalid".to_owned());
    }
    let lock: ModpackLock = serde_json::from_slice(
        &fs::read(&path).map_err(|_| "could not read the modpack update inventory".to_owned())?,
    )
    .map_err(|_| "the saved modpack update inventory is invalid".to_owned())?;
    let unique_paths = lock
        .managed_files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    if lock.schema_version != 1
        || lock.managed_files.is_empty()
        || lock.managed_files.len() > MAX_CURSEFORGE_SERVER_PACK_ENTRIES
        || unique_paths.len() != lock.managed_files.len()
        || lock.managed_files.iter().any(|file| {
            file.sha256.len() != 64
                || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                || validate_relative_path(&file.path, &MrpackLimits::default()).is_err()
                || modpack_inventory_path_is_mutable(&file.path)
        })
    {
        return Err("the saved modpack update inventory is invalid".to_owned());
    }
    Ok(lock)
}

fn validate_lock_identity(lock: &ModpackLock, installed: &InstalledModpack) -> Result<(), String> {
    if lock.schema_version != 1
        || lock.provider != installed.provider
        || lock.project_id != installed.project_id
        || lock.version_id != installed.version_id
    {
        return Err(
            "the saved modpack inventory does not match the installed release; no files were changed"
                .to_owned(),
        );
    }
    Ok(())
}

fn remove_modpack_update_workspace(root: &Path, workspace: &Path) -> Result<(), String> {
    if workspace.parent() != Some(root)
        || !workspace
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".helix-modpack-update-"))
    {
        return Err("refused to remove an untrusted modpack update workspace".to_owned());
    }
    match fs::symlink_metadata(workspace) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(workspace)
                .map_err(|_| "could not remove the modpack update workspace".to_owned())
        }
        Ok(_) => Err("the modpack update workspace changed during cleanup".to_owned()),
        Err(_) => Err("could not inspect the modpack update workspace".to_owned()),
    }
}

fn preflight_modpack_activation(
    data_root: &Path,
    staging_root: &Path,
    old_lock: &ModpackLock,
    new_lock: &ModpackLock,
) -> Result<HashSet<String>, String> {
    require_real_directory(data_root, "installed modpack")?;
    require_real_directory(staging_root, "prepared modpack update")?;
    let old_by_path = old_lock
        .managed_files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<HashMap<_, _>>();
    let mut preserved = HashSet::new();

    for file in &new_lock.managed_files {
        let staged = safe_modpack_file(staging_root, &file.path)?;
        if file_sha256(&staged)? != file.sha256 {
            return Err(format!(
                "the prepared update changed after verification at {}",
                file.path
            ));
        }
    }

    for file in &old_lock.managed_files {
        let current = data_root.join(&file.path);
        match fs::symlink_metadata(&current) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                ensure_safe_modpack_parent(data_root, &file.path, false)?;
                if file_sha256(&current)? != file.sha256 {
                    if is_integrity_critical_modpack_path(&file.path) {
                        return Err(format!(
                            "{} is a locally changed executable pack file; restore or remove it before updating",
                            file.path
                        ));
                    }
                    preserved.insert(file.path.clone());
                }
            }
            Ok(_) => {
                return Err(format!("{} is no longer a regular file", file.path));
            }
            Err(_) => return Err(format!("could not inspect {}", file.path)),
        }
    }

    for file in &new_lock.managed_files {
        if old_by_path.contains_key(file.path.as_str()) {
            continue;
        }
        let current = data_root.join(&file.path);
        match fs::symlink_metadata(&current) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                ensure_safe_modpack_parent(data_root, &file.path, false)?;
                if is_integrity_critical_modpack_path(&file.path) {
                    return Err(format!(
                        "{} is an unmanaged executable file that conflicts with the update",
                        file.path
                    ));
                }
                preserved.insert(file.path.clone());
            }
            Ok(_) => {
                return Err(format!(
                    "{} conflicts with the update and is not a regular file",
                    file.path
                ));
            }
            Err(_) => return Err(format!("could not inspect {}", file.path)),
        }
    }
    Ok(preserved)
}

fn activate_modpack_files(
    instance_root: &Path,
    data_root: &Path,
    staging_root: &Path,
    old_lock: &ModpackLock,
    new_lock: &ModpackLock,
    preserved: &HashSet<String>,
) -> Result<ModpackActivation, String> {
    let rollback_root = instance_root.join(format!(
        ".helix-modpack-update-rollback-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir(&rollback_root)
        .map_err(|_| "could not create the modpack rollback workspace".to_owned())?;
    fs::set_permissions(&rollback_root, fs::Permissions::from_mode(0o700))
        .map_err(|_| "could not protect the modpack rollback workspace".to_owned())?;
    let mut activation = ModpackActivation {
        rollback_root,
        moved_old_files: Vec::new(),
        activated_new_files: Vec::new(),
        old_lock_moved: false,
    };
    let result = (|| {
        for file in &old_lock.managed_files {
            if preserved.contains(&file.path) {
                continue;
            }
            let current = data_root.join(&file.path);
            match fs::symlink_metadata(&current) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Ok(metadata)
                    if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {}
                Ok(_) => return Err(format!("{} changed during update activation", file.path)),
                Err(_) => return Err(format!("could not inspect {} during activation", file.path)),
            }
            let rollback = activation.rollback_root.join(&file.path);
            ensure_safe_modpack_parent(&activation.rollback_root, &file.path, true)?;
            fs::rename(&current, &rollback)
                .map_err(|_| format!("could not stage the previous {} for rollback", file.path))?;
            activation.moved_old_files.push(file.path.clone());
        }

        let old_lock_path = data_root.join(MODPACK_LOCK_FILE);
        let rollback_lock = activation.rollback_root.join(MODPACK_LOCK_FILE);
        fs::rename(&old_lock_path, &rollback_lock).map_err(|_| {
            "could not stage the previous modpack inventory for rollback".to_owned()
        })?;
        activation.old_lock_moved = true;

        for file in &new_lock.managed_files {
            if preserved.contains(&file.path) {
                continue;
            }
            let staged = safe_modpack_file(staging_root, &file.path)?;
            let destination = data_root.join(&file.path);
            ensure_safe_modpack_parent(data_root, &file.path, true)?;
            if fs::symlink_metadata(&destination).is_ok() {
                return Err(format!("{} appeared during update activation", file.path));
            }
            fs::rename(&staged, &destination)
                .map_err(|_| format!("could not activate the updated {}", file.path))?;
            activation.activated_new_files.push(file.path.clone());
        }
        fs::rename(
            staging_root.join(MODPACK_LOCK_FILE),
            data_root.join(MODPACK_LOCK_FILE),
        )
        .map_err(|_| "could not activate the new modpack inventory".to_owned())?;
        sync_directory(data_root)?;
        sync_directory(&activation.rollback_root)?;
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        let rollback = rollback_modpack_files(data_root, &activation);
        if rollback.is_ok() {
            let _ = remove_modpack_update_workspace(instance_root, &activation.rollback_root);
        }
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback) => format!("{error}; immediate file rollback failed: {rollback}"),
        });
    }
    Ok(activation)
}

fn rollback_modpack_files(data_root: &Path, activation: &ModpackActivation) -> Result<(), String> {
    for relative in activation.activated_new_files.iter().rev() {
        let path = data_root.join(relative);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                fs::remove_file(&path)
                    .map_err(|_| format!("could not remove the failed updated {relative}"))?;
            }
            Ok(_) => {
                return Err(format!(
                    "the failed updated {relative} is not a regular file"
                ));
            }
            Err(_) => return Err(format!("could not inspect the failed updated {relative}")),
        }
    }
    if activation.old_lock_moved {
        let active_lock = data_root.join(MODPACK_LOCK_FILE);
        if active_lock.exists() {
            fs::remove_file(&active_lock)
                .map_err(|_| "could not remove the failed modpack inventory".to_owned())?;
        }
    }
    for relative in activation.moved_old_files.iter().rev() {
        let rollback = safe_modpack_file(&activation.rollback_root, relative)?;
        let destination = data_root.join(relative);
        ensure_safe_modpack_parent(data_root, relative, true)?;
        fs::rename(&rollback, &destination)
            .map_err(|_| format!("could not restore the previous {relative}"))?;
    }
    if activation.old_lock_moved {
        fs::rename(
            activation.rollback_root.join(MODPACK_LOCK_FILE),
            data_root.join(MODPACK_LOCK_FILE),
        )
        .map_err(|_| "could not restore the previous modpack inventory".to_owned())?;
    }
    sync_directory(data_root)
}

fn require_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| format!("the {label} directory is unavailable"))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("the {label} directory is unsafe"));
    }
    Ok(())
}

fn safe_modpack_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    validate_relative_path(relative, &MrpackLimits::default())?;
    ensure_safe_modpack_parent(root, relative, false)?;
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| format!("the modpack file {relative} is unavailable"))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!("the modpack file {relative} is unsafe"));
    }
    Ok(path)
}

fn ensure_safe_modpack_parent(root: &Path, relative: &str, create: bool) -> Result<(), String> {
    validate_relative_path(relative, &MrpackLimits::default())?;
    require_real_directory(root, "modpack root")?;
    let relative_path = Path::new(relative);
    let Some(parent) = relative_path.parent() else {
        return Ok(());
    };
    let mut current = root.to_path_buf();
    for component in parent.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                fs::create_dir(&current).map_err(|_| {
                    format!("could not create the modpack directory for {relative}")
                })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Ok(_) => return Err(format!("a parent of {relative} is not a real directory")),
            Err(_) => return Err(format!("could not inspect a parent of {relative}")),
        }
    }
    Ok(())
}

fn is_integrity_critical_modpack_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let root = lower.split('/').next().unwrap_or_default();
    lower.ends_with(".jar")
        || lower.ends_with(".sh")
        || lower.ends_with(".bat")
        || lower.ends_with(".cmd")
        || matches!(
            root,
            "server.jar" | "libraries" | "versions" | "unix_args.txt" | "run.sh" | "run.bat"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn installed(version_id: &str) -> InstalledModpack {
        InstalledModpack {
            schema_version: 1,
            provider: helix_privd::ModpackProvider::Curseforge,
            project_id: "925200".to_owned(),
            project_title: "All the Mods 10".to_owned(),
            version_id: version_id.to_owned(),
            version_name: version_id.to_owned(),
            version_number: version_id.to_owned(),
            minecraft_version: "1.21.1".to_owned(),
            loader: "neoforge".to_owned(),
            loader_version: "21.1.249".to_owned(),
        }
    }

    #[test]
    fn catalog_dates_are_compared_as_utc_seconds() {
        assert_eq!(
            catalog_utc_second("2026-09-04T08:12:13.456Z").as_deref(),
            Some("2026-09-04T08:12:13")
        );
        assert!(catalog_utc_second("2026-09-04 08:12:13Z").is_none());
        assert!(catalog_utc_second("2026-09-04T08:12:13-06:00").is_none());
    }

    #[test]
    fn modpack_inventory_excludes_worlds_and_helix_owned_runtime_state() {
        let root = tempfile::tempdir().expect("temporary modpack");
        fs::create_dir(root.path().join("mods")).expect("mods");
        fs::create_dir(root.path().join("world")).expect("world");
        fs::create_dir(root.path().join("logs")).expect("logs");
        fs::write(root.path().join("mods/example.jar"), b"jar").expect("mod");
        fs::write(root.path().join("server.jar"), b"server").expect("server");
        fs::write(root.path().join("server.properties"), b"motd=mine\n").expect("settings");
        fs::write(root.path().join("world/level.dat"), b"world").expect("world data");
        fs::write(root.path().join("logs/latest.log"), b"log").expect("log");

        let lock = build_modpack_lock(root.path(), &installed("old")).expect("inventory");
        let paths = lock
            .managed_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["mods/example.jar", "server.jar"]);
    }

    #[test]
    fn activation_preserves_local_config_and_rolls_back_exact_pack_files() {
        let instance_root = tempfile::tempdir().expect("instance root");
        let data = instance_root.path().join("server");
        let staging = instance_root.path().join(".helix-modpack-update-stage");
        fs::create_dir(&data).expect("data");
        fs::create_dir(&staging).expect("staging");
        fs::create_dir(data.join("mods")).expect("old mods");
        fs::create_dir(data.join("config")).expect("old config");
        fs::create_dir(data.join("world")).expect("world");
        fs::write(data.join("mods/old.jar"), b"old jar").expect("old jar");
        fs::write(data.join("config/pack.toml"), b"pack=true\n").expect("old config");
        fs::write(data.join("world/level.dat"), b"valuable world").expect("world");
        let old_lock = build_modpack_lock(&data, &installed("old")).expect("old lock");
        write_modpack_lock(&data, &old_lock).expect("write old lock");
        fs::write(data.join("config/pack.toml"), b"owner=true\n").expect("owner config");

        fs::create_dir(staging.join("mods")).expect("new mods");
        fs::create_dir(staging.join("config")).expect("new config");
        fs::write(staging.join("mods/new.jar"), b"new jar").expect("new jar");
        fs::write(staging.join("config/pack.toml"), b"pack=false\n").expect("new config");
        let new_lock = build_modpack_lock(&staging, &installed("new")).expect("new lock");
        write_modpack_lock(&staging, &new_lock).expect("write new lock");

        let preserved = preflight_modpack_activation(&data, &staging, &old_lock, &new_lock)
            .expect("safe preflight");
        assert_eq!(preserved, HashSet::from(["config/pack.toml".to_owned()]));
        let activation = activate_modpack_files(
            instance_root.path(),
            &data,
            &staging,
            &old_lock,
            &new_lock,
            &preserved,
        )
        .expect("activate update");
        assert!(!data.join("mods/old.jar").exists());
        assert_eq!(
            fs::read(data.join("mods/new.jar")).expect("new jar"),
            b"new jar"
        );
        assert_eq!(
            fs::read(data.join("config/pack.toml")).expect("preserved config"),
            b"owner=true\n"
        );
        assert_eq!(
            fs::read(data.join("world/level.dat")).expect("preserved world"),
            b"valuable world"
        );

        rollback_modpack_files(&data, &activation).expect("rollback");
        assert_eq!(
            fs::read(data.join("mods/old.jar")).expect("old jar"),
            b"old jar"
        );
        assert!(!data.join("mods/new.jar").exists());
        assert_eq!(
            fs::read(data.join("config/pack.toml")).expect("owner config"),
            b"owner=true\n"
        );
        validate_lock_identity(
            &read_modpack_lock(&data).expect("restored lock"),
            &installed("old"),
        )
        .expect("old lock restored");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_boot_rollback_restores_the_complete_backup_including_world_data() {
        use std::sync::Mutex;

        let temporary = tempfile::tempdir().expect("temporary roots");
        let state_root = temporary.path().join("state");
        let instance_root = temporary.path().join("instances");
        let backup_root = temporary.path().join("backups");
        let custom_import_root = state_root.join("imports");
        for path in [
            &state_root,
            &instance_root,
            &backup_root,
            &custom_import_root,
            &instance_root.join(".failed"),
        ] {
            fs::create_dir_all(path).expect("managed root");
        }
        let manager = NativeManager {
            state_root,
            instance_root: instance_root.clone(),
            backup_root: backup_root.clone(),
            docker_binary: PathBuf::from("/bin/true"),
            console_retention: crate::native::ConsoleRetention {
                maximum_bytes: 16 * 1024 * 1024,
                files: 2,
            },
            backup_trash_retention_days: 30,
            custom_artifact_roots: Vec::new(),
            custom_import_root,
            uploads: Mutex::new(HashMap::new()),
            operations: Mutex::new(HashSet::new()),
            port_policies: Mutex::new(()),
            console_archives: Mutex::new(HashMap::new()),
            console_stops: Mutex::new(HashMap::new()),
            tps_cache: Mutex::new(HashMap::new()),
            amp: None,
        };
        let id = "6f55caa9-1264-4baf-8335-d3f31a704614";
        let data = instance_root.join(id);
        fs::create_dir_all(data.join("world")).expect("world");
        fs::create_dir_all(data.join("mods")).expect("mods");
        fs::write(data.join("world/level.dat"), b"safe world").expect("world data");
        fs::write(data.join("mods/old.jar"), b"old mod").expect("old mod");
        fs::write(data.join("server.jar"), b"old server").expect("old server");
        let installed = installed("old");
        write_modpack_lock(
            &data,
            &build_modpack_lock(&data, &installed).expect("old inventory"),
        )
        .expect("write old inventory");
        let manifest = InstanceManifest {
            schema_version: MANIFEST_VERSION,
            kind: crate::native::GameKind::Minecraft,
            id: id.to_owned(),
            name: "Rollback test".to_owned(),
            instance_name: "rollback-test".to_owned(),
            container_name: format!("helix-game-{id}"),
            software: MinecraftSoftware::NeoForge,
            minecraft_version: "1.21.1".to_owned(),
            build: "21.1.249".to_owned(),
            java_version: 21,
            runtime_image: "eclipse-temurin@sha256:test".to_owned(),
            artifact_url: "https://example.invalid/server.jar".to_owned(),
            artifact_sha256: "a".repeat(64),
            memory_mb: 4096,
            cpu_millis: 0,
            max_players: 20,
            game_port: 25_565,
            query_port: 0,
            rcon_port: 30_000,
            rcon_password: "secret".to_owned(),
            start_on_boot: false,
            run_uid: 0,
            created_at_unix_ms: 1,
            unix_args: None,
            backup_keep_count: 0,
            backup_keep_days: 0,
            modpack: Some(installed),
        };
        let backup_directory = backup_root.join(id);
        fs::create_dir(&backup_directory).expect("backup directory");
        let archive = backup_directory.join("1788506432788.tar.gz");
        run_program(
            Path::new("/usr/bin/tar"),
            &[
                "--numeric-owner".to_owned(),
                "--one-file-system".to_owned(),
                "-czf".to_owned(),
                archive.to_string_lossy().into_owned(),
                "-C".to_owned(),
                data.to_string_lossy().into_owned(),
                ".".to_owned(),
            ],
            30,
        )
        .expect("backup archive");

        fs::write(data.join("world/level.dat"), b"migrated world").expect("migrated world");
        fs::remove_file(data.join("mods/old.jar")).expect("remove old mod");
        fs::write(data.join("mods/new.jar"), b"new mod").expect("new mod");
        let recovery = manager
            .restore_modpack_safety_backup(&manifest, &data, &archive)
            .expect("full rollback");

        assert_eq!(
            fs::read(data.join("world/level.dat")).expect("restored world"),
            b"safe world"
        );
        assert_eq!(
            fs::read(data.join("mods/old.jar")).expect("restored old mod"),
            b"old mod"
        );
        assert!(!data.join("mods/new.jar").exists());
        assert_eq!(
            fs::read(recovery.join("world/level.dat")).expect("failed update recovery"),
            b"migrated world"
        );
    }

    #[test]
    fn preflight_rejects_a_locally_changed_mod_jar() {
        let root = tempfile::tempdir().expect("root");
        let data = root.path().join("data");
        let staging = root.path().join("staging");
        fs::create_dir_all(data.join("mods")).expect("data mods");
        fs::create_dir_all(staging.join("mods")).expect("staged mods");
        fs::write(data.join("mods/example.jar"), b"publisher bytes").expect("old mod");
        let old_lock = build_modpack_lock(&data, &installed("old")).expect("old lock");
        fs::write(data.join("mods/example.jar"), b"owner replacement").expect("changed mod");
        fs::write(staging.join("mods/example.jar"), b"new publisher bytes").expect("new mod");
        let new_lock = build_modpack_lock(&staging, &installed("new")).expect("new lock");
        let error = preflight_modpack_activation(&data, &staging, &old_lock, &new_lock)
            .expect_err("changed executable must fail closed");
        assert!(error.contains("locally changed executable"));
    }

    #[test]
    fn curseforge_server_pack_is_bounded_and_keeps_helix_owned_files_out() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let staging = temporary.path().join("staging");
        fs::create_dir(&staging).expect("staging directory");
        let archive_path = temporary.path().join("server-pack.zip");
        let archive_file = File::create(&archive_path).expect("server-pack archive");
        let mut archive = ZipWriter::new(archive_file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        archive
            .start_file("mods/server-mod.jar", options)
            .expect("mod entry");
        archive.write_all(b"server mod bytes").expect("mod body");
        archive
            .start_file("config/server.toml", options)
            .expect("config entry");
        archive.write_all(b"enabled=true\n").expect("config body");
        archive
            .start_file("server.properties", options)
            .expect("owned entry");
        archive
            .write_all(b"online-mode=false\n")
            .expect("owned body");
        archive.finish().expect("finish archive");

        let deadline = Instant::now() + Duration::from_secs(10);
        let plan = inspect_curseforge_server_pack(&archive_path, &staging, deadline)
            .expect("valid server pack");
        assert_eq!(plan.mod_jar_count, 1);
        assert_eq!(plan.skipped_helix_owned_files, 1);
        extract_curseforge_server_pack(&archive_path, &staging, &plan, deadline)
            .expect("extract server pack");
        assert_eq!(
            fs::read(staging.join("mods/server-mod.jar")).expect("extracted mod"),
            b"server mod bytes"
        );
        assert!(!staging.join("server.properties").exists());
    }

    #[test]
    fn modrinth_slugs_follow_the_documented_v2_character_set() {
        for slug in [
            "smoothserver",
            "fps-max+",
            "server_pack.v2",
            "author's-pack",
            "pack!(test)",
        ] {
            validate_slug(slug).expect("documented Modrinth slug");
        }
        for slug in ["ab", "space slug", "slash/slug", "line\nfeed"] {
            assert!(validate_slug(slug).is_err());
        }
        assert_eq!(percent_encode("fps-max+"), "fps-max%2B");
    }

    #[test]
    fn preview_reasons_mark_lifecycle_ready_loaders() {
        for (loader, status) in [
            ("fabric", "fabric_candidate"),
            ("forge", "forge_candidate"),
            ("neoforge", "neoforge_candidate"),
            ("quilt", "quilt_candidate"),
        ] {
            let (found, reason) = loader_preview_status(&[loader.to_owned()], "required");
            assert_eq!(found, status);
            assert!(!reason.is_empty());
        }
        let (status, reason) = loader_preview_status(&["rift".to_owned()], "required");
        assert_eq!(status, "incompatible");
        assert!(reason.contains("lifecycle-ready"));
    }

    #[test]
    fn only_listed_release_fabric_versions_with_an_unambiguous_mrpack_are_installable() {
        let version = json!({
            "id": "version123",
            "project_id": "project123",
            "name": "Release",
            "version_number": "1.0",
            "version_type": "release",
            "status": "listed",
            "game_versions": ["1.21.1"],
            "loaders": ["fabric"],
            "files": [{
                "filename": "pack.mrpack",
                "primary": true,
                "url": "https://cdn.modrinth.com/data/project/version/pack.mrpack",
                "size": 4096,
                "hashes": {"sha512": "a".repeat(128)}
            }]
        });
        ensure_installable_version(&version, "required").expect("stable Fabric release");
        select_mrpack_file(&version).expect("Modrinth-declared file");

        let mut beta = version.clone();
        beta["version_type"] = json!("beta");
        assert!(ensure_installable_version(&beta, "required").is_err());
        let mut neoforge = version;
        neoforge["loaders"] = json!(["neoforge"]);
        ensure_installable_version(&neoforge, "required").expect("stable NeoForge release");

        let ambiguous = json!({
            "files": [
                {
                    "filename": "first.mrpack",
                    "primary": true,
                    "url": "https://cdn.modrinth.com/data/project/version/first.mrpack",
                    "size": 4096,
                    "hashes": {"sha512": "a".repeat(128)}
                },
                {
                    "filename": "second.mrpack",
                    "primary": true,
                    "url": "https://cdn.modrinth.com/data/project/version/second.mrpack",
                    "size": 4096,
                    "hashes": {"sha512": "b".repeat(128)}
                }
            ]
        });
        assert!(select_mrpack_file(&ambiguous).is_err());
    }

    #[test]
    fn curseforge_modpack_files_are_normalized_for_the_shared_release_contract() {
        let file = json!({
            "id": 9876543,
            "modId": 123456,
            "isAvailable": true,
            "displayName": "Adventure Pack 1.2.0",
            "fileName": "adventure-pack-1.2.0.zip",
            "releaseType": 1,
            "fileDate": "2026-08-20T10:30:00Z",
            "fileLength": 2_000_000,
            "downloadCount": 42,
            "serverPackFileId": 9876544,
            "gameVersions": ["1.21.1", "Forge"],
        });
        let normalized = sanitize_curseforge_modpack_version(&file).expect("CurseForge release");
        assert_eq!(normalized["id"], "9876543");
        assert_eq!(normalized["downloads"], 42);
        assert_eq!(normalized["status"], "available");
        assert_eq!(normalized["date_published"], "2026-08-20T10:30:00Z");
        assert_eq!(normalized["version_type"], "release");
        assert_eq!(normalized["loaders"], json!(["forge"]));
        assert_eq!(normalized["installable"], true);
        assert_eq!(normalized["mrpack_file"]["size"], 2_000_000);
    }

    #[test]
    fn curseforge_search_defers_loader_judgment_when_summary_metadata_is_empty() {
        let hit = json!({
            "id": 925200,
            "slug": "all-the-mods-10",
            "name": "All the Mods 10",
            "summary": "A large modpack",
            "downloadCount": 21_000_000,
            "authors": [{"name": "ATMTeam"}],
            "latestFilesIndexes": [],
        });
        let normalized = sanitize_curseforge_search_hit(&hit).expect("CurseForge search hit");
        assert_eq!(normalized["compatibility_status"], "unverified");
        assert!(
            normalized["compatibility_reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("Open releases"))
        );
        assert_eq!(normalized["requires_version_check"], true);
    }

    #[test]
    fn curseforge_modpack_files_fail_closed_without_hiding_the_release() {
        let beta = json!({
            "id": 9876543,
            "isAvailable": true,
            "displayName": "Adventure Pack beta",
            "fileName": "adventure-pack-beta.zip",
            "releaseType": 2,
            "fileLength": 2_000_000,
            "gameVersions": ["1.21.1", "NeoForge"],
        });
        let normalized = sanitize_curseforge_modpack_version(&beta).expect("CurseForge beta");
        assert_eq!(normalized["downloads"], 0);
        assert_eq!(normalized["version_type"], "beta");
        assert_eq!(normalized["installable"], false);
        assert!(
            normalized["compatibility_reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("stable CurseForge releases"))
        );

        let mut unsafe_archive = beta;
        unsafe_archive["releaseType"] = json!(1);
        unsafe_archive["fileName"] = json!("../pack.zip");
        let normalized = sanitize_curseforge_modpack_version(&unsafe_archive)
            .expect("visible but unavailable unsafe release");
        assert_eq!(normalized["installable"], false);
        assert!(normalized["mrpack_file"].is_null());
    }

    #[test]
    fn curseforge_additional_server_pack_requires_the_exact_parent_release() {
        let client = json!({"id": 7854204, "serverPackFileId": null, "alternateFileId": 7854213});
        let mut server = json!({"id": 7854213, "modId": 1298402, "isServerPack": false,
            "parentProjectFileId": 7854204, "isAvailable": true});
        assert_eq!(
            curseforge_server_pack_link(&client).unwrap(),
            (7854213, true)
        );
        validate_curseforge_server_pack_link("1298402", &client, &server, 7854213, true).unwrap();
        for parent in [json!(null), json!(0), json!(7854205)] {
            server["parentProjectFileId"] = parent;
            assert!(
                validate_curseforge_server_pack_link("1298402", &client, &server, 7854213, true)
                    .is_err()
            );
        }
        server["parentProjectFileId"] = json!(7854204);
        server["modId"] = json!(1);
        assert!(
            validate_curseforge_server_pack_link("1298402", &client, &server, 7854213, true)
                .is_err()
        );
    }

    #[test]
    fn curseforge_server_links_never_fall_back_to_client_mods() {
        let client = json!({"id": 10, "serverPackFileId": 11, "alternateFileId": 12});
        assert_eq!(curseforge_server_pack_link(&client).unwrap(), (11, false));
        let mut server = json!({"id": 11, "modId": 1, "isAvailable": true});
        validate_curseforge_server_pack_link("1", &client, &server, 11, false).unwrap();
        server["isAvailable"] = json!(false);
        assert!(validate_curseforge_server_pack_link("1", &client, &server, 11, false).is_err());
        assert!(
            curseforge_server_pack_link(&json!({"serverPackFileId": 0, "alternateFileId": null}))
                .unwrap_err()
                .contains("will not install the client mod list")
        );
        let file = json!({"id": 10, "gameVersions": ["1.21.1", "NeoForge"], "releaseType": 1,
            "fileName": "client.zip", "fileLength": 100});
        let normalized = sanitize_curseforge_modpack_version(&file).unwrap();
        assert_eq!(normalized["installable"], false);
        assert!(
            normalized["compatibility_reason"]
                .as_str()
                .unwrap()
                .contains("no linked server pack")
        );
    }

    #[test]
    fn cleanup_is_bounded_to_an_exact_staging_child() {
        let root = Path::new("/srv/helix/instances");
        assert!(
            remove_staging_directory(root, Path::new("/srv/helix/instances/server"))
                .expect_err("non-staging path")
                .contains("refused")
        );
        assert!(
            remove_staging_directory(root, Path::new("/srv/helix/.helix-modpack-staging-id"))
                .expect_err("outside root")
                .contains("refused")
        );
    }

    #[test]
    fn rollback_container_name_is_derived_only_from_the_random_instance_id() {
        let id = "6f55caa9-1264-4baf-8335-d3f31a704614";
        assert_eq!(
            modpack_container_name(id),
            "helix-game-6f55caa9-1264-4baf-8335-d3f31a704614"
        );
    }
}
