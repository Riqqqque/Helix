use super::{
    Artifact, InstanceManifest, MANIFEST_VERSION, MAX_METADATA_BYTES, MAX_SERVER_JAR_BYTES,
    MinecraftCreateSpec, MinecraftModpackCreateSpec, MinecraftSoftware, NativeManager,
    allocate_rcon_port, allocate_run_uid, ensure_port_available, file_sha256, instance_name,
    marketplace::modrinth_icon_proxy_url, now_unix_ms, server_properties, validate_create_spec,
    write_manifest, write_new_file,
};
use helix_privd::mrpack::{
    MrpackLimits, extract_overrides, inspect_mrpack, prepare_download_path,
    require_exact_https_host, verify_download, verify_sha512,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::Path,
    time::{Duration, Instant},
};
use uuid::Uuid;

const MODRINTH_API_HOST: &str = "api.modrinth.com";
const MODRINTH_CDN_HOST: &str = "cdn.modrinth.com";
const MAX_SEARCH_QUERY_BYTES: usize = 120;
const MAX_SEARCH_OFFSET: u32 = 10_000;
const MAX_SEARCH_LIMIT: u8 = 50;
const MAX_PROJECT_BODY_CHARS: usize = 128 * 1024;
const MAX_VERSIONS_RETURNED: usize = 200;
const INSTALL_DEADLINE: Duration = Duration::from_secs(45 * 60);
const DISK_HEADROOM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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

impl NativeManager {
    pub fn minecraft_modpack_search(
        &self,
        query: &str,
        offset: u32,
        limit: u8,
    ) -> Result<Value, String> {
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
                "loader": "fabric",
                "stable_releases_only": true,
                "server_capable_only": true,
                "other_loaders_preview_only": ["forge", "neoforge", "quilt"],
            },
            "source": "Modrinth",
            "collected_at_unix_ms": now_unix_ms(),
        }))
    }

    pub fn minecraft_modpack_project(&self, project_id: &str) -> Result<Value, String> {
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
        validate_modrinth_id(&request.project_id, "project")?;
        validate_modrinth_id(&request.version_id, "version")?;
        let base_spec = MinecraftCreateSpec {
            name: request.name.clone(),
            software: MinecraftSoftware::Fabric,
            version: "latest".to_owned(),
            memory_mb: request.memory_mb,
            max_players: request.max_players,
            game_port: request.game_port,
            start_on_boot: request.start_on_boot,
            eula_accepted: request.eula_accepted,
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
        ensure_port_available(request.game_port, true)?;
        let rcon_port = allocate_rcon_port(&manifests)?;
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
            self.curl_no_redirect(
                &resolved.file_url,
                &archive_path,
                resolved.file_size,
                remaining_download_seconds(deadline)?,
            )?;
            let archive_metadata = fs::symlink_metadata(&archive_path)
                .map_err(|_| "the downloaded modpack archive is unavailable".to_owned())?;
            if !archive_metadata.file_type().is_file()
                || archive_metadata.len() != resolved.file_size
            {
                return Err(
                    "the downloaded modpack archive did not match its declared size".to_owned(),
                );
            }
            verify_sha512(&archive_path, &resolved.file_sha512, "the modpack archive")?;

            progress(
                "Validating paths, hashes, loader pins, and safety bounds",
                22,
            );
            let limits = MrpackLimits::default();
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
            let artifact =
                self.resolve_pinned_fabric(&plan.minecraft_version, &plan.fabric_loader_version)?;
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
                    file.size,
                    remaining_download_seconds(deadline)?,
                )?;
                verify_download(&output, file)?;
            }

            progress("Pinning the exact Minecraft and Fabric server runtime", 58);
            let jar_path = staging_path.join("server.jar");
            let artifact_sha256 = self.download_pinned_fabric_artifact(
                &artifact,
                &jar_path,
                remaining_download_seconds(deadline)?,
            )?;
            write_new_file(&staging_path.join("eula.txt"), b"eula=true\n", 0o640)?;
            let rcon_password = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            let server_spec = MinecraftCreateSpec {
                version: plan.minecraft_version.clone(),
                ..base_spec.clone()
            };
            write_new_file(
                &staging_path.join("server.properties"),
                server_properties(&server_spec, rcon_port, &rcon_password).as_bytes(),
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
            let manifest = InstanceManifest {
                schema_version: MANIFEST_VERSION,
                id: id.clone(),
                name: request.name.trim().to_owned(),
                instance_name: instance_name.clone(),
                container_name: container_name.clone(),
                software: MinecraftSoftware::Fabric,
                minecraft_version: artifact.version,
                build: artifact.build,
                java_version: artifact.java_version,
                runtime_image,
                artifact_url: artifact.url,
                artifact_sha256,
                memory_mb: request.memory_mb,
                max_players: request.max_players,
                game_port: request.game_port,
                rcon_port,
                rcon_password,
                start_on_boot: request.start_on_boot,
                run_uid,
                created_at_unix_ms: now_unix_ms(),
            };
            write_manifest(&manifest_path, &manifest)?;
            self.chown_instance(&data_path, run_uid)?;
            self.protect_instance_artifacts(&data_path, run_uid)?;

            progress("Creating the isolated Helix workload", 75);
            self.create_container(&manifest, &data_path)?;
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
            self.ensure_console_archiver(&manifest)?;
            progress("Online", 100);
            Ok(json!({
                "schema_version": 1,
                "instance_id": format!("helix:{id}"),
                "instance_name": instance_name,
                "game_port": request.game_port,
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
                    "fabric_loader_version": plan.fabric_loader_version,
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

    fn download_pinned_fabric_artifact(
        &self,
        artifact: &Artifact,
        destination: &Path,
        maximum_seconds: u64,
    ) -> Result<String, String> {
        if !matches!(artifact.software, MinecraftSoftware::Fabric) {
            return Err("the modpack runtime resolved to a non-Fabric server".to_owned());
        }
        require_exact_https_host(&artifact.url, "meta.fabricmc.net")?;
        let partial = destination.with_extension("jar.partial");
        self.curl_no_redirect(
            &artifact.url,
            &partial,
            MAX_SERVER_JAR_BYTES,
            maximum_seconds,
        )?;
        let metadata = fs::symlink_metadata(&partial)
            .map_err(|_| "the pinned Fabric server download is unavailable".to_owned())?;
        if !metadata.file_type().is_file()
            || metadata.len() < 16 * 1024
            || metadata.len() > MAX_SERVER_JAR_BYTES
        {
            let _ = fs::remove_file(&partial);
            return Err(
                "the pinned Fabric server file is outside the expected size range".to_owned(),
            );
        }
        let sha256 = file_sha256(&partial)?;
        fs::rename(&partial, destination)
            .map_err(|_| "could not commit the pinned Fabric server file".to_owned())?;
        Ok(sha256)
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
        "requires_version_check": status == "fabric_candidate",
        "web_url": format!(
            "https://modrinth.com/modpack/{}",
            percent_encode(&slug)
        ),
        "icon_url": modrinth_icon_proxy_url(hit.get("icon_url").and_then(Value::as_str)),
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
    if !loaders.iter().any(|loader| loader == "fabric") {
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
        .and_then(Value::as_u64)
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
            "incompatible",
            "NeoForge packs are preview-only until Helix has a lifecycle-ready NeoForge server loader"
                .to_owned(),
        );
    }
    if loaders.iter().any(|loader| loader == "forge") {
        return (
            "incompatible",
            "Forge packs are preview-only until Helix has a lifecycle-ready Forge server loader"
                .to_owned(),
        );
    }
    if loaders.iter().any(|loader| loader == "quilt") {
        return (
            "incompatible",
            "Quilt packs are preview-only until Helix has a lifecycle-ready Quilt server loader"
                .to_owned(),
        );
    }
    (
        "incompatible",
        "No lifecycle-ready Fabric server release was declared".to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn preview_reasons_never_claim_unsupported_loaders_are_installable() {
        for (loader, expected) in [
            ("forge", "Forge"),
            ("neoforge", "NeoForge"),
            ("quilt", "Quilt"),
        ] {
            let (status, reason) = loader_preview_status(&[loader.to_owned()], "required");
            assert_eq!(status, "incompatible");
            assert!(reason.contains(expected));
        }
        let (status, _) = loader_preview_status(&["fabric".to_owned()], "required");
        assert_eq!(status, "fabric_candidate");
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
        assert!(ensure_installable_version(&neoforge, "required").is_err());

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
