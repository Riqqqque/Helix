use std::{
    fs, io,
    path::{Path, PathBuf},
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StaticRootError {
    #[error("could not inspect static asset path {path}: {source}")]
    Inspect { path: PathBuf, source: io::Error },
    #[error("static asset path must not be a symbolic link or reparse point: {0}")]
    Link(PathBuf),
    #[error("static asset path is neither a regular file nor a directory: {0}")]
    UnsupportedEntry(PathBuf),
    #[cfg(unix)]
    #[error("static asset path is owned by uid {found}, expected uid {expected}: {path}")]
    UnsafeOwner {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[cfg(unix)]
    #[error("static asset path has group/other write bits set ({mode:#o}): {path}")]
    UnsafeMode { path: PathBuf, mode: u32 },
}

/// Validate the complete packaged asset tree before it is handed to `ServeDir`.
///
/// Production validation also checks every existing ancestor, so an
/// unprivileged user cannot replace a trusted child through a writable parent.
/// This remains a startup preflight rather than a descriptor-pinned walk: root
/// can still change a checked path later, so the packaged tree and its ancestors
/// must remain under root control for the daemon lifetime.
pub fn validate_static_root(root: &Path) -> Result<(), StaticRootError> {
    #[cfg(unix)]
    let expected_uid = if cfg!(test) {
        use std::os::unix::fs::MetadataExt as _;
        fs::symlink_metadata(root)
            .map_err(|source| StaticRootError::Inspect {
                path: root.to_path_buf(),
                source,
            })?
            .uid()
    } else {
        0
    };

    #[cfg(unix)]
    {
        if !cfg!(test) {
            validate_unix_ancestors(root, expected_uid, None)?;
        }
        validate_entry(root, expected_uid)
    }

    #[cfg(not(unix))]
    {
        if !cfg!(test) {
            validate_non_unix_ancestors(root, None)?;
        }
        validate_entry(root)
    }
}

#[cfg(unix)]
fn validate_entry(path: &Path, expected_uid: u32) -> Result<(), StaticRootError> {
    let metadata = inspect_unix_entry(path, expected_uid)?;
    validate_file_type_and_children(path, &metadata, |child| validate_entry(child, expected_uid))
}

#[cfg(unix)]
fn inspect_unix_entry(path: &Path, expected_uid: u32) -> Result<fs::Metadata, StaticRootError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = fs::symlink_metadata(path).map_err(|source| StaticRootError::Inspect {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(StaticRootError::Link(path.to_path_buf()));
    }
    if metadata.uid() != expected_uid {
        return Err(StaticRootError::UnsafeOwner {
            path: path.to_path_buf(),
            found: metadata.uid(),
            expected: expected_uid,
        });
    }
    let mode = metadata.mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(StaticRootError::UnsafeMode {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(metadata)
}

#[cfg(unix)]
fn validate_unix_ancestors(
    root: &Path,
    expected_uid: u32,
    stop_after: Option<&Path>,
) -> Result<(), StaticRootError> {
    for ancestor in root.ancestors().skip(1) {
        let metadata = inspect_unix_entry(ancestor, expected_uid)?;
        if !metadata.is_dir() {
            return Err(StaticRootError::UnsupportedEntry(ancestor.to_path_buf()));
        }
        if stop_after.is_some_and(|stop| stop == ancestor) {
            break;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_entry(path: &Path) -> Result<(), StaticRootError> {
    let metadata = inspect_non_unix_entry(path)?;
    validate_file_type_and_children(path, &metadata, validate_entry)
}

#[cfg(not(unix))]
fn inspect_non_unix_entry(path: &Path) -> Result<fs::Metadata, StaticRootError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| StaticRootError::Inspect {
        path: path.to_path_buf(),
        source,
    })?;

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(StaticRootError::Link(path.to_path_buf()));
        }
    }

    if metadata.file_type().is_symlink() {
        return Err(StaticRootError::Link(path.to_path_buf()));
    }
    Ok(metadata)
}

#[cfg(not(unix))]
fn validate_non_unix_ancestors(
    root: &Path,
    stop_after: Option<&Path>,
) -> Result<(), StaticRootError> {
    for ancestor in root.ancestors().skip(1) {
        let metadata = inspect_non_unix_entry(ancestor)?;
        if !metadata.is_dir() {
            return Err(StaticRootError::UnsupportedEntry(ancestor.to_path_buf()));
        }
        if stop_after.is_some_and(|stop| stop == ancestor) {
            break;
        }
    }
    Ok(())
}

fn validate_file_type_and_children(
    path: &Path,
    metadata: &fs::Metadata,
    mut validate_child: impl FnMut(&Path) -> Result<(), StaticRootError>,
) -> Result<(), StaticRootError> {
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(StaticRootError::UnsupportedEntry(path.to_path_buf()));
    }

    let entries = fs::read_dir(path).map_err(|source| StaticRootError::Inspect {
        path: path.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| StaticRootError::Inspect {
            path: path.to_path_buf(),
            source,
        })?;
        validate_child(&entry.path())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_static_tree_is_accepted() {
        let root = tempfile::tempdir().expect("temporary directory");
        fs::create_dir(root.path().join("assets")).expect("create assets");
        fs::write(root.path().join("index.html"), "<!doctype html>").expect("write index");
        fs::write(root.path().join("assets/app.js"), "void 0").expect("write asset");

        validate_static_root(root.path()).expect("regular tree");
    }

    #[cfg(unix)]
    #[test]
    fn internal_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory");
        fs::write(root.path().join("index.html"), "<!doctype html>").expect("write index");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        symlink(outside.path(), root.path().join("leak.txt")).expect("create symlink");

        assert!(matches!(
            validate_static_root(root.path()),
            Err(StaticRootError::Link(path)) if path.ends_with("leak.txt")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn group_writable_asset_is_rejected() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().expect("temporary directory");
        let asset = root.path().join("index.html");
        fs::write(&asset, "<!doctype html>").expect("write index");
        fs::set_permissions(&asset, fs::Permissions::from_mode(0o664)).expect("set mode");

        assert!(matches!(
            validate_static_root(root.path()),
            Err(StaticRootError::UnsafeMode { path, .. }) if path == asset
        ));
    }

    #[cfg(unix)]
    #[test]
    fn writable_ancestor_is_rejected_by_production_policy() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let boundary = tempfile::tempdir().expect("temporary directory");
        let writable_parent = boundary.path().join("replaceable");
        let root = writable_parent.join("web");
        fs::create_dir_all(&root).expect("create static root");
        fs::write(root.join("index.html"), "<!doctype html>").expect("write index");
        fs::set_permissions(&writable_parent, fs::Permissions::from_mode(0o775))
            .expect("make ancestor writable");
        let expected_uid = fs::symlink_metadata(&root).expect("root metadata").uid();

        assert!(matches!(
            validate_unix_ancestors(&root, expected_uid, Some(boundary.path())),
            Err(StaticRootError::UnsafeMode { path, .. }) if path == writable_parent
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_ancestor_is_rejected_by_production_policy() {
        use std::os::unix::fs::{MetadataExt as _, symlink};

        let boundary = tempfile::tempdir().expect("temporary directory");
        let real_parent = boundary.path().join("real");
        let real_root = real_parent.join("web");
        fs::create_dir_all(&real_root).expect("create static root");
        fs::write(real_root.join("index.html"), "<!doctype html>").expect("write index");
        let alias = boundary.path().join("replaceable");
        symlink(&real_parent, &alias).expect("create ancestor symlink");
        let root_via_alias = alias.join("web");
        let expected_uid = fs::symlink_metadata(&real_root)
            .expect("root metadata")
            .uid();

        assert!(matches!(
            validate_unix_ancestors(&root_via_alias, expected_uid, Some(boundary.path())),
            Err(StaticRootError::Link(path)) if path == alias
        ));
    }
}
