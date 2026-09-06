//! Typed runtime configuration with conservative production defaults.

use serde::Deserialize;
use std::{
    env, fs,
    io::{self, Read as _},
    net::SocketAddr,
    path::{Path, PathBuf},
};
use thiserror::Error;

const DEFAULT_LISTEN: &str = "127.0.0.1:8080";
const MAX_CONFIG_FILE_BYTES: u64 = 64 * 1024;

/// Fully resolved daemon configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    pub listen: SocketAddr,
    /// Private state root. It must remain disjoint from `web_root` because the
    /// latter is exposed through the static-file service.
    pub data_dir: PathBuf,
    /// Root of compiled public assets. Existing symbolic links are resolved
    /// during validation so this root cannot alias or contain `data_dir`.
    pub web_root: PathBuf,
}

/// Command-line values applied after file and environment configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigOverrides {
    pub listen: Option<SocketAddr>,
    pub data_dir: Option<PathBuf>,
    pub web_root: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read Helix configuration at {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("could not parse Helix configuration at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error(
        "Helix configuration at {path} exceeds the {maximum}-byte limit (observed at least {observed} bytes)"
    )]
    TooLarge {
        path: PathBuf,
        observed: u64,
        maximum: u64,
    },
    #[error("invalid {field}: {message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    server: Option<ServerConfig>,
    paths: Option<PathConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerConfig {
    listen: Option<SocketAddr>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathConfig {
    data_dir: Option<PathBuf>,
    web_root: Option<PathBuf>,
}

impl RuntimeConfig {
    /// Load defaults, then an optional TOML file, environment variables, and
    /// finally explicit command-line overrides.
    pub fn load(
        explicit_config: Option<&Path>,
        overrides: ConfigOverrides,
    ) -> Result<Self, ConfigError> {
        let mut config = Self::platform_defaults();
        let selected_path = explicit_config
            .map(Path::to_path_buf)
            .or_else(|| env::var_os("HELIX_CONFIG").map(PathBuf::from));

        if let Some(path) = selected_path {
            config.apply_file(&path, true)?;
        } else {
            let default_path = default_config_path();
            config.apply_file(&default_path, false)?;
        }

        config.apply_environment()?;
        config.apply_overrides(overrides);
        config.validate()?;
        Ok(config)
    }

    #[must_use]
    pub fn platform_defaults() -> Self {
        Self {
            listen: DEFAULT_LISTEN
                .parse()
                .expect("the compiled default listen address is valid"),
            data_dir: default_data_dir(),
            web_root: default_web_root(),
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_absolute("paths.data_dir", &self.data_dir)?;
        validate_absolute("paths.web_root", &self.web_root)?;

        if !self.listen.ip().is_loopback() {
            return Err(ConfigError::Invalid {
                field: "server.listen",
                message: "remote binds are disabled until TLS, trusted-proxy handling, and remote deployment security are implemented and validated".to_owned(),
            });
        }

        validate_disjoint_roots(&self.data_dir, &self.web_root)?;
        Ok(())
    }

    fn apply_file(&mut self, path: &Path, required: bool) -> Result<(), ConfigError> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(source) if !required && source.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let metadata = file.metadata().map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        if metadata.len() > MAX_CONFIG_FILE_BYTES {
            return Err(ConfigError::TooLarge {
                path: path.to_path_buf(),
                observed: metadata.len(),
                maximum: MAX_CONFIG_FILE_BYTES,
            });
        }
        let mut text = String::new();
        file.take(MAX_CONFIG_FILE_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|source| ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        let observed = u64::try_from(text.len()).unwrap_or(u64::MAX);
        if observed > MAX_CONFIG_FILE_BYTES {
            return Err(ConfigError::TooLarge {
                path: path.to_path_buf(),
                observed,
                maximum: MAX_CONFIG_FILE_BYTES,
            });
        }
        let parsed: FileConfig = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;

        if let Some(server) = parsed.server
            && let Some(listen) = server.listen
        {
            self.listen = listen;
        }
        if let Some(paths) = parsed.paths {
            if let Some(data_dir) = paths.data_dir {
                self.data_dir = data_dir;
            }
            if let Some(web_root) = paths.web_root {
                self.web_root = web_root;
            }
        }
        Ok(())
    }

    fn apply_environment(&mut self) -> Result<(), ConfigError> {
        if let Ok(value) = env::var("HELIX_LISTEN") {
            self.listen = value.parse().map_err(|error| ConfigError::Invalid {
                field: "HELIX_LISTEN",
                message: format!("{error}"),
            })?;
        }
        if let Some(value) = env::var_os("HELIX_DATA_DIR") {
            self.data_dir = PathBuf::from(value);
        }
        if let Some(value) = env::var_os("HELIX_WEB_ROOT") {
            self.web_root = PathBuf::from(value);
        }
        Ok(())
    }

    fn apply_overrides(&mut self, overrides: ConfigOverrides) {
        if let Some(listen) = overrides.listen {
            self.listen = listen;
        }
        if let Some(data_dir) = overrides.data_dir {
            self.data_dir = data_dir;
        }
        if let Some(web_root) = overrides.web_root {
            self.web_root = web_root;
        }
    }
}

fn validate_absolute(field: &'static str, path: &Path) -> Result<(), ConfigError> {
    if !path.is_absolute() {
        return Err(ConfigError::Invalid {
            field,
            message: format!("{} must be an absolute path", path.display()),
        });
    }
    Ok(())
}

fn validate_disjoint_roots(data_dir: &Path, web_root: &Path) -> Result<(), ConfigError> {
    let resolved_data = resolve_for_containment("paths.data_dir", data_dir)?;
    let resolved_web = resolve_for_containment("paths.web_root", web_root)?;

    if path_is_within(&resolved_data, &resolved_web)
        || path_is_within(&resolved_web, &resolved_data)
    {
        return Err(ConfigError::Invalid {
            field: "paths",
            message: "data_dir and web_root must be disjoint after resolving existing filesystem components; neither root may contain the other".to_owned(),
        });
    }

    Ok(())
}

/// Resolve the longest existing ancestor and append any not-yet-created
/// suffix. This detects aliases through existing symbolic links without
/// requiring package-created directories to exist before configuration can be
/// validated.
fn resolve_for_containment(field: &'static str, path: &Path) -> Result<PathBuf, ConfigError> {
    for ancestor in path.ancestors() {
        match fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                let suffix = path
                    .strip_prefix(ancestor)
                    .map_err(|error| ConfigError::Invalid {
                        field,
                        message: format!(
                            "could not resolve trusted root {}: {error}",
                            path.display()
                        ),
                    })?;

                for component in suffix.components() {
                    match component {
                        std::path::Component::Normal(part) => resolved.push(part),
                        std::path::Component::CurDir => {}
                        std::path::Component::ParentDir => {
                            return Err(ConfigError::Invalid {
                                field,
                                message: format!(
                                    "trusted root {} contains unresolved parent traversal",
                                    path.display()
                                ),
                            });
                        }
                        std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                            return Err(ConfigError::Invalid {
                                field,
                                message: format!(
                                    "trusted root {} has an invalid unresolved suffix",
                                    path.display()
                                ),
                            });
                        }
                    }
                }

                return Ok(resolved);
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                match fs::symlink_metadata(ancestor) {
                    Ok(_) => {
                        return Err(ConfigError::Invalid {
                            field,
                            message: format!(
                                "trusted root component {} exists but could not be resolved: {source}",
                                ancestor.display()
                            ),
                        });
                    }
                    Err(metadata_error) if metadata_error.kind() == io::ErrorKind::NotFound => {}
                    Err(metadata_error) => {
                        return Err(ConfigError::Invalid {
                            field,
                            message: format!(
                                "could not inspect trusted root component {}: {metadata_error}",
                                ancestor.display()
                            ),
                        });
                    }
                }
            }
            Err(source) => {
                return Err(ConfigError::Invalid {
                    field,
                    message: format!(
                        "could not resolve trusted root component {}: {source}",
                        ancestor.display()
                    ),
                });
            }
        }
    }

    Err(ConfigError::Invalid {
        field,
        message: format!(
            "trusted root {} has no resolvable existing ancestor",
            path.display()
        ),
    })
}

#[cfg(not(windows))]
fn path_is_within(candidate: &Path, root: &Path) -> bool {
    candidate.starts_with(root)
}

#[cfg(windows)]
fn path_is_within(candidate: &Path, root: &Path) -> bool {
    let mut candidate_components = candidate.components();
    root.components().all(|root_component| {
        candidate_components
            .next()
            .is_some_and(|candidate_component| {
                candidate_component
                    .as_os_str()
                    .to_string_lossy()
                    .to_lowercase()
                    == root_component.as_os_str().to_string_lossy().to_lowercase()
            })
    })
}

#[cfg(target_os = "windows")]
fn default_config_path() -> PathBuf {
    PathBuf::from(r"C:\ProgramData\Helix\helix.toml")
}

#[cfg(not(target_os = "windows"))]
fn default_config_path() -> PathBuf {
    PathBuf::from("/etc/helix/helix.toml")
}

#[cfg(target_os = "windows")]
fn default_data_dir() -> PathBuf {
    PathBuf::from(r"C:\ProgramData\Helix\data")
}

#[cfg(not(target_os = "windows"))]
fn default_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/helix")
}

#[cfg(target_os = "windows")]
fn default_web_root() -> PathBuf {
    PathBuf::from(r"C:\Program Files\Helix\web")
}

#[cfg(not(target_os = "windows"))]
fn default_web_root() -> PathBuf {
    PathBuf::from("/usr/share/helix/web")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_roots(data_dir: PathBuf, web_root: PathBuf) -> RuntimeConfig {
        RuntimeConfig {
            listen: DEFAULT_LISTEN.parse().expect("test listen address"),
            data_dir,
            web_root,
        }
    }

    fn assert_path_configuration_is_rejected(config: &RuntimeConfig) {
        assert!(matches!(
            config.validate(),
            Err(ConfigError::Invalid { .. })
        ));
    }

    fn create_test_directory_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).expect("create test directory symlink");
            true
        }

        #[cfg(windows)]
        {
            match std::os::windows::fs::symlink_dir(target, link) {
                Ok(()) => true,
                Err(error) if error.raw_os_error() == Some(1314) => false,
                Err(error) => panic!("create test directory symlink: {error}"),
            }
        }
    }

    #[test]
    fn explicit_file_overrides_default_listen_address() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("helix.toml");
        fs::write(&path, "[server]\nlisten = '127.0.0.1:9010'\n").expect("write test config");

        let config = RuntimeConfig::load(Some(&path), ConfigOverrides::default())
            .expect("load valid config");
        assert_eq!(config.listen.port(), 9010);
    }

    #[test]
    fn unknown_keys_fail_closed() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("helix.toml");
        fs::write(&path, "mystery = true\n").expect("write test config");

        let error = RuntimeConfig::load(Some(&path), ConfigOverrides::default())
            .expect_err("unknown configuration must fail");
        assert!(matches!(error, ConfigError::Parse { .. }));
    }

    #[test]
    fn configuration_file_size_limit_accepts_the_boundary_and_rejects_one_more_byte() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("helix.toml");
        let prefix = "[server]\nlisten = '127.0.0.1:9010'\n#";
        let maximum = usize::try_from(MAX_CONFIG_FILE_BYTES).expect("config limit fits usize");
        let mut boundary = prefix.to_owned();
        boundary.push_str(&"x".repeat(maximum - boundary.len()));
        assert_eq!(boundary.len(), maximum);
        fs::write(&path, &boundary).expect("write boundary config");

        let config = RuntimeConfig::load(Some(&path), ConfigOverrides::default())
            .expect("boundary-sized configuration is accepted");
        assert_eq!(config.listen.port(), 9010);

        boundary.push('x');
        fs::write(&path, boundary).expect("write oversized config");
        assert!(matches!(
            RuntimeConfig::load(Some(&path), ConfigOverrides::default()),
            Err(ConfigError::TooLarge {
                observed,
                maximum: MAX_CONFIG_FILE_BYTES,
                ..
            }) if observed == MAX_CONFIG_FILE_BYTES + 1
        ));
    }

    #[test]
    fn relative_trusted_roots_are_rejected() {
        let mut config = RuntimeConfig::platform_defaults();
        config.data_dir = PathBuf::from("relative-data");

        assert!(matches!(
            config.validate(),
            Err(ConfigError::Invalid { .. })
        ));
    }

    #[test]
    fn identical_roots_are_rejected_after_canonicalization() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let shared = temp.path().join("shared");
        fs::create_dir(&shared).expect("create shared directory");

        assert_path_configuration_is_rejected(&config_with_roots(shared.clone(), shared));
    }

    #[test]
    fn web_root_cannot_contain_the_data_root() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let web_root = temp.path().join("public");
        let data_dir = web_root.join("private-state");

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[test]
    fn data_root_cannot_contain_the_web_root() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let data_dir = temp.path().join("private-state");
        let web_root = data_dir.join("public");

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[test]
    fn similarly_named_sibling_roots_are_allowed() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = config_with_roots(temp.path().join("state"), temp.path().join("state-static"));

        config.validate().expect("sibling roots are disjoint");
    }

    #[test]
    fn unresolved_descendants_are_compared_from_their_existing_ancestor() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let data_dir = temp.path().join("not-created").join("state");
        let web_root = data_dir.join("compiled-assets");

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[cfg(windows)]
    #[test]
    fn unresolved_windows_roots_are_compared_case_insensitively() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let data_dir = temp.path().join("Private-State");
        let web_root = temp.path().join("private-state").join("compiled-assets");

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[test]
    fn existing_parent_traversal_is_canonicalized_before_comparison() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let real = temp.path().join("real");
        let decoy = real.join("decoy");
        let data_dir = real.join("state");
        fs::create_dir_all(&decoy).expect("create decoy directory");
        fs::create_dir(&data_dir).expect("create state directory");
        let web_root = decoy.join("..").join("state").join("assets");

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[test]
    fn unresolved_parent_traversal_cannot_hide_overlap() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let data_dir = temp.path().join("assets");
        fs::create_dir(&data_dir).expect("create data directory");
        let web_root = temp
            .path()
            .join("not-created")
            .join("..")
            .join("assets")
            .join("compiled");

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[test]
    fn symlinked_web_root_cannot_alias_the_data_root() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let data_dir = temp.path().join("state");
        let web_root = temp.path().join("public-link");
        fs::create_dir(&data_dir).expect("create state directory");
        if !create_test_directory_symlink(&data_dir, &web_root) {
            eprintln!("skipping symlink assertion: Windows symlink privilege is unavailable");
            return;
        }

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[test]
    fn symlinked_existing_ancestor_is_resolved_for_missing_descendants() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let real = temp.path().join("real");
        let data_dir = real.join("state");
        let alias = temp.path().join("alias");
        fs::create_dir_all(&data_dir).expect("create state directory");
        if !create_test_directory_symlink(&real, &alias) {
            eprintln!("skipping symlink assertion: Windows symlink privilege is unavailable");
            return;
        }
        let web_root = alias.join("state").join("assets-not-created");

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[test]
    fn dangling_symlink_fails_closed() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let data_dir = temp.path().join("state");
        let web_root = temp.path().join("dangling-public");
        let missing_target = temp.path().join("missing-target");
        if !create_test_directory_symlink(&missing_target, &web_root) {
            eprintln!("skipping symlink assertion: Windows symlink privilege is unavailable");
            return;
        }

        assert_path_configuration_is_rejected(&config_with_roots(data_dir, web_root));
    }

    #[test]
    fn explicit_missing_file_is_an_error() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("missing.toml");

        assert!(matches!(
            RuntimeConfig::load(Some(&path), ConfigOverrides::default()),
            Err(ConfigError::Read { .. })
        ));
    }

    #[test]
    fn remote_bind_is_rejected_until_the_remote_security_boundary_exists() {
        let mut config = RuntimeConfig::platform_defaults();
        config.listen = "0.0.0.0:8080".parse().expect("test address");

        assert!(matches!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "server.listen",
                ..
            })
        ));
    }
}
