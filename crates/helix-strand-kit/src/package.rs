//! Pack and unpack installable UI-only Strand zip archives.

use super::{
    MAX_MANIFEST_BYTES, StrandKind, StrandKitError, ValidatedManifest, validate_strand_project,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, read::ZipFile, write::SimpleFileOptions};

pub const MAX_PACKAGE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_UNCOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_PACKAGE_FILES: usize = 64;
pub const MAX_FILE_BYTES: u64 = 512 * 1024;
pub const MANIFEST_NAME: &str = "strand.toml";

const ALLOWED_UI_EXTENSIONS: &[&str] = &[
    "css", "html", "ico", "jpeg", "jpg", "js", "json", "md", "png", "svg", "txt", "webp", "woff2",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrandAsset {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnpackedStrand {
    pub manifest: ValidatedManifest,
    pub assets: Vec<StrandAsset>,
    pub digest_sha256: String,
}

impl UnpackedStrand {
    #[must_use]
    pub fn asset(&self, path: &str) -> Option<&StrandAsset> {
        self.assets.iter().find(|asset| asset.path == path)
    }

    #[must_use]
    pub fn ui_entry(&self) -> &str {
        self.manifest
            .ui_entry
            .as_deref()
            .expect("installable Strands declare a UI entry")
    }
}

pub fn pack_strand_project(path: &Path) -> Result<Vec<u8>, StrandKitError> {
    let manifest = validate_strand_project(path)?;
    if !manifest.installable {
        return Err(StrandKitError::NotInstallable {
            path: manifest.path.clone(),
            message: not_installable_reason(&manifest),
        });
    }
    let root = project_root(path)?;
    let mut files = Vec::new();
    collect_file(&root, MANIFEST_NAME, &mut files)?;
    collect_ui_tree(&root.join("ui"), "ui", &mut files)?;
    let entry = manifest
        .ui_entry
        .as_deref()
        .expect("installable Strands declare a UI entry");
    if !files.iter().any(|(name, _)| name == entry) {
        return Err(StrandKitError::PackageInvalid {
            message: format!("UI entry {entry} is missing from the package"),
        });
    }
    write_strand_zip(&files)
}

pub fn unpack_strand_package(bytes: &[u8]) -> Result<UnpackedStrand, StrandKitError> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(StrandKitError::PackageTooLarge {
            observed: bytes.len() as u64,
            maximum: MAX_PACKAGE_BYTES,
        });
    }
    if bytes.len() < 32 {
        return Err(StrandKitError::PackageInvalid {
            message: "Strand package is too small to be a valid zip".to_owned(),
        });
    }

    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|source| StrandKitError::PackageInvalid {
            message: format!("Strand package is not a readable zip: {source}"),
        })?;
    if archive.len() > MAX_PACKAGE_FILES {
        return Err(StrandKitError::PackageInvalid {
            message: format!(
                "Strand package contains {} entries; the limit is {MAX_PACKAGE_FILES}",
                archive.len()
            ),
        });
    }

    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut uncompressed_total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|source| StrandKitError::PackageInvalid {
                message: format!("Strand package entry {index} could not be read: {source}"),
            })?;
        validate_zip_entry(&entry)?;
        if entry.is_dir() {
            continue;
        }
        let path = normalize_zip_path(entry.name())?;
        if !seen.insert(path.clone()) {
            return Err(StrandKitError::PackageInvalid {
                message: format!("Strand package contains duplicate path {path}"),
            });
        }
        let declared = entry.size();
        if declared > MAX_FILE_BYTES {
            return Err(StrandKitError::PackageInvalid {
                message: format!(
                    "{path} is {declared} bytes; each packaged file must be at most {MAX_FILE_BYTES}"
                ),
            });
        }
        uncompressed_total = uncompressed_total.saturating_add(declared);
        if uncompressed_total > MAX_UNCOMPRESSED_BYTES {
            return Err(StrandKitError::PackageInvalid {
                message: format!(
                    "uncompressed Strand package exceeds {MAX_UNCOMPRESSED_BYTES} bytes"
                ),
            });
        }
        let mut bytes = Vec::new();
        entry
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| StrandKitError::PackageInvalid {
                message: format!("could not read {path}: {source}"),
            })?;
        let observed = bytes.len() as u64;
        if observed > MAX_FILE_BYTES || (declared > 0 && observed != declared) {
            return Err(StrandKitError::PackageInvalid {
                message: format!("{path} exceeded its declared size"),
            });
        }
        files.push((path, bytes));
    }

    let Some((_, manifest_bytes)) = files.iter().find(|(name, _)| name == MANIFEST_NAME) else {
        return Err(StrandKitError::PackageInvalid {
            message: "Strand package is missing strand.toml".to_owned(),
        });
    };
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(StrandKitError::ManifestTooLarge {
            path: PathBuf::from(MANIFEST_NAME),
            observed: manifest_bytes.len() as u64,
            maximum: MAX_MANIFEST_BYTES,
        });
    }

    let temporary = tempfile::Builder::new()
        .prefix(".helix-strand-unpack-")
        .tempdir()
        .map_err(|source| StrandKitError::Io {
            operation: "create unpack directory",
            path: PathBuf::from("."),
            source,
        })?;
    let staged = temporary.path();
    fs::write(staged.join(MANIFEST_NAME), manifest_bytes).map_err(|source| StrandKitError::Io {
        operation: "write",
        path: staged.join(MANIFEST_NAME),
        source,
    })?;
    let manifest = validate_strand_project(staged)?;
    if !manifest.installable {
        return Err(StrandKitError::NotInstallable {
            path: PathBuf::from(MANIFEST_NAME),
            message: not_installable_reason(&manifest),
        });
    }
    let entry = manifest
        .ui_entry
        .as_deref()
        .expect("installable Strands declare a UI entry");
    if !files.iter().any(|(name, _)| name == entry) {
        return Err(StrandKitError::PackageInvalid {
            message: format!("UI entry {entry} is missing from the package"),
        });
    }
    for (path, _) in &files {
        if path != MANIFEST_NAME && !is_allowed_ui_path(path) {
            return Err(StrandKitError::PackageInvalid {
                message: format!("{path} is not an allowed UI asset"),
            });
        }
    }

    Ok(UnpackedStrand {
        manifest,
        assets: files
            .into_iter()
            .map(|(path, bytes)| StrandAsset { path, bytes })
            .collect(),
        digest_sha256: hex_sha256(bytes),
    })
}

pub fn content_type_for_asset(path: &str) -> &'static str {
    match extension_of(path) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("ico") => "image/x-icon",
        Some("jpeg" | "jpg") => "image/jpeg",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("md" | "txt") => "text/plain; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn write_strand_zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, StrandKitError> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        for (name, bytes) in files {
            zip.start_file(name, options)
                .map_err(|source| StrandKitError::PackageInvalid {
                    message: format!("could not write {name}: {source}"),
                })?;
            zip.write_all(bytes)
                .map_err(|source| StrandKitError::PackageInvalid {
                    message: format!("could not write {name}: {source}"),
                })?;
        }
        zip.finish()
            .map_err(|source| StrandKitError::PackageInvalid {
                message: format!("could not finish Strand zip: {source}"),
            })?;
    }
    let bytes = cursor.into_inner();
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(StrandKitError::PackageTooLarge {
            observed: bytes.len() as u64,
            maximum: MAX_PACKAGE_BYTES,
        });
    }
    Ok(bytes)
}

fn collect_ui_tree(
    directory: &Path,
    relative: &str,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), StrandKitError> {
    let metadata = fs::symlink_metadata(directory).map_err(|source| StrandKitError::Io {
        operation: "inspect",
        path: directory.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StrandKitError::PackageInvalid {
            message: format!("{} must be a real UI directory", directory.display()),
        });
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|source| StrandKitError::Io {
            operation: "read",
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| StrandKitError::Io {
            operation: "read",
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.contains('\\') || name.contains('/') {
            return Err(StrandKitError::PackageInvalid {
                message: format!("UI file {name} uses a disallowed name"),
            });
        }
        let child = entry.path();
        let child_relative = format!("{relative}/{name}");
        let metadata = fs::symlink_metadata(&child).map_err(|source| StrandKitError::Io {
            operation: "inspect",
            path: child.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(StrandKitError::PackageInvalid {
                message: format!("{child_relative} is a symbolic link"),
            });
        }
        if metadata.is_dir() {
            collect_ui_tree(&child, &child_relative, files)?;
        } else if metadata.is_file() {
            if !is_allowed_ui_path(&child_relative) {
                return Err(StrandKitError::PackageInvalid {
                    message: format!("{child_relative} is not an allowed UI asset"),
                });
            }
            if metadata.len() > MAX_FILE_BYTES {
                return Err(StrandKitError::PackageInvalid {
                    message: format!(
                        "{child_relative} is {} bytes; each packaged file must be at most {MAX_FILE_BYTES}",
                        metadata.len()
                    ),
                });
            }
            let bytes = fs::read(&child).map_err(|source| StrandKitError::Io {
                operation: "read",
                path: child,
                source,
            })?;
            files.push((child_relative, bytes));
        } else {
            return Err(StrandKitError::PackageInvalid {
                message: format!("{child_relative} is not a regular file"),
            });
        }
        if files.len() > MAX_PACKAGE_FILES {
            return Err(StrandKitError::PackageInvalid {
                message: format!("Strand package exceeds {MAX_PACKAGE_FILES} files"),
            });
        }
    }
    Ok(())
}

fn collect_file(
    directory: &Path,
    file_name: &str,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), StrandKitError> {
    let path = directory.join(file_name);
    let metadata = fs::symlink_metadata(&path).map_err(|source| StrandKitError::Io {
        operation: "inspect",
        path: path.clone(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StrandKitError::PackageInvalid {
            message: format!("{} must be a regular file", path.display()),
        });
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(StrandKitError::PackageInvalid {
            message: format!(
                "{} is {} bytes; each packaged file must be at most {MAX_FILE_BYTES}",
                path.display(),
                metadata.len()
            ),
        });
    }
    let bytes = fs::read(&path).map_err(|source| StrandKitError::Io {
        operation: "read",
        path,
        source,
    })?;
    files.push((file_name.to_owned(), bytes));
    Ok(())
}

fn project_root(path: &Path) -> Result<PathBuf, StrandKitError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(path.to_path_buf()),
        Ok(_) => {
            path.parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| StrandKitError::PackageInvalid {
                    message: "Strand manifest has no project directory".to_owned(),
                })
        }
        Err(source) => Err(StrandKitError::Io {
            operation: "inspect",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn validate_zip_entry<R: Read>(entry: &ZipFile<'_, R>) -> Result<(), StrandKitError> {
    if entry.encrypted() {
        return Err(StrandKitError::PackageInvalid {
            message: "encrypted Strand package entries are not supported".to_owned(),
        });
    }
    if entry.is_symlink() {
        return Err(StrandKitError::PackageInvalid {
            message: "Strand packages must not contain symbolic links".to_owned(),
        });
    }
    if !entry.is_file() && !entry.is_dir() {
        return Err(StrandKitError::PackageInvalid {
            message: "Strand packages must not contain device or special files".to_owned(),
        });
    }
    if let Some(mode) = entry.unix_mode() {
        let kind = mode & 0o170000;
        if kind != 0 && kind != 0o100000 && kind != 0o040000 {
            return Err(StrandKitError::PackageInvalid {
                message: "Strand packages must not contain device or special files".to_owned(),
            });
        }
        if mode & 0o4000 != 0 || mode & 0o2000 != 0 {
            return Err(StrandKitError::PackageInvalid {
                message: "Strand packages must not contain setuid or setgid files".to_owned(),
            });
        }
    }
    if !matches!(
        entry.compression(),
        CompressionMethod::Stored | CompressionMethod::Deflated
    ) {
        return Err(StrandKitError::PackageInvalid {
            message: "Strand package uses an unsupported compression method".to_owned(),
        });
    }
    Ok(())
}

fn normalize_zip_path(name: &str) -> Result<String, StrandKitError> {
    let normalized = name.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains('\0')
        || normalized.contains("//")
        || normalized.ends_with('/')
    {
        return Err(StrandKitError::PackageInvalid {
            message: format!("Strand package path {name} is not allowed"),
        });
    }
    let path = Path::new(&normalized);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StrandKitError::PackageInvalid {
            message: format!("Strand package path {name} is not allowed"),
        });
    }
    if normalized != MANIFEST_NAME && !normalized.starts_with("ui/") {
        return Err(StrandKitError::PackageInvalid {
            message: format!("{normalized} is outside the allowed Strand package layout"),
        });
    }
    Ok(normalized)
}

pub(crate) fn is_allowed_ui_path(path: &str) -> bool {
    path.starts_with("ui/")
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && ALLOWED_UI_EXTENSIONS
            .iter()
            .any(|extension| extension_of(path) == Some(*extension))
}

fn extension_of(path: &str) -> Option<&str> {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .and_then(|extension| {
            ALLOWED_UI_EXTENSIONS
                .iter()
                .copied()
                .find(|known| *known == extension)
        })
}

fn not_installable_reason(manifest: &ValidatedManifest) -> String {
    if manifest.schema == super::PREVIEW_SCHEMA {
        "helix.strand/preview-1 is authoring metadata only; pack helix.strand/1 UI-only projects"
            .to_owned()
    } else if manifest.kind != StrandKind::UiOnly {
        "only ui-only Strands can be packed and installed in this Helix version".to_owned()
    } else if manifest.ui_entry.is_none() {
        "installable Strands must declare [ui].entry".to_owned()
    } else {
        "this Strand is not installable in this Helix version".to_owned()
    }
}
