use super::{
    InstanceManifest, MinecraftSoftware, NativeManager, backup_id_from_path, native_id,
    now_unix_ms, require_https_host, run_program, software_name, valid_hex,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha512};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_SEARCH_QUERY_BYTES: usize = 120;
const MAX_SEARCH_OFFSET: u32 = 10_000;
const MAX_SEARCH_LIMIT: u8 = 50;
const MAX_PROJECT_BODY_CHARS: usize = 128 * 1024;
const MAX_MARKETPLACE_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MARKETPLACE_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_DEPENDENCY_PROJECTS: usize = 32;
const MAX_INSTALL_RECORD_BYTES: u64 = 256 * 1024;

#[derive(Clone)]
struct ContentProfile {
    kind: &'static str,
    search_project_type: &'static str,
    directory: &'static str,
    accepted_loaders: &'static [&'static str],
}

#[derive(Clone)]
struct ResolvedContent {
    project_id: String,
    project_slug: String,
    project_title: String,
    version_id: String,
    version_number: String,
    file: ResolvedFile,
    optional_dependencies: Vec<String>,
}

#[derive(Clone)]
struct ResolvedFile {
    filename: String,
    url: String,
    size: u64,
    sha512: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InstallRecord {
    schema_version: u32,
    project_id: String,
    project_slug: String,
    project_title: String,
    version_id: String,
    version_number: String,
    content_directory: String,
    files: Vec<String>,
    installed_at_unix_ms: u64,
}

struct RecordBackup {
    path: PathBuf,
    previous: Option<Vec<u8>>,
}

#[derive(Default)]
struct ContentMutation {
    installed: Vec<PathBuf>,
    previous_files: Vec<(PathBuf, PathBuf)>,
    records: Vec<RecordBackup>,
}

impl NativeManager {
    pub fn marketplace_search(
        &self,
        instance_id: &str,
        query: &str,
        offset: u32,
        limit: u8,
    ) -> Result<Value, String> {
        let manifest = self.load_manifest(native_id(instance_id))?;
        let profile = content_profile(manifest.software)?;
        validate_search(query, offset, limit)?;
        let facets = json!([
            [format!("all_project_types:{}", profile.search_project_type)],
            [format!("versions:{}", manifest.minecraft_version)],
            profile
                .accepted_loaders
                .iter()
                .map(|loader| format!("categories:{loader}"))
                .collect::<Vec<_>>(),
            ["server_side:required", "server_side:optional"]
        ]);
        let facets = serde_json::to_string(&facets)
            .map_err(|_| "could not encode the marketplace filter".to_owned())?;
        let url = format!(
            "https://api.modrinth.com/v2/search?query={}&facets={}&index=relevance&offset={offset}&limit={limit}",
            percent_encode(query.trim()),
            percent_encode(&facets)
        );
        let response = self.fetch_json(&url, &["api.modrinth.com"])?;
        sanitize_search_response(&response, instance_id, &manifest, &profile)
    }

    pub fn marketplace_project(
        &self,
        instance_id: &str,
        project_id: &str,
    ) -> Result<Value, String> {
        validate_modrinth_id(project_id, "project")?;
        let manifest = self.load_manifest(native_id(instance_id))?;
        let profile = content_profile(manifest.software)?;
        let project = self.fetch_modrinth_project(project_id)?;
        validate_project_compatibility(&project, &profile)?;
        let resolved_project_id = required_text(&project, "id", 64)?;
        validate_modrinth_id(&resolved_project_id, "project")?;
        let versions = self.compatible_modrinth_versions(
            &resolved_project_id,
            &manifest.minecraft_version,
            &profile,
        )?;
        let versions = versions
            .as_array()
            .into_iter()
            .flatten()
            .take(100)
            .map(sanitize_version_summary)
            .collect::<Result<Vec<_>, _>>()?;
        let version_count = versions.len();
        let slug = required_text(&project, "slug", 128)?;
        Ok(json!({
            "schema_version": 1,
            "instance_id": instance_id,
            "compatibility": {
                "minecraft_version": manifest.minecraft_version,
                "server_software": software_name(manifest.software),
                "content_kind": profile.kind,
                "accepted_loaders": profile.accepted_loaders,
                "install_directory": profile.directory,
            },
            "project": {
                "id": required_text(&project, "id", 64)?,
                "slug": slug,
                "title": required_text(&project, "title", 256)?,
                "description": optional_text(&project, "description", 2_048),
                "body": optional_text_chars(&project, "body", MAX_PROJECT_BODY_CHARS),
                "project_type": required_text(&project, "project_type", 32)?,
                "content_kind": profile.kind,
                "server_side": required_text(&project, "server_side", 32)?,
                "downloads": project.get("downloads").and_then(Value::as_u64).unwrap_or(0),
                "followers": project.get("followers").and_then(Value::as_u64).unwrap_or(0),
                "license": project.pointer("/license/name").and_then(Value::as_str).map(|value| clean_text(value, 128)),
                "source_url": safe_https_url(project.get("source_url").and_then(Value::as_str)),
                "issues_url": safe_https_url(project.get("issues_url").and_then(Value::as_str)),
                "wiki_url": safe_https_url(project.get("wiki_url").and_then(Value::as_str)),
                "web_url": format!("https://modrinth.com/{}/{slug}", profile.kind),
                "icon_url": modrinth_icon_proxy_url(project.get("icon_url").and_then(Value::as_str)),
            },
            "versions": versions,
            "version_count_returned": version_count,
            "version_results_truncated": version_count == 100,
            "body_format": "plain_text",
            "collected_at_unix_ms": now_unix_ms(),
        }))
    }

    pub fn install_marketplace_content(
        &self,
        instance_id: &str,
        project_id: &str,
        version_id: Option<&str>,
    ) -> Result<Value, String> {
        validate_modrinth_id(project_id, "project")?;
        if let Some(version_id) = version_id {
            validate_modrinth_id(version_id, "version")?;
        }
        let manifest = self.load_manifest(native_id(instance_id))?;
        let profile = content_profile(manifest.software)?;
        let _operation = self.begin_instance_operation(&manifest.id, "marketplace install")?;
        let resolved = self.resolve_content_tree(
            project_id,
            version_id,
            &manifest.minecraft_version,
            &profile,
        )?;
        let root = resolved
            .first()
            .ok_or_else(|| "the marketplace returned no installable content".to_owned())?;
        let staging = self.marketplace_staging_path()?;
        let staged_files = match self.download_content_tree(&resolved, &staging) {
            Ok(files) => files,
            Err(error) => {
                let _ = remove_directory_if_present(&staging);
                return Err(error);
            }
        };

        let was_running = self.container_running(&manifest.container_name);
        if was_running
            && let Err(error) = self.docker(
                ["stop", "--time", "45", manifest.container_name.as_str()],
                75,
            )
        {
            let _ = remove_directory_if_present(&staging);
            return Err(error);
        }
        let backup = match self.archive_data(&manifest) {
            Ok(path) => path,
            Err(error) => {
                let restart = self.restart_if_previously_running(&manifest, was_running);
                let _ = remove_directory_if_present(&staging);
                return Err(match restart {
                    Ok(()) => format!("content installation stopped before changes: {error}"),
                    Err(restart) => format!(
                        "content installation stopped before changes: {error}; the previous server also failed to restart: {restart}"
                    ),
                });
            }
        };

        let mut mutation = ContentMutation::default();
        let commit_result = self.commit_content_tree(
            &manifest,
            &profile,
            &resolved,
            &staged_files,
            &staging,
            &mut mutation,
        );
        if let Err(error) = commit_result {
            let rollback = rollback_content(&mut mutation);
            let restart = self.restart_if_previously_running(&manifest, was_running);
            let _ = remove_directory_if_present(&staging);
            return Err(combine_install_failure(error, rollback, restart));
        }

        let restart_result = self.restart_if_previously_running(&manifest, was_running);
        if let Err(error) = restart_result {
            let _ = self.docker(
                ["stop", "--time", "20", manifest.container_name.as_str()],
                45,
            );
            let rollback = rollback_content(&mut mutation);
            let recovery = self.restart_if_previously_running(&manifest, was_running);
            let _ = remove_directory_if_present(&staging);
            return Err(combine_install_failure(
                format!("Minecraft rejected the new content during startup: {error}"),
                rollback,
                recovery,
            ));
        }

        let _ = remove_directory_if_present(&staging);
        Ok(json!({
            "schema_version": 1,
            "instance_id": instance_id,
            "project_id": root.project_id,
            "project_slug": root.project_slug,
            "project_title": root.project_title,
            "version_id": root.version_id,
            "version_number": root.version_number,
            "installed_projects": resolved.iter().map(|item| json!({
                "project_id": item.project_id,
                "project_slug": item.project_slug,
                "project_title": item.project_title,
                "version_id": item.version_id,
                "version_number": item.version_number,
                "filename": item.file.filename,
            })).collect::<Vec<_>>(),
            "dependency_count": resolved.len().saturating_sub(1),
            "optional_dependencies_not_installed": resolved.iter().flat_map(|item| item.optional_dependencies.clone()).collect::<Vec<_>>(),
            "backup_id": backup_id_from_path(&backup),
            "server_was_running": was_running,
            "restart_required": !was_running,
            "runtime_validation_performed": was_running,
            "rollback_on_failed_startup": was_running,
        }))
    }

    fn fetch_modrinth_project(&self, project_id: &str) -> Result<Value, String> {
        self.fetch_json(
            &format!("https://api.modrinth.com/v2/project/{project_id}"),
            &["api.modrinth.com"],
        )
    }

    fn compatible_modrinth_versions(
        &self,
        project_id: &str,
        minecraft_version: &str,
        profile: &ContentProfile,
    ) -> Result<Value, String> {
        let loaders = serde_json::to_string(profile.accepted_loaders)
            .map_err(|_| "could not encode the loader filter".to_owned())?;
        let game_versions = serde_json::to_string(&[minecraft_version])
            .map_err(|_| "could not encode the Minecraft version filter".to_owned())?;
        self.fetch_json(
            &format!(
                "https://api.modrinth.com/v2/project/{project_id}/version?loaders={}&game_versions={}&include_changelog=false",
                percent_encode(&loaders),
                percent_encode(&game_versions)
            ),
            &["api.modrinth.com"],
        )
    }

    fn resolve_content_tree(
        &self,
        root_project_id: &str,
        root_version_id: Option<&str>,
        minecraft_version: &str,
        profile: &ContentProfile,
    ) -> Result<Vec<ResolvedContent>, String> {
        let mut pending = VecDeque::from([(
            Some(root_project_id.to_owned()),
            root_version_id.map(str::to_owned),
        )]);
        let mut resolved = Vec::new();
        let mut project_versions = HashMap::<String, String>::new();

        while let Some((project_hint, version_hint)) = pending.pop_front() {
            if resolved.len() >= MAX_DEPENDENCY_PROJECTS {
                return Err(format!(
                    "the content dependency tree exceeds the {MAX_DEPENDENCY_PROJECTS}-project safety limit"
                ));
            }
            let version = if let Some(version_id) = version_hint.as_deref() {
                validate_modrinth_id(version_id, "version")?;
                self.fetch_json(
                    &format!("https://api.modrinth.com/v2/version/{version_id}"),
                    &["api.modrinth.com"],
                )?
            } else {
                let project_id = project_hint.as_deref().ok_or_else(|| {
                    "a required dependency omitted its project identity".to_owned()
                })?;
                validate_modrinth_id(project_id, "project")?;
                let versions =
                    self.compatible_modrinth_versions(project_id, minecraft_version, profile)?;
                select_release_version(&versions)?.clone()
            };
            let project_id = required_text(&version, "project_id", 64)?;
            validate_modrinth_id(&project_id, "project")?;
            if let Some(project_hint) = project_hint.as_deref()
                && project_hint != project_id
            {
                return Err(
                    "a marketplace version did not belong to the requested project".to_owned(),
                );
            }
            let version_id = required_text(&version, "id", 64)?;
            validate_modrinth_id(&version_id, "version")?;
            if let Some(existing) = project_versions.get(&project_id) {
                if existing != &version_id {
                    return Err(format!(
                        "the dependency tree requested conflicting versions of project {project_id}"
                    ));
                }
                continue;
            }

            let project = self.fetch_modrinth_project(&project_id)?;
            validate_project_compatibility(&project, profile)?;
            validate_version_compatibility(&version, minecraft_version, profile)?;
            let file = select_primary_file(&version)?;
            let mut optional_dependencies = Vec::new();
            if let Some(dependencies) = version.get("dependencies").and_then(Value::as_array) {
                for dependency in dependencies.iter().take(128) {
                    let dependency_type = dependency
                        .get("dependency_type")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let dependency_project = dependency
                        .get("project_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let dependency_version = dependency
                        .get("version_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    match dependency_type {
                        "required" => pending.push_back((dependency_project, dependency_version)),
                        "optional" => optional_dependencies.push(
                            dependency_project
                                .or(dependency_version)
                                .unwrap_or_else(|| "unnamed optional dependency".to_owned()),
                        ),
                        "embedded" | "incompatible" => {}
                        _ => {
                            return Err(
                                "the marketplace returned an unknown dependency relationship"
                                    .to_owned(),
                            );
                        }
                    }
                }
                if dependencies.len() > 128 {
                    return Err("the content version has too many dependency records".to_owned());
                }
            }
            project_versions.insert(project_id.clone(), version_id.clone());
            resolved.push(ResolvedContent {
                project_id,
                project_slug: required_text(&project, "slug", 128)?,
                project_title: required_text(&project, "title", 256)?,
                version_id,
                version_number: required_text(&version, "version_number", 128)?,
                file,
                optional_dependencies,
            });
        }
        Ok(resolved)
    }

    fn marketplace_staging_path(&self) -> Result<PathBuf, String> {
        let root = self.instance_root.join(".helix-staging");
        if root.exists() {
            let metadata = fs::symlink_metadata(&root)
                .map_err(|_| "could not inspect the marketplace staging root".to_owned())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("the marketplace staging root is unsafe".to_owned());
            }
        } else {
            fs::create_dir(&root)
                .map_err(|_| "could not create the marketplace staging root".to_owned())?;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                .map_err(|_| "could not protect the marketplace staging root".to_owned())?;
        }
        let path = root.join(Uuid::new_v4().to_string());
        fs::create_dir(&path)
            .map_err(|_| "could not create the marketplace staging directory".to_owned())?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the marketplace staging directory".to_owned())?;
        Ok(path)
    }

    fn download_content_tree(
        &self,
        resolved: &[ResolvedContent],
        staging: &Path,
    ) -> Result<Vec<PathBuf>, String> {
        let mut staged = Vec::with_capacity(resolved.len());
        let mut total = 0_u64;
        for (index, content) in resolved.iter().enumerate() {
            total = total
                .checked_add(content.file.size)
                .ok_or_else(|| "the content download size overflowed".to_owned())?;
            if total > MAX_MARKETPLACE_TOTAL_BYTES {
                return Err("the content dependency download exceeds 1 GiB".to_owned());
            }
            require_https_host(&content.file.url, &["cdn.modrinth.com"])?;
            let path = staging.join(format!("{index:03}.jar"));
            self.curl_no_redirect(
                &content.file.url,
                &path,
                MAX_MARKETPLACE_FILE_BYTES,
                10 * 60,
            )?;
            let metadata =
                fs::metadata(&path).map_err(|_| "a marketplace download disappeared".to_owned())?;
            if metadata.len() != content.file.size
                || !(1_024..=MAX_MARKETPLACE_FILE_BYTES).contains(&metadata.len())
            {
                return Err("a marketplace download had an unexpected size".to_owned());
            }
            if !file_sha512(&path)?.eq_ignore_ascii_case(&content.file.sha512) {
                return Err(
                    "a marketplace download failed its Modrinth-declared SHA-512".to_owned(),
                );
            }
            staged.push(path);
        }
        Ok(staged)
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_content_tree(
        &self,
        manifest: &InstanceManifest,
        profile: &ContentProfile,
        resolved: &[ResolvedContent],
        staged_files: &[PathBuf],
        staging: &Path,
        mutation: &mut ContentMutation,
    ) -> Result<(), String> {
        if resolved.len() != staged_files.len() {
            return Err("the staged content plan was inconsistent".to_owned());
        }
        let data_path = self.instance_path(&manifest.id)?;
        let content_path = data_path.join(profile.directory);
        ensure_content_directory(&content_path, manifest.run_uid)?;
        let rollback_root = staging.join("rollback");
        fs::create_dir(&rollback_root)
            .map_err(|_| "could not create the content rollback directory".to_owned())?;
        let marketplace_root = self.state_root.join("marketplace");
        ensure_private_directory(&marketplace_root)?;
        let records_root = marketplace_root.join(&manifest.id);
        ensure_private_directory(&records_root)?;
        let mut destination_names = HashSet::new();

        for (index, (content, staged)) in resolved.iter().zip(staged_files).enumerate() {
            validate_content_filename(&content.file.filename)?;
            if !destination_names.insert(content.file.filename.clone()) {
                return Err(
                    "two content projects selected the same destination filename".to_owned(),
                );
            }
            let record_path = records_root.join(format!("{}.json", content.project_id));
            let previous_record =
                read_install_record(&record_path, &content.project_id, profile.directory)?;
            let previous_bytes = if record_path.exists() {
                Some(read_bounded_file(&record_path, MAX_INSTALL_RECORD_BYTES)?)
            } else {
                None
            };
            let previous_files = previous_record
                .as_ref()
                .map(|record| record.files.as_slice())
                .unwrap_or_default();
            let destination = content_path.join(&content.file.filename);
            if destination.exists() && !previous_files.contains(&content.file.filename) {
                return Err(format!(
                    "{} already exists and is not managed by Helix",
                    content.file.filename
                ));
            }
            for (old_index, filename) in previous_files.iter().enumerate() {
                validate_content_filename(filename)?;
                let old_path = content_path.join(filename);
                if old_path.exists() {
                    ensure_regular_file(&old_path)?;
                    let rollback = rollback_root.join(format!("{index:03}-{old_index:03}.jar"));
                    fs::rename(&old_path, &rollback)
                        .map_err(|_| "could not stage the previous managed content".to_owned())?;
                    mutation.previous_files.push((rollback, old_path));
                }
            }
            ensure_regular_file(staged)?;
            run_program(
                Path::new("/usr/bin/chown"),
                &[
                    format!("0:{}", manifest.run_uid),
                    staged.to_string_lossy().into_owned(),
                ],
                20,
            )?;
            fs::set_permissions(staged, fs::Permissions::from_mode(0o640))
                .map_err(|_| "could not protect the verified content file".to_owned())?;
            ensure_safe_content_directory(&content_path)?;
            fs::rename(staged, &destination)
                .map_err(|_| "could not commit the verified content file".to_owned())?;
            mutation.installed.push(destination);

            let record = InstallRecord {
                schema_version: 1,
                project_id: content.project_id.clone(),
                project_slug: content.project_slug.clone(),
                project_title: content.project_title.clone(),
                version_id: content.version_id.clone(),
                version_number: content.version_number.clone(),
                content_directory: profile.directory.to_owned(),
                files: vec![content.file.filename.clone()],
                installed_at_unix_ms: now_unix_ms(),
            };
            mutation.records.push(RecordBackup {
                path: record_path,
                previous: previous_bytes,
            });
            let record_path = &mutation
                .records
                .last()
                .ok_or_else(|| "could not track the content metadata rollback".to_owned())?
                .path;
            write_install_record(record_path, &record)?;
        }
        Ok(())
    }
}

fn content_profile(software: MinecraftSoftware) -> Result<ContentProfile, String> {
    match software {
        MinecraftSoftware::Paper => Ok(ContentProfile {
            kind: "plugin",
            search_project_type: "plugin",
            directory: "plugins",
            accepted_loaders: &["paper", "spigot", "bukkit"],
        }),
        MinecraftSoftware::Purpur => Ok(ContentProfile {
            kind: "plugin",
            search_project_type: "plugin",
            directory: "plugins",
            accepted_loaders: &["purpur", "paper", "spigot", "bukkit"],
        }),
        MinecraftSoftware::Folia => Ok(ContentProfile {
            kind: "plugin",
            search_project_type: "plugin",
            directory: "plugins",
            accepted_loaders: &["folia"],
        }),
        MinecraftSoftware::Fabric => Ok(ContentProfile {
            kind: "mod",
            search_project_type: "mod",
            directory: "mods",
            accepted_loaders: &["fabric"],
        }),
        MinecraftSoftware::Vanilla => Err(
            "Vanilla has no safe plugin or mod loader; choose Paper for plugins or Fabric for mods"
                .to_owned(),
        ),
        MinecraftSoftware::NeoForge => Err(
            "NeoForge marketplace installs will unlock with its native server runtime".to_owned(),
        ),
    }
}

fn validate_search(query: &str, offset: u32, limit: u8) -> Result<(), String> {
    let query = query.trim();
    if query.len() > MAX_SEARCH_QUERY_BYTES || query.chars().any(char::is_control) {
        return Err("marketplace search text is invalid".to_owned());
    }
    if offset > MAX_SEARCH_OFFSET {
        return Err("marketplace search offset is outside the allowed range".to_owned());
    }
    if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Err("marketplace search limit must be between 1 and 50".to_owned());
    }
    Ok(())
}

fn validate_modrinth_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!("marketplace {label} ID is invalid"));
    }
    Ok(())
}

fn validate_content_filename(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 180
        || !value.to_ascii_lowercase().ends_with(".jar")
        || value.starts_with('.')
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err("the marketplace returned an unsafe content filename".to_owned());
    }
    Ok(())
}

fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

fn sanitize_search_response(
    response: &Value,
    instance_id: &str,
    manifest: &InstanceManifest,
    profile: &ContentProfile,
) -> Result<Value, String> {
    let hits = response
        .get("hits")
        .and_then(Value::as_array)
        .ok_or_else(|| "the marketplace returned an invalid search result".to_owned())?;
    let hits = hits
        .iter()
        .take(usize::from(MAX_SEARCH_LIMIT))
        .map(|hit| {
            let project_id = required_text(hit, "project_id", 64)?;
            validate_modrinth_id(&project_id, "project")?;
            let slug = required_text(hit, "slug", 128)?;
            Ok(json!({
                "project_id": project_id,
                "slug": slug,
                "title": required_text(hit, "title", 256)?,
                "description": optional_text(hit, "description", 2_048),
                "author": optional_text(hit, "author", 128),
                "project_type": required_text(hit, "project_type", 32)?,
                "server_side": optional_text(hit, "server_side", 32),
                "downloads": hit.get("downloads").and_then(Value::as_u64).unwrap_or(0),
                "follows": hit.get("follows").and_then(Value::as_u64).unwrap_or(0),
                "latest_version": optional_text(hit, "latest_version", 128),
                "date_modified": optional_text(hit, "date_modified", 64),
                "web_url": format!("https://modrinth.com/{}/{slug}", profile.kind),
                "icon_url": modrinth_icon_proxy_url(hit.get("icon_url").and_then(Value::as_str)),
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({
        "schema_version": 1,
        "instance_id": instance_id,
        "compatibility": {
            "minecraft_version": manifest.minecraft_version,
            "server_software": software_name(manifest.software),
            "content_kind": profile.kind,
            "accepted_loaders": profile.accepted_loaders,
            "install_directory": profile.directory,
        },
        "total_hits": response.get("total_hits").and_then(Value::as_u64).unwrap_or(0),
        "offset": response.get("offset").and_then(Value::as_u64).unwrap_or(0),
        "limit": response.get("limit").and_then(Value::as_u64).unwrap_or(0),
        "hits": hits,
        "collected_at_unix_ms": now_unix_ms(),
    }))
}

fn validate_project_compatibility(project: &Value, profile: &ContentProfile) -> Result<(), String> {
    let project_type = required_text(project, "project_type", 32)?;
    if project_type != "mod" {
        return Err(format!(
            "this server accepts {} content, not {project_type} projects",
            profile.kind
        ));
    }
    let project_loaders = project
        .get("loaders")
        .and_then(Value::as_array)
        .ok_or_else(|| "this project did not declare any supported loaders".to_owned())?;
    if !project_loaders.iter().any(|value| {
        value.as_str().is_some_and(|loader| {
            profile
                .accepted_loaders
                .iter()
                .any(|accepted| loader.eq_ignore_ascii_case(accepted))
        })
    }) {
        return Err("this project does not support the server's software family".to_owned());
    }
    match required_text(project, "server_side", 32)?.as_str() {
        "required" | "optional" => Ok(()),
        "unsupported" => Err("this project does not run on a server".to_owned()),
        _ => Err("this project has not declared safe server compatibility".to_owned()),
    }
}

fn validate_version_compatibility(
    version: &Value,
    minecraft_version: &str,
    profile: &ContentProfile,
) -> Result<(), String> {
    let game_versions = version
        .get("game_versions")
        .and_then(Value::as_array)
        .ok_or_else(|| "the content version omitted Minecraft compatibility".to_owned())?;
    if !game_versions
        .iter()
        .any(|value| value.as_str() == Some(minecraft_version))
    {
        return Err(format!(
            "the selected content version does not support Minecraft {minecraft_version}"
        ));
    }
    let loaders = version
        .get("loaders")
        .and_then(Value::as_array)
        .ok_or_else(|| "the content version omitted loader compatibility".to_owned())?;
    if !loaders.iter().any(|value| {
        value.as_str().is_some_and(|loader| {
            profile
                .accepted_loaders
                .iter()
                .any(|accepted| loader.eq_ignore_ascii_case(accepted))
        })
    }) {
        return Err(
            "the selected content version does not support this server software".to_owned(),
        );
    }
    if let Some(environment) = version.get("environment").and_then(Value::as_str)
        && matches!(environment, "client_only" | "singleplayer_only")
    {
        return Err("the selected content version cannot run on a dedicated server".to_owned());
    }
    Ok(())
}

fn select_release_version(versions: &Value) -> Result<&Value, String> {
    let versions = versions
        .as_array()
        .ok_or_else(|| "the marketplace returned invalid version results".to_owned())?;
    versions
        .iter()
        .find(|version| version.get("version_type").and_then(Value::as_str) == Some("release"))
        .ok_or_else(|| {
            "this project has no release build for the server's exact loader and Minecraft version"
                .to_owned()
        })
}

fn select_primary_file(version: &Value) -> Result<ResolvedFile, String> {
    let files = version
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "the content version returned no files".to_owned())?;
    let file = files
        .iter()
        .find(|file| file.get("primary").and_then(Value::as_bool) == Some(true))
        .or_else(|| (files.len() == 1).then(|| &files[0]))
        .ok_or_else(|| "the content version has no unambiguous primary file".to_owned())?;
    let filename = required_text(file, "filename", 180)?;
    validate_content_filename(&filename)?;
    let url = required_text(file, "url", 4_096)?;
    require_https_host(&url, &["cdn.modrinth.com"])?;
    let size = file
        .get("size")
        .and_then(Value::as_u64)
        .filter(|size| (1_024..=MAX_MARKETPLACE_FILE_BYTES).contains(size))
        .ok_or_else(|| "the content file size is outside the safety limit".to_owned())?;
    let sha512 = file
        .pointer("/hashes/sha512")
        .and_then(Value::as_str)
        .filter(|hash| valid_hex(hash, 128))
        .ok_or_else(|| "the content file omitted a valid SHA-512 checksum".to_owned())?
        .to_owned();
    Ok(ResolvedFile {
        filename,
        url,
        size,
        sha512,
    })
}

fn sanitize_version_summary(version: &Value) -> Result<Value, String> {
    let id = required_text(version, "id", 64)?;
    validate_modrinth_id(&id, "version")?;
    Ok(json!({
        "id": id,
        "name": required_text(version, "name", 256)?,
        "version_number": required_text(version, "version_number", 128)?,
        "version_type": required_text(version, "version_type", 32)?,
        "date_published": optional_text(version, "date_published", 64),
        "downloads": version.get("downloads").and_then(Value::as_u64).unwrap_or(0),
        "game_versions": bounded_string_array(version.get("game_versions"), 64, 64),
        "loaders": bounded_string_array(version.get("loaders"), 64, 64),
        "has_primary_file": version.get("files").and_then(Value::as_array).is_some_and(|files| files.iter().any(|file| file.get("primary").and_then(Value::as_bool) == Some(true)) || files.len() == 1),
    }))
}

fn required_text(value: &Value, field: &str, maximum: usize) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= maximum)
        .map(|text| clean_text(text, maximum))
        .ok_or_else(|| format!("the marketplace field {field} was missing or invalid"))
}

fn optional_text(value: &Value, field: &str, maximum: usize) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|text| clean_text(text, maximum))
}

fn optional_text_chars(value: &Value, field: &str, maximum: usize) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|text| clean_text_chars(text, maximum))
}

fn clean_text(value: &str, maximum_bytes: usize) -> String {
    let boundary = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= maximum_bytes)
        .last()
        .unwrap_or(0);
    let end = if value.len() <= maximum_bytes {
        value.len()
    } else {
        boundary
    };
    value[..end]
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .collect()
}

fn clean_text_chars(value: &str, maximum_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .take(maximum_chars)
        .collect()
}

fn safe_https_url(value: Option<&str>) -> Option<String> {
    value
        .filter(|url| url.starts_with("https://") && url.len() <= 4_096)
        .filter(|url| !url.chars().any(char::is_control))
        .map(str::to_owned)
}

pub(super) fn modrinth_icon_proxy_url(value: Option<&str>) -> Option<String> {
    let url = value?;
    require_https_host(url, &["cdn.modrinth.com"]).ok()?;
    let path = url.strip_prefix("https://cdn.modrinth.com")?;
    if path.len() > 512
        || !path.starts_with("/data/")
        || path.contains(['?', '#', '\\', '%'])
        || path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
        || ![".png", ".jpg", ".jpeg", ".webp", ".gif"]
            .iter()
            .any(|extension| path.to_ascii_lowercase().ends_with(extension))
    {
        return None;
    }
    Some(format!(
        "/api/v1/marketplace/modrinth/image?path={}",
        percent_encode(path)
    ))
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

fn file_sha512(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|_| "could not open the downloaded marketplace content".to_owned())?;
    let mut digest = Sha512::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "could not hash the downloaded marketplace content".to_owned())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let mut output = String::with_capacity(128);
    for byte in digest.finalize() {
        let _ = write!(output, "{byte:02x}");
    }
    Ok(output)
}

fn ensure_content_directory(path: &Path, run_uid: u32) -> Result<(), String> {
    if path.exists() {
        ensure_safe_content_directory(path)?;
        return Ok(());
    }
    fs::create_dir(path).map_err(|_| "could not create the server content directory".to_owned())?;
    run_program(
        Path::new("/usr/bin/chown"),
        &[
            format!("{run_uid}:{run_uid}"),
            path.to_string_lossy().into_owned(),
        ],
        20,
    )?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o750))
        .map_err(|_| "could not protect the server content directory".to_owned())
}

fn ensure_safe_content_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "could not inspect the server content directory".to_owned())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("the server content directory is unsafe".to_owned());
    }
    Ok(())
}

fn ensure_regular_file(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "could not inspect a content file".to_owned())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("a content file is not a safe regular file".to_owned());
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    if !path.exists() {
        fs::create_dir(path)
            .map_err(|_| "could not create the content metadata directory".to_owned())?;
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "could not inspect the content metadata directory".to_owned())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("the content metadata directory is unsafe".to_owned());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| "could not protect the content metadata directory".to_owned())
}

fn read_install_record(
    path: &Path,
    expected_project_id: &str,
    expected_content_directory: &str,
) -> Result<Option<InstallRecord>, String> {
    if !path.exists() {
        return Ok(None);
    }
    ensure_regular_file(path)?;
    let body = read_bounded_file(path, MAX_INSTALL_RECORD_BYTES)?;
    let record: InstallRecord = serde_json::from_slice(&body)
        .map_err(|_| "an existing content install record is invalid".to_owned())?;
    validate_install_record(&record, expected_project_id, expected_content_directory)?;
    Ok(Some(record))
}

fn validate_install_record(
    record: &InstallRecord,
    expected_project_id: &str,
    expected_content_directory: &str,
) -> Result<(), String> {
    validate_modrinth_id(expected_project_id, "project")?;
    validate_modrinth_id(&record.project_id, "project")?;
    validate_modrinth_id(&record.version_id, "version")?;
    if record.schema_version != 1
        || record.project_id != expected_project_id
        || record.content_directory != expected_content_directory
        || record.files.is_empty()
        || record.files.len() > 64
        || record.project_slug.is_empty()
        || record.project_slug.len() > 128
        || record.project_slug.chars().any(char::is_control)
        || record.project_title.is_empty()
        || record.project_title.len() > 256
        || record.project_title.chars().any(char::is_control)
        || record.version_number.is_empty()
        || record.version_number.len() > 128
        || record.version_number.chars().any(char::is_control)
    {
        return Err("an existing content install record is unsupported".to_owned());
    }
    let mut filenames = HashSet::with_capacity(record.files.len());
    for filename in &record.files {
        validate_content_filename(filename)?;
        if !filenames.insert(filename) {
            return Err("an existing content install record repeats a content file".to_owned());
        }
    }
    Ok(())
}

fn read_bounded_file(path: &Path, maximum: u64) -> Result<Vec<u8>, String> {
    let metadata =
        fs::metadata(path).map_err(|_| "could not inspect a metadata file".to_owned())?;
    if metadata.len() == 0 || metadata.len() > maximum {
        return Err("a content metadata file is outside the safety limit".to_owned());
    }
    fs::read(path).map_err(|_| "could not read a content metadata file".to_owned())
}

fn write_install_record(path: &Path, record: &InstallRecord) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(record)
        .map_err(|_| "could not encode the content install record".to_owned())?;
    if body.len() > usize::try_from(MAX_INSTALL_RECORD_BYTES).unwrap_or(usize::MAX) {
        return Err("the content install record is too large".to_owned());
    }
    write_atomic_private(path, &body)
}

fn write_atomic_private(path: &Path, body: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "the content metadata path has no parent".to_owned())?;
    let temporary = parent.join(format!(".{}.partial", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "could not stage the content metadata".to_owned())?;
        file.write_all(body)
            .and_then(|()| file.sync_all())
            .map_err(|_| "could not persist the content metadata".to_owned())?;
        fs::rename(&temporary, path)
            .map_err(|_| "could not commit the content metadata".to_owned())?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "could not sync the content metadata directory".to_owned())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn rollback_content(mutation: &mut ContentMutation) -> Result<(), String> {
    let mut errors = Vec::new();
    for installed in mutation.installed.iter().rev() {
        if installed.exists() && fs::remove_file(installed).is_err() {
            errors.push(format!("could not remove {}", installed.display()));
        }
    }
    for (rollback, original) in mutation.previous_files.iter().rev() {
        if rollback.exists() && fs::rename(rollback, original).is_err() {
            errors.push(format!("could not restore {}", original.display()));
        }
    }
    for record in mutation.records.iter().rev() {
        let result = match &record.previous {
            Some(body) => write_atomic_private(&record.path, body),
            None => {
                if record.path.exists() {
                    fs::remove_file(&record.path)
                        .map_err(|_| "could not remove a new content record".to_owned())
                } else {
                    Ok(())
                }
            }
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

fn combine_install_failure(
    install: String,
    rollback: Result<(), String>,
    restart: Result<(), String>,
) -> String {
    let mut message = install;
    if let Err(error) = rollback {
        message.push_str("; rollback also failed: ");
        message.push_str(&error);
    }
    if let Err(error) = restart {
        message.push_str("; the previous server also failed to restart: ");
        message.push_str(&error);
    }
    message
}

fn remove_directory_if_present(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "could not inspect the marketplace staging directory".to_owned())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("the marketplace staging directory is unsafe".to_owned());
    }
    fs::remove_dir_all(path)
        .map_err(|_| "could not clean the marketplace staging directory".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_and_modrinth_identifiers_are_strict() {
        assert!(validate_search("fast async world edit", 0, 20).is_ok());
        assert!(validate_search(&"x".repeat(121), 0, 20).is_err());
        assert!(validate_search("ok", 10_001, 20).is_err());
        assert!(validate_search("ok", 0, 0).is_err());
        assert!(validate_modrinth_id("1bokaNcj", "project").is_ok());
        assert!(validate_modrinth_id("../../etc", "project").is_err());
        assert_eq!(percent_encode("a b&c"), "a%20b%26c");
    }

    #[test]
    fn icon_urls_are_reduced_to_a_same_origin_proxy_path() {
        assert_eq!(
            modrinth_icon_proxy_url(Some("https://cdn.modrinth.com/data/abc/icon.png")),
            Some("/api/v1/marketplace/modrinth/image?path=%2Fdata%2Fabc%2Ficon.png".to_owned())
        );
        assert!(
            modrinth_icon_proxy_url(Some("https://cdn.modrinth.com.evil.test/data/abc/icon.png"))
                .is_none()
        );
        assert!(
            modrinth_icon_proxy_url(Some("https://cdn.modrinth.com/data/../icon.png")).is_none()
        );
        assert!(
            modrinth_icon_proxy_url(Some("https://cdn.modrinth.com/data/abc/icon.svg")).is_none()
        );
    }

    #[test]
    fn profiles_never_mix_mods_plugins_or_folia_fallbacks() {
        let paper = content_profile(MinecraftSoftware::Paper).unwrap();
        assert_eq!(paper.kind, "plugin");
        assert!(paper.accepted_loaders.contains(&"spigot"));
        let folia = content_profile(MinecraftSoftware::Folia).unwrap();
        assert_eq!(folia.accepted_loaders, ["folia"]);
        let fabric = content_profile(MinecraftSoftware::Fabric).unwrap();
        assert_eq!(fabric.kind, "mod");
        assert_eq!(fabric.directory, "mods");
        assert!(content_profile(MinecraftSoftware::Vanilla).is_err());
    }

    #[test]
    fn modrinth_plugin_projects_use_loader_metadata_as_the_boundary() {
        let project = json!({
            "project_type": "mod",
            "loaders": ["bukkit", "paper", "purpur", "spigot"],
            "server_side": "required"
        });
        let paper = content_profile(MinecraftSoftware::Paper).unwrap();
        assert!(validate_project_compatibility(&project, &paper).is_ok());
        let fabric = content_profile(MinecraftSoftware::Fabric).unwrap();
        assert!(validate_project_compatibility(&project, &fabric).is_err());
        let client_only = json!({
            "project_type": "mod",
            "loaders": ["fabric"],
            "server_side": "unsupported"
        });
        assert!(validate_project_compatibility(&client_only, &fabric).is_err());
    }

    #[test]
    fn version_compatibility_requires_exact_game_and_loader() {
        let profile = content_profile(MinecraftSoftware::Fabric).unwrap();
        let compatible = json!({
            "game_versions": ["1.21.11"],
            "loaders": ["fabric"]
        });
        assert!(validate_version_compatibility(&compatible, "1.21.11", &profile).is_ok());
        assert!(validate_version_compatibility(&compatible, "1.21.10", &profile).is_err());
        let wrong_loader = json!({
            "game_versions": ["1.21.11"],
            "loaders": ["neoforge"]
        });
        assert!(validate_version_compatibility(&wrong_loader, "1.21.11", &profile).is_err());
        let client_only = json!({
            "game_versions": ["1.21.11"],
            "loaders": ["fabric"],
            "environment": "client_only"
        });
        assert!(validate_version_compatibility(&client_only, "1.21.11", &profile).is_err());
    }

    #[test]
    fn primary_file_requires_safe_jar_sha512_and_exact_cdn() {
        let good = json!({
            "files": [{
                "primary": true,
                "filename": "example-1.0.jar",
                "url": "https://cdn.modrinth.com/data/abc/versions/def/example.jar",
                "size": 2048,
                "hashes": {"sha512": "a".repeat(128)}
            }]
        });
        assert_eq!(
            select_primary_file(&good).unwrap().filename,
            "example-1.0.jar"
        );
        let mut wrong_host = good.clone();
        wrong_host["files"][0]["url"] = json!("https://cdn.modrinth.com.evil.test/a.jar");
        assert!(select_primary_file(&wrong_host).is_err());
        let mut traversal = good;
        traversal["files"][0]["filename"] = json!("../plugin.jar");
        assert!(select_primary_file(&traversal).is_err());
    }

    #[test]
    fn release_selection_does_not_silently_choose_beta() {
        let versions = json!([
            {"id": "beta", "version_type": "beta"},
            {"id": "stable", "version_type": "release"}
        ]);
        assert_eq!(select_release_version(&versions).unwrap()["id"], "stable");
        assert!(select_release_version(&json!([{"version_type": "beta"}])).is_err());
    }

    #[test]
    fn install_records_reject_unknown_fields_and_unsafe_files() {
        let valid = json!({
            "schema_version": 1,
            "project_id": "abc",
            "project_slug": "example",
            "project_title": "Example",
            "version_id": "def",
            "version_number": "1.0",
            "content_directory": "plugins",
            "files": ["example.jar"],
            "installed_at_unix_ms": 1
        });
        let record: InstallRecord = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(record.files, ["example.jar"]);
        validate_install_record(&record, "abc", "plugins").unwrap();
        let mut unknown = valid;
        unknown["path"] = json!("/etc/passwd");
        assert!(serde_json::from_value::<InstallRecord>(unknown).is_err());
        assert!(validate_content_filename("../example.jar").is_err());
        assert!(validate_content_filename("example.zip").is_err());

        let mut wrong_project = serde_json::to_value(&record).unwrap();
        wrong_project["project_id"] = json!("other-project");
        let wrong_project: InstallRecord = serde_json::from_value(wrong_project).unwrap();
        assert!(validate_install_record(&wrong_project, "abc", "plugins").is_err());

        let mut wrong_directory = serde_json::to_value(&record).unwrap();
        wrong_directory["content_directory"] = json!("mods");
        let wrong_directory: InstallRecord = serde_json::from_value(wrong_directory).unwrap();
        assert!(validate_install_record(&wrong_directory, "abc", "plugins").is_err());

        let mut repeated_file = serde_json::to_value(&record).unwrap();
        repeated_file["files"] = json!(["example.jar", "example.jar"]);
        let repeated_file: InstallRecord = serde_json::from_value(repeated_file).unwrap();
        assert!(validate_install_record(&repeated_file, "abc", "plugins").is_err());
    }
}
