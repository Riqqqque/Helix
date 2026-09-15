use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::Read as _,
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Component, Path},
};

const MAX_ENTRIES: usize = 2_048;
const MAX_CHANGED_FILES: usize = 32;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_SCAN_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
struct Baseline {
    boot: String,
    files: BTreeMap<String, String>,
    limited: bool,
}

// The private baseline contains hashes and relative paths, never configuration values.
pub(super) fn inspect(
    root: &Path,
    world: &str,
    state: &Path,
    boot: Option<&str>,
    online: bool,
) -> Value {
    if !online {
        return response("stopped", Vec::new(), false);
    }
    let Some(boot) = boot.filter(|boot| !boot.is_empty() && boot.len() <= 64) else {
        return response("unknown", Vec::new(), true);
    };
    observe(root, world, state, boot).unwrap_or_else(|_| response("unknown", Vec::new(), true))
}

fn observe(root: &Path, world: &str, state: &Path, boot: &str) -> Result<Value, String> {
    let directory = state
        .parent()
        .ok_or("missing configuration state directory")?;
    fs::create_dir_all(directory).map_err(|_| "configuration state unavailable")?;
    if fs::symlink_metadata(directory)
        .map_err(|_| "configuration state unavailable")?
        .file_type()
        .is_symlink()
    {
        return Err("configuration state is a symlink".into());
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|_| "configuration state permissions unavailable")?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(state.with_extension("lock"))
        .map_err(|_| "configuration check busy")?;
    lock.try_lock_exclusive()
        .map_err(|_| "configuration check busy")?;
    let scan = scan(root, world);
    let previous = if state
        .try_exists()
        .map_err(|_| "configuration baseline unavailable")?
    {
        let content = super::read_small_regular_file(state, 1024 * 1024, "configuration baseline")?;
        Some(
            serde_json::from_str::<Baseline>(&content)
                .map_err(|_| "invalid configuration baseline")?,
        )
    } else {
        None
    };
    let Some(previous) = previous.filter(|previous| previous.boot == boot) else {
        let baseline = Baseline {
            boot: boot.to_owned(),
            files: scan.files,
            limited: scan.limited,
        };
        super::write_private_text(
            state,
            &serde_json::to_string(&baseline)
                .map_err(|_| "could not encode configuration baseline")?,
        )?;
        return Ok(response(
            if scan.limited {
                "unknown"
            } else {
                "no_changes"
            },
            Vec::new(),
            scan.limited,
        ));
    };
    let mut changed = scan
        .files
        .iter()
        .filter(|(path, hash)| match previous.files.get(*path) {
            Some(old) => old != *hash,
            None => !previous.limited,
        })
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    // Incomplete scans cannot distinguish missing files from files they did not reach.
    if !scan.limited {
        changed.extend(
            previous
                .files
                .keys()
                .filter(|path| !scan.files.contains_key(*path))
                .cloned(),
        );
    }
    changed.sort();
    let limited = scan.limited || previous.limited || changed.len() > MAX_CHANGED_FILES;
    changed.truncate(MAX_CHANGED_FILES);
    Ok(response(
        if changed.is_empty() {
            if limited { "unknown" } else { "no_changes" }
        } else {
            "changed"
        },
        changed,
        limited,
    ))
}

fn response(state: &str, files: Vec<String>, limited: bool) -> Value {
    json!({"state": state, "files": files, "limited": limited})
}

fn scan<'a>(root: &'a Path, world: &str) -> Scan<'a> {
    let mut scan = Scan {
        root,
        visited: 0,
        bytes: 0,
        files: BTreeMap::new(),
        limited: false,
    };
    for name in ["server.properties", "paper.yml", "spigot.yml", "bukkit.yml"] {
        scan.visit(&root.join(name), 0);
    }
    let world = Path::new(world);
    if !world.as_os_str().is_empty()
        && world
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        let mut path = root.to_path_buf();
        let mut safe = true;
        for part in world.components() {
            path.push(part);
            if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                safe = false;
                break;
            }
        }
        if safe {
            scan.visit(&path.join("serverconfig"), 0);
        } else {
            scan.limited = true;
        }
    } else {
        scan.limited = true;
    }
    for name in ["config", "plugins"] {
        scan.visit(&root.join(name), 0);
    }
    scan
}

struct Scan<'a> {
    root: &'a Path,
    visited: usize,
    bytes: u64,
    files: BTreeMap<String, String>,
    limited: bool,
}

impl Scan<'_> {
    fn visit(&mut self, path: &Path, depth: usize) {
        if self.visited >= MAX_ENTRIES || self.bytes >= MAX_SCAN_BYTES {
            self.limited = true;
            return;
        }
        self.visited += 1;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {
                self.limited = true;
                return;
            }
        };
        if metadata.file_type().is_symlink() {
            self.limited = true;
            return;
        }
        if metadata.is_dir() {
            if depth >= 4 {
                self.limited = true;
                return;
            }
            let entries = match fs::read_dir(path) {
                Ok(entries) => entries,
                Err(_) => {
                    self.limited = true;
                    return;
                }
            };
            for entry in entries {
                if self.visited >= MAX_ENTRIES || self.bytes >= MAX_SCAN_BYTES {
                    self.limited = true;
                    break;
                }
                match entry {
                    Ok(entry) => self.visit(&entry.path(), depth + 1),
                    Err(_) => self.limited = true,
                }
            }
        } else if metadata.is_file() && is_config(path) {
            let Ok(relative) = path.strip_prefix(self.root) else {
                self.limited = true;
                return;
            };
            if metadata.len() > MAX_FILE_BYTES || self.bytes + metadata.len() > MAX_SCAN_BYTES {
                self.limited = true;
                return;
            }
            let result = (|| {
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(
                        (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
                    )
                    .open(path)?;
                if !file.metadata()?.is_file() {
                    return Err(std::io::Error::other("not a regular config"));
                }
                let mut bytes = Vec::new();
                file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
                Ok(bytes)
            })();
            match result {
                Ok(bytes) if bytes.len() as u64 <= MAX_FILE_BYTES => {
                    self.bytes += bytes.len() as u64;
                    let mut hash = String::with_capacity(64);
                    for byte in Sha256::digest(&bytes) {
                        let _ = write!(hash, "{byte:02x}");
                    }
                    self.files
                        .insert(relative.to_string_lossy().replace('\\', "/"), hash);
                }
                _ => self.limited = true,
            }
        }
    }
}

fn is_config(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if path.components().any(|part| part.as_os_str() == "plugins")
        && !name.contains("config")
        && !name.contains("settings")
    {
        return false;
    }
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some(
            "properties"
                | "toml"
                | "snbt"
                | "yaml"
                | "yml"
                | "json"
                | "json5"
                | "conf"
                | "cfg"
                | "ini"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_changes_across_refreshes_and_clears_on_new_boot() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let baseline = state.path().join("baseline.json");
        fs::create_dir_all(root.path().join("world/serverconfig")).unwrap();
        let file = root.path().join("world/serverconfig/ftbchunks-world.snbt");
        fs::write(&file, "default").unwrap();
        assert_eq!(
            inspect(root.path(), "world", &baseline, Some("boot-1"), true)["state"],
            "no_changes"
        );
        fs::write(&file, "always secret").unwrap();
        let changed = inspect(root.path(), "world", &baseline, Some("boot-1"), true);
        assert_eq!(changed["state"], "changed");
        assert_eq!(
            changed["files"],
            json!(["world/serverconfig/ftbchunks-world.snbt"])
        );
        assert!(!changed.to_string().contains("secret"));
        assert!(!fs::read_to_string(&baseline).unwrap().contains("default"));
        assert_eq!(
            changed,
            inspect(root.path(), "world", &baseline, Some("boot-1"), true)
        );
        assert_eq!(
            inspect(root.path(), "world", &baseline, Some("boot-2"), true)["state"],
            "no_changes"
        );
    }

    #[test]
    fn unchanged_writes_and_reverted_edits_do_not_warn() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let baseline = state.path().join("baseline.json");
        let file = root.path().join("server.properties");
        fs::write(&file, "a=1").unwrap();
        inspect(root.path(), "world", &baseline, Some("boot"), true);
        fs::write(&file, "a=1").unwrap();
        assert_eq!(
            inspect(root.path(), "world", &baseline, Some("boot"), true)["state"],
            "no_changes"
        );
        fs::remove_file(&file).unwrap();
        assert_eq!(
            inspect(root.path(), "world", &baseline, Some("boot"), true)["state"],
            "changed"
        );
        fs::write(&file, "a=1").unwrap();
        assert_eq!(
            inspect(root.path(), "world", &baseline, Some("boot"), true)["state"],
            "no_changes"
        );
    }

    #[test]
    fn bounds_scans_and_skips_symlinks_and_traversal() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.toml"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("config")).unwrap();
        let result = scan(root.path(), "../other");
        assert!(result.files.is_empty());
        assert!(result.limited);
        fs::remove_file(root.path().join("config")).unwrap();
        fs::create_dir(root.path().join("config")).unwrap();
        fs::File::create(root.path().join("config/large.toml"))
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(scan(root.path(), "world").limited);
    }

    #[test]
    fn missing_runtime_and_corrupt_baselines_are_unknown() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let baseline = state.path().join("baseline.json");
        assert_eq!(
            inspect(root.path(), "world", &baseline, None, true)["state"],
            "unknown"
        );
        assert_eq!(
            inspect(root.path(), "world", &baseline, None, false)["state"],
            "stopped"
        );
        fs::write(&baseline, "broken").unwrap();
        assert_eq!(
            inspect(root.path(), "world", &baseline, Some("boot"), true)["state"],
            "unknown"
        );
    }
}
