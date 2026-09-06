//! Scaffolding, validation, and zip packaging for Strand extensions.
//!
//! `helix.strand/1` UI-only packages can be packed and installed. Portable Wasm
//! and native sidecars stay preview-only until their isolated host exists.

mod package;

use semver::{Op, Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    error::Error as StdError,
    fmt, fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
};
use thiserror::Error;
use uuid::Uuid;

pub use package::{
    MAX_FILE_BYTES, MAX_PACKAGE_BYTES, MAX_PACKAGE_FILES, MAX_UNCOMPRESSED_BYTES, StrandAsset,
    UnpackedStrand, content_type_for_asset, hex_sha256, pack_strand_project, unpack_strand_package,
};

pub const PREVIEW_SCHEMA: &str = "helix.strand/preview-1";
pub const INSTALL_SCHEMA: &str = "helix.strand/1";
pub const PREVIEW_HOST_API: &str = "preview-1";
pub const INSTALL_HOST_API: &str = "1";
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const MAX_CAPABILITIES: usize = 32;
pub const MAX_CAPABILITY_ORIGINS: usize = 8;

const DEFAULT_HELIX_COMPATIBILITY: &str = ">=1.0.0, <2.0.0";
const DEFAULT_LICENSE: &str = "AGPL-3.0-or-later";
const INSTALLABLE_CAPABILITIES: &[&str] = &[
    "helix:metrics.read",
    "helix:storage.kv",
    "helix:net.https",
    "helix:ui.page",
    "helix:ui.widget",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrandKind {
    Portable,
    UiOnly,
}

impl fmt::Display for StrandKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Portable => "portable",
            Self::UiOnly => "ui-only",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequest {
    pub name: String,
    pub reason: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UiSpec {
    pub entry: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    pub memory_mib: u16,
    pub timeout_ms: u32,
    pub concurrent_calls: u16,
    pub queue_depth: u16,
    pub storage_mib: u32,
    pub outbound_requests_per_minute: u16,
    pub log_kib_per_minute: u16,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_mib: 32,
            timeout_ms: 1_000,
            concurrent_calls: 2,
            queue_depth: 32,
            storage_mib: 16,
            outbound_requests_per_minute: 0,
            log_kib_per_minute: 64,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Compatibility {
    helix: String,
    host_api: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestDocument {
    schema: String,
    id: String,
    slug: String,
    name: String,
    version: String,
    description: String,
    license: String,
    publisher: String,
    kind: StrandKind,
    #[serde(default)]
    capabilities: Vec<CapabilityRequest>,
    compatibility: Compatibility,
    limits: ResourceLimits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ui: Option<UiSpec>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedManifest {
    pub path: PathBuf,
    pub schema: String,
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub version: Version,
    pub description: String,
    pub license: String,
    pub publisher: String,
    pub kind: StrandKind,
    pub capabilities: Vec<CapabilityRequest>,
    pub helix_compatibility: VersionReq,
    pub host_api: String,
    pub limits: ResourceLimits,
    pub ui_entry: Option<String>,
    pub installable: bool,
}

fn helix_version_satisfies(required: &VersionReq, installed: &Version) -> bool {
    if required.matches(installed) {
        return true;
    }
    // Strands packed for the 0.1 preview stay loadable on the 1.x private-LAN line.
    installed.major == 1
        && Version::parse("0.1.0-alpha.1").is_ok_and(|preview| required.matches(&preview))
}

impl ValidatedManifest {
    pub fn ensure_helix_compatible(&self, helix_version: &str) -> Result<(), StrandKitError> {
        let version =
            Version::parse(helix_version).map_err(|error| StrandKitError::PackageInvalid {
                message: format!("Helix version is not valid SemVer: {error}"),
            })?;
        if helix_version_satisfies(&self.helix_compatibility, &version) {
            Ok(())
        } else {
            Err(StrandKitError::NotInstallable {
                path: self.path.clone(),
                message: format!(
                    "this Strand requires Helix {}; this host is {helix_version}",
                    self.helix_compatibility
                ),
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaffoldOptions {
    pub destination: PathBuf,
    pub slug: String,
    pub name: Option<String>,
    pub publisher: String,
    pub license: String,
    pub kind: StrandKind,
}

impl ScaffoldOptions {
    #[must_use]
    pub fn new(destination: PathBuf, slug: String, kind: StrandKind) -> Self {
        Self {
            destination,
            slug,
            name: None,
            publisher: "Local developer".to_owned(),
            license: DEFAULT_LICENSE.to_owned(),
            kind,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaffoldResult {
    pub root: PathBuf,
    pub manifest: ValidatedManifest,
}

#[derive(Debug, Error)]
pub enum StrandKitError {
    #[error("could not {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Strand manifest {path} exceeds the {maximum}-byte limit (observed {observed} bytes)")]
    ManifestTooLarge {
        path: PathBuf,
        observed: u64,
        maximum: u64,
    },
    #[error("Strand manifest {path} must be a regular file and not a symbolic link")]
    UnsafeManifest { path: PathBuf },
    #[error("Strand manifest {path} is not valid UTF-8: {message}")]
    InvalidUtf8 { path: PathBuf, message: String },
    #[error("could not parse Strand manifest {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not serialize the generated Strand manifest: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("the destination already exists: {path}; Strand scaffolding never overwrites files")]
    DestinationExists { path: PathBuf },
    #[error("invalid Strand destination {path}: {message}")]
    InvalidDestination { path: PathBuf, message: String },
    #[error("Strand package is not installable: {message}")]
    NotInstallable { path: PathBuf, message: String },
    #[error("Strand package is invalid: {message}")]
    PackageInvalid { message: String },
    #[error("Strand package exceeds the {maximum}-byte zip limit (observed {observed} bytes)")]
    PackageTooLarge { observed: u64, maximum: u64 },
    #[error(transparent)]
    Validation(#[from] ManifestValidationError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestValidationError {
    path: PathBuf,
    issues: Vec<String>,
}

impl ManifestValidationError {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn issues(&self) -> &[String] {
        &self.issues
    }
}

impl fmt::Display for ManifestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "Strand manifest {} failed validation:",
            self.path.display()
        )?;
        for issue in &self.issues {
            writeln!(formatter, "- {issue}")?;
        }
        Ok(())
    }
}

impl StdError for ManifestValidationError {}

pub fn validate_strand_project(path: &Path) -> Result<ValidatedManifest, StrandKitError> {
    let manifest_path = locate_manifest(path)?;
    let metadata = fs::symlink_metadata(&manifest_path).map_err(|source| StrandKitError::Io {
        operation: "inspect",
        path: manifest_path.clone(),
        source,
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(StrandKitError::UnsafeManifest {
            path: manifest_path,
        });
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(StrandKitError::ManifestTooLarge {
            path: manifest_path,
            observed: metadata.len(),
            maximum: MAX_MANIFEST_BYTES,
        });
    }

    let file = fs::File::open(&manifest_path).map_err(|source| StrandKitError::Io {
        operation: "open",
        path: manifest_path.clone(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| StrandKitError::Io {
            operation: "read",
            path: manifest_path.clone(),
            source,
        })?;
    let observed = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if observed > MAX_MANIFEST_BYTES {
        return Err(StrandKitError::ManifestTooLarge {
            path: manifest_path,
            observed,
            maximum: MAX_MANIFEST_BYTES,
        });
    }
    let text = std::str::from_utf8(&bytes).map_err(|source| StrandKitError::InvalidUtf8 {
        path: manifest_path.clone(),
        message: source.to_string(),
    })?;
    let document: ManifestDocument =
        toml::from_str(text).map_err(|source| StrandKitError::Parse {
            path: manifest_path.clone(),
            source,
        })?;
    validate_document(manifest_path, document).map_err(StrandKitError::from)
}

pub fn scaffold_strand(options: &ScaffoldOptions) -> Result<ScaffoldResult, StrandKitError> {
    let destination = &options.destination;
    if destination.file_name().is_none() {
        return Err(StrandKitError::InvalidDestination {
            path: destination.clone(),
            message: "choose a package directory, not a filesystem root".to_owned(),
        });
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(StrandKitError::DestinationExists {
                path: destination.clone(),
            });
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(StrandKitError::Io {
                operation: "inspect",
                path: destination.clone(),
                source,
            });
        }
    }

    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = fs::metadata(parent).map_err(|source| StrandKitError::Io {
        operation: "inspect destination parent",
        path: parent.to_path_buf(),
        source,
    })?;
    if !parent_metadata.is_dir() {
        return Err(StrandKitError::InvalidDestination {
            path: destination.clone(),
            message: format!("parent {} is not a directory", parent.display()),
        });
    }

    let name = options
        .name
        .clone()
        .unwrap_or_else(|| display_name_from_slug(&options.slug));
    let installable = options.kind == StrandKind::UiOnly;
    let document = ManifestDocument {
        schema: if installable {
            INSTALL_SCHEMA.to_owned()
        } else {
            PREVIEW_SCHEMA.to_owned()
        },
        id: Uuid::new_v4().hyphenated().to_string(),
        slug: options.slug.clone(),
        name: name.clone(),
        version: "0.1.0".to_owned(),
        description: format!("A Helix Strand named {name}."),
        license: options.license.clone(),
        publisher: options.publisher.clone(),
        kind: options.kind,
        capabilities: Vec::new(),
        compatibility: Compatibility {
            helix: DEFAULT_HELIX_COMPATIBILITY.to_owned(),
            host_api: if installable {
                INSTALL_HOST_API.to_owned()
            } else {
                PREVIEW_HOST_API.to_owned()
            },
        },
        limits: ResourceLimits::default(),
        ui: installable.then(|| UiSpec {
            entry: "ui/index.html".to_owned(),
        }),
    };
    validate_document(destination.join("strand.toml"), document.clone())?;

    let temporary = tempfile::Builder::new()
        .prefix(".helix-strand-")
        .tempdir_in(parent)
        .map_err(|source| StrandKitError::Io {
            operation: "create temporary scaffold directory in",
            path: parent.to_path_buf(),
            source,
        })?;
    let staged_root = temporary.path();
    if installable {
        fs::create_dir(staged_root.join("ui")).map_err(|source| StrandKitError::Io {
            operation: "create",
            path: staged_root.join("ui"),
            source,
        })?;
    } else {
        fs::create_dir(staged_root.join("src")).map_err(|source| StrandKitError::Io {
            operation: "create",
            path: staged_root.join("src"),
            source,
        })?;
    }

    let header = if installable {
        "# Installable UI Strand. Pack it with helixctl strand pack, then install the zip from the Strands page.\n"
    } else {
        "# Portable preview manifest. Helix can validate this file, but the Wasm host is not available yet.\n"
    };
    let mut manifest_text = String::from(header);
    let serialized = toml::to_string_pretty(&document)?;
    manifest_text.push_str(&serialized.replace(
        "capabilities = []\n",
        "# Deny by default. Add [[capabilities]] entries only for host calls this Strand actually makes.\ncapabilities = []\n",
    ));
    write_scaffold_file(staged_root.join("strand.toml"), manifest_text.as_bytes())?;
    write_scaffold_file(
        staged_root.join("README.md"),
        scaffold_readme(&name, &options.slug, options.kind).as_bytes(),
    )?;
    if installable {
        write_scaffold_file(
            staged_root.join("ui").join("helix.js"),
            scaffold_host_sdk().as_bytes(),
        )?;
        write_scaffold_file(
            staged_root.join("ui").join("style.css"),
            scaffold_ui_css().as_bytes(),
        )?;
        write_scaffold_file(
            staged_root.join("ui").join("index.html"),
            scaffold_ui_html(&name).as_bytes(),
        )?;
    } else {
        write_scaffold_file(
            staged_root.join("src").join("README.md"),
            source_readme(options.kind).as_bytes(),
        )?;
    }
    write_scaffold_file(staged_root.join(".gitignore"), b"/dist/\n*.strand.zip\n")?;

    validate_strand_project(staged_root)?;
    match publish_directory_no_replace(staged_root, destination) {
        Ok(()) => {}
        Err(source)
            if matches!(
                source.kind(),
                io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            return Err(StrandKitError::DestinationExists {
                path: destination.clone(),
            });
        }
        Err(source) => {
            return Err(StrandKitError::Io {
                operation: "publish scaffold to",
                path: destination.clone(),
                source,
            });
        }
    }

    let manifest = validate_strand_project(destination)?;
    Ok(ScaffoldResult {
        root: destination.clone(),
        manifest,
    })
}

fn locate_manifest(path: &Path) -> Result<PathBuf, StrandKitError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(StrandKitError::UnsafeManifest {
            path: path.to_path_buf(),
        }),
        Ok(metadata) if metadata.is_dir() => Ok(path.join("strand.toml")),
        Ok(_) => Ok(path.to_path_buf()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(source) => Err(StrandKitError::Io {
            operation: "inspect",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn validate_document(
    path: PathBuf,
    document: ManifestDocument,
) -> Result<ValidatedManifest, ManifestValidationError> {
    let mut issues = Vec::new();
    let installable_schema = document.schema == INSTALL_SCHEMA;
    if document.schema != PREVIEW_SCHEMA && document.schema != INSTALL_SCHEMA {
        issues.push(format!(
            "schema must be {PREVIEW_SCHEMA:?} or {INSTALL_SCHEMA:?}"
        ));
    }
    if installable_schema && document.kind != StrandKind::UiOnly {
        issues.push(
            "helix.strand/1 currently installs ui-only packages; portable Wasm remains preview-only"
                .to_owned(),
        );
    }
    if document.schema == PREVIEW_SCHEMA && document.ui.is_some() {
        issues.push("helix.strand/preview-1 cannot declare [ui]; use helix.strand/1".to_owned());
    }

    let id = match Uuid::parse_str(&document.id) {
        Ok(id) if id.is_nil() => {
            issues.push("id must not be the nil UUID".to_owned());
            None
        }
        Ok(id) if document.id != id.hyphenated().to_string() => {
            issues.push("id must use canonical lowercase hyphenated UUID form".to_owned());
            Some(id)
        }
        Ok(id) => Some(id),
        Err(error) => {
            issues.push(format!("id must be a UUID: {error}"));
            None
        }
    };

    validate_slug(&document.slug, &mut issues);
    validate_text("name", &document.name, 1, 80, &mut issues);
    validate_text("description", &document.description, 1, 240, &mut issues);
    validate_text("license", &document.license, 1, 96, &mut issues);
    validate_text("publisher", &document.publisher, 1, 120, &mut issues);

    let version = match Version::parse(&document.version) {
        Ok(version) => Some(version),
        Err(error) => {
            issues.push(format!(
                "version must be valid Semantic Versioning: {error}"
            ));
            None
        }
    };
    validate_text(
        "compatibility.helix",
        &document.compatibility.helix,
        1,
        120,
        &mut issues,
    );
    let helix_compatibility = match VersionReq::parse(&document.compatibility.helix) {
        Ok(requirement) if !version_requirement_is_bounded(&requirement) => {
            issues.push(
                "compatibility.helix must declare both a lower and upper compatibility bound"
                    .to_owned(),
            );
            Some(requirement)
        }
        Ok(requirement) => Some(requirement),
        Err(error) => {
            issues.push(format!(
                "compatibility.helix must be a valid Semantic Version requirement: {error}"
            ));
            None
        }
    };
    let expected_host_api = if installable_schema {
        INSTALL_HOST_API
    } else {
        PREVIEW_HOST_API
    };
    if document.compatibility.host_api != expected_host_api {
        issues.push(format!(
            "compatibility.host_api must be {expected_host_api:?} for schema {}",
            document.schema
        ));
    }

    validate_capabilities(&document.capabilities, installable_schema, &mut issues);
    validate_limits(&document.limits, &mut issues);
    if installable_schema
        && document
            .capabilities
            .iter()
            .any(|capability| capability.name == "helix:net.https")
        && document.limits.outbound_requests_per_minute == 0
    {
        issues.push(
            "helix:net.https requires limits.outbound_requests_per_minute of at least 1".to_owned(),
        );
    }

    let ui_entry = match &document.ui {
        Some(ui) if installable_schema => {
            if !package::is_allowed_ui_path(&ui.entry) || !ui.entry.ends_with(".html") {
                issues.push(
                    "ui.entry must be an HTML file under ui/ with an allowed extension".to_owned(),
                );
            }
            Some(ui.entry.clone())
        }
        None if installable_schema => {
            issues.push("helix.strand/1 ui-only packages must declare [ui].entry".to_owned());
            None
        }
        Some(_) => None,
        None => None,
    };

    if !issues.is_empty() {
        return Err(ManifestValidationError { path, issues });
    }

    Ok(ValidatedManifest {
        path,
        schema: document.schema,
        id: id.expect("validated ID is present"),
        slug: document.slug,
        name: document.name,
        version: version.expect("validated version is present"),
        description: document.description,
        license: document.license,
        publisher: document.publisher,
        kind: document.kind,
        capabilities: document.capabilities,
        helix_compatibility: helix_compatibility.expect("validated requirement is present"),
        host_api: document.compatibility.host_api,
        limits: document.limits,
        ui_entry,
        installable: installable_schema && document.kind == StrandKind::UiOnly,
    })
}

fn version_requirement_is_bounded(requirement: &VersionReq) -> bool {
    let mut has_lower_bound = false;
    let mut has_upper_bound = false;
    for comparator in &requirement.comparators {
        match comparator.op {
            Op::Greater | Op::GreaterEq => has_lower_bound = true,
            Op::Less | Op::LessEq => has_upper_bound = true,
            Op::Exact | Op::Tilde | Op::Caret | Op::Wildcard => {
                has_lower_bound = true;
                has_upper_bound = true;
            }
            _ => return false,
        }
    }
    has_lower_bound && has_upper_bound
}

#[cfg(any(
    target_os = "android",
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "visionos",
    target_os = "watchos",
    target_os = "redox",
))]
fn publish_directory_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}

#[cfg(windows)]
fn publish_directory_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(not(any(
    windows,
    target_os = "android",
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "visionos",
    target_os = "watchos",
    target_os = "redox",
)))]
fn publish_directory_no_replace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-overwrite directory publication is unavailable on this platform",
    ))
}

fn validate_slug(slug: &str, issues: &mut Vec<String>) {
    let valid_length = (2..=48).contains(&slug.len());
    let valid_shape = slug.split('-').all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    }) && slug
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase())
        && slug
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    if !valid_length || !valid_shape {
        issues.push(
            "slug must be 2-48 lowercase ASCII characters, start with a letter, and use single hyphens between alphanumeric segments"
                .to_owned(),
        );
    }
}

fn validate_text(
    field: &str,
    value: &str,
    minimum: usize,
    maximum: usize,
    issues: &mut Vec<String>,
) {
    let length = value.chars().count();
    if !(minimum..=maximum).contains(&length) {
        issues.push(format!(
            "{field} must contain {minimum}-{maximum} characters"
        ));
    }
    if value.trim() != value {
        issues.push(format!(
            "{field} must not have leading or trailing whitespace"
        ));
    }
    if value.chars().any(is_forbidden_display_character) {
        issues.push(format!(
            "{field} must not contain control or bidirectional formatting characters"
        ));
    }
}

fn is_forbidden_display_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn validate_capabilities(
    capabilities: &[CapabilityRequest],
    installable_schema: bool,
    issues: &mut Vec<String>,
) {
    if capabilities.len() > MAX_CAPABILITIES {
        issues.push(format!(
            "capabilities contains {} entries; the limit is {MAX_CAPABILITIES}",
            capabilities.len()
        ));
    }

    let mut names = HashSet::new();
    for (index, capability) in capabilities.iter().enumerate() {
        let field = format!("capabilities[{index}]");
        if !is_capability_name(&capability.name) {
            issues.push(format!(
                "{field}.name must use a narrow canonical name such as helix:metrics.read"
            ));
        }
        if installable_schema && !INSTALLABLE_CAPABILITIES.contains(&capability.name.as_str()) {
            issues.push(format!(
                "{field}.name {:?} is not a host call this Helix version mediates",
                capability.name
            ));
        }
        if !names.insert(capability.name.as_str()) {
            issues.push(format!(
                "{field}.name duplicates capability {:?}",
                capability.name
            ));
        }
        validate_text(
            &format!("{field}.reason"),
            &capability.reason,
            8,
            200,
            issues,
        );
        if capability.name == "helix:net.https" {
            if capability.origins.is_empty() {
                issues.push(format!(
                    "{field}.origins must list the exact https origins this Strand may call"
                ));
            }
            if capability.origins.len() > MAX_CAPABILITY_ORIGINS {
                issues.push(format!(
                    "{field}.origins contains {} entries; the limit is {MAX_CAPABILITY_ORIGINS}",
                    capability.origins.len()
                ));
            }
            let mut origins = HashSet::new();
            for origin in &capability.origins {
                if !is_https_origin(origin) {
                    issues.push(format!(
                        "{field}.origins value {origin:?} must be an exact https://host or https://host:port origin with no path"
                    ));
                }
                if !origins.insert(origin.as_str()) {
                    issues.push(format!("{field}.origins duplicates {origin:?}"));
                }
            }
        } else if !capability.origins.is_empty() {
            issues.push(format!("{field}.origins is only valid for helix:net.https"));
        }
    }
}

fn is_https_origin(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    if rest.is_empty()
        || rest.contains('/')
        || rest.contains('?')
        || rest.contains('#')
        || rest.contains('@')
        || rest.contains('\\')
        || rest.contains(' ')
    {
        return false;
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => (host, Some(port)),
        _ => (rest, None),
    };
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if let Some(port) = port {
        match port.parse::<u16>() {
            Ok(443 | 0) | Err(_) => return false,
            Ok(_) => {}
        }
    }
    if host.eq_ignore_ascii_case("localhost") || host.ends_with('.') {
        return false;
    }
    host.split('.').all(|label| {
        (1..=63).contains(&label.len())
            && label
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && label
                .bytes()
                .last()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn is_capability_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("helix:") else {
        return false;
    };
    if name.len() > 96 {
        return false;
    }
    rest.split('.').all(|segment| {
        (1..=32).contains(&segment.len())
            && segment
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            && segment
                .bytes()
                .last()
                .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    })
}

fn validate_limits(limits: &ResourceLimits, issues: &mut Vec<String>) {
    validate_limit("limits.memory_mib", limits.memory_mib, 8, 512, issues);
    validate_limit("limits.timeout_ms", limits.timeout_ms, 10, 30_000, issues);
    validate_limit(
        "limits.concurrent_calls",
        limits.concurrent_calls,
        1,
        32,
        issues,
    );
    validate_limit("limits.queue_depth", limits.queue_depth, 1, 1_024, issues);
    validate_limit("limits.storage_mib", limits.storage_mib, 0, 4_096, issues);
    validate_limit(
        "limits.outbound_requests_per_minute",
        limits.outbound_requests_per_minute,
        0,
        600,
        issues,
    );
    validate_limit(
        "limits.log_kib_per_minute",
        limits.log_kib_per_minute,
        1,
        1_024,
        issues,
    );
}

fn validate_limit<T>(field: &str, value: T, minimum: T, maximum: T, issues: &mut Vec<String>)
where
    T: Copy + fmt::Display + PartialOrd,
{
    if value < minimum || value > maximum {
        issues.push(format!(
            "{field} must be between {minimum} and {maximum}; received {value}"
        ));
    }
}

fn display_name_from_slug(slug: &str) -> String {
    slug.split('-')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut characters = segment.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_scaffold_file(path: PathBuf, contents: &[u8]) -> Result<(), StrandKitError> {
    fs::write(&path, contents).map_err(|source| StrandKitError::Io {
        operation: "write",
        path,
        source,
    })
}

fn scaffold_readme(name: &str, slug: &str, kind: StrandKind) -> String {
    if kind == StrandKind::UiOnly {
        format!(
            "# {name}\n\n`{slug}` is a UI-only Helix Strand. It runs as isolated dashboard HTML and talks to Helix through declared host calls only.\n\n## Build and install\n\n```text\nhelixctl strand check .\nhelixctl strand pack . -o {slug}.strand.zip\n```\n\nOpen **Strands** in the Helix dashboard, upload the zip or paste an https zip URL, review the capabilities, and enable it. Share the zip with another owner the same way; Helix does not operate a store.\n\nKeep `capabilities = []` until a host call is required. `helix:net.https` needs an exact origin allowlist. Strands never get a root shell, the privileged broker, or ambient sockets.\n"
        )
    } else {
        format!(
            "# {name}\n\n`{slug}` is a portable Strand preview. Helix can validate this project, but the Wasm host is not installable yet.\n\n```text\nhelixctl strand check .\n```\n\nUse `--kind ui-only` when you want a package that can be packed and installed today.\n"
        )
    }
}

fn source_readme(kind: StrandKind) -> String {
    format!(
        "# Source placeholder\n\nThis directory is reserved for a future {kind} Wasm implementation. This Helix version installs UI-only packages, not portable components.\n"
    )
}

fn scaffold_host_sdk() -> String {
    HELIX_HOST_SDK.to_owned()
}

const HELIX_HOST_SDK: &str = r#"(function (root) {
  "use strict";
  var pending = Object.create(null);
  var nextId = 1;
  function call(method, params) {
    return new Promise(function (resolve, reject) {
      var id = String(nextId++);
      var timer = root.setTimeout(function () {
        if (!pending[id]) return;
        delete pending[id];
        reject(new Error("Strand host call timed out"));
      }, 30000);
      pending[id] = {
        resolve: function (value) { root.clearTimeout(timer); resolve(value); },
        reject: function (error) { root.clearTimeout(timer); reject(error); }
      };
      root.parent.postMessage({ type: "helix-strand", id: id, method: method, params: params || {} }, "*");
    });
  }
  root.addEventListener("message", function (event) {
    if (event.source !== root.parent) return;
    var msg = event.data;
    if (!msg || msg.type !== "helix-strand-result" || !pending[msg.id]) return;
    var job = pending[msg.id];
    delete pending[msg.id];
    if (msg.ok) job.resolve(msg.result);
    else job.reject(new Error(msg.error || "Strand host call failed"));
  });
  root.helix = {
    call: call,
    metrics: { snapshot: function () { return call("metrics.snapshot"); } },
    storage: {
      get: function (key) { return call("storage.get", { key: key }); },
      set: function (key, value) { return call("storage.set", { key: key, value: value }); },
      remove: function (key) { return call("storage.delete", { key: key }); },
      list: function () { return call("storage.list"); }
    },
    net: { fetch: function (request) { return call("net.fetch", request); } }
  };
})(window);
"#;

fn scaffold_ui_css() -> String {
    ":root{color-scheme:dark;font-family:Inter,system-ui,sans-serif;background:#10140f;color:#e8edd8}body{margin:0;padding:20px}main{max-width:640px}h1{font-size:1.35rem;margin:0 0 8px}p,pre{color:#b7c0a4}pre{white-space:pre-wrap;background:#1a2118;border:1px solid #2a3326;border-radius:10px;padding:12px}button{border:0;border-radius:8px;padding:8px 12px;background:#d7f64d;color:#10140f;font-weight:650}html.is-widget body{padding:12px}html.is-widget .lead,html.is-widget p:first-of-type{display:none}\n".to_owned()
}

fn scaffold_ui_html(name: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{name}</title>\n<link rel=\"stylesheet\" href=\"style.css\">\n</head>\n<body>\n<main>\n<h1>{name}</h1>\n<p>This Strand talks to Helix only through declared host calls. Add capabilities in strand.toml when you need metrics, namespaced storage, or an allowlisted HTTPS origin.</p>\n<p id=\"status\">Ready.</p>\n<button type=\"button\" id=\"probe\">Read host metrics if granted</button>\n<pre id=\"out\" hidden></pre>\n</main>\n<script src=\"helix.js\"></script>\n<script>\ndocument.documentElement.classList.toggle(\"is-widget\", location.hash === \"#helix-widget\");\ndocument.getElementById(\"probe\").addEventListener(\"click\", function () {{\n  var out = document.getElementById(\"out\");\n  var status = document.getElementById(\"status\");\n  helix.metrics.snapshot().then(function (snapshot) {{\n    status.textContent = \"Host metrics received.\";\n    out.hidden = false;\n    out.textContent = JSON.stringify(snapshot, null, 2);\n  }}).catch(function (error) {{\n    status.textContent = error.message;\n  }});\n}});\n</script>\n</body>\n</html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_document() -> ManifestDocument {
        ManifestDocument {
            schema: PREVIEW_SCHEMA.to_owned(),
            id: "b893d568-327d-4b6e-b0b6-0b7a58e0c852".to_owned(),
            slug: "system-health".to_owned(),
            name: "System Health".to_owned(),
            version: "0.1.0".to_owned(),
            description: "Shows a bounded summary of host health.".to_owned(),
            license: DEFAULT_LICENSE.to_owned(),
            publisher: "Helix example".to_owned(),
            kind: StrandKind::Portable,
            capabilities: vec![CapabilityRequest {
                name: "helix:metrics.read".to_owned(),
                reason: "Read the selected node's bounded health summary.".to_owned(),
                optional: false,
                origins: Vec::new(),
            }],
            compatibility: Compatibility {
                helix: DEFAULT_HELIX_COMPATIBILITY.to_owned(),
                host_api: PREVIEW_HOST_API.to_owned(),
            },
            limits: ResourceLimits::default(),
            ui: None,
        }
    }

    fn write_document(path: &Path, document: &ManifestDocument) {
        fs::write(
            path,
            toml::to_string_pretty(document).expect("serialize fixture"),
        )
        .expect("write fixture");
    }

    #[test]
    fn valid_manifest_returns_typed_summary() {
        let temp = tempfile::tempdir().expect("temporary directory");
        write_document(&temp.path().join("strand.toml"), &valid_document());

        let manifest = validate_strand_project(temp.path()).expect("valid manifest");
        assert_eq!(manifest.slug, "system-health");
        assert_eq!(manifest.version, Version::new(0, 1, 0));
        assert_eq!(manifest.capabilities.len(), 1);
        assert_eq!(manifest.kind, StrandKind::Portable);
    }

    #[test]
    fn unknown_fields_fail_closed() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("strand.toml");
        let mut text = toml::to_string_pretty(&valid_document()).expect("serialize fixture");
        text.push_str("\nrun_as_root = true\n");
        fs::write(&path, text).expect("write fixture");

        assert!(matches!(
            validate_strand_project(&path),
            Err(StrandKitError::Parse { .. })
        ));
    }

    #[test]
    fn semantic_errors_are_reported_together() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("strand.toml");
        let mut document = valid_document();
        document.id = Uuid::nil().to_string();
        document.slug = "Bad--Slug".to_owned();
        document.version = "tomorrow".to_owned();
        document.publisher = "trusted \u{202e}resilbup".to_owned();
        document.compatibility.helix = "*".to_owned();
        document.capabilities.push(document.capabilities[0].clone());
        document.limits.memory_mib = 4_000;
        write_document(&path, &document);

        let error = validate_strand_project(&path).expect_err("invalid manifest");
        let StrandKitError::Validation(validation) = error else {
            panic!("expected semantic validation error");
        };
        assert!(validation.issues().len() >= 6);
        let message = validation.to_string();
        assert!(message.contains("nil UUID"));
        assert!(message.contains("bidirectional"));
        assert!(message.contains("duplicates capability"));
        assert!(message.contains("limits.memory_mib"));
    }

    #[test]
    fn compatibility_requires_both_bounds() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("strand.toml");
        let mut document = valid_document();
        document.compatibility.helix = ">=0.1.0".to_owned();
        write_document(&path, &document);

        let error = validate_strand_project(&path).expect_err("unbounded compatibility");
        assert!(error.to_string().contains("both a lower and upper"));

        document.compatibility.helix = "^0.1.0".to_owned();
        write_document(&path, &document);
        validate_strand_project(&path).expect("caret requirement is bounded");
    }

    #[test]
    fn oversized_manifest_is_rejected_before_parsing() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("strand.toml");
        let oversized = usize::try_from(MAX_MANIFEST_BYTES + 1).expect("limit fits usize");
        fs::write(&path, vec![b'x'; oversized]).expect("write oversized fixture");

        assert!(matches!(
            validate_strand_project(&path),
            Err(StrandKitError::ManifestTooLarge { .. })
        ));
    }

    #[test]
    fn invalid_utf8_is_rejected_without_lossy_parsing() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("strand.toml");
        fs::write(&path, [0xff, 0xfe, 0xfd]).expect("write invalid UTF-8 fixture");

        assert!(matches!(
            validate_strand_project(&path),
            Err(StrandKitError::InvalidUtf8 { .. })
        ));
    }

    #[test]
    fn scaffold_is_valid_and_never_overwrites() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let destination = temp.path().join("weather-card");
        let options = ScaffoldOptions::new(
            destination.clone(),
            "weather-card".to_owned(),
            StrandKind::UiOnly,
        );

        let created = scaffold_strand(&options).expect("create scaffold");
        assert_eq!(created.manifest.name, "Weather Card");
        assert_eq!(created.manifest.kind, StrandKind::UiOnly);
        assert!(created.manifest.installable);
        assert!(destination.join("README.md").is_file());
        assert!(destination.join("ui").join("index.html").is_file());
        assert!(destination.join("ui").join("helix.js").is_file());
        assert!(
            fs::read_to_string(destination.join("strand.toml"))
                .expect("read generated manifest")
                .contains("Deny by default")
        );

        let packed = pack_strand_project(&destination).expect("pack scaffold");
        let unpacked = unpack_strand_package(&packed).expect("unpack scaffold");
        assert_eq!(unpacked.manifest.slug, "weather-card");
        assert!(unpacked.asset("ui/index.html").is_some());

        assert!(matches!(
            scaffold_strand(&options),
            Err(StrandKitError::DestinationExists { .. })
        ));
    }

    #[test]
    fn invalid_scaffold_input_leaves_no_destination() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let destination = temp.path().join("bad-strand");
        let mut options = ScaffoldOptions::new(
            destination.clone(),
            "Not Valid".to_owned(),
            StrandKind::Portable,
        );
        options.publisher.clear();

        assert!(matches!(
            scaffold_strand(&options),
            Err(StrandKitError::Validation(_))
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn concurrent_scaffolds_publish_exactly_once() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let destination = temp.path().join("one-winner");
        let options = ScaffoldOptions::new(
            destination.clone(),
            "one-winner".to_owned(),
            StrandKind::Portable,
        );

        let threads = (0..8)
            .map(|_| {
                let options = options.clone();
                std::thread::spawn(move || scaffold_strand(&options))
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().expect("scaffold thread"))
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(StrandKitError::DestinationExists { .. })))
                .count(),
            7,
            "{results:#?}"
        );
        validate_strand_project(&destination).expect("winning scaffold is complete");
    }

    #[test]
    fn checked_in_reference_manifest_stays_valid() {
        let reference = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("strands")
            .join("system-health");
        let manifest = validate_strand_project(&reference).expect("reference Strand is valid");
        assert_eq!(manifest.slug, "system-health");
        assert!(manifest.installable);
        let packed = pack_strand_project(&reference).expect("pack reference Strand");
        let unpacked = unpack_strand_package(&packed).expect("unpack reference Strand");
        assert!(unpacked.asset("ui/index.html").is_some());
        assert!(unpacked.asset("ui/helix.js").is_some());
    }

    #[test]
    fn preview_packages_cannot_be_packed() {
        let temp = tempfile::tempdir().expect("temporary directory");
        write_document(&temp.path().join("strand.toml"), &valid_document());
        assert!(matches!(
            pack_strand_project(temp.path()),
            Err(StrandKitError::NotInstallable { .. })
        ));
    }

    #[test]
    fn https_origins_must_be_exact_hosts() {
        assert!(is_https_origin("https://api.open-meteo.com"));
        assert!(is_https_origin("https://example.com:8443"));
        assert!(!is_https_origin("http://example.com"));
        assert!(!is_https_origin("https://example.com/path"));
        assert!(!is_https_origin("https://user:pass@example.com"));
        assert!(!is_https_origin("https://localhost"));
    }

    #[test]
    fn installable_helix_range_matches_current_helix() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let destination = temp.path().join("compat-card");
        let created = scaffold_strand(&ScaffoldOptions::new(
            destination,
            "compat-card".to_owned(),
            StrandKind::UiOnly,
        ))
        .expect("scaffold");
        created
            .manifest
            .ensure_helix_compatible(env!("CARGO_PKG_VERSION"))
            .expect("current Helix is inside the default range");
        assert!(created.manifest.ensure_helix_compatible("9.9.9").is_err());
    }

    #[test]
    fn preview_helix_range_still_loads_on_1x() {
        let required = VersionReq::parse(">=0.1.0-alpha.1, <0.2.0").expect("preview range");
        assert!(helix_version_satisfies(
            &required,
            &Version::parse("1.0.0").expect("1.0.0")
        ));
        assert!(!helix_version_satisfies(
            &required,
            &Version::parse("2.0.0").expect("2.0.0")
        ));
        let future = VersionReq::parse(">=2.0.0, <3.0.0").expect("future range");
        assert!(!helix_version_satisfies(
            &future,
            &Version::parse("1.0.0").expect("1.0.0")
        ));
    }

    #[test]
    fn checked_in_https_probe_packs() {
        let reference = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("strands")
            .join("https-probe");
        let manifest = validate_strand_project(&reference).expect("https-probe is valid");
        assert!(manifest.installable);
        assert!(
            manifest
                .capabilities
                .iter()
                .any(|capability| capability.name == "helix:net.https")
        );
        pack_strand_project(&reference).expect("pack https-probe");
    }
}
