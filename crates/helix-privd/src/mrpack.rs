//! Strict, platform-neutral validation and extraction for Modrinth modpacks.
//!
//! Network resolution stays in the privileged Linux broker. This module only
//! accepts an archive verified against Modrinth-declared integrity metadata and
//! turns it into a bounded plan that
//! can be assembled inside a fresh staging directory.

use serde::Deserialize;
use sha1::Sha1;
use sha2::{Digest as _, Sha512};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use unicode_normalization::UnicodeNormalization as _;
use zip::{CompressionMethod, ZipArchive};

const INDEX_NAME: &str = "modrinth.index.json";
const FORMAT_VERSION: u32 = 1;
const GAME_ID: &str = "minecraft";
const TRUSTED_CDN_HOST: &str = "cdn.modrinth.com";

#[derive(Clone, Copy, Debug)]
pub struct MrpackLimits {
    pub maximum_archive_bytes: u64,
    pub maximum_index_bytes: u64,
    pub maximum_archive_entries: usize,
    pub maximum_files: usize,
    pub maximum_file_bytes: u64,
    pub maximum_download_bytes: u64,
    pub maximum_override_bytes: u64,
    pub maximum_unpacked_bytes: u64,
    pub maximum_path_bytes: usize,
    pub maximum_path_segment_bytes: usize,
    pub maximum_path_depth: usize,
    pub maximum_compression_ratio: u64,
}

impl Default for MrpackLimits {
    fn default() -> Self {
        Self {
            maximum_archive_bytes: 256 * 1024 * 1024,
            maximum_index_bytes: 4 * 1024 * 1024,
            maximum_archive_entries: 8_192,
            maximum_files: 4_096,
            maximum_file_bytes: 512 * 1024 * 1024,
            maximum_download_bytes: 8 * 1024 * 1024 * 1024,
            maximum_override_bytes: 2 * 1024 * 1024 * 1024,
            maximum_unpacked_bytes: 10 * 1024 * 1024 * 1024,
            maximum_path_bytes: 512,
            maximum_path_segment_bytes: 128,
            maximum_path_depth: 24,
            maximum_compression_ratio: 250,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MrpackDownload {
    pub path: String,
    pub url: String,
    pub size: u64,
    pub sha1: String,
    pub sha512: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MrpackPlan {
    pub name: String,
    pub version_id: String,
    pub summary: Option<String>,
    pub minecraft_version: String,
    pub fabric_loader_version: String,
    pub files: Vec<MrpackDownload>,
    pub skipped_optional_files: usize,
    pub skipped_client_only_files: usize,
    pub declared_download_bytes: u64,
    pub override_bytes: u64,
    archive_bytes: u64,
    override_entries: Vec<OverrideEntry>,
}

impl MrpackPlan {
    #[must_use]
    pub fn required_staging_bytes(&self) -> u64 {
        self.archive_bytes
            .saturating_add(self.declared_download_bytes)
            .saturating_add(self.override_bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverrideLayer {
    Common,
    Server,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OverrideEntry {
    archive_index: usize,
    path: String,
    size: u64,
    layer: OverrideLayer,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    #[serde(rename = "formatVersion")]
    format_version: u32,
    game: String,
    #[serde(rename = "versionId")]
    version_id: String,
    name: String,
    summary: Option<String>,
    files: Vec<IndexFile>,
    dependencies: Dependencies,
}

#[derive(Debug, Deserialize)]
struct Dependencies {
    minecraft: Option<String>,
    #[serde(rename = "fabric-loader")]
    fabric_loader: Option<String>,
    forge: Option<String>,
    neoforge: Option<String>,
    #[serde(rename = "quilt-loader")]
    quilt_loader: Option<String>,
    #[serde(flatten)]
    unsupported: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexFile {
    path: String,
    hashes: IndexHashes,
    #[serde(default)]
    env: Environment,
    downloads: Vec<String>,
    #[serde(rename = "fileSize")]
    file_size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexHashes {
    sha1: String,
    sha512: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    client: EnvironmentSupport,
    server: EnvironmentSupport,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            client: EnvironmentSupport::Required,
            server: EnvironmentSupport::Required,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum EnvironmentSupport {
    Required,
    Optional,
    Unsupported,
}

pub fn inspect_mrpack(
    archive_path: &Path,
    limits: &MrpackLimits,
    deadline: Instant,
) -> Result<MrpackPlan, String> {
    ensure_deadline(deadline)?;
    let metadata = fs::symlink_metadata(archive_path)
        .map_err(|_| "could not inspect the downloaded modpack".to_owned())?;
    if !metadata.file_type().is_file() {
        return Err("the downloaded modpack is not a regular file".to_owned());
    }
    let archive_bytes = metadata.len();
    if archive_bytes == 0 || archive_bytes > limits.maximum_archive_bytes {
        return Err(format!(
            "the modpack archive exceeds the {}-byte safety limit",
            limits.maximum_archive_bytes
        ));
    }

    let file =
        File::open(archive_path).map_err(|_| "could not open the downloaded modpack".to_owned())?;
    let mut archive = ZipArchive::new(file)
        .map_err(|_| "the downloaded modpack is not a valid ZIP archive".to_owned())?;
    if archive.is_empty() || archive.len() > limits.maximum_archive_entries {
        return Err(format!(
            "the modpack archive exceeds the {}-entry safety limit",
            limits.maximum_archive_entries
        ));
    }

    let mut index_bytes = None;
    let mut override_entries = Vec::new();
    let mut override_bytes = 0_u64;
    let mut total_unpacked_bytes = 0_u64;
    let mut common_paths = HashMap::new();
    let mut server_paths = HashSet::new();

    for archive_index in 0..archive.len() {
        ensure_deadline(deadline)?;
        let mut entry = archive
            .by_index(archive_index)
            .map_err(|_| "could not read a modpack archive entry".to_owned())?;
        validate_zip_entry_kind(&entry)?;
        let raw_name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| "the modpack contains a non-UTF-8 archive path".to_owned())?;
        if raw_name != entry.name() {
            return Err("the modpack contains an ambiguous archive path encoding".to_owned());
        }
        let is_directory = entry.is_dir();
        let normalized_name = raw_name.trim_end_matches('/');
        if normalized_name.is_empty() {
            return Err("the modpack contains an empty archive path".to_owned());
        }
        validate_relative_path(normalized_name, limits)?;
        if is_directory && raw_name != format!("{normalized_name}/") {
            return Err("the modpack contains an ambiguous directory path".to_owned());
        }
        validate_compression_bounds(&entry, limits)?;
        total_unpacked_bytes = total_unpacked_bytes
            .checked_add(entry.size())
            .ok_or_else(|| "the modpack unpacked size overflowed".to_owned())?;
        if total_unpacked_bytes > limits.maximum_unpacked_bytes {
            return Err(format!(
                "the modpack exceeds the {}-byte unpacked safety limit",
                limits.maximum_unpacked_bytes
            ));
        }

        if normalized_name == INDEX_NAME {
            if is_directory || index_bytes.is_some() {
                return Err("the modpack must contain one root index file".to_owned());
            }
            if entry.size() > limits.maximum_index_bytes {
                return Err(format!(
                    "the modpack index exceeds the {}-byte safety limit",
                    limits.maximum_index_bytes
                ));
            }
            let bytes = read_bounded(&mut entry, limits.maximum_index_bytes, "modpack index")?;
            index_bytes = Some(bytes);
            continue;
        }

        let (layer, relative) = if let Some(path) = normalized_name.strip_prefix("overrides/") {
            (OverrideLayer::Common, path)
        } else if let Some(path) = normalized_name.strip_prefix("server-overrides/") {
            (OverrideLayer::Server, path)
        } else if normalized_name == "overrides"
            || normalized_name == "server-overrides"
            || normalized_name == "client-overrides"
            || normalized_name.starts_with("client-overrides/")
        {
            continue;
        } else {
            return Err(format!(
                "the modpack contains unsupported root archive content: {normalized_name}"
            ));
        };

        if is_directory {
            continue;
        }
        validate_relative_path(relative, limits)?;
        reject_helix_owned_path(relative)?;
        if entry.size() > limits.maximum_file_bytes {
            return Err(format!(
                "the override {relative} exceeds the {}-byte per-file safety limit",
                limits.maximum_file_bytes
            ));
        }
        override_bytes = override_bytes
            .checked_add(entry.size())
            .ok_or_else(|| "the modpack override size overflowed".to_owned())?;
        if override_bytes > limits.maximum_override_bytes {
            return Err(format!(
                "the modpack overrides exceed the {}-byte safety limit",
                limits.maximum_override_bytes
            ));
        }
        let key = collision_key(relative);
        let inserted = match layer {
            OverrideLayer::Common => common_paths.insert(key, relative.to_owned()).is_none(),
            OverrideLayer::Server => {
                if common_paths
                    .get(&key)
                    .is_some_and(|common| common != relative)
                {
                    return Err(format!(
                        "the server override changes only the case or normalization of a common override: {relative}"
                    ));
                }
                server_paths.insert(key)
            }
        };
        if !inserted {
            return Err(format!(
                "the modpack contains a duplicate override path: {relative}"
            ));
        }
        override_entries.push(OverrideEntry {
            archive_index,
            path: relative.to_owned(),
            size: entry.size(),
            layer,
        });
    }

    let index_bytes = index_bytes
        .ok_or_else(|| "the modpack is missing the root modrinth.index.json file".to_owned())?;
    let index: Index = serde_json::from_slice(&index_bytes)
        .map_err(|error| format!("the modpack index is invalid: {error}"))?;
    validate_index(
        index,
        override_entries,
        override_bytes,
        archive_bytes,
        limits,
    )
}

fn validate_index(
    index: Index,
    override_entries: Vec<OverrideEntry>,
    override_bytes: u64,
    archive_bytes: u64,
    limits: &MrpackLimits,
) -> Result<MrpackPlan, String> {
    if index.format_version != FORMAT_VERSION {
        return Err(format!(
            "unsupported Modrinth modpack format version {}; Helix supports version {FORMAT_VERSION}",
            index.format_version
        ));
    }
    if index.game != GAME_ID {
        return Err("the selected Modrinth pack is not a Minecraft modpack".to_owned());
    }
    validate_label(&index.name, "modpack name", 256)?;
    validate_label(&index.version_id, "modpack version", 128)?;
    if let Some(summary) = index.summary.as_deref() {
        validate_label(summary, "modpack summary", 2_048)?;
    }
    let minecraft_version =
        required_dependency(index.dependencies.minecraft.as_deref(), "Minecraft version")?;
    if index.dependencies.forge.is_some() {
        return Err("Forge modpacks are preview-only because Helix does not yet have a lifecycle-ready Forge server loader".to_owned());
    }
    if index.dependencies.neoforge.is_some() {
        return Err("NeoForge modpacks are preview-only because Helix does not yet have a lifecycle-ready NeoForge server loader".to_owned());
    }
    if index.dependencies.quilt_loader.is_some() {
        return Err("Quilt modpacks are preview-only because Helix does not yet have a lifecycle-ready Quilt server loader".to_owned());
    }
    if let Some(dependency) = index.dependencies.unsupported.keys().next() {
        return Err(format!(
            "the modpack loader dependency {dependency} is not supported; Helix modpack creation is currently Fabric-only"
        ));
    }
    let fabric_loader_version = required_dependency(
        index.dependencies.fabric_loader.as_deref(),
        "Fabric Loader version",
    )?;
    if index.files.len() > limits.maximum_files {
        return Err(format!(
            "the modpack index exceeds the {}-file safety limit",
            limits.maximum_files
        ));
    }

    let mut files = Vec::new();
    let mut skipped_optional_files = 0;
    let mut skipped_client_only_files = 0;
    let mut declared_download_bytes = 0_u64;
    let mut output_paths = HashMap::<String, String>::new();
    for entry in &override_entries {
        output_paths.insert(collision_key(&entry.path), entry.path.clone());
    }

    for file in index.files {
        validate_relative_path(&file.path, limits)?;
        reject_helix_owned_path(&file.path)?;
        if file.file_size == 0 || file.file_size > limits.maximum_file_bytes {
            return Err(format!(
                "{} exceeds the {}-byte per-file safety limit",
                file.path, limits.maximum_file_bytes
            ));
        }
        validate_hex(&file.hashes.sha1, 40, "SHA-1")?;
        validate_hex(&file.hashes.sha512, 128, "SHA-512")?;
        let url = select_trusted_download(&file.downloads)?;
        if file.env.server == EnvironmentSupport::Unsupported {
            if file.env.client == EnvironmentSupport::Unsupported {
                return Err(format!(
                    "{} is unsupported on both client and server",
                    file.path
                ));
            }
            skipped_client_only_files += 1;
            continue;
        }
        if file.env.server == EnvironmentSupport::Optional {
            skipped_optional_files += 1;
            continue;
        }
        let key = collision_key(&file.path);
        if let Some(existing) = output_paths.insert(key, file.path.clone()) {
            return Err(format!(
                "the modpack maps multiple files to the same server path: {existing} and {}",
                file.path
            ));
        }
        declared_download_bytes = declared_download_bytes
            .checked_add(file.file_size)
            .ok_or_else(|| "the modpack download size overflowed".to_owned())?;
        if declared_download_bytes > limits.maximum_download_bytes {
            return Err(format!(
                "the modpack exceeds the {}-byte download safety limit",
                limits.maximum_download_bytes
            ));
        }
        files.push(MrpackDownload {
            path: file.path,
            url,
            size: file.file_size,
            sha1: file.hashes.sha1.to_ascii_lowercase(),
            sha512: file.hashes.sha512.to_ascii_lowercase(),
        });
    }

    Ok(MrpackPlan {
        name: index.name,
        version_id: index.version_id,
        summary: index.summary,
        minecraft_version: minecraft_version.to_owned(),
        fabric_loader_version: fabric_loader_version.to_owned(),
        files,
        skipped_optional_files,
        skipped_client_only_files,
        declared_download_bytes,
        override_bytes,
        archive_bytes,
        override_entries,
    })
}

pub fn extract_overrides(
    archive_path: &Path,
    destination: &Path,
    plan: &MrpackPlan,
    limits: &MrpackLimits,
    deadline: Instant,
) -> Result<(), String> {
    ensure_deadline(deadline)?;
    let destination_metadata = fs::symlink_metadata(destination)
        .map_err(|_| "the modpack staging directory does not exist".to_owned())?;
    if !destination_metadata.file_type().is_dir() || destination_metadata.file_type().is_symlink() {
        return Err("the modpack staging destination is not a real directory".to_owned());
    }
    let file = File::open(archive_path)
        .map_err(|_| "could not reopen the downloaded modpack".to_owned())?;
    let mut archive = ZipArchive::new(file)
        .map_err(|_| "the downloaded modpack could not be reopened".to_owned())?;
    let mut written = HashSet::new();
    for layer in [OverrideLayer::Common, OverrideLayer::Server] {
        for planned in plan
            .override_entries
            .iter()
            .filter(|entry| entry.layer == layer)
        {
            ensure_deadline(deadline)?;
            let mut entry = archive
                .by_index(planned.archive_index)
                .map_err(|_| "the modpack changed after validation".to_owned())?;
            validate_zip_entry_kind(&entry)?;
            if entry.size() != planned.size {
                return Err("the modpack changed after validation".to_owned());
            }
            let raw_name = std::str::from_utf8(entry.name_raw())
                .map_err(|_| "the modpack changed after validation".to_owned())?;
            let prefix = match layer {
                OverrideLayer::Common => "overrides/",
                OverrideLayer::Server => "server-overrides/",
            };
            if raw_name.strip_prefix(prefix) != Some(planned.path.as_str()) {
                return Err("the modpack changed after validation".to_owned());
            }
            validate_relative_path(&planned.path, limits)?;
            reject_helix_owned_path(&planned.path)?;
            let output = destination.join(Path::new(&planned.path));
            let parent = output
                .parent()
                .ok_or_else(|| "the modpack produced an invalid output path".to_owned())?;
            create_safe_directories(destination, parent)?;
            let key = collision_key(&planned.path);
            let may_replace = layer == OverrideLayer::Server && written.contains(&key);
            let mut options = OpenOptions::new();
            options.write(true);
            if may_replace {
                let metadata = fs::symlink_metadata(&output).map_err(|_| {
                    "a server override could not inspect the common override".to_owned()
                })?;
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                    return Err("a server override targeted a non-regular file".to_owned());
                }
                options.truncate(true);
            } else {
                options.create_new(true);
            }
            let mut output_file = options
                .open(&output)
                .map_err(|_| "could not create a staged modpack override".to_owned())?;
            copy_bounded(
                &mut entry,
                &mut output_file,
                planned.size,
                deadline,
                "modpack override",
            )?;
            output_file
                .sync_all()
                .map_err(|_| "could not persist a staged modpack override".to_owned())?;
            written.insert(key);
        }
    }
    Ok(())
}

pub fn verify_download(path: &Path, expected: &MrpackDownload) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| format!("{} was not downloaded", expected.path))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "{} is not a regular downloaded file",
            expected.path
        ));
    }
    if metadata.len() != expected.size {
        return Err(format!(
            "{} did not match its declared file size",
            expected.path
        ));
    }
    let mut file =
        File::open(path).map_err(|_| format!("{} could not be verified", expected.path))?;
    let mut sha1 = Sha1::new();
    let mut sha512 = Sha512::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| format!("{} could not be verified", expected.path))?;
        if read == 0 {
            break;
        }
        sha1.update(&buffer[..read]);
        sha512.update(&buffer[..read]);
    }
    let actual_sha1 = hex_digest(sha1.finalize());
    let actual_sha512 = hex_digest(sha512.finalize());
    if actual_sha1 != expected.sha1 || actual_sha512 != expected.sha512 {
        return Err(format!(
            "{} failed its Modrinth-declared checksum",
            expected.path
        ));
    }
    Ok(())
}

pub fn prepare_download_path(
    destination: &Path,
    relative_path: &str,
    limits: &MrpackLimits,
) -> Result<PathBuf, String> {
    validate_relative_path(relative_path, limits)?;
    reject_helix_owned_path(relative_path)?;
    let metadata = fs::symlink_metadata(destination)
        .map_err(|_| "the modpack staging directory does not exist".to_owned())?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("the modpack staging destination is not a real directory".to_owned());
    }
    let output = destination.join(Path::new(relative_path));
    let parent = output
        .parent()
        .ok_or_else(|| "the modpack produced an invalid output path".to_owned())?;
    create_safe_directories(destination, parent)?;
    match fs::symlink_metadata(&output) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(output),
        Ok(_) => Err(format!(
            "the modpack output already exists in staging: {relative_path}"
        )),
        Err(_) => Err("could not inspect a staged modpack output".to_owned()),
    }
}

pub fn validate_relative_path(path: &str, limits: &MrpackLimits) -> Result<(), String> {
    if path.is_empty()
        || path.len() > limits.maximum_path_bytes
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains('\0')
        || path.chars().any(char::is_control)
    {
        return Err(format!("the modpack contains an unsafe path: {path}"));
    }
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() > limits.maximum_path_depth {
        return Err(format!("the modpack path is too deep: {path}"));
    }
    for segment in segments {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.len() > limits.maximum_path_segment_bytes
            || segment.contains(':')
            || segment.ends_with(['.', ' '])
        {
            return Err(format!("the modpack contains an unsafe path: {path}"));
        }
        let device_stem = segment.split('.').next().unwrap_or_default();
        if is_windows_device_name(device_stem) {
            return Err(format!(
                "the modpack path uses a reserved device name: {path}"
            ));
        }
    }
    Ok(())
}

pub fn verify_sha512(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    validate_hex(expected, 128, "SHA-512")?;
    let mut file = File::open(path).map_err(|_| format!("could not open {label}"))?;
    let mut sha512 = Sha512::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| format!("could not verify {label}"))?;
        if read == 0 {
            break;
        }
        sha512.update(&buffer[..read]);
    }
    if hex_digest(sha512.finalize()) != expected.to_ascii_lowercase() {
        return Err(format!("{label} failed its Modrinth-declared checksum"));
    }
    Ok(())
}

fn validate_zip_entry_kind<R: Read + ?Sized>(
    entry: &zip::read::ZipFile<'_, R>,
) -> Result<(), String> {
    if entry.encrypted() {
        return Err("encrypted modpack archive entries are not supported".to_owned());
    }
    if entry.is_symlink() {
        return Err("the modpack archive contains a symbolic link".to_owned());
    }
    if !entry.is_file() && !entry.is_dir() {
        return Err("the modpack archive contains a device or special file".to_owned());
    }
    if let Some(mode) = entry.unix_mode() {
        let kind = mode & 0o170000;
        if kind != 0 && kind != 0o100000 && kind != 0o040000 {
            return Err("the modpack archive contains a device or special file".to_owned());
        }
    }
    if !matches!(
        entry.compression(),
        CompressionMethod::Stored | CompressionMethod::Deflated
    ) {
        return Err("the modpack archive uses an unsupported compression method".to_owned());
    }
    Ok(())
}

fn validate_compression_bounds<R: Read + ?Sized>(
    entry: &zip::read::ZipFile<'_, R>,
    limits: &MrpackLimits,
) -> Result<(), String> {
    if entry.size() > limits.maximum_unpacked_bytes {
        return Err("a modpack archive entry exceeds the unpacked safety limit".to_owned());
    }
    if entry.size() > 0 {
        if entry.compressed_size() == 0 {
            return Err("a modpack archive entry has an invalid compressed size".to_owned());
        }
        let ratio = entry.size().div_ceil(entry.compressed_size());
        if ratio > limits.maximum_compression_ratio {
            return Err(format!(
                "a modpack archive entry exceeds the {}:1 compression-ratio limit",
                limits.maximum_compression_ratio
            ));
        }
    }
    Ok(())
}

fn read_bounded(
    reader: &mut impl Read,
    maximum_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| format!("could not read the {label}"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum_bytes {
        return Err(format!("the {label} exceeds its safety limit"));
    }
    Ok(bytes)
}

fn copy_bounded(
    reader: &mut impl Read,
    writer: &mut impl Write,
    expected_bytes: u64,
    deadline: Instant,
    label: &str,
) -> Result<(), String> {
    let mut written = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        ensure_deadline(deadline)?;
        let read = reader
            .read(&mut buffer)
            .map_err(|_| format!("could not read a {label}"))?;
        if read == 0 {
            break;
        }
        written = written
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| format!("the {label} size overflowed"))?;
        if written > expected_bytes {
            return Err(format!("a {label} exceeded its declared size"));
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|_| format!("could not write a {label}"))?;
    }
    if written != expected_bytes {
        return Err(format!("a {label} did not match its declared size"));
    }
    Ok(())
}

fn create_safe_directories(root: &Path, parent: &Path) -> Result<(), String> {
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| "a modpack output escaped its staging directory".to_owned())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err("a modpack output parent is not a real directory".to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|_| "could not create a staged modpack directory".to_owned())?;
            }
            Err(_) => return Err("could not inspect a staged modpack directory".to_owned()),
        }
    }
    Ok(())
}

fn select_trusted_download(downloads: &[String]) -> Result<String, String> {
    if downloads.is_empty() {
        return Err("a modpack file has no declared download".to_owned());
    }
    if downloads.len() != 1 {
        return Err(
            "a modpack file must have exactly one trusted Modrinth CDN download".to_owned(),
        );
    }
    let url = &downloads[0];
    require_exact_https_host(url, TRUSTED_CDN_HOST)?;
    Ok(url.clone())
}

pub fn require_exact_https_host(url: &str, expected_host: &str) -> Result<(), String> {
    let remainder = url
        .strip_prefix("https://")
        .ok_or_else(|| "the modpack returned a non-HTTPS download".to_owned())?;
    let (authority, path) = remainder
        .split_once('/')
        .ok_or_else(|| "the modpack returned an invalid download URL".to_owned())?;
    if !authority.eq_ignore_ascii_case(expected_host)
        || authority.contains(['@', ':', '\\'])
        || path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', '\0'])
        || path.contains('#')
        || url
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("the modpack returned an untrusted download host".to_owned());
    }
    Ok(())
}

fn reject_helix_owned_path(path: &str) -> Result<(), String> {
    let key = collision_key(path);
    let root = key.split('/').next().unwrap_or_default();
    if matches!(root, "server.jar" | "eula.txt" | "server.properties") || root.starts_with(".helix")
    {
        return Err(format!(
            "the modpack attempts to overwrite a Helix-owned server file: {path}"
        ));
    }
    Ok(())
}

fn required_dependency<'a>(value: Option<&'a str>, label: &str) -> Result<&'a str, String> {
    let value = value.ok_or_else(|| format!("the modpack does not pin its {label}"))?;
    validate_label(value, label, 128)?;
    Ok(value)
}

fn validate_label(value: &str, label: &str, maximum_bytes: usize) -> Result<(), String> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > maximum_bytes
        || value.chars().any(char::is_control)
    {
        return Err(format!("the modpack has an invalid {label}"));
    }
    Ok(())
}

fn validate_hex(value: &str, length: usize, label: &str) -> Result<(), String> {
    if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("the modpack contains an invalid {label} checksum"));
    }
    Ok(())
}

fn ensure_deadline(deadline: Instant) -> Result<(), String> {
    if Instant::now() > deadline {
        Err("the modpack exceeded its processing time limit".to_owned())
    } else {
        Ok(())
    }
}

fn collision_key(path: &str) -> String {
    path.nfkc().flat_map(char::to_lowercase).collect()
}

fn is_windows_device_name(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    matches!(value.as_str(), "con" | "prn" | "aux" | "nul")
        || value.strip_prefix("com").is_some_and(|number| {
            matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || value.strip_prefix("lpt").is_some_and(|number| {
            matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn checksums(content: &[u8]) -> (String, String) {
        let mut sha1 = Sha1::new();
        sha1.update(content);
        let mut sha512 = Sha512::new();
        sha512.update(content);
        (hex_digest(sha1.finalize()), hex_digest(sha512.finalize()))
    }

    fn index(file_path: &str, file: &[u8], server: &str, dependencies: &str) -> Vec<u8> {
        let (sha1, sha512) = checksums(file);
        format!(
            r#"{{"formatVersion":1,"game":"minecraft","versionId":"1.0.0","name":"Safe Pack","summary":"Fixture","files":[{{"path":"{file_path}","hashes":{{"sha1":"{sha1}","sha512":"{sha512}"}},"env":{{"client":"required","server":"{server}"}},"downloads":["https://cdn.modrinth.com/data/project/versions/version/file.jar"],"fileSize":{}}}],"dependencies":{dependencies}}}"#,
            file.len()
        )
        .into_bytes()
    }

    fn write_pack(path: &Path, index: &[u8], entries: &[(&str, &[u8], u32)]) {
        let file = File::create(path).expect("create fixture");
        let mut zip = ZipWriter::new(file);
        zip.start_file(INDEX_NAME, SimpleFileOptions::default())
            .expect("index entry");
        zip.write_all(index).expect("index bytes");
        for (name, bytes, permissions) in entries {
            let options = SimpleFileOptions::default().unix_permissions(*permissions);
            zip.start_file(*name, options).expect("fixture entry");
            zip.write_all(bytes).expect("fixture bytes");
        }
        zip.finish().expect("finish fixture");
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(10)
    }

    #[test]
    fn valid_fabric_pack_layers_server_overrides_and_verifies_downloads() {
        let temp = TempDir::new().expect("tempdir");
        let archive = temp.path().join("pack.mrpack");
        let payload = b"integrity verified mod";
        let json = index(
            "mods/example.jar",
            payload,
            "required",
            r#"{"minecraft":"1.21.1","fabric-loader":"0.16.14"}"#,
        );
        write_pack(
            &archive,
            &json,
            &[
                ("overrides/config/example.txt", b"common", 0o100644),
                ("server-overrides/config/example.txt", b"server", 0o100644),
            ],
        );
        let plan = inspect_mrpack(&archive, &MrpackLimits::default(), deadline())
            .expect("inspect valid pack");
        assert_eq!(plan.minecraft_version, "1.21.1");
        assert_eq!(plan.fabric_loader_version, "0.16.14");
        assert_eq!(plan.files.len(), 1);

        let staging = temp.path().join("staging");
        fs::create_dir(&staging).expect("staging");
        extract_overrides(
            &archive,
            &staging,
            &plan,
            &MrpackLimits::default(),
            deadline(),
        )
        .expect("extract overrides");
        assert_eq!(
            fs::read(staging.join("config/example.txt")).expect("override"),
            b"server"
        );

        let downloaded = temp.path().join("example.jar");
        fs::write(&downloaded, payload).expect("download fixture");
        verify_download(&downloaded, &plan.files[0]).expect("verify both hashes");
        fs::write(&downloaded, b"wrong size").expect("corrupt fixture");
        assert!(verify_download(&downloaded, &plan.files[0]).is_err());
    }

    #[test]
    fn client_only_and_optional_files_are_not_installable() {
        let temp = TempDir::new().expect("tempdir");
        let archive = temp.path().join("pack.mrpack");
        let payload = b"client only";
        let (client_sha1, client_sha512) = checksums(payload);
        let (sha1, sha512) = checksums(b"optional");
        let client = serde_json::to_vec(&serde_json::json!({
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "1.0.0",
            "name": "Environment fixture",
            "files": [
                {
                    "path": "mods/client.jar",
                    "hashes": {"sha1": client_sha1, "sha512": client_sha512},
                    "env": {"client": "required", "server": "unsupported"},
                    "downloads": ["https://cdn.modrinth.com/data/p/v/client.jar"],
                    "fileSize": payload.len(),
                },
                {
                    "path": "mods/optional.jar",
                    "hashes": {"sha1": sha1, "sha512": sha512},
                    "env": {"client": "optional", "server": "optional"},
                    "downloads": ["https://cdn.modrinth.com/data/p/v/o.jar"],
                    "fileSize": 8,
                }
            ],
            "dependencies": {"minecraft": "1.21.1", "fabric-loader": "0.16.14"},
        }))
        .expect("serialize fixture");
        write_pack(&archive, &client, &[]);
        let plan =
            inspect_mrpack(&archive, &MrpackLimits::default(), deadline()).expect("inspect pack");
        assert!(plan.files.is_empty());
        assert_eq!(plan.skipped_client_only_files, 1);
        assert_eq!(plan.skipped_optional_files, 1);
    }

    #[test]
    fn missing_file_environment_defaults_to_required_on_the_server() {
        let temp = TempDir::new().expect("tempdir");
        let archive = temp.path().join("pack.mrpack");
        let payload = b"unrestricted server mod";
        let (sha1, sha512) = checksums(payload);
        let index = serde_json::to_vec(&serde_json::json!({
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "1.0.0",
            "name": "No env fixture",
            "files": [{
                "path": "mods/server.jar",
                "hashes": {"sha1": sha1, "sha512": sha512},
                "downloads": ["https://cdn.modrinth.com/data/p/v/server.jar"],
                "fileSize": payload.len(),
            }],
            "dependencies": {"minecraft": "1.21.1", "fabric-loader": "0.16.14"},
        }))
        .expect("serialize fixture");
        write_pack(&archive, &index, &[]);

        let plan = inspect_mrpack(&archive, &MrpackLimits::default(), deadline())
            .expect("missing env is unrestricted");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.skipped_optional_files, 0);
        assert_eq!(plan.skipped_client_only_files, 0);
    }

    #[test]
    fn rejects_traversal_reserved_and_helix_owned_paths() {
        let limits = MrpackLimits::default();
        for path in [
            "../server.jar",
            "/etc/passwd",
            "mods\\evil.jar",
            "C:/evil.jar",
            "mods/CON.txt",
            "server.properties",
            ".helix-secret/token",
        ] {
            let path_error = validate_relative_path(path, &limits).is_err();
            let owned_error = reject_helix_owned_path(path).is_err();
            assert!(path_error || owned_error, "expected rejection for {path}");
        }
    }

    #[test]
    fn rejects_symlink_and_duplicate_casefolded_outputs() {
        let temp = TempDir::new().expect("tempdir");
        let archive = temp.path().join("symlink.mrpack");
        let json = index(
            "mods/a.jar",
            b"a",
            "required",
            r#"{"minecraft":"1.21.1","fabric-loader":"0.16.14"}"#,
        );
        let file = File::create(&archive).expect("create symlink fixture");
        let mut zip = ZipWriter::new(file);
        zip.start_file(INDEX_NAME, SimpleFileOptions::default())
            .expect("index entry");
        zip.write_all(&json).expect("index bytes");
        zip.add_symlink("overrides/link", "target", SimpleFileOptions::default())
            .expect("symlink entry");
        zip.finish().expect("finish symlink fixture");
        assert!(inspect_mrpack(&archive, &MrpackLimits::default(), deadline()).is_err());

        let duplicate = temp.path().join("duplicate.mrpack");
        write_pack(
            &duplicate,
            &json,
            &[("overrides/Mods/A.jar", b"override", 0o100644)],
        );
        let error = inspect_mrpack(&duplicate, &MrpackLimits::default(), deadline())
            .expect_err("collision must fail");
        assert!(error.contains("same server path"));
    }

    #[test]
    fn rejects_unsupported_loader_dependencies_with_precise_reason() {
        for (dependency, reason) in [
            ("forge", "Forge"),
            ("neoforge", "NeoForge"),
            ("quilt-loader", "Quilt"),
        ] {
            let temp = TempDir::new().expect("tempdir");
            let archive = temp.path().join("pack.mrpack");
            let dependencies = format!(r#"{{"minecraft":"1.21.1","{dependency}":"1.0"}}"#);
            let json = index("mods/a.jar", b"a", "required", &dependencies);
            write_pack(&archive, &json, &[]);
            let error = inspect_mrpack(&archive, &MrpackLimits::default(), deadline())
                .expect_err("loader must fail");
            assert!(error.contains(reason), "{error}");
        }
    }

    #[test]
    fn future_loader_dependency_fails_closed_with_a_clear_reason() {
        let temp = TempDir::new().expect("tempdir");
        let archive = temp.path().join("pack.mrpack");
        let json = index(
            "mods/a.jar",
            b"a",
            "required",
            r#"{"minecraft":"1.21.1","future-loader":"2.0"}"#,
        );
        write_pack(&archive, &json, &[]);

        let error = inspect_mrpack(&archive, &MrpackLimits::default(), deadline())
            .expect_err("unknown loader must fail closed");
        assert!(error.contains("future-loader"));
        assert!(error.contains("Fabric-only"), "{error}");
    }

    #[test]
    fn enforces_declared_file_and_compression_bounds() {
        let temp = TempDir::new().expect("tempdir");
        let archive = temp.path().join("pack.mrpack");
        let json = index(
            "mods/a.jar",
            b"a",
            "required",
            r#"{"minecraft":"1.21.1","fabric-loader":"0.16.14"}"#,
        );
        write_pack(&archive, &json, &[]);
        let limits = MrpackLimits {
            maximum_files: 0,
            ..MrpackLimits::default()
        };
        let error = inspect_mrpack(&archive, &limits, deadline()).expect_err("file limit");
        assert!(error.contains("file safety limit"));
    }

    #[test]
    fn trusted_download_host_rejects_redirect_style_and_authority_tricks() {
        for url in [
            "http://cdn.modrinth.com/data/a",
            "https://cdn.modrinth.com.evil.test/data/a",
            "https://cdn.modrinth.com@evil.test/data/a",
            "https://cdn.modrinth.com:443/data/a",
            "https://cdn.modrinth.com/data/a#fragment",
        ] {
            assert!(
                require_exact_https_host(url, TRUSTED_CDN_HOST).is_err(),
                "{url}"
            );
        }
        assert!(
            require_exact_https_host(
                "https://cdn.modrinth.com/data/project/versions/version/file.jar",
                TRUSTED_CDN_HOST
            )
            .is_ok()
        );
    }
}
